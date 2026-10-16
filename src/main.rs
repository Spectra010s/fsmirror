use std::{path::PathBuf, thread, time::Duration};

use clap::Parser;
use fstrace::{Event, diff, snapshot, snapshot_hashed};

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
    /// Skip content fingerprinting. Hashed snapshots are the default
    /// because a mirror that silently misses rewrites is worse than none;
    /// skip only for giant trees where re-reading every file every poll
    /// costs more than the misses it prevents.
    #[arg(long)]
    no_hash: bool,
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

    // Hashed snapshots by default: plain metadata misses same-stamp
    // same-size rewrites, and a mirror must not silently skip writes.
    let capture = if args.no_hash {
        snapshot
    } else {
        snapshot_hashed
    };
    // Startup sync: the watch loop only sees changes *after* it starts,
    // so populate the dest first or pre-existing files never arrive.
    // Copies missing or differing files; never deletes (even with
    // --delete: wiping a non-empty dest on first run would be destructive,
    // deletions flow only through the live loop).
    initial_sync(&args, capture);
    let mut prev = capture(&args.source, &args.excludes);
    loop {
        thread::sleep(Duration::from_millis(args.interval));
        let curr = capture(&args.source, &args.excludes);
        for event in diff(&prev, &curr) {
            apply(&args, &event);
        }
        prev = curr;
    }
}

// Bring dest up to date before the watch loop starts: copy every source
// file missing in dest or differing from it. Same-State comparison as
// the live loop, so no second opinion logic to drift.
fn initial_sync(
    args: &Args,
    capture: fn(
        &std::path::Path,
        &[String],
    ) -> std::collections::HashMap<String, fstrace::FileState>,
) {
    let src_map = capture(&args.source, &args.excludes);
    let dst_map = capture(&args.dest, &args.excludes);
    let mut copied = 0u32;
    for (path, state) in &src_map {
        let rel = match PathBuf::from(path).strip_prefix(&args.source) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        // Dest keys are absolute too; rebuild the dest-side key the same way.
        let dst_key = args.dest.join(&rel).to_string_lossy().into_owned();
        if dst_map.get(&dst_key) != Some(state) {
            let dst = args.dest.join(&rel);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(path, &dst).is_ok() {
                copied += 1;
            }
        }
    }
    if copied > 0 {
        eprintln!("initial sync: {copied} file(s)");
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
