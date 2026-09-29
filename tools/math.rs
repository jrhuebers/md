//! Pi's MIT-licensed terminal LaTeX renderer is used through a persistent
//! Node bridge. The Rust fallback keeps md usable when Node is unavailable.

use std::env;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct PiBridge {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

static PI_BRIDGE: OnceLock<Mutex<Option<PiBridge>>> = OnceLock::new();
static RENDER_LATEX: AtomicBool = AtomicBool::new(true);
static RESTART_AFTER_SHUTDOWN: AtomicBool = AtomicBool::new(false);
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: usize = 4096;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub fn set_enabled(enabled: bool) {
    RENDER_LATEX.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    RENDER_LATEX.load(Ordering::Relaxed)
}

pub fn shutdown() {
    if let Some(state) = PI_BRIDGE.get() {
        if let Ok(mut bridge) = state.lock() {
            stop_bridge(&mut bridge);
            RESTART_AFTER_SHUTDOWN.store(true, Ordering::Relaxed);
        }
    }
}

pub fn render_inline(source: &str) -> String {
    if !enabled() {
        return source.to_string();
    }
    pi_render(source, false).unwrap_or_else(|| render_inline_fallback(source))
}

pub fn render_display(source: &str) -> Vec<String> {
    if !enabled() {
        return vec![source.to_string()];
    }
    if let Some(rendered) = pi_render(source, true) {
        return rendered.lines().map(ToOwned::to_owned).collect();
    }
    render_display_fallback(source)
}

fn pi_render(source: &str, display: bool) -> Option<String> {
    let bridge = PI_BRIDGE.get_or_init(|| Mutex::new(start_bridge()));
    let mut state = bridge.lock().ok()?;
    if RESTART_AFTER_SHUTDOWN.swap(false, Ordering::Relaxed) && state.is_none() {
        *state = start_bridge();
    }
    let bridge = state.as_mut()?;
    let request = format!("{{\"display\":{},\"source\":\"{}\"}}\n", display, json_escape(source));
    // Keep writes below the minimum POSIX pipe capacity, even if the helper stalls.
    if request.len() > MAX_REQUEST_BYTES {
        return None;
    }
    if bridge.stdin.write_all(request.as_bytes()).is_err() || bridge.stdin.flush().is_err() {
        stop_bridge(&mut state);
        return None;
    }
    match read_response(&mut bridge.stdout) {
        Ok(response) => parse_result(&response),
        Err(_) => {
            stop_bridge(&mut state);
            None
        }
    }
}

fn start_bridge() -> Option<PiBridge> {
    let helper = env::var_os("MD_PI_MATH_HELPER")
        .map(PathBuf::from)
        .or_else(|| {
            let current = env::current_dir().ok()?.join("tools/pi-math.mjs");
            current.is_file().then_some(current)
        })
        .or_else(|| {
            let executable = env::current_exe().ok()?;
            let path = executable.parent()?.parent()?.join("share/md/tools/pi-math.mjs");
            path.is_file().then_some(path)
        })
        .or_else(|| {
            let home = env::var_os("HOME")?;
            let path = PathBuf::from(home).join("md/tools/pi-math.mjs");
            path.is_file().then_some(path)
        })?;
    let mut child = Command::new("node")
        .arg(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    Some(PiBridge {
        stdin: child.stdin.take()?,
        stdout: child.stdout.take()?,
        child,
    })
}

fn stop_bridge(state: &mut Option<PiBridge>) {
    if let Some(mut bridge) = state.take() {
        let _ = bridge.child.kill();
        let _ = bridge.child.wait();
    }
}

fn read_response(stdout: &mut ChildStdout) -> io::Result<String> {
    let deadline = Instant::now() + BRIDGE_TIMEOUT;
    let mut response = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "math helper did not respond"));
        }
        let mut fd = libc::pollfd { fd: stdout.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let timeout = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut fd, 1, timeout) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            continue;
        }
        if fd.revents & libc::POLLIN == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "math helper closed its output"));
        }
        let mut chunk = [0; 4096];
        let count = stdout.read(&mut chunk)?;
        if count == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "math helper closed its output"));
        }
        response.extend_from_slice(&chunk[..count]);
        if response.len() > MAX_RESPONSE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "math helper response too large"));
        }
        if let Some(end) = response.iter().position(|byte| *byte == b'\n') {
            return String::from_utf8(response[..end].to_vec())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
    }
}

fn json_escape(value: &str) -> String {
    value.chars().fold(String::new(), |mut output, character| {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
        output
    })
}

fn parse_result(response: &str) -> Option<String> {
    let marker = "\"result\":";
    let start = response.find(marker)? + marker.len();
    let value = response[start..].trim_start();
    if value.starts_with("null") {
        return None;
    }
    let mut chars = value.strip_prefix('"')?.chars();
    let mut result = String::new();
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(result),
            '\\' => match chars.next()? {
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                '\\' => result.push('\\'),
                '"' => result.push('"'),
                _ => return None,
            },
            character => result.push(character),
        }
    }
    None
}

fn render_inline_fallback(source: &str) -> String {
    normalize(&Parser::new(source).parse_all())
}

fn render_display_fallback(source: &str) -> Vec<String> {
    let source = source.trim();
    if let Some(lines) = render_matrix(source) {
        return lines;
    }
    if let Some((before, numerator, denominator, after)) = split_fraction(source) {
        let before = render_inline_fallback(before);
        let numerator = render_inline_fallback(numerator);
        let denominator = render_inline_fallback(denominator);
        let after = render_inline_fallback(after);
        let fraction_width = numerator.chars().count().max(denominator.chars().count()).max(1) + 2;
        let numerator = center(&numerator, fraction_width);
        let denominator = center(&denominator, fraction_width);
        let rule = format!(" {} ", "─".repeat(fraction_width - 2));
        let left = before.chars().count();
        let right = after.chars().count();
        let mut lines = Vec::new();
        lines.push(format!("{before}{numerator}{after}"));
        lines.push(format!("{}{}{}", " ".repeat(left), rule, " ".repeat(right)));
        lines.push(format!("{before}{denominator}{after}"));
        return lines;
    }
    vec![render_inline_fallback(source)]
}

fn center(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(value.chars().count());
    let left = padding / 2;
    format!("{}{}{}", " ".repeat(left), value, " ".repeat(padding - left))
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn split_fraction(source: &str) -> Option<(&str, &str, &str, &str)> {
    let start = source.find("\\frac")?;
    let mut position = start + 5;
    let numerator = group_slice(source, &mut position)?;
    let denominator = group_slice(source, &mut position)?;
    Some((&source[..start], numerator, denominator, &source[position..]))
}

fn group_slice<'a>(source: &'a str, position: &mut usize) -> Option<&'a str> {
    while source.as_bytes().get(*position).is_some_and(|byte| byte.is_ascii_whitespace()) {
        *position += 1;
    }
    if source.as_bytes().get(*position) != Some(&b'{') {
        return None;
    }
    let start = *position + 1;
    *position += 1;
    let mut depth = 1;
    while *position < source.len() {
        match source.as_bytes()[*position] {
            b'\\' => *position += 2,
            b'{' => {
                depth += 1;
                *position += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let end = *position;
                    *position += 1;
                    return Some(&source[start..end]);
                }
                *position += 1;
            }
            _ => *position += 1,
        }
    }
    None
}

fn render_matrix(source: &str) -> Option<Vec<String>> {
    let environments = [
        ("pmatrix", "⎛", "⎞", "⎜", "⎟", "⎝", "⎠"),
        ("bmatrix", "⎡", "⎤", "⎢", "⎥", "⎣", "⎦"),
        ("Bmatrix", "⎧", "⎫", "⎨", "⎬", "⎩", "⎭"),
        ("vmatrix", "│", "│", "│", "│", "│", "│"),
        ("matrix", "", "", "", "", "", ""),
        ("cases", "⎧", "", "⎨", "", "⎩", ""),
    ];
    for (name, top_left, top_right, middle_left, middle_right, bottom_left, bottom_right) in environments {
        let begin = format!("\\begin{{{name}}}");
        let end = format!("\\end{{{name}}}");
        let Some(start) = source.find(&begin) else { continue };
        let body_start = start + begin.len();
        let end_pos = source[body_start..].find(&end).map(|offset| body_start + offset)?;
        let body = &source[body_start..end_pos];
        let rows: Vec<Vec<String>> = body
            .split("\\\\")
            .map(|row| row.split('&').map(render_inline_fallback).collect())
            .filter(|row: &Vec<String>| row.iter().any(|cell| !cell.is_empty()))
            .collect();
        if rows.is_empty() {
            return Some(vec![String::new()]);
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|column| rows.iter().map(|row| row.get(column).map_or(0, |cell| cell.chars().count())).max().unwrap_or(0))
            .collect();
        let mut lines = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let cells = (0..columns)
                .map(|column| {
                    let cell = row.get(column).cloned().unwrap_or_default();
                    format!("{}{}", cell, " ".repeat(widths[column].saturating_sub(cell.chars().count())))
                })
                .collect::<Vec<_>>()
                .join(" │ ");
            let (left, right) = if name == "matrix" {
                ("", "")
            } else if index == 0 {
                (top_left, top_right)
            } else if index + 1 == rows.len() {
                (bottom_left, bottom_right)
            } else {
                (middle_left, middle_right)
            };
            lines.push(format!("{left} {cells} {right}").trim().to_string());
        }
        return Some(lines);
    }
    None
}

struct Parser {
    chars: Vec<char>,
    position: usize,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self { chars: source.chars().collect(), position: 0 }
    }

    fn parse_all(mut self) -> String {
        self.parse_until(None)
    }

    fn parse_until(&mut self, closing: Option<char>) -> String {
        let mut output = String::new();
        while self.position < self.chars.len() {
            let character = self.chars[self.position];
            if Some(character) == closing {
                self.position += 1;
                break;
            }
            match character {
                '{' => {
                    self.position += 1;
                    output.push_str(&self.parse_until(Some('}')));
                }
                '^' | '_' => {
                    self.position += 1;
                    let value = self.parse_atom();
                    let kind = if character == '^' { "sup" } else { "sub" };
                    output.push_str(&script(&value, kind));
                }
                '\\' => output.push_str(&self.parse_command()),
                character if character.is_whitespace() => {
                    self.position += 1;
                    if !output.ends_with(' ') {
                        output.push(' ');
                    }
                }
                _ => {
                    self.position += 1;
                    output.push(character);
                }
            }
        }
        output
    }

    fn parse_atom(&mut self) -> String {
        while self.position < self.chars.len() && self.chars[self.position].is_whitespace() {
            self.position += 1;
        }
        if self.position >= self.chars.len() {
            return String::new();
        }
        if self.chars[self.position] == '{' {
            self.position += 1;
            return self.parse_until(Some('}'));
        }
        if self.chars[self.position] == '\\' {
            return self.parse_command();
        }
        let character = self.chars[self.position];
        self.position += 1;
        character.to_string()
    }

    fn parse_command(&mut self) -> String {
        self.position += 1;
        if self.position >= self.chars.len() {
            return "\\".to_string();
        }
        if !self.chars[self.position].is_ascii_alphabetic() {
            let character = self.chars[self.position];
            self.position += 1;
            return match character {
                ',' | ';' | ':' | '!' => String::new(),
                '%' => "%".to_string(),
                '{' | '}' | '_' | '#' | '$' | '&' => character.to_string(),
                _ => character.to_string(),
            };
        }
        let start = self.position;
        while self.position < self.chars.len() && self.chars[self.position].is_ascii_alphabetic() {
            self.position += 1;
        }
        let command: String = self.chars[start..self.position].iter().collect();
        match command.as_str() {
            "frac" => {
                let numerator = self.parse_atom();
                let denominator = self.parse_atom();
                format_fraction(&numerator, &denominator)
            }
            "sqrt" => {
                let value = self.parse_atom();
                format_root(&value)
            }
            "text" | "mbox" | "mathrm" | "mathbf" | "mathit" | "mathcal" | "mathfrak" | "operatorname" => self.parse_atom(),
            "mathbb" => blackboard(&self.parse_atom()),
            "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "sinh" | "cosh" | "tanh" | "log" | "ln" | "exp" | "lim" | "min" | "max" | "det" | "ker" => command,
            "left" | "right" | "limits" | "nolimits" | "displaystyle" | "textstyle"
            | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "Bigl" | "biggl" | "Biggl" | "bigr" | "Bigr" | "biggr" | "Biggr" => String::new(),
            "," | ";" | ":" | "!" => String::new(),
            "overline" | "bar" => accent(self.parse_atom(), '\u{0305}'),
            "hat" | "widehat" => accent(self.parse_atom(), '\u{0302}'),
            "tilde" | "widetilde" => accent(self.parse_atom(), '\u{0303}'),
            "vec" | "overrightarrow" => accent(self.parse_atom(), '\u{20d7}'),
            _ => command_symbol(&command).unwrap_or_else(|| format!("\\{command}")),
        }
    }
}

fn format_fraction(numerator: &str, denominator: &str) -> String {
    let numerator = normalize(numerator);
    let denominator = normalize(denominator);
    if numerator.chars().all(|character| character.is_alphanumeric() || ".".contains(character))
        && denominator.chars().all(|character| character.is_alphanumeric() || ".".contains(character))
    {
        format!("{numerator}/{denominator}")
    } else {
        format!("({numerator})/({denominator})")
    }
}

fn format_root(value: &str) -> String {
    let value = normalize(value);
    if value.chars().count() == 1 || value.chars().all(|character| character.is_alphanumeric() || character == '.') {
        format!("√{value}")
    } else {
        format!("√({value})")
    }
}

fn accent(value: String, mark: char) -> String {
    let value = normalize(&value);
    if value.chars().count() == 1 {
        format!("{value}{mark}")
    } else {
        value.chars().map(|character| format!("{character}{mark}")).collect()
    }
}

fn script(value: &str, kind: &str) -> String {
    let mut result = String::new();
    let map = if kind == "sup" { superscript } else { subscript };
    for character in normalize(value).chars() {
        if let Some(mapped) = map(character) {
            result.push(mapped);
        } else {
            return format!("{}({})", if kind == "sup" { "^" } else { "_" }, value);
        }
    }
    result
}

fn superscript(character: char) -> Option<char> {
    Some(match character {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵',
        '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹', '+' => '⁺', '-' => '⁻',
        '=' => '⁼', '(' => '⁽', ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ', _ => return None,
    })
}

fn subscript(character: char) -> Option<char> {
    Some(match character {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅',
        '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉', '+' => '₊', '-' => '₋',
        '=' => '₌', '(' => '₍', ')' => '₎', 'a' => 'ₐ', 'e' => 'ₑ', 'h' => 'ₕ',
        'i' => 'ᵢ', 'j' => 'ⱼ', 'k' => 'ₖ', 'l' => 'ₗ', 'm' => 'ₘ', 'n' => 'ₙ',
        'o' => 'ₒ', 'p' => 'ₚ', 'r' => 'ᵣ', 's' => 'ₛ', 't' => 'ₜ', 'x' => 'ₓ', _ => return None,
    })
}

fn blackboard(value: &str) -> String {
    value.chars().map(|character| match character {
        'C' => 'ℂ', 'H' => 'ℍ', 'N' => 'ℕ', 'P' => 'ℙ', 'Q' => 'ℚ', 'R' => 'ℝ', 'Z' => 'ℤ', _ => character,
    }).collect()
}

fn command_symbol(command: &str) -> Option<String> {
    let symbol = match command {
        "alpha" => 'α', "beta" => 'β', "gamma" => 'γ', "delta" => 'δ', "epsilon" => 'ε',
        "varepsilon" => 'ϵ', "zeta" => 'ζ', "eta" => 'η', "theta" => 'θ', "vartheta" => 'ϑ',
        "iota" => 'ι', "kappa" => 'κ', "lambda" => 'λ', "mu" => 'μ', "nu" => 'ν', "xi" => 'ξ',
        "pi" => 'π', "varpi" => 'ϖ', "rho" => 'ρ', "sigma" => 'σ', "tau" => 'τ', "upsilon" => 'υ',
        "phi" => 'φ', "varphi" => 'ϕ', "chi" => 'χ', "psi" => 'ψ', "omega" => 'ω',
        "Gamma" => 'Γ', "Delta" => 'Δ', "Theta" => 'Θ', "Lambda" => 'Λ', "Xi" => 'Ξ',
        "Pi" => 'Π', "Sigma" => 'Σ', "Upsilon" => 'Υ', "Phi" => 'Φ', "Psi" => 'Ψ', "Omega" => 'Ω',
        "pm" => '±', "mp" => '∓', "times" => '×', "div" => '÷', "cdot" => '·', "ast" => '∗', "star" => '⋆', "circ" => '∘', "bullet" => '•',
        "oplus" => '⊕', "ominus" => '⊖', "otimes" => '⊗', "oslash" => '⊘', "odot" => '⊙', "bigcirc" => '○',
        "le" | "leq" | "leqslant" => '≤', "ge" | "geq" | "geqslant" => '≥', "ne" | "neq" => '≠', "approx" => '≈',
        "equiv" => '≡', "sim" => '∼', "simeq" => '≃', "cong" => '≅', "propto" => '∝', "parallel" => '∥', "perp" => '⊥',
        "ll" => '≪', "gg" => '≫', "in" => '∈', "notin" => '∉', "ni" => '∋', "subset" => '⊂', "subseteq" => '⊆',
        "supset" => '⊃', "supseteq" => '⊇', "to" | "rightarrow" | "longrightarrow" => '→', "leftarrow" | "longleftarrow" => '←',
        "leftrightarrow" | "longleftrightarrow" => '↔', "Rightarrow" | "Longrightarrow" => '⇒', "Leftarrow" | "Longleftarrow" => '⇐', "Leftrightarrow" | "Longleftrightarrow" => '⇔',
        "hookleftarrow" => '↩', "hookrightarrow" => '↪', "mapsto" => '↦', "uparrow" => '↑', "downarrow" => '↓',
        "infty" => '∞', "partial" => '∂', "nabla" => '∇', "forall" => '∀', "exists" => '∃', "neg" => '¬',
        "land" | "wedge" => '∧', "lor" | "vee" => '∨', "cup" => '∪', "cap" => '∩', "bigcap" => '⋂', "bigcup" => '⋃', "bigwedge" => '⋀', "bigvee" => '⋁',
        "sum" => '∑', "prod" => '∏', "int" => '∫', "iint" => '∬', "iiint" => '∭', "oint" => '∮',
        "therefore" => '∴', "because" => '∵', "angle" => '∠', "degree" => '°', "hbar" => 'ℏ',
        "ldots" | "dots" => '…', "cdots" => '⋯', "vdots" => '⋮', "ddots" => '⋱', "ell" => 'ℓ', "Re" => 'ℜ', "Im" => 'ℑ',
        "langle" => '⟨', "rangle" => '⟩', "lfloor" => '⌊', "rfloor" => '⌋', "lceil" => '⌈', "rceil" => '⌉', _ => return None,
    };
    Some(symbol.to_string())
}
