# md

`md` is a lightweight terminal Markdown viewer that renders readable Markdown
and hands it to a pager. It is designed for people who want a small,
user-local alternative to Glow, with predictable terminal output and a little
more control over math rendering.

## What it does

- Renders headings, paragraphs, emphasis, links, code spans, fenced code,
  nested lists, blockquotes, rules, and common Markdown structure.
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
- Uses `$PAGER` when set, or `less -R` by default.
- Opens directories with an interactive Markdown file picker with paging,
  live file counts, navigation, and editor shortcuts.

`md` is intentionally a pager-oriented viewer rather than a full CommonMark or
GFM implementation. It has no mouse-driven document UI and does not require
Glow.

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
render_latex: true
```

`render_latex: false` leaves supported math delimiters and source unchanged.
Named styles can customize foreground/background colors and left/right margins.
See [`.config/md.yaml`](.config/md.yaml) for the complete example and
[`docs/md.md`](docs/md.md) for the full configuration and behavior reference.

## Build and install

The viewer is a single Rust source file and can be built without a Cargo
project:

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

## License and attribution

The terminal viewer is maintained in this repository. The vendored Pi TUI
LaTeX renderer is MIT-licensed; see [`vendor/pi-tui/NOTICE`](vendor/pi-tui/NOTICE)
for attribution.
