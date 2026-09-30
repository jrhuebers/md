use std::env;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use unicode_width::UnicodeWidthStr;

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
    search_selected_bg: u8,
    search_selected_fg: u8,
    search_other_bg: u8,
    search_other_fg: u8,
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
            search_selected_bg: 208,
            search_selected_fg: 0,
            search_other_bg: 226,
            search_other_fg: 0,
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
            search_selected_bg: 208,
            search_selected_fg: 0,
            search_other_bg: 226,
            search_other_fg: 0,
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
            "search_selected_bg" => self.search_selected_bg = parsed,
            "search_selected_fg" => self.search_selected_fg = parsed,
            "search_other_bg" => self.search_other_bg = parsed,
            "search_other_fg" => self.search_other_fg = parsed,
            "margin_left" => self.margin_left = parsed as usize,
            "margin_right" => self.margin_right = parsed as usize,
            _ => {}
        }
    }
}

struct Config {
    style: String,
    width: usize,
    max_line_length: usize,
    render_latex: bool,
    pager: String,
    themes: HashMap<String, Theme>,
}

impl Config {
    fn default() -> Self {
        let mut themes = HashMap::new();
        themes.insert("glow-light".to_string(), Theme::glow_light());
        themes.insert("glow-dark".to_string(), Theme::glow_dark());
        Self { style: "glow-light".to_string(), width: 0, max_line_length: 100, render_latex: true, pager: "less -R".to_string(), themes }
    }

    fn theme(self) -> io::Result<(Theme, usize, usize, bool, String)> {
        let theme = self.themes.get(&self.style).cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("unknown md style: {}", self.style))
        })?;
        Ok((theme, self.width, self.max_line_length, self.render_latex, self.pager))
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
    let (theme, configured_width, max_line_length, render_latex, pager) = config;
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
        println!("md 0.6.43");
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
    let editable_path = if paths.len() == 1 && paths[0] != "-" { Some(Path::new(&paths[0])) } else { None };
    loop {
        let input = match read_input(&paths) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("md: {error}");
                std::process::exit(2);
            }
        };
        let built_in = env::var("PAGER").unwrap_or_else(|_| pager.clone()) == "builtin";
        let rendered = render_document(&input, &theme, width, max_line_length);
        // Do not leave the math helper attached to the terminal while the pager runs.
        math::shutdown();
        let result = if built_in {
            pager::run(&rendered, editable_path.is_some(), 60, 2, false, pager::SearchColors { selected_bg: theme.search_selected_bg, selected_fg: theme.search_selected_fg, other_bg: theme.search_other_bg, other_fg: theme.search_other_fg }, |_| rendered.clone())
            .map(|action| matches!(action, pager::Action::Edit))
        } else {
            page(&rendered, &pager).map(|_| false)
        };
        match result {
            Ok(true) => {
                if let Some(path) = editable_path {
                    if let Err(error) = run_editor(path) {
                        eprintln!("md: {error}");
                        std::process::exit(1);
                    }
                }
            }
            Ok(false) => break,
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
            } else if let Some(value) = content.strip_prefix("max_line_length:") {
                config.max_line_length = value.trim().parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "md.yaml max_line_length must be an integer")
                })?;
            } else if let Some(value) = content.strip_prefix("render_latex:") {
                config.render_latex = value.trim().parse::<bool>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "md.yaml render_latex must be true or false")
                })?;
            } else if let Some(value) = content.strip_prefix("pager:") {
                let pager = value.trim().trim_matches(['"', '\'']).trim().to_string();
                if pager.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "md.yaml pager must not be empty"));
                }
                config.pager = pager;
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

fn normalized_lines(input: &str) -> Vec<String> {
    let raw_lines: Vec<&str> = input.lines().map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    let has_frontmatter = raw_lines.first().is_some_and(|line| line.trim() == "---")
        && raw_lines.iter().skip(1).any(|line| line.trim() == "---");
    let mut lines: Vec<String> = Vec::new();
    let mut last_was_list = false;
    let mut at_document_start = true;
    let mut in_frontmatter = false;
    let mut in_code = false;
    for line in raw_lines {
        let trimmed = line.trim();
        if has_frontmatter && (at_document_start || in_frontmatter) {
            lines.push(line.to_string());
            if at_document_start && trimmed == "---" {
                in_frontmatter = true;
            } else if in_frontmatter && trimmed == "---" {
                in_frontmatter = false;
            }
            at_document_start = false;
            last_was_list = false;
            continue;
        }
        if in_code || is_fence(line.trim_start()) {
            lines.push(line.to_string());
            if is_fence(line.trim_start()) {
                in_code = !in_code;
            }
            at_document_start = false;
            last_was_list = false;
            continue;
        }
        at_document_start = false;
        let is_list = list_item(line).is_some();
        let is_continuation = last_was_list
            && !trimmed.is_empty()
            && line.chars().take_while(|character| character.is_whitespace()).count() >= 2
            && !is_list;
        if is_continuation {
            if let Some(previous) = lines.last_mut() {
                previous.push(' ');
                previous.push_str(trimmed);
            }
        } else {
            lines.push(line.to_string());
        }
        last_was_list = if trimmed.is_empty() { false } else { is_list || is_continuation };
    }
    lines
}

fn render_document(input: &str, theme: &Theme, terminal_width: usize, max_line_length: usize) -> String {
    let margin_width = theme.margin_left + theme.margin_right;
    let available = terminal_width.saturating_sub(margin_width);
    let column_width = if max_line_length == 0 {
        available
    } else {
        available.min(max_line_length)
    };
    let render_width = column_width + margin_width;
    let rendered = render_markdown(input, theme, render_width);
    let outer_padding = terminal_width.saturating_sub(render_width) / 2;
    if outer_padding == 0 {
        return rendered;
    }
    rendered
        .lines()
        .map(|line| format!("{}{}{}\n", " ".repeat(outer_padding), line, " ".repeat(outer_padding)))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    List,
    Quote,
}

fn render_markdown(input: &str, theme: &Theme, width: usize) -> String {
    let mut output = String::with_capacity(input.len() + input.len() / 8);
    push_line(&mut output, "", theme);
    let mut paragraph: Vec<String> = Vec::new();
    let normalized = normalized_lines(input);
    let has_frontmatter = normalized.first().is_some_and(|line| line.trim() == "---")
        && normalized.iter().skip(1).any(|line| line.trim() == "---");
    let mut in_frontmatter = has_frontmatter;
    let mut frontmatter_opening = has_frontmatter;
    let mut in_code = false;
    let mut code_highlighter = None;
    let mut math_block: Option<(String, String, String, bool)> = None;
    let mut previous_block: Option<BlockKind> = None;
    let mut suppress_blank = false;

    let mut lines = normalized.into_iter().peekable();
    while let Some(raw_line) = lines.next() {
        let line = raw_line.as_str();
        let trimmed = line.trim_start();

        if in_frontmatter {
            previous_block = None;
            let is_delimiter = frontmatter_opening || trimmed.trim() == "---";
            if frontmatter_opening {
                frontmatter_opening = false;
            } else if is_delimiter {
                in_frontmatter = false;
            }
            if is_delimiter {
                let rendered = render_rule(theme, width);
                push_line(&mut output, &rendered, theme);
            } else {
                let rendered = format!("{}{}{}", fg(theme.normal_fg), line, RESET);
                push_line(&mut output, &rendered, theme);
            }
            continue;
        }

        if is_fence(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            if in_code {
                in_code = false;
                code_highlighter = None;
            } else {
                in_code = true;
                code_highlighter = fence_language(trimmed)
                    .and_then(|language| make_code_highlighter(language, theme));
            }
            continue;
        }
        if in_code {
            previous_block = None;
            let content_width = width.saturating_sub(theme.margin_left + theme.margin_right);
            let code_width = 2 + visible_width(line);
            let mut rendered = code_style(theme);
            rendered.push_str("  ");
            if let Some(highlighter) = code_highlighter.as_mut() {
                rendered.push_str(&highlight_code_line(line, highlighter, theme));
                rendered.push_str(&code_style(theme));
            } else {
                rendered.push_str(line);
            }
            rendered.push_str(&" ".repeat(content_width.saturating_sub(code_width)));
            rendered.push_str(RESET);
            push_line(&mut output, &rendered, theme);
            continue;
        }
        if let Some((_, closing, _, quoted)) = math_block.as_ref() {
            let quoted = *quoted;
            let content = if quoted { quote_content(trimmed) } else { Some(line) };
            if let Some(content) = content {
                previous_block = if quoted { Some(BlockKind::Quote) } else { None };
                let closing = closing.clone();
                if let Some(end) = content.find(closing.as_str()) {
                    if let Some((opening, _, mut body, _)) = math_block.take() {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        body.push_str(&content[..end]);
                        push_math_display(&mut output, &opening, &closing, &body, quoted, theme, width);
                    }
                } else if let Some((_, _, body, _)) = math_block.as_mut() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(content);
                }
                continue;
            }
            // A quote ended without its closing delimiter; do not consume the next block.
            if let Some((opening, closing, body, _)) = math_block.take() {
                push_math_display(&mut output, &opening, &closing, &body, true, theme, width);
            }
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
        if !in_code && math_block.is_none() {
            let header = table_row(line);
            let alignments = lines.peek().and_then(|candidate| table_alignments(candidate));
            if let (Some(header), Some(alignments)) = (header, alignments) {
                if header.len() == alignments.len() {
                    let _separator = lines.next();
                    let mut rows = Vec::new();
                    while let Some(candidate) = lines.peek() {
                        if let Some(row) = table_row(candidate) {
                            rows.push(row);
                            let _ = lines.next();
                        } else {
                            break;
                        }
                    }
                    flush_paragraph(&mut paragraph, &mut output, theme, width);
                    render_table(&mut output, header, alignments, rows, theme, width);
                    previous_block = None;
                    continue;
                }
            }
        }
        if let Some((opening, closing)) = display_math_delimiter(trimmed) {
            previous_block = None;
            let body_start = opening.len();
            let rest = &trimmed[body_start..];
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            if let Some(end) = rest.find(closing) {
                push_math_display(&mut output, opening, closing, &rest[..end], false, theme, width);
            } else {
                math_block = Some((opening.to_string(), closing.to_string(), rest.to_string(), false));
            }
            continue;
        }
        if let Some((level, heading)) = heading(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            ensure_blank_line(&mut output, theme);
            // Continuations align under the first heading character, past the displayed marker.
            let prefix = if level == 1 { " ".to_string() } else { format!("{} ", "#".repeat(level)) };
            let content_width = width.saturating_sub(theme.margin_left + theme.margin_right);
            let available = content_width.saturating_sub(prefix.len() + usize::from(level == 1)).max(1);
            for (index, chunk) in wrap_text(heading.trim(), available).iter().enumerate() {
                let mut rendered = if level == 1 {
                    style(theme.h1_fg, Some(theme.h1_bg), true, false, false)
                } else {
                    style(theme.heading_fg, None, true, false, false)
                };
                if index == 0 {
                    rendered.push_str(&prefix);
                } else {
                    rendered.push_str(&" ".repeat(prefix.len()));
                }
                rendered.push_str(&render_inline(chunk, if level == 1 { theme.h1_fg } else { theme.heading_fg }, theme));
                if level == 1 {
                    rendered.push(' ');
                }
                rendered.push_str(RESET);
                push_line(&mut output, &rendered, theme);
            }
            push_line(&mut output, "", theme);
            suppress_blank = true;
            continue;
        }
        if is_rule(trimmed) {
            previous_block = None;
            flush_paragraph(&mut paragraph, &mut output, theme, width);
            let rendered = render_rule(theme, width);
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
        if let Some(content) = quote_content(trimmed) {
            if previous_block != Some(BlockKind::Quote) {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
                ensure_blank_line(&mut output, theme);
            } else {
                flush_paragraph(&mut paragraph, &mut output, theme, width);
            }
            previous_block = Some(BlockKind::Quote);
            let content = content.trim();
            if let Some((opening, closing)) = display_math_delimiter(content) {
                let rest = &content[opening.len()..];
                if let Some(end) = rest.find(closing) {
                    push_math_display(&mut output, opening, closing, &rest[..end], true, theme, width);
                } else {
                    math_block = Some((opening.to_string(), closing.to_string(), rest.to_string(), true));
                }
                continue;
            }
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
    if let Some((opening, closing, body, quoted)) = math_block {
        push_math_display(&mut output, &opening, &closing, &body, quoted, theme, width);
    }
    ensure_blank_line(&mut output, theme);
    output
}

fn quote_content(line: &str) -> Option<&str> {
    line.strip_prefix("> ").or_else(|| line.strip_prefix('>'))
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

fn push_math_display(output: &mut String, opening: &str, closing: &str, source: &str, quoted: bool, theme: &Theme, width: usize) {
    if quoted {
        push_quote_blank(output, theme);
    } else {
        ensure_blank_line(output, theme);
    }
    let lines: Vec<String> = if math::enabled() {
        math::render_display(source)
    } else {
        format!("{opening}{source}{closing}").lines().map(ToOwned::to_owned).collect()
    };
    let content_width = width.saturating_sub(theme.margin_left + theme.margin_right + if quoted { 2 } else { 0 });
    let block_width = lines.iter().map(|line| UnicodeWidthStr::width(line.as_str())).max().unwrap_or(0);
    let padding = content_width.saturating_sub(block_width) / 2;
    for line in lines {
        let prefix = if quoted { "│ " } else { "" };
        let rendered = format!("{}{}{}{}{}{}", fg(theme.normal_fg), if quoted { DIM } else { "" }, prefix, " ".repeat(padding), line, RESET);
        push_line(output, &rendered, theme);
    }
    if quoted {
        push_quote_blank(output, theme);
    } else {
        ensure_blank_line(output, theme);
    }
}

fn push_quote_blank(output: &mut String, theme: &Theme) {
    let rendered = format!("{}{}│ {}", fg(theme.normal_fg), DIM, RESET);
    let blank = format!("{}{}{}\n", " ".repeat(theme.margin_left), rendered, " ".repeat(theme.margin_right));
    if !output.ends_with(&blank) {
        output.push_str(&blank);
    }
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

#[derive(Clone, Copy)]
enum TableAlignment {
    Left,
    Center,
    Right,
}

// This follows Glamour's table shape: cell margins, vertical separators, and
// a rule below the header, but no enclosing top or bottom border.
fn table_row(line: &str) -> Option<Vec<String>> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut in_code = false;
    let mut escaped = false;
    let mut has_pipe = false;

    for character in line.trim().chars() {
        if escaped {
            if character == '|' {
                cell.push('|');
            } else {
                cell.push('\\');
                cell.push(character);
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '`' {
            in_code = !in_code;
            cell.push(character);
        } else if character == '|' && !in_code {
            has_pipe = true;
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(character);
        }
    }
    if escaped {
        cell.push('\\');
    }
    cells.push(cell.trim().to_string());
    if !has_pipe {
        return None;
    }
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    (!cells.is_empty()).then_some(cells)
}

fn table_alignments(line: &str) -> Option<Vec<TableAlignment>> {
    let cells = table_row(line)?;
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in cells {
        let cell = cell.trim();
        let left = cell.starts_with(':');
        let right = cell.ends_with(':');
        let start = usize::from(left);
        let end = cell.len().saturating_sub(usize::from(right));
        let dashes = &cell[start..end];
        if dashes.len() < 3 || !dashes.chars().all(|character| character == '-') {
            return None;
        }
        alignments.push(match (left, right) {
            (true, true) => TableAlignment::Center,
            (false, true) => TableAlignment::Right,
            _ => TableAlignment::Left,
        });
    }
    Some(alignments)
}

fn render_table(
    output: &mut String,
    header: Vec<String>,
    alignments: Vec<TableAlignment>,
    mut rows: Vec<Vec<String>>,
    theme: &Theme,
    width: usize,
) {
    let columns = header.len();
    for row in &mut rows {
        row.resize(columns, String::new());
        row.truncate(columns);
    }

    let mut column_widths = vec![1; columns];
    for (column, cell) in header.iter().enumerate() {
        column_widths[column] = column_widths[column].max(table_cell_width(cell, theme));
    }
    for row in &rows {
        for (column, cell) in row.iter().enumerate() {
            column_widths[column] = column_widths[column].max(table_cell_width(cell, theme));
        }
    }

    // Glamour gives every cell a one-column left margin. The remaining space
    // is shared by columns, shrinking the widest columns first when needed.
    let available = width.saturating_sub(theme.margin_left + theme.margin_right);
    let overhead = columns.saturating_mul(2).saturating_sub(1);
    let content_width = available.saturating_sub(overhead).max(columns);
    while column_widths.iter().sum::<usize>() > content_width {
        let widest = column_widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > 1)
            .max_by_key(|(_, width)| **width)
            .map(|(column, _)| column);
        let Some(column) = widest else { break };
        column_widths[column] -= 1;
    }

    ensure_blank_line(output, theme);
    render_table_row(output, &header, &column_widths, &alignments, theme);
    let separator = column_widths
        .iter()
        .map(|column_width| "─".repeat(column_width + 1))
        .collect::<Vec<_>>()
        .join("┼");
    push_line(output, &format!("{}{}{}", fg(theme.rule_fg), separator, RESET), theme);
    for row in rows {
        render_table_row(output, &row, &column_widths, &alignments, theme);
    }
    ensure_blank_line(output, theme);
}

fn table_cell_width(cell: &str, theme: &Theme) -> usize {
    visible_width(&render_inline(cell, theme.normal_fg, theme)).max(1)
}

fn render_table_row(
    output: &mut String,
    cells: &[String],
    column_widths: &[usize],
    alignments: &[TableAlignment],
    theme: &Theme,
) {
    let mut rendered_cells = Vec::new();
    let mut row_height = 1;
    for (column, cell) in cells.iter().enumerate() {
        let lines = wrap_text(cell, column_widths[column]);
        row_height = row_height.max(lines.len());
        rendered_cells.push(lines);
    }

    for line_number in 0..row_height {
        let mut rendered = String::new();
        for column in 0..cells.len() {
            if column > 0 {
                rendered.push_str(&format!("{}│{}", fg(theme.rule_fg), RESET));
            }
            let source = rendered_cells[column].get(line_number).map(String::as_str).unwrap_or("");
            let styled = render_inline(source, theme.normal_fg, theme);
            let styled = truncate_ansi(&styled, column_widths[column]);
            let used = visible_width(&styled);
            let padding = column_widths[column].saturating_sub(used);
            let (left, right) = match alignments[column] {
                TableAlignment::Left => (0, padding),
                TableAlignment::Right => (padding, 0),
                TableAlignment::Center => (padding / 2, padding - padding / 2),
            };
            rendered.push(' ');
            rendered.push_str(&" ".repeat(left));
            rendered.push_str(&styled);
            rendered.push_str(&" ".repeat(right));
            rendered.push_str(RESET);
        }
        push_line(output, &rendered, theme);
    }
}

fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut bytes = text.as_bytes();
    while !bytes.is_empty() {
        if bytes[0] == 0x1b {
            if bytes.get(1) == Some(&b'[') {
                let end = bytes[2..].iter().position(|byte| (0x40..=0x7e).contains(byte)).map(|index| index + 3).unwrap_or(bytes.len());
                bytes = &bytes[end..];
            } else if bytes.get(1) == Some(&b']') {
                let end = bytes[2..].iter().position(|byte| *byte == 0x07).map(|index| index + 3).unwrap_or(bytes.len());
                bytes = &bytes[end..];
            } else {
                bytes = &bytes[1..];
            }
        } else if let Some(character) = text[text.len() - bytes.len()..].chars().next() {
            width += 1;
            bytes = &bytes[character.len_utf8()..];
        } else {
            break;
        }
    }
    width
}

fn truncate_ansi(text: &str, width: usize) -> String {
    if visible_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut visible = 0;
    let mut position = 0;
    while position < text.len() && visible + 1 < width {
        let bytes = text.as_bytes();
        if bytes[position] == 0x1b {
            let start = position;
            position += 1;
            if bytes.get(position) == Some(&b'[') {
                position += 1;
                while position < text.len() {
                    let byte = bytes[position];
                    position += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            } else if bytes.get(position) == Some(&b']') {
                position += 1;
                while position < text.len() {
                    let byte = bytes[position];
                    position += 1;
                    if byte == 0x07 {
                        break;
                    }
                }
            }
            result.push_str(&text[start..position]);
            continue;
        }
        let character = text[position..].chars().next().unwrap();
        result.push(character);
        position += character.len_utf8();
        visible += 1;
    }
    result.push('…');
    result
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
        if text[position..].starts_with("**") || text[position..].starts_with("__") {
            let marker = &text[position..position + 2];
            if let Some(end) = text[position + 2..].find(marker) {
                position += 2 + end + 2;
                consume_punctuation(text, &mut position);
                let word = &text[start..position];
                tokens.push(WrapToken { text: word.to_string(), width: word.chars().count().saturating_sub(4), code: false, glued });
                continue;
            }
        }
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
        if text[position..].starts_with("$$") {
            if let Some(end) = text[position + 2..].find("$$") {
                position += 2 + end + 2;
                consume_punctuation(text, &mut position);
                let word = &text[start..position];
                tokens.push(WrapToken { text: word.to_string(), width: rendered_word_width(word), code: false, glued });
                continue;
            }
        } else if text[position..].starts_with('$') {
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
        } else if text[position..].starts_with("\\[") {
            if let Some(end) = text[position + 2..].find("\\]") {
                position += 2 + end + 2;
                consume_punctuation(text, &mut position);
                let word = &text[start..position];
                tokens.push(WrapToken { text: word.to_string(), width: rendered_word_width(word), code: false, glued });
                continue;
            }
        }
        while position < text.len() && !text.as_bytes()[position].is_ascii_whitespace() {
            // Split a word before an embedded code span, e.g. "(`git ...`)".
            // Otherwise wrapping can separate its backticks before render_inline sees them.
            if position > start && text.as_bytes()[position] == b'`' && text[position + 1..].contains('`') {
                break;
            }
            position += 1;
        }
        let word = &text[start..position];
        tokens.push(WrapToken { text: word.to_string(), width: word.chars().count(), code: false, glued });
    }
    tokens
}

fn rendered_word_width(word: &str) -> usize {
    if word.starts_with("$$") {
        if let Some(end) = word[2..].find("$$") {
            let end = end + 2;
            return math::render_inline(&word[2..end]).chars().count() + word[end + 2..].chars().count();
        }
    } else if word.starts_with('$') {
        if let Some(end) = find_unescaped(word, 1, '$') {
            return math::render_inline(&word[1..end]).chars().count() + word[end + 1..].chars().count();
        }
    } else if word.starts_with("\\(") {
        if let Some(end) = word[2..].find("\\)") {
            let end = end + 2;
            return math::render_inline(&word[2..end]).chars().count() + word[end + 2..].chars().count();
        }
    } else if word.starts_with("\\[") {
        if let Some(end) = word[2..].find("\\]") {
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

fn code_style(theme: &Theme) -> String {
    style(theme.inline_code_fg, Some(theme.inline_code_bg), false, false, false)
}

fn fence_language(fence: &str) -> Option<&str> {
    let marker = if fence.starts_with('~') { "~~~" } else { "```" };
    fence.strip_prefix(marker)?.split_whitespace().next().filter(|language| !language.is_empty())
}

fn make_code_highlighter(language: &str, theme: &Theme) -> Option<HighlightLines<'static>> {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let syntaxes = syntax_set();
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let syntax = syntaxes.find_syntax_by_token(language)?;
    let theme_name = if theme.inline_code_bg > 240 { "InspiredGitHub" } else { "base16-ocean.dark" };
    let syntax_theme = themes.themes.get(theme_name)?;
    Some(HighlightLines::new(syntax, syntax_theme))
}

fn highlight_code_line(line: &str, highlighter: &mut HighlightLines<'static>, theme: &Theme) -> String {
    let input = format!("{line}\n");
    let Some(input_line) = LinesWithEndings::from(&input).next() else {
        return line.to_string();
    };
    let Ok(ranges) = highlighter.highlight_line(input_line, syntax_set()) else {
        return line.to_string();
    };
    let mut rendered = String::with_capacity(line.len() + ranges.len() * 24);
    for (syntax_style, text) in ranges {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let color = syntax_style.foreground;
        rendered.push_str(&format!("\x1b[38;2;{};{};{}m", color.r, color.g, color.b));
        if syntax_style.font_style.contains(FontStyle::BOLD) {
            rendered.push_str(BOLD);
        }
        if syntax_style.font_style.contains(FontStyle::ITALIC) {
            rendered.push_str(ITALIC);
        }
        if syntax_style.font_style.contains(FontStyle::UNDERLINE) {
            rendered.push_str(UNDERLINE);
        }
        rendered.push_str(text);
        rendered.push_str(RESET);
        rendered.push_str(&code_style(theme));
    }
    rendered
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn render_rule(theme: &Theme, width: usize) -> String {
    let content_width = width.saturating_sub(theme.margin_left + theme.margin_right);
    format!("{}{}{}", fg(theme.rule_fg), "─".repeat(content_width), RESET)
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
    } else if rest.starts_with("\\[") {
        ("\\[", "\\]")
    } else if rest.starts_with("$$") {
        ("$$", "$$")
    } else if rest.starts_with('$')
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
    render_inline_with_bold(input, base_foreground, theme, false)
}

fn render_inline_with_bold(input: &str, base_foreground: u8, theme: &Theme, bold_active: bool) -> String {
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
                output.push_str(&render_inline_with_bold(&input[index + 2..index + 2 + end], base_foreground, theme, true));
                output.push_str(&restore_inline(base_foreground, bold_active));
                index += end + 4;
            } else {
                output.push_str(marker);
                index += 2;
            }
            continue;
        }
        if rest.starts_with('`') {
            if let Some(end) = input[index + 1..].find('`') {
                output.push_str(&code_style(theme));
                output.push_str(&input[index + 1..index + 1 + end]);
                output.push_str(&restore_inline(base_foreground, bold_active));
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
                output.push_str(&restore_inline(base_foreground, bold_active));
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

fn restore_inline(foreground: u8, bold_active: bool) -> String {
    let mut output = restore(foreground);
    if bold_active {
        output.push_str(BOLD);
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

fn page(rendered: &str, configured_pager: &str) -> io::Result<()> {
    let pager = env::var("PAGER").unwrap_or_else(|_| configured_pager.to_string());
    let words = shell_words(&pager).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid pager command"))?;
    if words.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty pager command"));
    }

    let mut child = Command::new(&words[0])
        .args(&words[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(rendered.as_bytes());
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_less_and_accepts_a_configured_pager() {
        assert_eq!(Config::default().pager, "less -R");
        let config = parse_config("pager: more -R\n").expect("pager config should parse");
        assert_eq!(config.pager, "more -R");
        let config = parse_config("pager: builtin\n").expect("built-in pager config should parse");
        assert_eq!(config.pager, "builtin");
        let config = parse_config("style: glow-dark\nstyles:\n  glow-dark:\n    search_selected_bg: 201\n    search_selected_fg: 0\n    search_other_bg: 24\n    search_other_fg: 15\n").expect("search colors should parse");
        let (theme, _, _, _, _) = config.theme().expect("theme should resolve");
        assert_eq!((theme.search_selected_bg, theme.search_selected_fg, theme.search_other_bg, theme.search_other_fg), (201, 0, 24, 15));
    }

    #[test]
    fn search_defaults_are_black_on_orange_and_black_on_yellow() {
        for theme in [Theme::glow_light(), Theme::glow_dark()] {
            assert_eq!((theme.search_selected_fg, theme.search_selected_bg), (0, 208));
            assert_eq!((theme.search_other_fg, theme.search_other_bg), (0, 226));
        }
    }

    #[test]
    fn wrapped_headings_align_continuations_after_displayed_markers() {
        let strip = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
        for level in 2..=6 {
            let source = format!("{} Alpha beta gamma delta", "#".repeat(level));
            let rendered = render_document(&source, &Theme::glow_light(), 22, 0);
            let plain = strip.replace_all(&rendered, "");
            let lines: Vec<_> = plain.lines().collect();
            let first = lines.iter().position(|line| line.contains("Alpha")).unwrap();
            let marker = format!("{} ", "#".repeat(level));
            assert!(lines[first].starts_with(&format!(" {marker}")), "{lines:?}");
            assert!(lines[first + 1].starts_with(&" ".repeat(1 + marker.len())), "{lines:?}");
            assert!(lines[first + 1].contains("delta"), "{lines:?}");
            assert!(lines[first + 2].trim().is_empty(), "{lines:?}");
            assert!(lines.iter().all(|line| UnicodeWidthStr::width(*line) <= 22), "{lines:?}");
        }
    }

    #[test]
    fn wraps_h1_without_displayed_hashes() {
        let rendered = render_document("# Alpha beta gamma delta", &Theme::glow_light(), 18, 0);
        let strip = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
        let plain = strip.replace_all(&rendered, "");
        let lines: Vec<_> = plain.lines().collect();
        assert!(lines.iter().any(|line| line.starts_with("  Alpha beta")));
        assert!(lines.iter().any(|line| line.starts_with("  gamma delta")));
        assert!(!plain.contains('#'));
    }

    #[test]
    fn wraps_code_after_punctuation_without_exposing_backticks() {
        let text = "Git commit hash (`git rev-parse HEAD`) and `uv.lock` hash";
        let chunks = wrap_text(text, 23);
        assert!(chunks.iter().any(|chunk| chunk.ends_with("`git`")));
        assert!(chunks.iter().any(|chunk| chunk.starts_with("`rev-parse HEAD`")));
        for chunk in chunks {
            assert_eq!(chunk.matches('`').count() % 2, 0, "unpaired backtick: {chunk}");
            let rendered = render_inline(&chunk, 234, &Theme::glow_light());
            assert!(!rendered.contains('`'), "visible backtick: {}", chunk);
        }
    }

    #[test]
    fn renders_fenced_code_with_the_inline_code_style() {
        let theme = Theme::glow_light();
        let rendered = render_document("`inline`\n\n```\nfirst\nsecond\n```", &theme, 80, 0);
        let styled_code = format!("{}  first{}{}", code_style(&theme), " ".repeat(71), RESET);
        assert!(rendered.contains(&styled_code));
        assert!(rendered.contains(&format!("{}  second{}{}", code_style(&theme), " ".repeat(70), RESET)));
    }

    #[test]
    fn highlights_known_fenced_languages_and_preserves_source_text() {
        let rendered = render_document("```bash\necho \"$HOME\"\n```", &Theme::glow_dark(), 80, 0);
        assert!(rendered.contains("\x1b[38;2;"));
        assert!(rendered.contains("echo"));
        assert!(rendered.contains("$"));
        assert!(rendered.contains("HOME"));
        assert!(!rendered.contains("```"));
    }

    #[test]
    fn renders_display_math_inside_a_blockquote() {
        let input = "> Quoted $x^2$.\n> $$\n> x^2 + y^2\n> $$\n> Afterward.\n\nOutside.";
        let rendered = render_document(input, &Theme::glow_dark(), 60, 0);
        assert!(!rendered.contains("$$"));
        assert!(rendered.lines().any(|line| line.contains("│ ") && line.contains("x² + y²")));
        assert!(rendered.lines().any(|line| line.contains("│ ") && line.contains("Afterward.")));
        assert!(rendered.lines().any(|line| line.contains("Outside.") && !line.contains('│')));
    }

    #[test]
    fn aligns_matrix_rows_with_invisible_source_separators() {
        let zwsp = "\u{200b}";
        let source = format!(
            "\\tilde D_\\ell{zwsp} = \\begin{{pmatrix}} 0 & D_\\ell \\\\{zwsp}{zwsp}D_\\ell^\\top & {zwsp}0{zwsp} \\end{{pmatrix}}"
        );
        let lines = math::render_display(&source);
        assert_eq!(lines.len(), 2);
        assert!(!lines.iter().any(|line| line.contains(zwsp)));
        for (upper, lower) in [('⎛', '⎝'), ('│', '│'), ('⎞', '⎠')] {
            let position = |line: &str, symbol: char| {
                UnicodeWidthStr::width(&line[..line.find(symbol).expect("matrix symbol")])
            };
            assert_eq!(position(&lines[0], upper), position(&lines[1], lower), "{lines:?}");
        }
    }

    #[test]
    fn keeps_a_space_before_display_limit_operators() {
        for operator in ["lim", "limsup"] {
            let source = format!("\\lambda_k = \\{operator}_{{n \\to \\infty}} \\frac{{1}}{{n}} \\log n");
            let lines = math::render_display(&source);
            assert!(lines.iter().any(|line| line.contains(&format!("= {operator}"))), "{lines:?}");
        }
    }

    #[test]
    fn aligns_a_fraction_after_a_combining_accent() {
        let lines = math::render_display("\\bar a^{(\\ell)} = \\frac1N \\sum_{i=1}^{N} a_i^{(\\ell)}.");
        assert_eq!(lines.len(), 3);
        let column = |line: &str, character: char| {
            let byte = line.find(character).expect("expected fraction character");
            UnicodeWidthStr::width(&line[..byte])
        };
        assert_eq!(column(&lines[0], '1'), column(&lines[1], '─'));
        assert_eq!(column(&lines[1], '─'), column(&lines[2], 'N'));
    }

    #[test]
    fn renders_bracketed_math_inside_a_blockquote() {
        let rendered = render_document("> Before.\n> \\[\n> x^2\n> \\]\n> After.", &Theme::glow_light(), 50, 0);
        assert!(!rendered.contains("\\["));
        assert!(rendered.lines().any(|line| line.contains("│ ") && line.contains("x²")));
    }

    #[test]
    fn preserves_lines_inside_fenced_code_blocks() {
        let source = "```\nplugins:\n  - search\n  - mkdocstrings:\n      default_handler: python\n      handlers:\n        python:\n          paths:\n            - libs/shared_db/src\n            - services/user_api/src\n```";
        let lines = normalized_lines(source);
        assert!(lines.contains(&"  - mkdocstrings:".to_string()));
        assert!(lines.contains(&"      default_handler: python".to_string()));
        assert!(!lines.iter().any(|line| line.contains("mkdocstrings:      default_handler")));
    }

    #[test]
    fn preserves_frontmatter_lines_and_expands_rules_to_the_content_width() {
        let theme = Theme::glow_light();
        let rendered = render_document("---\nname: writer\nrunner:\n  type: external-cli\n---", &theme, 30, 0);
        let rule = format!("{}{}{}", fg(theme.rule_fg), "─".repeat(28), RESET);
        assert_eq!(rendered.matches(&rule).count(), 2);
        assert!(rendered.contains("name: writer\x1b[0m \n"));
        assert!(rendered.contains("runner:\x1b[0m \n"));
        assert!(rendered.contains("  type: external-cli\x1b[0m \n"));
    }

    #[test]
    fn parses_glamour_style_table_rows_and_alignment() {
        assert_eq!(table_row("| Name | Age |"), Some(vec!["Name".into(), "Age".into()]));
        assert!(matches!(table_alignments("| --- | :---: | ---: |"), Some(ref alignments) if alignments.len() == 3));
        assert_eq!(table_row("Name | Note with `|`"), Some(vec!["Name".into(), "Note with `|`".into()]));
    }

    #[test]
    fn renders_tables_with_aligned_columns_and_header_rule() {
        let rendered = render_document(
            "| Name | Age |\n| --- | ---: |\n| Alice | 30 |",
            &Theme::glow_light(),
            40,
            0,
        );
        assert!(rendered.contains("│"));
        assert!(rendered.contains("┼"));
        assert!(rendered.contains("Alice"));
        assert!(!rendered.contains("| --- |"));
    }
}
