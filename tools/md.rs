use std::env;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

mod math;
mod pager;

// The built-in defaults mirror Glamour's LightStyle and DarkStyle, which Glow uses.
#[derive(Clone)]
struct Theme {
    normal_fg: u8,
    heading_fg: u8,
    h1_fg: u8,
    h1_bg: u8,
    rule_fg: u8,
    link_fg: u8,
    link_text_fg: u8,
    inline_code_fg: u8,
    inline_code_bg: u8,
    code_block_fg: u8,
    margin_left: usize,
    margin_right: usize,
}

impl Theme {
    fn glow_light() -> Self {
        Self {
            normal_fg: 234,
            heading_fg: 27,
            h1_fg: 228,
            h1_bg: 63,
            rule_fg: 249,
            link_fg: 36,
            link_text_fg: 29,
            inline_code_fg: 203,
            inline_code_bg: 254,
            code_block_fg: 242,
            margin_left: 1,
            margin_right: 1,
        }
    }

    fn glow_dark() -> Self {
        Self {
            normal_fg: 252,
            heading_fg: 39,
            h1_fg: 228,
            h1_bg: 63,
            rule_fg: 240,
            link_fg: 30,
            link_text_fg: 35,
            inline_code_fg: 203,
            inline_code_bg: 236,
            code_block_fg: 244,
            margin_left: 1,
            margin_right: 1,
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        let parsed = match value.parse::<u8>() {
            Ok(value) => value,
            Err(_) => return,
        };
        match key {
            "normal_fg" => self.normal_fg = parsed,
            "heading_fg" => self.heading_fg = parsed,
            "h1_fg" => self.h1_fg = parsed,
            "h1_bg" => self.h1_bg = parsed,
            "rule_fg" => self.rule_fg = parsed,
            "link_fg" => self.link_fg = parsed,
            "link_text_fg" => self.link_text_fg = parsed,
            "inline_code_fg" => self.inline_code_fg = parsed,
            "inline_code_bg" => self.inline_code_bg = parsed,
            "code_block_fg" => self.code_block_fg = parsed,
            "margin_left" => self.margin_left = parsed as usize,
            "margin_right" => self.margin_right = parsed as usize,
            _ => {}
        }
    }
}

struct Config {
    style: String,
    width: usize,
    render_latex: bool,
    pager_scroll_speed: usize,
    themes: HashMap<String, Theme>,
}

impl Config {
    fn default() -> Self {
        let mut themes = HashMap::new();
        themes.insert("glow-light".to_string(), Theme::glow_light());
        themes.insert("glow-dark".to_string(), Theme::glow_dark());
        Self { style: "glow-light".to_string(), width: 0, render_latex: true, pager_scroll_speed: 60, themes }
    }

    fn theme(self) -> io::Result<(Theme, usize, bool, usize)> {
        let theme = self.themes.get(&self.style).cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("unknown md style: {}", self.style))
        })?;
        Ok((theme, self.width, self.render_latex, self.pager_scroll_speed))
    }
}

const PICKER_H1_FG: u8 = 228;
const PICKER_H1_BG: u8 = 63;
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ITALIC: &str = "\x1b[3m";
const UNDERLINE: &str = "\x1b[4m";
const DIM: &str = "\x1b[2m";

fn main() {
    let config = match load_config().and_then(Config::theme) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("md: {error}");
            std::process::exit(2);
        }
    };
    let (theme, configured_width, render_latex, pager_scroll_speed) = config;
    math::set_enabled(render_latex);
    let width = if configured_width == 0 {
        terminal_columns().unwrap_or(80) as usize
    } else {
        configured_width
    };
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("Usage: md [FILE ...]\n\nRender Markdown and read it in a pager. Use - for standard input.\nWhen given a directory, select a Markdown file interactively.");
        return;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("md 0.6.18");
        return;
    }

    let paths = match choose_paths(&args) {
        Ok(Some(paths)) => paths,
        Ok(None) => return,
        Err(error) => {
            eprintln!("md: {error}");
            std::process::exit(2);
        }
    };
    let editable_path = if paths.len() == 1 && paths[0] != "-" {
        Some(PathBuf::from(&paths[0]))
    } else {
        None
    };
    loop {
        let input = match read_input(&paths) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("md: {error}");
                std::process::exit(2);
            }
        };
        let render_width = if env::var_os("PAGER").is_none() { width.saturating_sub(1) } else { width };
        let rendered = render_markdown(&input, &theme, render_width);
        match page(&rendered, editable_path.as_deref(), pager_scroll_speed, |new_width| {
            render_markdown(&input, &theme, new_width)
        }) {
            Ok(PageAction::Done) => break,
            Ok(PageAction::Edit) => {
                if let Some(path) = editable_path.as_deref() {
                    if let Err(error) = run_editor(path) {
                        eprintln!("md: {error}");
                        std::process::exit(1);
                    }
                }
            }
            Err(error) => {
                eprintln!("md: {error}");
                std::process::exit(1);
            }
        }
    }
}

fn load_config() -> io::Result<Config> {
    let path = if let Ok(directory) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(directory).join("md.yaml")
    } else {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/md.yaml")
    };
    if !path.exists() {
        return Ok(Config::default());
    }

    let contents = fs::read_to_string(&path)?;
    parse_config(&contents)
}

fn parse_config(contents: &str) -> io::Result<Config> {
    let mut config = Config::default();
    let mut in_styles = false;
    let mut current_style: Option<String> = None;

    for raw_line in contents.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|character| *character == ' ').count();
        let content = line.trim();
        if indent == 0 {
            current_style = None;
            if let Some(value) = content.strip_prefix("style:") {
                config.style = value.trim().trim_matches(['"', '\'']).to_string();
            } else if let Some(value) = content.strip_prefix("width:") {
                config.width = value.trim().parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "md.yaml width must be an integer")
                })?;
            } else if let Some(value) = content.strip_prefix("render_latex:") {
                config.render_latex = value.trim().parse::<bool>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "md.yaml render_latex must be true or false")
                })?;
            } else if let Some(value) = content.strip_prefix("pager_scroll_speed:") {
                config.pager_scroll_speed = value.trim().parse::<usize>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "md.yaml pager_scroll_speed must be a positive integer")
                })?;
                if config.pager_scroll_speed == 0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "md.yaml pager_scroll_speed must be a positive integer"));
                }
            } else if content == "styles:" {
                in_styles = true;
            }
            continue;
        }
        if in_styles && indent == 2 && content.ends_with(':') {
            let name = content.trim_end_matches(':').trim().to_string();
            config.themes.entry(name.clone()).or_insert_with(Theme::glow_light);
            current_style = Some(name);
            continue;
        }
        if in_styles && indent >= 4 {
            if let (Some(name), Some((key, value))) = (current_style.as_ref(), content.split_once(':')) {
                if let Some(theme) = config.themes.get_mut(name) {
                    theme.set(key.trim(), value.trim());
                }
            }
        }
    }
    Ok(config)
}

fn choose_paths(args: &[String]) -> io::Result<Option<Vec<String>>> {
    if args.len() == 1 {
        let candidate = Path::new(&args[0]);
        if candidate.is_dir() {
            return select_paths(candidate);
        }
    }
    if args.is_empty() && io::stdin().is_terminal() {
        return select_paths(Path::new("."));
    }
    Ok(Some(args.to_vec()))
}

fn read_input(paths: &[String]) -> io::Result<String> {
    if paths.is_empty() {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        return Ok(input);
    }

    let mut combined = String::new();
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            combined.push_str("\n\n");
        }
        if path == "-" {
            io::stdin().read_to_string(&mut combined)?;
        } else {
            combined.push_str(&fs::read_to_string(path)?);
        }
    }
    Ok(combined)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    List,
    Quote,
}

fn render_markdown(input: &str, theme: &Theme, width: usize) -> String {
    let mut output = String::with_capacity(input.len() + input.len() / 8);
    let mut paragraph: Vec<String> = Vec::new();
    let mut in_code = false;
    let mut math_block: Option<(String, String, String)> = None;
    let mut previous_block: Option<BlockKind> = None;
    let mut suppress_blank = false;

    for raw_line in input.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim_start();

        if is_fence(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            in_code = !in_code;
            continue;
        }
        if in_code {
            previous_block = None;
            let mut rendered = String::from("  ");
            rendered.push_str(&fg(theme.code_block_fg));
            rendered.push_str(line);
            rendered.push_str(RESET);
            push_line(&mut output, &rendered, theme);
            continue;
        }
        if let Some((_, closing, _)) = math_block.as_ref() {
            previous_block = None;
            let closing = closing.clone();
            if let Some(end) = line.find(closing.as_str()) {
                if let Some((opening, _, mut body)) = math_block.take() {
                    body.push_str(&line[..end]);
                    push_math_display(&mut output, &opening, &closing, &body, theme, width);
                }
            } else if let Some((_, _, body)) = math_block.as_mut() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(line);
            }
            continue;
        }
        if line.trim().is_empty() {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            if suppress_blank {
                suppress_blank = false;
            } else {
                ensure_blank_line(&mut output, theme);
            }
            continue;
        }
        suppress_blank = false;
        if let Some((opening, closing)) = display_math_delimiter(trimmed) {
            previous_block = None;
            let body_start = opening.len();
            let rest = &trimmed[body_start..];
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            if let Some(end) = rest.find(closing) {
                push_math_display(&mut output, opening, closing, &rest[..end], theme, width);
            } else {
                math_block = Some((opening.to_string(), closing.to_string(), rest.to_string()));
            }
            continue;
        }
        if let Some((level, heading)) = heading(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            ensure_blank_line(&mut output, theme);
            let mut rendered = if level == 1 {
                style(theme.h1_fg, Some(theme.h1_bg), true, false, false)
            } else {
                style(theme.heading_fg, None, true, false, false)
            };
            if level == 1 {
                rendered.push(' ');
            } else {
                // Glow keeps the Markdown heading marker for H2 through H6.
                rendered.push_str(&"#".repeat(level));
                rendered.push(' ');
            }
            rendered.push_str(&render_inline(heading.trim(), if level == 1 { theme.h1_fg } else { theme.heading_fg }, theme));
            if level == 1 {
                rendered.push(' ');
            }
            rendered.push_str(RESET);
            push_line(&mut output, &rendered, theme);
            push_line(&mut output, "", theme);
            suppress_blank = true;
            continue;
        }
        if is_rule(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            let rendered = format!("{}────────────────────────────────────────{}", fg(theme.rule_fg), RESET);
            push_line(&mut output, &rendered, theme);
            continue;
        }
        if let Some((depth, marker, content)) = list_item(line) {
            if previous_block != Some(BlockKind::List) {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
                ensure_blank_line(&mut output, theme);
            } else {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
            }
            previous_block = Some(BlockKind::List);
            let prefix = format!("{}{}", "  ".repeat(depth), marker);
            let prefix_width = prefix.chars().count();
            let available = width.saturating_sub(theme.margin_left + theme.margin_right + prefix_width).max(1);
            for (index, chunk) in wrap_text(content, available).iter().enumerate() {
                let line_prefix = if index == 0 {
                    prefix.clone()
                } else {
                    " ".repeat(prefix_width)
                };
                let rendered = format!("{}{}{}", fg(theme.normal_fg), line_prefix, render_inline(chunk, theme.normal_fg, theme));
                push_line(&mut output, &format!("{}{}", rendered, RESET), theme);
            }
            continue;
        }
        if let Some(content) = trimmed.strip_prefix("> ").or_else(|| trimmed.strip_prefix('>')) {
            if previous_block != Some(BlockKind::Quote) {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
                ensure_blank_line(&mut output, theme);
            } else {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
            }
            previous_block = Some(BlockKind::Quote);
            let prefix_width = 2;
            let available = width.saturating_sub(theme.margin_left + theme.margin_right + prefix_width).max(1);
            for chunk in wrap_text(content.trim(), available).iter() {
                let prefix = "│ ";
                let rendered = format!("{}{}{}{}", fg(theme.normal_fg), DIM, prefix, render_inline(chunk, theme.normal_fg, theme));
                push_line(&mut output, &format!("{}{}", rendered, RESET), theme);
            }
            continue;
        }
        if previous_block.is_some() {
            ensure_blank_line(&mut output, theme);
            previous_block = None;
        }
        paragraph.push(line.trim().to_string());
    }

    flush_paragraph(&mut paragraph, &mut output, theme, width);
    if let Some((opening, closing, body)) = math_block {
        push_math_display(&mut output, &opening, &closing, &body, theme, width);
    }
    output
}

fn display_math_delimiter(line: &str) -> Option<(&str, &str)> {
    if line.starts_with("$$") {
        Some(("$$", "$$"))
    } else if line.starts_with("\\[") {
        Some(("\\[", "\\]"))
    } else {
        None
    }
}

fn push_math_display(output: &mut String, opening: &str, closing: &str, source: &str, theme: &Theme, width: usize) {
    ensure_blank_line(output, theme);
    let lines: Vec<String> = if math::enabled() {
        math::render_display(source)
    } else {
        format!("{opening}{source}{closing}").lines().map(ToOwned::to_owned).collect()
    };
    let content_width = width.saturating_sub(theme.margin_left + theme.margin_right);
    let block_width = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
    let padding = content_width.saturating_sub(block_width) / 2;
    for line in lines {
        let rendered = format!("{}{}{}{}", fg(theme.normal_fg), " ".repeat(padding), line, RESET);
        push_line(output, &rendered, theme);
    }
    ensure_blank_line(output, theme);
}

fn flush_paragraph(paragraph: &mut Vec<String>, output: &mut String, theme: &Theme, width: usize) {
    if paragraph.is_empty() {
        return;
    }
    let joined = join_paragraph(paragraph);
    let available = width.saturating_sub(theme.margin_left + theme.margin_right).max(1);
    for chunk in wrap_text(&joined, available) {
        let rendered = format!("{}{}{}", fg(theme.normal_fg), render_inline(&chunk, theme.normal_fg, theme), RESET);
        push_line(output, &rendered, theme);
    }
    paragraph.clear();
}

fn join_paragraph(lines: &[String]) -> String {
    let mut joined = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let starts_with_punctuation = line.chars().next().is_some_and(|character| ",.;:!?)]}".contains(character));
        if !joined.is_empty() && !starts_with_punctuation {
            joined.push(' ');
        }
        joined.push_str(line);
    }
    joined
}

fn push_line(output: &mut String, content: &str, theme: &Theme) {
    output.push_str(&" ".repeat(theme.margin_left));
    output.push_str(content);
    output.push_str(&" ".repeat(theme.margin_right));
    output.push('\n');
}

fn ensure_blank_line(output: &mut String, theme: &Theme) {
    let blank = format!("{}{}\n", " ".repeat(theme.margin_left), " ".repeat(theme.margin_right));
    if !output.is_empty() && !output.ends_with(&blank) {
        output.push_str(&blank);
    }
}

struct WrapToken {
    text: String,
    width: usize,
    code: bool,
    glued: bool,
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut code_open = false;
    for token in paragraph_tokens(text) {
        let separator_width = usize::from(!current.is_empty() && !token.glued);
        if !current.is_empty() && current_width + separator_width + token.width > width {
            if code_open {
                current.push('`');
                code_open = false;
            }
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if token.code {
            if !current.is_empty() && !token.glued {
                current.push(' ');
                current_width += 1;
            }
            if !code_open {
                current.push('`');
                code_open = true;
            }
            current.push_str(&token.text);
            current_width += token.width;
        } else {
            if code_open {
                current.push('`');
                code_open = false;
            }
            if !current.is_empty() && !token.glued {
                current.push(' ');
                current_width += 1;
            }
            current.push_str(&token.text);
            current_width += token.width;
        }
    }
    if code_open {
        current.push('`');
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn paragraph_tokens(text: &str) -> Vec<WrapToken> {
    let mut tokens = Vec::new();
    let mut position = 0;
    while position < text.len() {
        let whitespace_start = position;
        while position < text.len() && text.as_bytes()[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= text.len() {
            break;
        }
        let glued = !tokens.is_empty() && position == whitespace_start;
        let start = position;
        if text[position..].starts_with('`') {
            if let Some(end) = text[position + 1..].find('`') {
                let end = position + 1 + end;
                let content = &text[position + 1..end];
                let mut parts = content.split_whitespace().peekable();
                if parts.peek().is_none() {
                    tokens.push(WrapToken { text: String::new(), width: 0, code: true, glued });
                } else {
                    let mut first = true;
                    for part in parts {
                        tokens.push(WrapToken {
                            text: part.to_string(),
                            width: part.chars().count(),
                            code: true,
                            glued: if first { glued } else { false },
                        });
                        first = false;
                    }
                }
                position = end + 1;
                let punctuation_start = position;
                consume_punctuation(text, &mut position);
                if position > punctuation_start {
                    tokens.push(WrapToken {
                        text: text[punctuation_start..position].to_string(),
                        width: position - punctuation_start,
                        code: false,
                        glued: true,
                    });
                }
                continue;
            }
        }
        if text[position..].starts_with('$') && !text[position..].starts_with("$$") {
            if let Some(end) = find_unescaped(text, position + 1, '$') {
                position = end + 1;
                consume_punctuation(text, &mut position);
                let word = &text[start..position];
                tokens.push(WrapToken { text: word.to_string(), width: rendered_word_width(word), code: false, glued });
                continue;
            }
        } else if text[position..].starts_with("\\(") {
            if let Some(end) = text[position + 2..].find("\\)") {
                position += 2 + end + 2;
                consume_punctuation(text, &mut position);
                let word = &text[start..position];
                tokens.push(WrapToken { text: word.to_string(), width: rendered_word_width(word), code: false, glued });
                continue;
            }
        }
        while position < text.len() && !text.as_bytes()[position].is_ascii_whitespace() {
            position += 1;
        }
        let word = &text[start..position];
        tokens.push(WrapToken { text: word.to_string(), width: word.chars().count(), code: false, glued });
    }
    tokens
}

fn rendered_word_width(word: &str) -> usize {
    if word.starts_with('$') && !word.starts_with("$$") {
        if let Some(end) = find_unescaped(word, 1, '$') {
            return math::render_inline(&word[1..end]).chars().count() + word[end + 1..].chars().count();
        }
    } else if word.starts_with("\\(") {
        if let Some(end) = word[2..].find("\\)") {
            let end = end + 2;
            return math::render_inline(&word[2..end]).chars().count() + word[end + 2..].chars().count();
        }
    }
    word.chars().count()
}

fn consume_punctuation(text: &str, position: &mut usize) {
    while *position < text.len() && matches!(text.as_bytes()[*position], b'.' | b',' | b';' | b':' | b'!' | b'?') {
        *position += 1;
    }
}

fn find_unescaped(text: &str, start: usize, delimiter: char) -> Option<usize> {
    let mut search = start;
    while let Some(offset) = text[search..].find(delimiter) {
        let position = search + offset;
        let backslashes = text[..position].chars().rev().take_while(|character| *character == '\\').count();
        if backslashes % 2 == 0 {
            return Some(position);
        }
        search = position + delimiter.len_utf8();
    }
    None
}

fn fg(color: u8) -> String {
    format!("\x1b[38;5;{color}m")
}

fn style(foreground: u8, background: Option<u8>, bold: bool, italic: bool, underline: bool) -> String {
    let mut result = fg(foreground);
    if let Some(background) = background {
        result.push_str(&format!("\x1b[48;5;{background}m"));
    }
    if bold {
        result.push_str(BOLD);
    }
    if italic {
        result.push_str(ITALIC);
    }
    if underline {
        result.push_str(UNDERLINE);
    }
    result
}

fn restore(foreground: u8) -> String {
    format!("{RESET}{}", fg(foreground))
}

fn is_fence(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|character| *character == '#').count();
    if (1..=6).contains(&level) && line.chars().nth(level) == Some(' ') {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|character| !character.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|character| character == '-')
            || compact.chars().all(|character| character == '*')
            || compact.chars().all(|character| character == '_'))
}

fn list_item(line: &str) -> Option<(usize, &str, &str)> {
    let indent = line.chars().take_while(|character| *character == ' ' || *character == '\t').count();
    let content = &line[indent..];
    let depth = indent.saturating_add(1) / 2;
    if let Some(item) = content.strip_prefix("- ").or_else(|| content.strip_prefix("* ")).or_else(|| content.strip_prefix("+ ")) {
        return Some((depth, "• ", item));
    }
    let dot = content.find(". ")?;
    if dot > 0 && content[..dot].chars().all(|character| character.is_ascii_digit()) {
        return Some((depth, &content[..dot + 2], &content[dot + 2..]));
    }
    None
}

fn inline_math_at(input: &str, index: usize) -> Option<(&str, usize)> {
    let rest = &input[index..];
    let (opening, closing) = if rest.starts_with("\\(") {
        ("\\(", "\\)")
    } else if rest.starts_with('$')
        && !rest.starts_with("$$")
        && !rest[1..].chars().next().is_some_and(|character| character.is_whitespace())
    {
        ("$", "$")
    } else {
        return None;
    };
    let start = index + opening.len();
    let end = input[start..].find(closing)? + start;
    if end == start || input[start..end].contains('\n') {
        return None;
    }
    Some((&input[start..end], end + closing.len() - index))
}

fn underscore_in_word(input: &str, index: usize) -> bool {
    let previous = input[..index].chars().next_back();
    let next = input[index + 1..].chars().next();
    previous.is_some_and(|character| character.is_alphanumeric())
        && next.is_some_and(|character| character.is_alphanumeric())
}

fn render_inline(input: &str, base_foreground: u8, theme: &Theme) -> String {
    let mut output = String::with_capacity(input.len() + 16);
    output.push_str(&fg(base_foreground));
    let mut index = 0;
    while index < input.len() {
        let rest = &input[index..];
        if let Some((source, consumed)) = inline_math_at(input, index) {
            if math::enabled() {
                output.push_str(&math::render_inline(source));
            } else {
                output.push_str(&input[index..index + consumed]);
            }
            index += consumed;
            continue;
        }
        if rest.starts_with("**") || rest.starts_with("__") {
            if rest.starts_with("__") && underscore_in_word(input, index) {
                output.push_str("__");
                index += 2;
                continue;
            }
            let marker = &input[index..index + 2];
            if let Some(end) = input[index + 2..].find(marker) {
                output.push_str(BOLD);
                output.push_str(&render_inline(&input[index + 2..index + 2 + end], base_foreground, theme));
                output.push_str(&restore(base_foreground));
                index += end + 4;
            } else {
                output.push_str(marker);
                index += 2;
            }
            continue;
        }
        if rest.starts_with('`') {
            if let Some(end) = input[index + 1..].find('`') {
                output.push_str(&style(theme.inline_code_fg, Some(theme.inline_code_bg), false, false, false));
                output.push_str(&input[index + 1..index + 1 + end]);
                output.push_str(&restore(base_foreground));
                index += end + 2;
                continue;
            }
        }
        if rest.starts_with('[') {
            if let Some(close) = input[index + 1..].find("](") {
                let close = index + 1 + close;
                if let Some(end) = input[close + 2..].find(')') {
                    let end = close + 2 + end;
                    output.push_str(&style(theme.link_text_fg, None, true, false, true));
                    output.push_str(&input[index + 1..close]);
                    output.push_str(&style(theme.link_fg, None, false, false, true));
                    output.push_str(" <");
                    output.push_str(&input[close + 2..end]);
                    output.push_str(">");
                    output.push_str(&restore(base_foreground));
                    index = end + 1;
                    continue;
                }
            }
        }
        if rest.starts_with('*') || rest.starts_with('_') {
            if rest.starts_with('_') && underscore_in_word(input, index) {
                output.push('_');
                index += 1;
                continue;
            }
            let marker = &input[index..index + 1];
            if let Some(end) = input[index + 1..].find(marker) {
                output.push_str(ITALIC);
                output.push_str(&input[index + 1..index + 1 + end]);
                output.push_str(&restore(base_foreground));
                index += end + 2;
            } else {
                output.push_str(marker);
                index += 1;
            }
            continue;
        }
        let character = rest.chars().next().unwrap();
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn select_paths(directory: &Path) -> io::Result<Option<Vec<String>>> {
    let (sender, receiver) = mpsc::channel();
    let root = directory.to_path_buf();
    thread::spawn(move || {
        let _ = scan_markdown_files(&root, &root, &sender);
    });

    let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let saved = stty(&["-g"])?;
    stty(&["-icanon", "-echo", "min", "0", "time", "0"])?;
    let action = picker_loop(&mut tty, receiver)?;
    let _ = restore_tty(&saved);
    print!("\x1b[2J\x1b[H");
    io::stdout().flush()?;

    match action {
        Some(PickerAction::Open(path)) => Ok(Some(vec![directory.join(path).to_string_lossy().into_owned()])),
        Some(PickerAction::Edit(path)) => {
            let path = directory.join(path);
            run_editor(&path)?;
            Ok(Some(vec![path.to_string_lossy().into_owned()]))
        }
        None => Ok(None),
    }
}

fn scan_markdown_files(directory: &Path, root: &Path, sender: &mpsc::Sender<PathBuf>) -> io::Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            let _ = scan_markdown_files(&path, root, sender);
        } else if path.is_file() && is_markdown_file(&path) {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            if sender.send(relative).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn is_markdown_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()).map(|extension| extension.to_ascii_lowercase()).as_deref(),
        Some("md") | Some("mdown") | Some("mkdn") | Some("mkd") | Some("markdown")
    )
}

enum PickerAction {
    Open(PathBuf),
    Edit(PathBuf),
}

fn picker_loop(tty: &mut File, receiver: Receiver<PathBuf>) -> io::Result<Option<PickerAction>> {
    let mut files = Vec::new();
    let mut selected = 0usize;
    let mut scanning = true;
    let mut dirty = true;
    let mut last_draw = Instant::now() - Duration::from_secs(1);

    loop {
        let selected_path = files.get(selected).cloned();
        loop {
            match receiver.try_recv() {
                Ok(path) => {
                    files.push(path);
                    dirty = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    scanning = false;
                    break;
                }
            }
        }
        if dirty || last_draw.elapsed() >= Duration::from_millis(100) {
            files.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
            if let Some(path) = selected_path {
                selected = files.iter().position(|candidate| *candidate == path).unwrap_or(0);
            } else if !files.is_empty() {
                selected = selected.min(files.len() - 1);
            }
            draw_picker(&files, selected, scanning)?;
            dirty = false;
            last_draw = Instant::now();
        }

        if !scanning && files.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no Markdown files found"));
        }
        if let Some(key) = read_key(tty)? {
            let visible = picker_visible_rows();
            match key {
                Key::Up => selected = selected.saturating_sub(1),
                Key::Down if !files.is_empty() => selected = (selected + 1).min(files.len() - 1),
                Key::PreviousPage if !files.is_empty() => selected = page_move(selected, files.len(), visible, -1),
                Key::NextPage if !files.is_empty() => selected = page_move(selected, files.len(), visible, 1),
                Key::Enter if !files.is_empty() => return Ok(Some(PickerAction::Open(files[selected].clone()))),
                Key::Edit if !files.is_empty() => return Ok(Some(PickerAction::Edit(files[selected].clone()))),
                Key::Quit => return Ok(None),
                Key::Other | Key::Down | Key::Enter | Key::Edit | Key::PreviousPage | Key::NextPage => {}
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    PreviousPage,
    NextPage,
    Enter,
    Edit,
    Quit,
    Other,
}

fn read_key(tty: &mut File) -> io::Result<Option<Key>> {
    let mut byte = [0u8; 1];
    if tty.read(&mut byte)? == 0 {
        return Ok(None);
    }
    let key = match byte[0] {
        b'k' | 0x10 => Key::Up,
        b'j' | 0x0e => Key::Down,
        b'h' => Key::PreviousPage,
        b'l' => Key::NextPage,
        b'\r' | b'\n' => Key::Enter,
        b'e' => Key::Edit,
        b'q' | 0x03 | 0x1b => {
            if byte[0] == 0x1b {
                let mut escape = [0u8; 2];
                let mut received = 0;
                let deadline = Instant::now() + Duration::from_millis(50);
                while received < escape.len() && Instant::now() < deadline {
                    let count = tty.read(&mut escape[received..])?;
                    if count == 0 {
                        thread::sleep(Duration::from_millis(1));
                    } else {
                        received += count;
                    }
                }
                if received == escape.len() {
                    match escape {
                        [b'[', b'A'] => return Ok(Some(Key::Up)),
                        [b'[', b'B'] => return Ok(Some(Key::Down)),
                        [b'[', b'D'] => return Ok(Some(Key::PreviousPage)),
                        [b'[', b'C'] => return Ok(Some(Key::NextPage)),
                        _ => {}
                    }
                }
            }
            Key::Quit
        }
        _ => Key::Other,
    };
    Ok(Some(key))
}

fn picker_visible_rows() -> usize {
    terminal_rows().unwrap_or(24).saturating_sub(5).max(1) as usize
}

fn page_move(selected: usize, file_count: usize, visible: usize, direction: isize) -> usize {
    let page = selected / visible;
    let page_count = file_count.div_ceil(visible);
    let target_page = if direction < 0 {
        page.saturating_sub(1)
    } else {
        (page + 1).min(page_count.saturating_sub(1))
    };
    let cursor = selected % visible;
    let target_start = target_page * visible;
    let target_len = (file_count - target_start).min(visible);
    target_start + cursor.min(target_len.saturating_sub(1))
}

fn truncate_terminal(text: &str, width: usize) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let head_len = (width - 1) / 2;
    let tail_len = width - 1 - head_len;
    let head: String = characters[..head_len].iter().collect();
    let tail: String = characters[characters.len() - tail_len..].iter().collect();
    format!("{head}…{tail}")
}

fn page_indicator(page_count: usize, current_page: usize, width: usize) -> String {
    let max_pages = width.saturating_sub(2).max(1) / 2;
    let mut result = String::new();
    let (start, end, leading, trailing) = if page_count <= max_pages {
        (0, page_count, false, false)
    } else if current_page < max_pages / 2 {
        (0, max_pages.saturating_sub(1), false, true)
    } else if current_page + max_pages / 2 >= page_count {
        (page_count - max_pages.saturating_sub(1), page_count, true, false)
    } else {
        let half = max_pages.saturating_sub(2) / 2;
        (current_page.saturating_sub(half), current_page + half + 1, true, true)
    };
    if leading {
        result.push_str("… ");
    }
    for page in start..end {
        result.push_str(if page == current_page { "● " } else { "• " });
    }
    if trailing {
        result.push('…');
    }
    result
}

fn draw_picker(files: &[PathBuf], selected: usize, scanning: bool) -> io::Result<()> {
    let visible = picker_visible_rows();
    let columns = terminal_columns().unwrap_or(80) as usize;
    let current_page = selected / visible;
    let first = current_page * visible;
    let last = (first + visible).min(files.len());

    let mut screen = String::from("\x1b[2J\x1b[H");
    screen.push(' ');
    screen.push_str(&style(PICKER_H1_FG, Some(PICKER_H1_BG), true, false, false));
    screen.push_str(" md ");
    screen.push_str(RESET);
    let mut header = format!(" select a Markdown file  │  {} file{} found", files.len(), if files.len() == 1 { "" } else { "s" });
    if scanning {
        header.push_str(" (searching…)");
    }
    screen.push_str(&truncate_terminal(&header, columns.saturating_sub(5).max(1)));
    screen.push_str("\n\n");
    for (index, path) in files.iter().enumerate().skip(first).take(last - first) {
        screen.push(' ');
        if index == selected {
            screen.push_str("\x1b[38;2;238;111;248m\x1b[1m❯ ");
        } else {
            screen.push_str("\x1b[38;2;4;181;117m  ");
        }
        screen.push_str(&truncate_terminal(&path.to_string_lossy(), columns.saturating_sub(3).max(1)));
        screen.push_str(RESET);
        screen.push('\n');
    }
    for _ in last - first..visible {
        screen.push('\n');
    }
    let page_count = files.len().div_ceil(visible).max(1);
    screen.push_str("\n ");
    let indicator = page_indicator(page_count, current_page, columns);
    screen.push_str("\x1b[38;5;250m");
    for character in indicator.chars() {
        if character == '●' {
            screen.push_str("\x1b[38;5;240m●\x1b[38;5;250m");
        } else {
            screen.push(character);
        }
    }
    screen.push_str(RESET);
    screen.push_str("\n ");
    screen.push_str(DIM);
    screen.push_str(&truncate_terminal("↑/↓ or j/k  ←/→ or h/l page  enter open  e edit  q quit", columns.saturating_sub(1).max(1)));
    screen.push_str(RESET);
    print!("{screen}");
    io::stdout().flush()
}

fn terminal_columns() -> Option<u16> {
    if let Ok(columns) = env::var("COLUMNS") {
        if let Ok(columns) = columns.parse() {
            return Some(columns);
        }
    }
    let output = Command::new("stty").args(["-F", "/dev/tty", "size"]).output().ok()?;
    String::from_utf8_lossy(&output.stdout).split_whitespace().nth(1)?.parse().ok()
}

fn terminal_rows() -> Option<u16> {
    let output = Command::new("stty").args(["-F", "/dev/tty", "size"]).output().ok()?;
    String::from_utf8_lossy(&output.stdout).split_whitespace().next()?.parse().ok()
}

fn stty(arguments: &[&str]) -> io::Result<String> {
    let output = Command::new("stty").arg("-F").arg("/dev/tty").args(arguments).output()?;
    if !output.status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "unable to configure terminal"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn restore_tty(saved: &str) -> io::Result<()> {
    let output = Command::new("stty").args(["-F", "/dev/tty", saved]).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "unable to restore terminal"))
    }
}

enum PageAction {
    Done,
    Edit,
}

fn write_less_edit_keymap() -> io::Result<PathBuf> {
    let base = env::temp_dir().join(format!("md-lesskey-{}", std::process::id()));
    let source_path = base.with_extension("source");
    let compiled_path = base.with_extension("compiled");
    fs::write(&source_path, "#command\ne quit e\n")?;
    let status = Command::new("lesskey")
        .args(["-o", compiled_path.to_string_lossy().as_ref(), source_path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    let _ = fs::remove_file(source_path);
    if status.success() {
        Ok(compiled_path)
    } else {
        let _ = fs::remove_file(&compiled_path);
        Err(io::Error::new(io::ErrorKind::Other, "lesskey could not compile the editor keymap"))
    }
}

fn run_editor(path: &Path) -> io::Result<()> {
    let editor = env::var("VISUAL").or_else(|_| env::var("EDITOR")).unwrap_or_else(|_| "vi".to_string());
    let words = shell_words(&editor).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid editor command"))?;
    if words.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty editor command"));
    }
    let status = Command::new(&words[0])
        .args(&words[1..])
        .arg(path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, format!("editor exited with {status}")))
    }
}

fn page<F>(rendered: &str, editable_path: Option<&Path>, pager_scroll_speed: usize, rerender: F) -> io::Result<PageAction>
where
    F: FnMut(usize) -> String,
{
    if env::var_os("PAGER").is_none() {
        return match pager::run(rendered, editable_path.is_some(), pager_scroll_speed, rerender)? {
            pager::Action::Done => Ok(PageAction::Done),
            pager::Action::Edit => Ok(PageAction::Edit),
        };
    }
    let use_default_pager = false;
    let keymap = if use_default_pager && editable_path.is_some() {
        Some(write_less_edit_keymap()?)
    } else {
        None
    };
    let pager = if let Some(path) = &keymap {
        format!("less -R -k {}", path.display())
    } else {
        env::var("PAGER").unwrap_or_else(|_| "less -R".to_string())
    };
    let words = shell_words(&pager).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid PAGER"))?;
    if words.is_empty() {
        if let Some(path) = keymap { let _ = fs::remove_file(path); }
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty PAGER"));
    }

    let mut child = match Command::new(&words[0])
        .args(&words[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            if let Some(path) = keymap { let _ = fs::remove_file(path); }
            return Err(error);
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(rendered.as_bytes());
    }
    let status = child.wait()?;
    if let Some(path) = keymap { let _ = fs::remove_file(path); }
    if status.code() == Some('e' as i32) {
        Ok(PageAction::Edit)
    } else if status.success() {
        Ok(PageAction::Done)
    } else {
        Err(io::Error::new(io::ErrorKind::Other, format!("pager exited with {status}")))
    }
}

fn shell_words(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                word.push(character);
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if escaped || quote.is_some() {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    Some(words)
}
