use std::{path::PathBuf, thread, time::Duration};

use clap::Parser;
use fstrace::{Event, diff, snapshot};

/// Mirror a live tree elsewhere: poll the source, copy what changed.
#[derive(Parser, Debug)]
#[command(name = "fsmirror", about = "Mirror a live tree, source-only.")]
struct Args {
    /// Directory to watch.
    source: PathBuf,
    /// Directory to mirror into.
    dest: PathBuf,
    /// Exclude a file or folder (repeatable, `*`/`?` patterns allowed).
    #[arg(long = "exclude")]
    excludes: Vec<String>,
    /// Milliseconds between polls.
    #[arg(long, default_value_t = 1000)]
    interval: u64,
    /// Mirror deletions too. Without this, the dest only ever gains files.
    #[arg(long)]
    delete: bool,
}

fn main() {
    let args = Args::parse();

    if !args.source.is_dir() {
        eprintln!(
            "error: source is not a directory: {}",
            args.source.display()
        );
        std::process::exit(1);
    }
    if let Err(e) = std::fs::create_dir_all(&args.dest) {
        eprintln!("error: cannot create dest {}: {e}", args.dest.display());
        std::process::exit(1);
    }

    let mut prev = snapshot(&args.source, &args.excludes);
    loop {
        thread::sleep(Duration::from_millis(args.interval));
        let curr = snapshot(&args.source, &args.excludes);
        for event in diff(&prev, &curr) {
            apply(&args, &event);
        }
        prev = curr;
    }
}

// Copy one event into the dest tree. Paths are stored absolute in the
// snapshot, so strip the source prefix to get the dest-relative path.
fn apply(args: &Args, event: &Event) {
    let path = match event {
        Event::Created { path } | Event::Modified { path } | Event::Deleted { path } => path,
    };
    let src = PathBuf::from(path);
    let rel = match src.strip_prefix(&args.source) {
        Ok(r) => r,
        Err(_) => return,
    };
    let dst = args.dest.join(rel);

    match event {
        Event::Created { .. } | Event::Modified { .. } => {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::copy(&src, &dst) {
                eprintln!("error: copy {}: {e}", src.display());
            }
        }
        Event::Deleted { .. } => {
            // Append-only unless --delete: the dest never loses files
            // by default, so a bad poll cannot wipe the backup.
            if args.delete {
                let _ = std::fs::remove_file(&dst);
            }
        }
    }
}
