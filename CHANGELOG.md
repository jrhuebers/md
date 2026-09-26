# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.39] - 2026-09-26

### Added

- First built-in pager implementation with automatic terminal-size tracking,
  keyboard scrolling, a mouse-steerable right-side scrollbar, a position
  indicator, and `e` editing for single files.
- Configurable `max_line_length`, pager polling speed, per-event scroll step,
  and mouse capture through `~/.config/md.yaml`.
- Half-page navigation for PageUp/PageDown, `u`/`d`, and Ctrl+U/Ctrl+D.

### Changed

- Display math is centered as a single layout block with preserved fraction and
  script alignment.
- Paragraphs reflow soft line breaks while preserving Markdown block boundaries.
- Inline math and inline code wrap according to rendered terminal width; inline
  code may wrap at internal spaces while retaining its styling.
- Rendered documents receive exactly one blank line at their beginning and end.
- The default output path is the built-in pager; `$PAGER` remains an override.
- Mouse reporting is disabled inside Herdr so multiplexer text selection works.

### Fixed

- Preserve bold spans containing inline code, including styling after the code.
- Join indented continuation lines with their list item instead of inserting
  paragraph breaks inside wrapped list content.
- Render display-style math delimiters inside list continuations.
- Avoid full-screen clearing during ordinary scrolling, reducing redraw flicker.
- Preserve punctuation and inline math across source soft line breaks.
- Correct spacing between paragraphs, lists, blockquotes, and headings.

## [0.6.16] - 2026-09-26

### Added

- Initial public binary release for Linux x86_64, including the Pi math bridge
  and vendored renderer assets.
- Configurable Glow-inspired styles, margins, terminal width, and LaTeX
  conversion through `~/.config/md.yaml`.
- Interactive Markdown file picker with paging, live counts, navigation, and
  editor shortcuts.
- Pi-compatible Unicode rendering for inline and display LaTeX math.

### Changed

- Display math is centered as a single layout block with preserved fraction and
  script alignment.
- Paragraphs reflow soft line breaks while preserving Markdown block boundaries.
- Inline math and inline code wrap according to their rendered terminal width;
  inline code may wrap at internal spaces while retaining its styling.
- The default output path is pager-based: `$PAGER` is honored, with `less -R`
  as the fallback.

### Fixed

- Preserve bold spans containing inline code, including the bold styling after
  the code span.
- Join indented continuation lines with their list item instead of inserting
  paragraph breaks inside wrapped list content.
- Render display-style math delimiters used within list continuations without
  leaving raw `$$...$$` or `\\[...\\]` source in the output.
- Avoid full-screen clearing during ordinary scrolling, eliminating the
  associated redraw flicker.
- Spacing between paragraphs, lists, blockquotes, and headings.
- Punctuation and inline math split across source soft line breaks.

[Unreleased]: https://github.com/jrhuebers/md/compare/v0.6.39...HEAD
[0.6.39]: https://github.com/jrhuebers/md/compare/v0.6.16...v0.6.39
[0.6.16]: https://github.com/jrhuebers/md/releases/tag/v0.6.16
