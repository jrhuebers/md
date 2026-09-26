# md

`md` is a standalone user-local terminal Markdown viewer. Keep its source,
configuration, and documentation in this repository rather than in the
personal dotfiles repository.

## Build and install

Build the native binary with:

```sh
rustc -O -C strip=symbols tools/md.rs -o ~/.local/bin/md
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
