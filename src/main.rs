use {
    clap::Parser,
    fstrace::{
        EntryKind, Event, FileState, diff, fhash, fsize, matches_exclude, mtime, snapshot,
        snapshot_hashed,
    },
    std::{
        collections::HashMap,
        env, fs, io,
        path::{Path, PathBuf},
        process, thread,
        time::Duration,
    },
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};

// fs::copy opens the destination for writing, so it fails with EACCES when a
// previous copy left a read-only file behind. Git stores objects and packs as
// 0444, which makes those files impossible to update or repair once copied.
// Clear the read-only bit first; fs::copy restores the source mode afterwards.
// set_readonly(false) briefly opens write for all, which is why clippy flags
// it; the mode is restored by the copy that follows.
#[allow(clippy::permissions_set_readonly_false)]
fn copy_file(src: &Path, dst: &Path) -> io::Result<u64> {
    if let Ok(meta) = fs::metadata(dst) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(dst, perms);
        }
    }
    fs::copy(src, dst)
}

// State for a regular file, matching fstrace's field layout so source
// and dest states compare directly.
fn file_state(path: &Path, hash: bool) -> FileState {
    FileState {
        modified: mtime(path),
        size: fsize(path),
        hash: if hash { fhash(path) } else { None },
        kind: EntryKind::File,
        target: None,
    }
}

// State for a symlink. mtime and size come from the target, as in
// fstrace; the link itself is identified by kind and target.
fn link_state(path: &Path) -> FileState {
    FileState {
        modified: mtime(path),
        size: fsize(path),
        hash: None,
        kind: EntryKind::Symlink,
        target: fs::read_link(path)
            .ok()
            .map(|t| t.to_string_lossy().into_owned()),
    }
}

// Classify with symlink_metadata so a link stays a link. A missing path
// falls back to an empty file state; callers only act when it exists.
fn entry_state(path: &Path, hash: bool) -> FileState {
    if is_symlink(path) {
        link_state(path)
    } else {
        file_state(path, hash)
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// Links compare by kind and target alone: mtime and size come from the
// target, and a relative target resolves differently at the dest, so
// comparing those would recreate an identical link every run.
fn same_entry(a: &FileState, b: &FileState) -> bool {
    if a.kind == EntryKind::Symlink || b.kind == EntryKind::Symlink {
        return a.kind == b.kind && a.target == b.target;
    }
    a == b
}

// Remove whatever sits at dst without following links, because symlink()
// will not replace an existing path. A link or file unlinks; a real
// directory has to go recursively before a link can take its place.
fn remove_existing(dst: &Path) -> io::Result<()> {
    match fs::symlink_metadata(dst) {
        Ok(meta) if meta.file_type().is_dir() => fs::remove_dir_all(dst),
        Ok(_) => fs::remove_file(dst),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn make_symlink(target: &str, _src: &Path, dst: &Path) -> io::Result<()> {
    symlink(target, dst)
}

// Windows links carry their type, which a dangling target cannot reveal;
// the source side decides, defaulting to a file link.
#[cfg(windows)]
fn make_symlink(target: &str, src: &Path, dst: &Path) -> io::Result<()> {
    let is_dir = fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false);
    if is_dir {
        symlink_dir(target, dst)
    } else {
        symlink_file(target, dst)
    }
}

// Bring one dest entry in line with a source state. Links are recreated
// with their recorded target, never followed, unless --skip-links says
// leave them alone.
fn sync_entry(args: &Args, src: &Path, dst: &Path, state: &FileState) -> io::Result<()> {
    if state.kind == EntryKind::Symlink {
        if args.skip_links {
            return Ok(());
        }
        let target = state.target.as_deref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "symlink target unreadable")
        })?;
        remove_existing(dst)?;
        return make_symlink(target, src, dst);
    }
    // A stale link at the dest would make fs::copy write through it into
    // whatever it points at, so unlink it first.
    if is_symlink(dst) {
        remove_existing(dst)?;
    }
    copy_file(src, dst).map(|_| ())
}

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
    /// Leave symlinks untouched at the destination.
    // Without this, source links are recreated as links, never followed.
    #[arg(long)]
    skip_links: bool,
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

    // Refuse self-mirroring: copying a file onto itself truncates it,
    // and a dest inside the source grows the tree every poll, forever.
    if let Err(e) = guard_paths(&args.source, &args.dest) {
        eprintln!("error: {e}");
        process::exit(1);
    }

    // symlink_metadata: a source link is mirrored as a link, not as
    // whatever it happens to point at.
    match fs::symlink_metadata(&args.source) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => mirror_file(&args),
        Ok(meta) if meta.is_dir() => mirror_tree(&args),
        _ => {
            eprintln!(
                "error: source is not a file or directory: {}",
                args.source.display()
            );
            process::exit(1);
        }
    }
}

// Single file mode: track one source entry, recreate it on any change.
// Same guarantees as tree mode (hashed by default, append-only unless
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

    let src_state = || entry_state(&args.source, !args.no_hash);
    let dst_state = || entry_state(&dst, !args.no_hash);
    let mut prev = src_state();
    // Startup mirror, same as tree mode: sync when the source exists and
    // the dest differs.
    if fs::symlink_metadata(&args.source).is_ok()
        && !same_entry(&dst_state(), &prev)
        && let Err(e) = sync_entry(args, &args.source, &dst, &prev)
    {
        eprintln!("error: mirror {}: {e}", args.source.display());
    }
    if args.once {
        return;
    }
    loop {
        thread::sleep(Duration::from_millis(args.interval));
        // Dangling links still exist: symlink_metadata, not exists().
        if fs::symlink_metadata(&args.source).is_err() {
            // --skip-links leaves the dest link for a deleted source link.
            if args.delete && !(args.skip_links && prev.kind == EntryKind::Symlink) {
                let _ = remove_existing(&dst);
            }
            prev = src_state();
            continue;
        }
        let curr = src_state();
        if !same_entry(&curr, &prev) {
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = sync_entry(args, &args.source, &dst, &curr) {
                eprintln!("error: mirror {}: {e}", args.source.display());
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
            apply(args, &event, &prev, &curr);
        }
        prev = curr;
    }
}

// Refuse self-mirroring before anything else. Resolves the effective
// dest the same cp rule the modes use, then compares canonical paths
// (lexical fallback when paths don't exist yet).
fn guard_paths(source: &Path, dest: &Path) -> Result<(), String> {
    // Effective dest: for non-directory sources an existing directory
    // wins (cp rule); directory sources mirror the dir itself, never
    // joined. symlink_metadata keeps a source link out of the dir case.
    let src_is_dir = fs::symlink_metadata(source)
        .map(|m| m.is_dir())
        .unwrap_or(false);
    let eff = if !src_is_dir && dest.is_dir() {
        match source.file_name() {
            Some(name) => dest.join(name),
            None => dest.to_path_buf(),
        }
    } else {
        dest.to_path_buf()
    };
    let abs = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(p)
        }
    };
    let (src_abs, dst_abs) = (abs(source), abs(&eff));
    // Canonical when possible: kills `..`, symlinks, and `./` disguises.
    let src_can = fs::canonicalize(&src_abs).unwrap_or(src_abs);
    let dst_can = fs::canonicalize(&dst_abs).unwrap_or(dst_abs);
    if src_can == dst_can {
        return Err(format!(
            "source and dest resolve to the same path: {}",
            dst_can.display()
        ));
    }
    if dst_can.starts_with(&src_can) {
        return Err(format!(
            "dest lives inside the source and would mirror itself forever: {}",
            dst_can.display()
        ));
    }
    Ok(())
}
// Bring dest up to date before the watch loop starts: mirror every
// source entry missing in dest or differing from it. Same comparison as
// the live loop, so no second opinion logic to drift.
fn initial_mirror(args: &Args, capture: fn(&Path, &[String]) -> HashMap<String, FileState>) {
    let src_map = capture(&args.source, &args.excludes);
    let dst_map = capture(&args.dest, &args.excludes);
    let mut copied = 0u32;
    for (path, state) in &src_map {
        if args.skip_links && state.kind == EntryKind::Symlink {
            continue;
        }
        let rel = match PathBuf::from(path).strip_prefix(&args.source) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        // Dest keys are absolute too; rebuild the dest-side key the same way.
        let dst_key = args.dest.join(&rel).to_string_lossy().into_owned();
        if dst_map.get(&dst_key).is_some_and(|d| same_entry(d, state)) {
            continue;
        }
        let dst = args.dest.join(&rel);
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match sync_entry(args, Path::new(path), &dst, state) {
            Ok(()) => copied += 1,
            Err(e) => eprintln!("error: mirror {path}: {e}"),
        }
    }
    if copied > 0 {
        eprintln!("mirrored: {copied} file(s)");
    }
}

// Mirror one event into the dest tree. Paths are stored absolute in the
// snapshot, so strip the source prefix to get the dest-relative path.
fn apply(
    args: &Args,
    event: &Event,
    prev: &HashMap<String, FileState>,
    curr: &HashMap<String, FileState>,
) {
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
            // The poll already classified the entry; a re-stat here could
            // race a second change and lose the link target.
            let Some(state) = curr.get(path) else {
                return;
            };
            if args.skip_links && state.kind == EntryKind::Symlink {
                return;
            }
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = sync_entry(args, &src, &dst, state) {
                eprintln!("error: mirror {}: {e}", src.display());
            }
        }
        Event::Deleted { .. } => {
            // Append-only unless --delete: the dest never loses files
            // by default, so a bad poll cannot wipe the backup.
            if !args.delete {
                return;
            }
            // --skip-links leaves a deleted source link's dest alone.
            if args.skip_links && prev.get(path).is_some_and(|s| s.kind == EntryKind::Symlink) {
                return;
            }
            let _ = remove_existing(&dst);
        }
    }
}
