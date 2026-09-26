use std::io::{self, Read, Write};
use std::process::Command;
const RESET: &str = "\x1b[0m";

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
    Top,
    Bottom,
    Quit,
    Edit,
}

struct TerminalGuard {
    saved: String,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_tty(&self.saved);
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub fn run(rendered: &str, editable: bool) -> io::Result<Action> {
    let saved = stty(&["-g"])?;
    stty(&["-icanon", "-echo", "min", "0", "time", "1"])?;
    let _guard = TerminalGuard { saved };
    print!("\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l");
    io::stdout().flush()?;

    let lines: Vec<&str> = rendered.lines().collect();
    let total = lines.len().max(1);
    let mut offset = 0usize;
    let mut stdin = io::stdin();

    loop {
        let (rows, columns) = terminal_size().unwrap_or((24, 80));
        let viewport = rows.saturating_sub(1).max(1);
        let max_offset = total.saturating_sub(viewport);
        offset = offset.min(max_offset);
        draw(&lines, offset, total, viewport, columns.max(2), editable);

        if let Some(key) = read_key(&mut stdin)? {
            match key {
                Key::Up => offset = offset.saturating_sub(1),
                Key::Down => offset = (offset + 1).min(max_offset),
                Key::PageUp => offset = offset.saturating_sub(viewport),
                Key::PageDown => offset = (offset + viewport).min(max_offset),
                Key::Top => offset = 0,
                Key::Bottom => offset = max_offset,
                Key::Quit => return Ok(Action::Done),
                Key::Edit if editable => return Ok(Action::Edit),
                Key::Edit => {}
            }
        }
    }
}

fn draw(lines: &[&str], offset: usize, total: usize, viewport: usize, columns: usize, editable: bool) {
    let content_width = columns.saturating_sub(1).max(1);
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

    let mut screen = String::from("\x1b[2J\x1b[H");
    for row in 0..viewport {
        let line = lines.get(offset + row).copied().unwrap_or("");
        let content = truncate_ansi(line, content_width);
        screen.push_str(&content);
        let used = visible_width(&content);
        screen.push_str(&" ".repeat(content_width.saturating_sub(used)));
        screen.push_str(if row >= thumb_start && row < thumb_start + thumb_size {
            "\x1b[7m▐\x1b[0m"
        } else {
            "\x1b[2m│\x1b[0m"
        });
        screen.push('\n');
    }

    let percent = if max_offset == 0 { 100 } else { (offset * 100 / max_offset).min(100) };
    let edit_hint = if editable { "e edit  " } else { "" };
    let status = format!(" md  {percent:>3}%  {}/{}   ↑/↓ scroll  space/b page  g/G top/bottom  {edit_hint}q quit", offset + 1, total);
    let status = if status.len() > columns {
        status.chars().take(columns).collect::<String>()
    } else {
        format!("{status:<columns$}")
    };
    screen.push_str("\x1b[7m");
    screen.push_str(&status);
    screen.push_str(RESET);
    print!("{screen}");
    let _ = io::stdout().flush();
}

fn read_key(stdin: &mut io::Stdin) -> io::Result<Option<Key>> {
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
        b'g' => Some(Key::Top),
        b'G' => Some(Key::Bottom),
        21 => Some(Key::PageUp),
        27 => read_escape(stdin)?,
        _ => None,
    };
    Ok(key)
}

fn read_escape(stdin: &mut io::Stdin) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if stdin.read(&mut byte)? == 0 {
        return Ok(None);
    }
    if byte[0] == b'[' {
        if stdin.read(&mut byte)? == 0 {
            return Ok(None);
        }
        return Ok(match byte[0] {
            b'A' => Some(Key::Up),
            b'B' => Some(Key::Down),
            b'C' => Some(Key::PageDown),
            b'D' => Some(Key::PageUp),
            b'H' => Some(Key::Top),
            b'F' => Some(Key::Bottom),
            b'5' => {
                let _ = stdin.read(&mut byte)?;
                Some(Key::PageUp)
            }
            b'6' => {
                let _ = stdin.read(&mut byte)?;
                Some(Key::PageDown)
            }
            _ => None,
        });
    }
    Ok(None)
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
            width += 1;
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
        } else if used < width {
            output.push(character);
            used += 1;
        } else {
            break;
        }
    }
    output.push_str(RESET);
    output
}
