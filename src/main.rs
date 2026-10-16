use {
    clap::Parser,
    fstrace::{
        Event, FileState, diff, fhash, fsize, matches_exclude, mtime, snapshot, snapshot_hashed,
    },
    std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process, thread,
        time::Duration,
    },
};

/// Mirror a live tree elsewhere: poll the source, copy what changed.
#[derive(Parser, Debug)]
#[command(
    name = "fsmirror",
    about = "A command-line utility for mirroring files and folders."
)]
struct Args {
    /// File or directory to watch.
    source: PathBuf,
    /// Mirror target: directory for directory sources; directory or file
    // path for file sources (existing directory wins, like `cp`).
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
    /// Skip content fingerprinting.
    // Hashed snapshots are the default because a mirror that silently
    // misses rewrites is worse than none; skip only for giant trees
    // where re-reading every file every poll costs more than the
    // misses it prevents.
    #[arg(long)]
    no_hash: bool,
    /// Mirror once and exit.
    // For scripts and cron: populate the dest from current state,
    // then leave instead of watching.
    #[arg(long)]
    once: bool,
}

fn main() {
    let args = Args::parse();

    if args.source.is_file() {
        mirror_file(&args);
        return;
    }
    if !args.source.is_dir() {
        eprintln!(
            "error: source is not a file or directory: {}",
            args.source.display()
        );
        process::exit(1);
    }
    mirror_tree(&args);
}

// Single file mode: track one FileState, copy on any change. Same
// guarantees as tree mode (hashed by default, append-only unless
// --delete), none of the walking.
fn mirror_file(args: &Args) {
    let name = args
        .source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if args
        .excludes
        .iter()
        .any(|e| matches_exclude(&args.source.to_string_lossy(), name, e))
    {
        eprintln!(
            "error: source excluded by --exclude: {}",
            args.source.display()
        );
        process::exit(1);
    }

    // cp rule: existing directory wins, otherwise dest is the file path.
    let dst = if args.dest.is_dir() {
        args.dest.join(name)
    } else {
        args.dest.clone()
    };
    if let Some(parent) = dst.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("error: cannot create dest {}: {e}", parent.display());
        process::exit(1);
    }

    let state = |hash: bool| FileState {
        modified: mtime(&args.source),
        size: fsize(&args.source),
        hash: if hash { fhash(&args.source) } else { None },
    };
    let dst_state = || FileState {
        modified: mtime(&dst),
        size: fsize(&dst),
        hash: if !args.no_hash { fhash(&dst) } else { None },
    };
    let mut prev = state(!args.no_hash);
    // Startup mirror, same as tree mode: copy when missing or differing.
    if args.source.exists() && dst_state() != prev {
        let _ = fs::copy(&args.source, &dst);
    }
    if args.once {
        return;
    }
    loop {
        thread::sleep(Duration::from_millis(args.interval));
        if !args.source.exists() {
            if args.delete {
                let _ = fs::remove_file(&dst);
            }
            prev = state(!args.no_hash);
            continue;
        }
        let curr = state(!args.no_hash);
        if curr != prev {
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::copy(&args.source, &dst) {
                eprintln!("error: copy {}: {e}", args.source.display());
            }
            prev = curr;
        }
    }
}

fn mirror_tree(args: &Args) {
    if let Err(e) = fs::create_dir_all(&args.dest) {
        eprintln!("error: cannot create dest {}: {e}", args.dest.display());
        process::exit(1);
    }

    // Hashed snapshots by default: plain metadata misses same-stamp
    // same-size rewrites, and a mirror must not silently skip writes.
    let capture = if args.no_hash {
        snapshot
    } else {
        snapshot_hashed
    };
    // Startup mirror: the watch loop only sees changes *after* it starts,
    // so populate the dest first or pre-existing files never arrive.
    // Copies missing or differing files; never deletes (even with
    // --delete: wiping a non-empty dest on first run would be destructive,
    // deletions flow only through the live loop).
    initial_mirror(args, capture);
    if args.once {
        return;
    }
    let mut prev = capture(&args.source, &args.excludes);
    loop {
        thread::sleep(Duration::from_millis(args.interval));
        let curr = capture(&args.source, &args.excludes);
        for event in diff(&prev, &curr) {
            apply(args, &event);
        }
        prev = curr;
    }
}

// Bring dest up to date before the watch loop starts: copy every source
// file missing in dest or differing from it. Same-State comparison as
// the live loop, so no second opinion logic to drift.
fn initial_mirror(
    args: &Args,
    capture: fn(&Path, &[String]) -> HashMap<String, fstrace::FileState>,
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
                let _ = fs::create_dir_all(parent);
            }
            if fs::copy(path, &dst).is_ok() {
                copied += 1;
            }
        }
    }
    if copied > 0 {
        eprintln!("mirrored: {copied} file(s)");
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
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::copy(&src, &dst) {
                eprintln!("error: copy {}: {e}", src.display());
            }
        }
        Event::Deleted { .. } => {
            // Append-only unless --delete: the dest never loses files
            // by default, so a bad poll cannot wipe the backup.
            if args.delete {
                let _ = fs::remove_file(&dst);
            }
        }
    }
}
