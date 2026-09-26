# md

`md` is a lightweight terminal Markdown viewer that renders readable Markdown
and hands it to a pager. It is designed for people who want a small,
user-local alternative to Glow, with predictable terminal output and a little
more control over math rendering.

## What it does

- Renders headings, paragraphs, emphasis, links, code spans, fenced code, nested lists, blockquotes, rules, tables, and common Markdown structure.
- Reflows soft line breaks within paragraphs while preserving paragraph and
  block boundaries.
- Wraps long inline code spans at their internal spaces without losing code
  styling.
- Supports inline and display math with `$...$`, `\(...\)`, `$$...$$`, and
  `\[...\]`.
- Uses Pi's terminal LaTeX renderer for Unicode Greek letters, operators,
  scripts, fractions, roots, matrices, cases, and related constructs.
- Keeps inline math on one line; centers multiline display math and preserves
  its internal layout.
- Provides configurable Glow-inspired light and dark styles, colors, margins,
  terminal width, and optional LaTeX conversion.
- Uses a built-in terminal pager by default, with keyboard scrolling, an
  auto-adapting viewport, a mouse-steerable scrollbar (automatically disabled
  inside Herdr so multiplexer selection remains available), and a position
  indicator. PageUp/PageDown, `u`/`d`, and Ctrl+U/Ctrl+D make half-page jumps;
  Space moves to the next page and `b` to the previous page. `$PAGER` can override it with an external
  pager such as `less -R`.
- Opens directories with an interactive Markdown file picker with paging,
  live file counts, navigation, and editor shortcuts.

`md` adds one blank line of terminal breathing room at the beginning and end
of each rendered document. It is intentionally a pager-oriented viewer rather
than a full CommonMark or GFM implementation and does not require Glow.

See [`CHANGELOG.md`](CHANGELOG.md) for notable changes by release.

## Example

```sh
md README.md
md notes/derivation.md
printf '# Heading\n\nMarkdown from stdin.\n' | md -
```

Use `PAGER=cat md FILE` for a non-interactive render, or set another pager
through `$PAGER`.

## Configuration

Configuration normally lives at `~/.config/md.yaml`:

```yaml
style: glow-dark
width: 0
max_line_length: 100
render_latex: true
pager_poll_speed: 60
pager_scroll_step: 2
pager_mouse: false
```

`render_latex: false` leaves supported math delimiters and source unchanged.
`max_line_length` limits the Markdown content column and centers it when the
terminal is wider; it defaults to 100, while `0` uses the available width.
`pager_poll_speed` controls how often the built-in pager polls and processes
queued key events. `pager_scroll_step` controls the number of lines moved by one
`j`/`k` or arrow event and defaults to 2. `pager_mouse` enables mouse wheel and scrollbar interaction outside Herdr; it
defaults to `false`. When disabled, the scrollbar column is removed. Named styles can customize
foreground/background colors and left/right margins.
See [`.config/md.yaml`](.config/md.yaml) for the complete example and
[`docs/md.md`](docs/md.md) for the full configuration and behavior reference.

## Build and install

The viewer is a small Rust program and can be built without a Cargo project:

```sh
mkdir -p ~/.local/bin
rustc -O -C strip=symbols tools/md.rs -o ~/.local/bin/md
mkdir -p ~/.config
ln -sfn "$PWD/.config/md.yaml" ~/.config/md.yaml
```

The Pi-compatible math bridge uses Node.js and the vendored renderer under
`vendor/pi-tui/`. If Node is unavailable, `md` falls back to its smaller Rust
math renderer. The standalone build and installation details are in
[`docs/md.md`](docs/md.md).

## Binary releases

Tagged releases publish a Linux x86_64 tarball containing the `md` binary, the
Pi math bridge, its vendored renderer, and an example configuration. To install
a release manually:

```sh
version=0.6.41
archive="md-v${version}-x86_64-unknown-linux-gnu.tar.gz"
tar -xzf "$archive"
cd "md-v${version}-x86_64-unknown-linux-gnu"
install -Dm755 bin/md ~/.local/bin/md
mkdir -p ~/.local/share/md ~/.config
cp -a share/md/. ~/.local/share/md/
ln -sfn ~/.local/share/md/md.yaml ~/.config/md.yaml
```

Release archives include SHA-256 checksums. The binary release is optional;
building directly from `tools/md.rs` remains supported.

## License and attribution

The terminal viewer is maintained in this repository. The vendored Pi TUI
LaTeX renderer is MIT-licensed; see [`vendor/pi-tui/NOTICE`](vendor/pi-tui/NOTICE)
for attribution.
