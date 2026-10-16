# fsmirror

fsmirror is a command-line utility written in Rust for mirroring files and folders, either in a single run or continuously as they change.

## Features

- **Initial mirroring:** mirrors missing or differing files before watching for changes.
- **Single-run mode:** mirror once and exit with `--once`.
- **Continuous mirroring:** polls every second by default, with a configurable interval.
- **Content hashing:** detects content changes even when file size and modification time stay the same.
- **Exclusions:** skip files and folders with repeatable patterns.
- **Optional deletion mirroring:** retain deleted source files in the destination by default, or enable `--delete`.

Built on [fstrace](https://crates.io/crates/fstrace).

## Installation

Install from GitHub with Rust and Cargo:

```sh
cargo install --git https://github.com/Spectra010s/fsmirror.git
```

Or build locally:

```sh
git clone https://github.com/Spectra010s/fsmirror.git
cd fsmirror
cargo build --release
```

The compiled binary is available at `target/release/fsmirror`.

## Usage

```sh
fsmirror <SOURCE> <DEST> [OPTIONS]
```

Mirror a directory continuously:

```sh
fsmirror "$PWD/source" "$PWD/mirror"
```

fsmirror creates the destination if needed, mirrors existing files, and continues running until stopped with `Ctrl+C`.

### Mirror once

Use `--once` to mirror the source and exit:

```sh
fsmirror "$PWD/source" "$PWD/mirror" --once
```

### Mirror a file

Mirror an individual file to another location:

```sh
fsmirror "$PWD/config.toml" "$PWD/backup/config.toml"
```

If the destination is an existing directory, the source filename is preserved. Otherwise, the destination is treated as the target file path. Missing parent directories are created.

Add `--once` for a single run:

```sh
fsmirror "$PWD/config.toml" "$PWD/backup/config.toml" --once
```

### Exclude files and folders

Repeat `--exclude` for multiple patterns. Patterns support `*` and `?`; quote them to prevent shell expansion.

```sh
fsmirror "$PWD/source" "$PWD/mirror" \
  --exclude ".git" \
  --exclude "node_modules" \
  --exclude "*.tmp"
```

### Change the polling interval

Set the interval in milliseconds:

```sh
fsmirror "$PWD/source" "$PWD/mirror" --interval 500
```

### Mirror deletions

By default, deleting a source file leaves its destination copy intact. To propagate deletions detected while fsmirror is running:

```sh
fsmirror "$PWD/source" "$PWD/mirror" --delete
```

The initial mirroring pass never removes destination files, even with `--delete`. Files that exist only in the destination at startup are retained. Because `--once` exits after this pass, combining it with `--delete` does not remove destination-only files.

### Disable content hashing

Hashing reads file contents on each poll. For large directory trees, you can use metadata-based detection instead:

```sh
fsmirror "$PWD/source" "$PWD/mirror" --no-hash
```

Without hashing, changes that preserve both file size and modification time may be missed.

## Options

| Option | Description | Default |
| --- | --- | --- |
| `--once` | Mirror once and exit | Disabled |
| `--exclude <PATTERN>` | Exclude a file or folder; repeatable | None |
| `--interval <MS>` | Time between polls in milliseconds | `1000` |
| `--delete` | Propagate source-file deletions detected while running | Disabled |
| `--no-hash` | Disable content hashing | Disabled |
| `-h`, `--help` | Show help | |

## How it works

1. Compare the source and destination, mirroring missing or differing source files.
2. Exit if `--once` is enabled.
3. Record the source state.
4. Wait for the polling interval, then compare the new source state with the previous one.
5. Mirror created or modified files, and optionally remove deleted files from the destination.
6. Repeat.

Destination changes are not mirrored back to the source. Existing destination files may be overwritten by source files.

fsmirror keeps a current copy; it does not keep historical file versions.

## Author

Adeloye Adetayo ([Spectra010s](https://spectra010s.com))

## License

[MIT](LICENSE)
