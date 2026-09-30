# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Write release checksum files using the downloadable archive filename rather than a build-directory path.

## [0.7.0] - 2026-09-30

### Added

- Highlight fenced code blocks by language with Syntect, using the active light or dark style.
- Render display math inside blockquotes while retaining the quote bar and its indentation.
- Search visible text in the built-in pager with `/`, highlight matches, and move through matching occurrences with `n`/`N`.
- Configure selected and other search-match foregrounds and backgrounds per theme (`search_selected_fg`, `search_selected_bg`, `search_other_fg`, and `search_other_bg`); the tracked styles use black on orange for the selected match and black on yellow for the others.
- Show a light-grey insertion caret in the built-in pager's `/` search prompt.
- Wrap long headings, aligning continuation lines beneath the heading text after the displayed `#` markers.

### Changed

- Build with Cargo to include Syntect syntax definitions and themes.
- Restore the built-in pager in the tracked configuration while preserving external pager and `$PAGER` support.

### Fixed

- Time out stalled LaTeX helper responses and fall back to the Rust renderer instead of blocking the viewer.
- Stop the LaTeX helper before opening the external pager, so it cannot hold terminal descriptors during pager use.
- Align stacked fractions correctly after combining accents such as the overbar in `a̅`.
- Preserve spacing before stacked limit operators (`lim`, `limsup`, etc.) in display math.
- Ignore invisible word separators in math source so multirow matrices and fractions stay aligned.
- Let Esc clear an active search and its highlights in the built-in pager.
- Show search matches at normal intensity even inside dimmed blockquotes, then restore the original styling.
- Make literal pager searches case-insensitive for Unicode text, not just ASCII.

## [0.6.43] - 2026-09-27

### Changed

- Use the configurable external pager command, defaulting to `less -R`; `$PAGER` still overrides it.

### Removed

- Disable the custom built-in pager while investigating terminal-mode issues with Yazi.

## [0.6.42] - 2026-09-27

### Fixed

- Render fenced code blocks with the same foreground and background style as inline code, filling the entire content column as a rectangle.
- Render YAML frontmatter with full-width rules while preserving its source line breaks.
- Preserve every source line inside fenced code blocks instead of applying paragraph/list normalization.
- Keep the built-in pager's terminal input in blocking mode so a suspended parent terminal application (such as Yazi) does not mistake a timed-out read for EOF after the pager exits. Poll for incomplete escape sequences instead.

## [0.6.41] - 2026-09-26

### Added

- Render GitHub-Flavored Markdown tables with Glamour-style aligned columns, header rules, alignment markers, and narrow-terminal wrapping.

## [0.6.40] - 2026-09-26

### Fixed

- Keep inline code styled when its opening backtick follows punctuation and the span wraps across lines (for example, ``(`git rev-parse HEAD`)``).

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

[Unreleased]: https://github.com/jrhuebers/md/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/jrhuebers/md/compare/v0.6.43...v0.7.0
[0.6.43]: https://github.com/jrhuebers/md/compare/v0.6.42...v0.6.43
[0.6.42]: https://github.com/jrhuebers/md/compare/v0.6.41...v0.6.42
[0.6.41]: https://github.com/jrhuebers/md/compare/v0.6.40...v0.6.41
[0.6.40]: https://github.com/jrhuebers/md/compare/v0.6.39...v0.6.40
[0.6.39]: https://github.com/jrhuebers/md/compare/v0.6.16...v0.6.39
[0.6.16]: https://github.com/jrhuebers/md/releases/tag/v0.6.16
