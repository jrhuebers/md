# md

`md` is a small compiled Markdown viewer for terminal use. It renders Markdown
and sends it to a pager; it has no Glow dependency.

The configuration is `~/.config/md.yaml`, normally a symlink to the tracked
`.config/md.yaml`. It contains named style blocks and a root-level `style`
selection. The included `md-light`, `glow-light`, and `glow-dark` styles use
Glow/Glamour colors; `md-light` uses a black code-block foreground. `margin_left` and `margin_right` are independently
configurable; both default to one space.

`width: 0` follows the terminal width. Paragraphs are reflowed to that width,
and wrapped list continuation lines are indented beneath their bullet. Nested
lists retain two spaces of indentation per level. Single newlines are reflowed
while blank-line paragraph breaks remain. Inline math spans are kept intact
while wrapping paragraphs; inline code spans wrap at their internal spaces, and
punctuation at soft line breaks is joined naturally. Block transitions between
paragraphs, lists, and blockquotes receive a blank line.
Every heading is followed by one empty line. Rendered documents also have
exactly one blank line at the beginning and end.

Pager mode is always used. With `$PAGER` unset, `md` uses its built-in
terminal pager with keyboard scrolling, automatic terminal-size tracking, and
a mouse-steerable right-side scrollbar. Mouse reporting is automatically
disabled inside Herdr so the multiplexer can continue handling text selection.
Set `$PAGER` to use an external pager such as `less -R`. There is no separate
document TUI. Inline and display math using
`$...$`, `\(...\)`, `$$...$$`, and `\[...\]` is translated to
terminal-friendly Unicode, including common fractions, roots, scripts, Greek
letters, operators, matrices, and cases. Display math is centered as a single
layout block (preserving script and fraction alignment) and separated from
surrounding paragraphs by blank lines. Unsupported TeX remains readable
as source text.

When given a directory—or no argument from an interactive terminal—`md` opens a
small keyboard file picker. It recursively lists visible Markdown files while
skipping hidden files and directories. The header shows a live count as files
are discovered. Files are shown in pages; use arrow keys or `j`/`k` to move
within and across pages, or `h`/`l` and left/right to change pages. A dot bar
shows the active page. Long paths are middle-truncated to one terminal row,
so both their beginning and filename remain visible. Press `e` to edit the
selected file, or Enter to open it. The page dots and key hint stay at the
bottom of the terminal even on a short final page; press `q` to quit.

When viewing a single file in the built-in pager, press `e` to open the file
in `$VISUAL`, `$EDITOR`, or `vi`. The document is re-rendered after the editor
exits. This shortcut is unavailable when `$PAGER` selects an external pager.

## Configuration

Select a style and configure the terminal width in `~/.config/md.yaml`:

```yaml
style: glow-dark
width: 0
max_line_length: 100
render_latex: true
pager_poll_speed: 60
pager_scroll_step: 2
```

Set `render_latex: false` to leave `$...$`, `\(...\)`, `$$...$$`, and
`\[...\]` math source uncompiled. The default is `true`.
`max_line_length` limits the Markdown content column and centers it when the
terminal is wider; it defaults to 100, while `0` follows the available width.
`pager_poll_speed` controls how frequently the built-in pager polls and processes
queued key events. `pager_scroll_step` controls the number of lines moved by one
`j`/`k` or arrow event and defaults to 2.

Add or adjust a style block under `styles:` using the color fields and margin
fields shown in the tracked example.

## Build and install on Linux

The source is [`tools/md.rs`](../tools/md.rs). Math is rendered by the
vendored MIT-licensed Pi TUI renderer in `vendor/pi-tui/latex.js`, accessed
through one persistent Node bridge process. Node is therefore required for the
Pi math path; the Rust fallback remains available if the bridge cannot start.
The viewer itself builds to a native user-local binary. Tagged GitHub releases
also publish a Linux x86_64 archive containing the binary and Pi math assets.

```sh
mkdir -p ~/.local/bin
rustc -O -C strip=symbols ~/md/tools/md.rs -o ~/.local/bin/md
```

Deploy the tracked configuration:

```sh
mkdir -p ~/.config
ln -sfn ~/md/.config/md.yaml ~/.config/md.yaml
```

Keep `~/.local/bin` in `PATH`. Verify the installation with:

```sh
md --version
md README.md
md .
printf '# Heading\n\nMarkdown from stdin.\n' | md -
```

The built-in pager uses `q` to quit, `j`/`k` or arrow keys to scroll,
PageUp/PageDown, `u`/`d`, and Ctrl+U/Ctrl+D for half-page jumps, Space for the next page and `b` for the previous page, `g`/`G` for the beginning/end, mouse-wheel scrolling,
scrollbar click-and-drag outside Herdr, and `e` to edit a single file. Use
`PAGER=cat` for a non-interactive smoke test. The command accepts one or more
Markdown paths; `-` reads standard input.

## Removal

```sh
rm -f ~/.local/bin/md ~/.config/md.yaml
```
