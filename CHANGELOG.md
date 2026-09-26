# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Ensure exactly one blank line of terminal breathing room at the beginning and
  end of rendered documents.
- Add `max_line_length` to center Markdown in a narrower content column when
  the terminal has more horizontal space; the default is 100 columns.
- Scrollbar dragging now preserves the thumb position on mouse-down and follows
  the pointer's vertical delta until release; track clicks still jump.

### Added

- First built-in pager implementation with automatic terminal-size tracking,
  keyboard scrolling, a mouse-steerable right-side scrollbar, a position
  indicator, and `e` editing for single files.
- Renamed `pager_scroll_speed` to `pager_poll_speed` to reflect that it controls
  input polling rather than movement distance.
- Added `pager_scroll_step`, currently set to 2, for the number of lines moved
  by one `j`/`k` or arrow event.
- Half-page jumps for PageUp/PageDown, `u`/`d`, and Ctrl+U/Ctrl+D.
- Inline code spans now wrap at internal spaces while preserving their styling.

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

- Spacing between paragraphs, lists, blockquotes, and headings.
- Punctuation and inline math split across source soft line breaks.

[Unreleased]: https://github.com/jrhuebers/md/compare/v0.6.16...HEAD
[0.6.16]: https://github.com/jrhuebers/md/releases/tag/v0.6.16
