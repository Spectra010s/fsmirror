# Contributing to fsmirror

Contributions are welcome, whether you're fixing a bug, improving documentation, or proposing a feature.

## Getting started

Fork the repository, clone your fork, and create a branch for your changes:

```sh
git clone https://github.com/<your-username>/fsmirror.git
cd fsmirror
git switch -c fix/describe-your-change
```

You'll need a Rust toolchain that supports the 2024 edition.

Build the project:

```sh
cargo build
```

View the available options:

```sh
cargo run -- --help
```

## Making changes

Keep each contribution focused on a specific problem. For larger changes, open an issue first so we can discuss the approach.

When changing mirroring behavior, consider both files and directories, single-run and continuous modes, exclusions, hashing, and deletion handling. Add regression tests where appropriate.

Use temporary directories when testing mirroring operations.

## Checking your work

Before opening a pull request, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Manually verify the behavior affected by your changes.

## Commit messages

Use Conventional Commits with a type and scope:

```text
type(scope): short description
```

Examples:

```text
feat(mirror): support individual file sources
fix(exclude): handle wildcard patterns correctly
docs(readme): clarify deletion behavior
test(mirror): cover source file recreation
chore(deps): update fstrace
```

Use a short, imperative description explaining what the commit changes.

## Pull requests

In your pull request, explain:

- What problem the change addresses.
- What changed and why.
- How you verified it.
- Any changes to command-line behavior.

Link the relevant issue if one exists, and update documentation when changing options or behavior.

## Reporting bugs

Include the command you ran, what you expected, and what happened instead. Share your operating system, fsmirror version or commit, and a small reproduction when possible.

Remove private paths and sensitive file contents before sharing logs or examples.

## Suggesting features

Describe the use case and what you want to accomplish. Examples of the proposed command or behavior help make the suggestion easier to discuss.

## License

By contributing, you agree that your contributions will be licensed under the project's [MIT License](LICENSE).
