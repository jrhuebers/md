use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const RESET: &str = "\x1b[0m";

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
}

pub enum Action {
    Done,
    Edit,
}

#[derive(Clone, Copy)]
pub struct SearchColors {
    pub selected_bg: u8,
    pub selected_fg: u8,
    pub other_bg: u8,
    pub other_fg: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Match {
    row: usize,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Top,
    Bottom,
    Quit,
    Edit,
    Search,
    NextMatch,
    PreviousMatch,
    Enter,
    Backspace,
    Escape,
    Character(char),
    Mouse(MouseEvent),
}

#[derive(Clone, Copy)]
struct MouseEvent {
    button: u8,
    x: usize,
    y: usize,
    press: bool,
    motion: bool,
}

struct TerminalGuard {
    saved: String,
    mouse_enabled: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_tty(&self.saved);
        if self.mouse_enabled {
            print!("\x1b[?1006l\x1b[?1003l");
        }
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub fn run<F>(rendered: &str, editable: bool, poll_speed: usize, scroll_step: usize, mouse_enabled: bool, search_colors: SearchColors, mut rerender: F) -> io::Result<Action>
where
    F: FnMut(usize) -> String,
{
    let saved = stty(&["-g"])?;
    if let Err(error) = stty(&["-icanon", "-echo", "min", "1", "time", "0"]) {
        let _ = restore_tty(&saved);
        return Err(error);
    }
    let _guard = TerminalGuard { saved, mouse_enabled };
    print!("\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l");
    if mouse_enabled {
        print!("\x1b[?1003h\x1b[?1006h");
    }
    io::stdout().flush()?;

    let mut rendered = rendered.to_string();
    let mut lines = collect_lines(&rendered);
    let mut total = lines.len().max(1);
    let mut offset = 0usize;
    let mut query = String::new();
    let mut prompt: Option<String> = None;
    let mut matches = Vec::new();
    let mut active_match: Option<usize> = None;
    let mut dragging: Option<(usize, usize)> = None;
    let mut hovered = false;
    let mut dirty = true;
    let mut clear_screen = true;
    let mut last_size: Option<(usize, usize)> = None;
    let mut stdin = File::open("/dev/tty")?;
    let poll_interval = Duration::from_millis((1000 / poll_speed.max(1)) as u64);
    let scroll_step = scroll_step.max(1);

    loop {
        let (rows, columns) = terminal_size().unwrap_or((24, 80));
        let viewport = rows.saturating_sub(1).max(1);
        if let Some((old_rows, old_columns)) = last_size {
            if old_columns != columns {
                let old_viewport = old_rows.saturating_sub(1).max(1);
                let old_max = total.saturating_sub(old_viewport);
                let old_offset = offset;
                let content_width = if mouse_enabled { columns.saturating_sub(1).max(1) } else { columns };
                rendered = rerender(content_width);
                lines = collect_lines(&rendered);
                matches = search_matches(&lines, &query);
                active_match = None;
                total = lines.len().max(1);
                let new_max = total.saturating_sub(viewport);
                offset = if old_max == 0 {
                    0
                } else {
                    old_offset.saturating_mul(new_max) / old_max
                };
                dirty = true;
                clear_screen = true;
            } else if old_rows != rows {
                dirty = true;
                clear_screen = true;
            }
        }
        last_size = Some((rows, columns));

        let max_offset = total.saturating_sub(viewport);
        offset = offset.min(max_offset);
        if dirty {
            draw(&lines, offset, total, viewport, columns.max(2), editable, mouse_enabled, dragging.is_some() || hovered, clear_screen, &query, prompt.as_deref(), active_match.map(|index| (index + 1, matches.len())), active_match.map(|index| matches[index]), search_colors);
            dirty = false;
            clear_screen = false;
        }

        let Some(key) = read_key_timeout(&mut stdin, poll_interval, prompt.is_some())? else { continue };
        if let Some(input) = prompt.as_mut() {
            match key {
                Key::Character(character) => input.push(character),
                Key::Backspace => { input.pop(); }
                Key::Escape => { prompt = None; }
                Key::Enter => {
                    let entered = prompt.take().unwrap_or_default();
                    if !entered.is_empty() {
                        query = entered;
                        matches = search_matches(&lines, &query);
                        active_match = (!matches.is_empty()).then(|| matches.partition_point(|item| item.row < offset) % matches.len());
                        if let Some(index) = active_match {
                            offset = matches[index].row.min(total.saturating_sub(viewport));
                        }
                    }
                }
                _ => {}
            }
            dirty = true;
            continue;
        }
        match key {
                Key::Up => {
                    let next = offset.saturating_sub(scroll_step);
                    dirty |= next != offset;
                    offset = next;
                }
                Key::Down => {
                    let next = (offset + scroll_step).min(max_offset);
                    dirty |= next != offset;
                    offset = next;
                }
                Key::PageUp => {
                    let next = offset.saturating_sub(viewport);
                    dirty |= next != offset;
                    offset = next;
                }
                Key::PageDown => {
                    let next = (offset + viewport).min(max_offset);
                    dirty |= next != offset;
                    offset = next;
                }
                Key::HalfPageUp => {
                    let next = offset.saturating_sub((viewport / 2).max(1));
                    dirty |= next != offset;
                    offset = next;
                }
                Key::HalfPageDown => {
                    let next = (offset + (viewport / 2).max(1)).min(max_offset);
                    dirty |= next != offset;
                    offset = next;
                }
                Key::Top => {
                    dirty |= offset != 0;
                    offset = 0;
                }
                Key::Bottom => {
                    dirty |= offset != max_offset;
                    offset = max_offset;
                }
                Key::Quit => return Ok(Action::Done),
                Key::Edit if editable => return Ok(Action::Edit),
                Key::Edit => {}
                Key::Search => { prompt = Some(String::new()); dirty = true; }
                Key::NextMatch | Key::PreviousMatch => {
                    let forward = matches!(key, Key::NextMatch);
                    if !matches.is_empty() {
                        let index = active_match.map(|index| step_match(index, matches.len(), forward))
                            .unwrap_or_else(|| if forward {
                                matches.partition_point(|item| item.row < offset) % matches.len()
                            } else {
                                matches.partition_point(|item| item.row <= offset).checked_sub(1).unwrap_or(matches.len() - 1)
                            });
                        active_match = Some(index);
                        offset = matches[index].row.min(max_offset);
                        dirty = true;
                    }
                }
                Key::Escape if !query.is_empty() => {
                    query.clear();
                    matches.clear();
                    active_match = None;
                    dirty = true;
                }
                Key::Enter | Key::Backspace | Key::Escape | Key::Character(_) => {}
                Key::Mouse(event) => {
                    let was_dragging = dragging.is_some();
                    let was_hovered = hovered;
                    let next = handle_mouse(event, columns, viewport, total, offset, &mut dragging, &mut hovered);
                    dirty |= next != offset || was_dragging != dragging.is_some() || was_hovered != hovered;
                    offset = next;
                }
        }
    }
}

fn collect_lines(rendered: &str) -> Vec<String> {
    rendered.lines().map(ToOwned::to_owned).collect()
}

fn search_matches(lines: &[String], query: &str) -> Vec<Match> {
    if query.is_empty() { return Vec::new(); }
    lines.iter().enumerate().flat_map(|(row, line)| {
        match_ranges(&plain_text(line), query).into_iter().map(move |(start, end)| Match { row, start, end })
    }).collect()
}

fn match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() { return Vec::new(); }
    let needle = query.to_ascii_lowercase();
    text.to_ascii_lowercase().match_indices(&needle)
        .map(|(start, _)| (start, start + needle.len())).collect()
}

fn step_match(index: usize, total: usize, forward: bool) -> usize {
    if forward { (index + 1) % total } else { (index + total - 1) % total }
}

fn plain_text(line: &str) -> String {
    let mut result = String::new();
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            for control in chars.by_ref() {
                if control.is_ascii_alphabetic() { break; }
            }
        } else {
            result.push(character);
        }
    }
    result
}

#[derive(Default)]
struct Intensity {
    bold: bool,
    dim: bool,
}

fn highlight_matches(line: &str, query: &str, selected: Option<(usize, usize)>, colors: SearchColors) -> String {
    let ranges = match_ranges(&plain_text(line), query);
    if ranges.is_empty() { return line.to_string(); }
    let mut result = String::new();
    let mut chars = line.chars();
    let mut position = 0;
    let mut index = 0;
    let mut highlighting = false;
    let mut current_foreground = "\x1b[39m".to_string();
    let mut current_background = "\x1b[49m".to_string();
    let mut intensity = Intensity::default();
    let mut highlight_colors = String::new();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            let mut escape = String::from("\x1b");
            for control in chars.by_ref() {
                escape.push(control);
                if control.is_ascii_alphabetic() { break; }
            }
            update_colors(&escape, &mut current_foreground, &mut current_background, &mut intensity);
            result.push_str(&escape);
            if highlighting { result.push_str(&highlight_colors); }
            continue;
        }
        if index < ranges.len() && position == ranges[index].0 {
            let (fg, bg) = if selected == Some(ranges[index]) {
                (colors.selected_fg, colors.selected_bg)
            } else {
                (colors.other_fg, colors.other_bg)
            };
            highlight_colors = format!("\x1b[22;38;5;{fg};48;5;{bg}m");
            result.push_str(&highlight_colors);
            highlighting = true;
        }
        result.push(character);
        position += character.len_utf8();
        if index < ranges.len() && position == ranges[index].1 {
            result.push_str("\x1b[22m");
            if intensity.bold { result.push_str("\x1b[1m"); }
            if intensity.dim { result.push_str("\x1b[2m"); }
            result.push_str(&current_foreground);
            result.push_str(&current_background);
            highlighting = false;
            index += 1;
        }
    }
    result
}

fn update_colors(escape: &str, foreground: &mut String, background: &mut String, intensity: &mut Intensity) {
    let Some(params) = escape.strip_prefix("\x1b[").and_then(|value| value.strip_suffix('m')) else { return };
    let codes: Vec<_> = params.split(';').map(|code| code.parse::<u16>().unwrap_or(0)).collect();
    let mut index = 0;
    while index < codes.len() {
        let code = codes[index];
        match code {
            0 => {
                *foreground = "\x1b[39m".to_string();
                *background = "\x1b[49m".to_string();
                intensity.bold = false;
                intensity.dim = false;
            }
            1 => intensity.bold = true,
            2 => intensity.dim = true,
            22 => { intensity.bold = false; intensity.dim = false; }
            39 => *foreground = "\x1b[39m".to_string(),
            49 => *background = "\x1b[49m".to_string(),
            30..=37 | 90..=97 => *foreground = format!("\x1b[{code}m"),
            40..=47 | 100..=107 => *background = format!("\x1b[{code}m"),
            38 | 48 if codes.get(index + 1) == Some(&5) && codes.len() > index + 2 => {
                let saved = format!("\x1b[{code};5;{}m", codes[index + 2]);
                if code == 38 { *foreground = saved } else { *background = saved }
                index += 2;
            }
            38 | 48 if codes.get(index + 1) == Some(&2) && codes.len() > index + 4 => {
                let saved = format!("\x1b[{code};2;{};{};{}m", codes[index + 2], codes[index + 3], codes[index + 4]);
                if code == 38 { *foreground = saved } else { *background = saved }
                index += 4;
            }
            _ => {}
        }
        index += 1;
    }
}

fn read_key_timeout(stdin: &mut File, timeout: Duration, searching: bool) -> io::Result<Option<Key>> {
    let mut fd = PollFd { fd: stdin.as_raw_fd(), events: 1, revents: 0 };
    loop {
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        match unsafe { poll(&mut fd, 1, millis) } {
            0 => return Ok(None),
            n if n > 0 => return read_key(stdin, searching),
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            _ => return Err(io::Error::last_os_error()),
        }
    }
}

fn handle_mouse(event: MouseEvent, columns: usize, viewport: usize, total: usize, offset: usize, dragging: &mut Option<(usize, usize)>, hovered: &mut bool) -> usize {
    if event.button == 64 {
        return offset.saturating_sub(2);
    }
    if event.button == 65 {
        return (offset + 2).min(total.saturating_sub(viewport));
    }
    if event.motion {
        let row = event.y.saturating_sub(1);
        let (thumb_size, track) = scrollbar_metrics(viewport, total);
        let thumb_start = if track == 0 {
            0
        } else {
            offset * track / total.saturating_sub(viewport).max(1)
        };
        *hovered = event.x >= columns.saturating_sub(1)
            && row < viewport
            && row >= thumb_start
            && row < thumb_start + thumb_size;
        if let Some((start_y, start_offset)) = *dragging {
            let max_offset = total.saturating_sub(viewport);
            if track == 0 || max_offset == 0 {
                return start_offset;
            }
            let delta = event.y as isize - start_y as isize;
            let movement = (delta * max_offset as isize) / track as isize;
            return if movement < 0 {
                start_offset.saturating_sub((-movement) as usize)
            } else {
                (start_offset + movement as usize).min(max_offset)
            };
        }
        return offset;
    }
    if event.button != 0 {
        return offset;
    }
    if !event.press {
        *dragging = None;
        return offset;
    }
    if event.x < columns.saturating_sub(1) || event.y > viewport {
        *hovered = false;
        return offset;
    }

    let row = event.y.saturating_sub(1);
    let (thumb_size, track) = scrollbar_metrics(viewport, total);
    let thumb_start = if track == 0 {
        0
    } else {
        offset * track / total.saturating_sub(viewport).max(1)
    };
    if row >= thumb_start && row < thumb_start + thumb_size {
        // Start dragging without changing the scroll position. Subsequent
        // motion is relative to this exact pointer and offset pair.
        *dragging = Some((event.y, offset));
        *hovered = true;
        return offset;
    }
    *dragging = None;
    *hovered = false;
    scrollbar_offset(row, viewport, total)
}

fn scrollbar_metrics(viewport: usize, total: usize) -> (usize, usize) {
    let thumb_size = if total <= viewport {
        viewport
    } else {
        (viewport * viewport / total).max(1).min(viewport)
    };
    (thumb_size, viewport.saturating_sub(thumb_size))
}

fn scrollbar_offset(row: usize, viewport: usize, total: usize) -> usize {
    let max_offset = total.saturating_sub(viewport);
    let (thumb_size, track) = scrollbar_metrics(viewport, total);
    if max_offset == 0 || track == 0 {
        0
    } else {
        row.saturating_sub(thumb_size / 2).min(track) * max_offset / track
    }
}

fn draw(lines: &[String], offset: usize, total: usize, viewport: usize, columns: usize, editable: bool, mouse_enabled: bool, dragging: bool, clear_screen: bool, query: &str, prompt: Option<&str>, match_count: Option<(usize, usize)>, selected: Option<Match>, colors: SearchColors) {
    let content_width = if mouse_enabled { columns.saturating_sub(1).max(1) } else { columns };
    let thumb_size = if total <= viewport {
        viewport
    } else {
        (viewport * viewport / total).max(1)
    };
    let max_offset = total.saturating_sub(viewport);
    let thumb_start = if max_offset == 0 {
        0
    } else {
        offset * viewport.saturating_sub(thumb_size) / max_offset
    };

    let mut screen = if clear_screen {
        String::from("\x1b[2J\x1b[H")
    } else {
        String::from("\x1b[H")
    };
    for row in 0..viewport {
        let line = lines.get(offset + row).map(String::as_str).unwrap_or("");
        let selected_range = selected.filter(|item| item.row == offset + row).map(|item| (item.start, item.end));
        let content = truncate_ansi(&highlight_matches(line, query, selected_range, colors), content_width);
        screen.push_str(&content);
        let used = visible_width(&content);
        screen.push_str(&" ".repeat(content_width.saturating_sub(used)));
        if mouse_enabled {
            screen.push_str(if row >= thumb_start && row < thumb_start + thumb_size {
                if dragging { "\x1b[38;5;234m█\x1b[0m" } else { "\x1b[38;5;234m┃\x1b[0m" }
            } else {
                "\x1b[38;5;245m│\x1b[0m"
            });
        }
        screen.push('\n');
    }

    let percent = if max_offset == 0 { 100 } else { (offset * 100 / max_offset).min(100) };
    let edit_hint = if editable { "e edit  " } else { "" };
    let status = if let Some(input) = prompt {
        format!("/{input}")
    } else if !query.is_empty() {
        let count = match_count.map(|(index, total)| format!("{index}/{total}"))
            .unwrap_or_else(|| "not found".to_string());
        format!(" md  {percent:>3}%  {}/{}   /{query} ({count})  n/N next/prev  q quit", offset + 1, total)
    } else {
        format!(" md  {percent:>3}%  {}/{}  / search  n/N  ↑/↓ scroll  Space/b page  g/G  {edit_hint}q quit", offset + 1, total)
    };
    let status = truncate_plain(&status, columns);
    let status = format!("{status:<columns$}");
    screen.push_str("\x1b[7m");
    screen.push_str(&status);
    screen.push_str(RESET);
    print!("{screen}");
    let _ = io::stdout().flush();
}

fn read_key(stdin: &mut File, searching: bool) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if stdin.read(&mut byte)? == 0 {
        return Ok(None);
    }
    if searching {
        return Ok(match byte[0] {
            b'\r' | b'\n' => Some(Key::Enter),
            127 | 8 => Some(Key::Backspace),
            27 => { let _ = read_escape(stdin)?; Some(Key::Escape) }
            3 => Some(Key::Escape),
            32..=126 => Some(Key::Character(byte[0] as char)),
            0xc2..=0xf4 => {
                let length = if byte[0] < 0xe0 { 2 } else if byte[0] < 0xf0 { 3 } else { 4 };
                let mut bytes = vec![byte[0]];
                for _ in 1..length {
                    if !read_byte_timeout(stdin, &mut byte)? { return Ok(None); }
                    bytes.push(byte[0]);
                }
                std::str::from_utf8(&bytes).ok().and_then(|value| value.chars().next()).map(Key::Character)
            }
            _ => None,
        });
    }
    let key = match byte[0] {
        b'q' | 3 => Some(Key::Quit),
        b'e' => Some(Key::Edit),
        b'k' | 11 => Some(Key::Up),
        b'j' | b'\n' | b'\r' => Some(Key::Down),
        b'b' | 2 => Some(Key::PageUp),
        b' ' | 6 => Some(Key::PageDown),
        b'u' | 21 => Some(Key::HalfPageUp),
        b'd' | 4 => Some(Key::HalfPageDown),
        b'g' => Some(Key::Top),
        b'G' => Some(Key::Bottom),
        b'/' => Some(Key::Search),
        b'n' => Some(Key::NextMatch),
        b'N' => Some(Key::PreviousMatch),
        27 => read_escape(stdin)?,
        _ => None,
    };
    Ok(key)
}

fn read_escape(stdin: &mut File) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if !read_byte_timeout(stdin, &mut byte)? {
        return Ok(Some(Key::Escape));
    }
    if byte[0] != b'[' {
        return Ok(Some(Key::Escape));
    }
    if !read_byte_timeout(stdin, &mut byte)? {
        return Ok(None);
    }
    if byte[0] == b'<' {
        let mut sequence = String::new();
        loop {
            if !read_byte_timeout(stdin, &mut byte)? {
                return Ok(None);
            }
            let character = byte[0] as char;
            if character == 'M' || character == 'm' {
                let mut parts = sequence.split(';');
                let button = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
                let x = parts.next().and_then(|value| value.parse().ok()).unwrap_or(1);
                let y = parts.next().and_then(|value| value.parse().ok()).unwrap_or(1);
                return Ok(Some(Key::Mouse(MouseEvent { button: button & 3 | (button & 64), x, y, press: character == 'M', motion: button & 32 != 0 })));
            }
            sequence.push(character);
        }
    }
    Ok(match byte[0] {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::PageDown),
        b'D' => Some(Key::PageUp),
        b'H' => Some(Key::Top),
        b'F' => Some(Key::Bottom),
        b'5' => {
            let _ = read_byte_timeout(stdin, &mut byte)?;
            Some(Key::HalfPageUp)
        }
        b'6' => {
            let _ = read_byte_timeout(stdin, &mut byte)?;
            Some(Key::HalfPageDown)
        }
        _ => None,
    })
}

// VMIN=1 keeps other terminal readers from treating a timeout as EOF.
// Poll only while completing escape sequences so a lone Escape does not block.
fn read_byte_timeout(stdin: &mut File, byte: &mut [u8; 1]) -> io::Result<bool> {
    let mut fd = PollFd { fd: stdin.as_raw_fd(), events: 1, revents: 0 };
    loop {
        match unsafe { poll(&mut fd, 1, 100) } {
            0 => return Ok(false),
            n if n > 0 => return Ok(stdin.read(byte)? != 0),
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            _ => return Err(io::Error::last_os_error()),
        }
    }
}

fn terminal_size() -> Option<(usize, usize)> {
    let output = Command::new("stty").args(["-F", "/dev/tty", "size"]).output().ok()?;
    let size = String::from_utf8_lossy(&output.stdout);
    let mut values = size.split_whitespace().map(|value| value.parse().ok());
    let rows = values.next()??;
    let columns = values.next()??;
    (rows > 0 && columns > 0).then_some((rows, columns))
}

fn stty(arguments: &[&str]) -> io::Result<String> {
    let output = Command::new("stty").arg("-F").arg("/dev/tty").args(arguments).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "unable to configure terminal"))
    }
}

fn restore_tty(saved: &str) -> io::Result<()> {
    let status = Command::new("stty").args(["-F", "/dev/tty", saved]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "unable to restore terminal"))
    }
}

fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut escape = false;
    for character in text.chars() {
        if escape {
            if character.is_ascii_alphabetic() {
                escape = false;
            }
        } else if character == '\x1b' {
            escape = true;
        } else {
            width += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    width
}

fn truncate_ansi(text: &str, width: usize) -> String {
    let mut output = String::new();
    let mut used = 0;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            output.push(character);
            while let Some(control) = chars.next() {
                output.push(control);
                if control.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if used + UnicodeWidthChar::width(character).unwrap_or(0) <= width {
            output.push(character);
            used += UnicodeWidthChar::width(character).unwrap_or(0);
        } else {
            break;
        }
    }
    output.push_str(RESET);
    output
}

fn truncate_plain(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_each_visible_occurrence_and_wraps_in_both_directions() {
        let lines = vec!["\x1b[1mAlpha alpha\x1b[0m".into(), "unrelated".into(), "α alpha".into()];
        let matches = search_matches(&lines, "ALPHA");
        assert_eq!(matches, vec![
            Match { row: 0, start: 0, end: 5 },
            Match { row: 0, start: 6, end: 11 },
            Match { row: 2, start: 3, end: 8 },
        ]);
        assert_eq!(step_match(0, matches.len(), true), 1);
        assert_eq!(step_match(2, matches.len(), true), 0);
        assert_eq!(step_match(0, matches.len(), false), 2);
        assert_eq!(search_matches(&lines, "α"), vec![Match { row: 2, start: 0, end: 2 }]);
        assert!(search_matches(&lines, "missing").is_empty());
    }

    #[test]
    fn highlights_selected_and_other_matches_and_restores_original_colors() {
        let colors = SearchColors { selected_bg: 226, selected_fg: 0, other_bg: 0, other_fg: 15 };
        let original = "\x1b[38;5;39;48;5;236mAlpha alpha\x1b[0m";
        let highlighted = highlight_matches(original, "alpha", Some((0, 5)), colors);
        assert_eq!(plain_text(&highlighted), "Alpha alpha");
        assert!(highlighted.contains("\x1b[22;38;5;0;48;5;226mAlpha\x1b[22m\x1b[38;5;39m\x1b[48;5;236m"));
        assert!(highlighted.contains("\x1b[22;38;5;15;48;5;0malpha\x1b[22m\x1b[38;5;39m\x1b[48;5;236m"));
    }

    #[test]
    fn search_white_is_not_dimmed_by_blockquotes_and_original_intensity_returns() {
        let colors = SearchColors { selected_bg: 226, selected_fg: 0, other_bg: 0, other_fg: 15 };
        let line = "\x1b[1;2m│ text text\x1b[0m";
        let highlighted = highlight_matches(line, "text", Some((9, 13)), colors);
        assert!(highlighted.contains("\x1b[22;38;5;15;48;5;0mtext\x1b[22m\x1b[1m\x1b[2m"));
        assert!(highlighted.contains("\x1b[22;38;5;0;48;5;226mtext\x1b[22m\x1b[1m\x1b[2m"));
    }

    #[test]
    fn reasserts_search_colors_after_embedded_style_changes() {
        let colors = SearchColors { selected_bg: 201, selected_fg: 16, other_bg: 24, other_fg: 231 };
        let line = "\x1b[38;5;39mAl\x1b[38;2;4;5;6mpha\x1b[0m";
        let highlighted = highlight_matches(line, "alpha", Some((0, 5)), colors);
        assert!(highlighted.contains("\x1b[38;2;4;5;6m\x1b[22;38;5;16;48;5;201mpha"));
        assert!(highlighted.contains("pha\x1b[22m\x1b[38;2;4;5;6m\x1b[49m"));
    }
}
