//! Terminal access shared by the picker and pager on Linux and macOS.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};

pub fn size() -> Option<(u16, u16)> {
    let tty = File::open("/dev/tty").ok()?;
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0 {
        return None;
    }
    (size.ws_row > 0 && size.ws_col > 0).then_some((size.ws_row, size.ws_col))
}

// Both GNU and BSD stty accept terminal settings on standard input; their
// device-selection flags (-F and -f) are not portable between the two.
pub fn stty(arguments: &[&str]) -> io::Result<String> {
    let tty = File::open("/dev/tty")?;
    let output = Command::new("stty").args(arguments).stdin(Stdio::from(tty)).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("stty failed: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn restore_tty(saved: &str) -> io::Result<()> {
    stty(&[saved]).map(|_| ())
}
