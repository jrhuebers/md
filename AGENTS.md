# md

`md` is a standalone user-local terminal Markdown viewer. Keep its source,
configuration, and documentation in this repository rather than in the
personal dotfiles repository.

## Repository workflow

Before making changes in this repository, pull the latest changes from the remote
branch (`git pull --ff-only`). After committing intended changes, push the commit
to the remote branch. Do not leave committed work only in the local repository.

## Build and install

Build and install the native binary with Cargo:

```sh
cargo build --release --locked
install -Dm755 target/release/md ~/.local/bin/md
```

Deploy the tracked configuration with:

```sh
mkdir -p ~/.config
ln -sfn ~/md/.config/md.yaml ~/.config/md.yaml
```

After source changes, rebuild and run the smoke tests documented in
[`docs/md.md`](docs/md.md). Keep the standalone repository clean and commit
intended changes only.

Maintain [`CHANGELOG.md`](CHANGELOG.md) using the Keep a Changelog structure:
keep an `Unreleased` section at the top, group notable user-facing changes by
`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, or `Security`, and move
those entries into a dated version section when publishing a release.
