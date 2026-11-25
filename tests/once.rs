use std::{fs, process::Command, time::SystemTime};

#[test]
fn second_once_run_skips_unchanged_file() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("fsmirror-once-{}-{unique}", std::process::id()));
    let source = root.join("source");
    let dest = root.join("dest");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file.txt"), "unchanged").unwrap();

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_fsmirror"))
            .args([&source, &dest])
            .arg("--once")
            .output()
            .unwrap()
    };

    let first = run();
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stderr).contains("mirrored: 1 file(s)"));

    let second = run();
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains("mirrored:"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn once_run_applies_mode_only_changes() {
    use std::os::unix::fs::PermissionsExt;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("fsmirror-mode-{}-{unique}", std::process::id()));
    let source = root.join("source");
    let dest = root.join("dest");
    let source_dir = source.join("dir");
    let source_file = source_dir.join("file.txt");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(&source_file, "unchanged").unwrap();
    fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&source_file, fs::Permissions::from_mode(0o644)).unwrap();

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_fsmirror"))
            .args([&source, &dest])
            .arg("--once")
            .output()
            .unwrap()
    };
    assert!(run().status.success());

    fs::set_permissions(&source_file, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(run().status.success());
    assert_eq!(
        fs::read_to_string(dest.join("dir/file.txt")).unwrap(),
        "unchanged"
    );
    assert_eq!(
        fs::metadata(dest.join("dir/file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(dest.join("dir")).unwrap().permissions().mode() & 0o777,
        0o700
    );

    fs::remove_dir_all(root).unwrap();
}
