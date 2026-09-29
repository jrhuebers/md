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

pub fn run<F>(rendered: &str, editable: bool, poll_speed: usize, scroll_step: usize, mouse_enabled: bool, mut rerender: F) -> io::Result<Action>
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
            draw(&lines, offset, total, viewport, columns.max(2), editable, mouse_enabled, dragging.is_some() || hovered, clear_screen);
            dirty = false;
            clear_screen = false;
        }

        match read_key_timeout(&mut stdin, poll_interval)? {
            Some(key) => match key {
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
                Key::Mouse(event) => {
                    let was_dragging = dragging.is_some();
                    let was_hovered = hovered;
                    let next = handle_mouse(event, columns, viewport, total, offset, &mut dragging, &mut hovered);
                    dirty |= next != offset || was_dragging != dragging.is_some() || was_hovered != hovered;
                    offset = next;
                }
            },
            None => {},
        }
    }
}

fn collect_lines(rendered: &str) -> Vec<String> {
    rendered.lines().map(ToOwned::to_owned).collect()
}

fn read_key_timeout(stdin: &mut File, timeout: Duration) -> io::Result<Option<Key>> {
    let mut fd = PollFd { fd: stdin.as_raw_fd(), events: 1, revents: 0 };
    loop {
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        match unsafe { poll(&mut fd, 1, millis) } {
            0 => return Ok(None),
            n if n > 0 => return read_key(stdin),
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

fn draw(lines: &[String], offset: usize, total: usize, viewport: usize, columns: usize, editable: bool, mouse_enabled: bool, dragging: bool, clear_screen: bool) {
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
        let content = truncate_ansi(line, content_width);
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
    let status = format!(" md  {percent:>3}%  {}/{}   ↑/↓ line  PgUp/PgDn/u/d half  Space/b page  g/G top/bottom  {edit_hint}q quit", offset + 1, total);
    let status = truncate_plain(&status, columns);
    let status = format!("{status:<columns$}");
    screen.push_str("\x1b[7m");
    screen.push_str(&status);
    screen.push_str(RESET);
    print!("{screen}");
    let _ = io::stdout().flush();
}

fn read_key(stdin: &mut File) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if stdin.read(&mut byte)? == 0 {
        return Ok(None);
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
        27 => read_escape(stdin)?,
        _ => None,
    };
    Ok(key)
}

fn read_escape(stdin: &mut File) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if !read_byte_timeout(stdin, &mut byte)? {
        return Ok(None);
    }
    if byte[0] != b'[' {
        return Ok(None);
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
