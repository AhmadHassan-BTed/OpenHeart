//! Minimal YAML reader for Compose files and Kubernetes manifests.
//!
//! Supported: block mappings and sequences (including sequences written at the indentation
//! of their key), flow collections (`[a, b]`, `{k: v}`, spanning lines), plain, single- and
//! double-quoted scalars, literal and folded block scalars, anchors, aliases, merge keys
//! (`<<`) and multiple documents (`---`). Tags are ignored. Every node keeps the 1-based line
//! it starts on, so facts read from it carry their source location.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Null,
    Scalar(String),
    Seq(Vec<YamlNode>),
    Map(Vec<(String, YamlNode)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct YamlNode {
    pub value: Yaml,
    pub line: u32,
}

impl YamlNode {
    fn new(value: Yaml, line: u32) -> Self {
        Self { value, line }
    }

    /// Value of `key` in a mapping (`None` for other node kinds).
    pub fn get(&self, key: &str) -> Option<&YamlNode> {
        match &self.value {
            Yaml::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Node reached by following `keys` through nested mappings.
    pub fn path(&self, keys: &[&str]) -> Option<&YamlNode> {
        let mut cur = self;
        for key in keys {
            cur = cur.get(key)?;
        }
        Some(cur)
    }

    /// Scalar text (`None` for null, sequences and mappings).
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Yaml::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> &[YamlNode] {
        match &self.value {
            Yaml::Seq(items) => items,
            _ => &[],
        }
    }

    pub fn as_map(&self) -> &[(String, YamlNode)] {
        match &self.value {
            Yaml::Map(entries) => entries,
            _ => &[],
        }
    }

    pub fn is_map(&self) -> bool {
        matches!(self.value, Yaml::Map(_))
    }

    /// Scalar items of a sequence, or the keys of a mapping (Compose accepts both forms for
    /// `depends_on` and `networks`), each with its line.
    pub fn names(&self) -> Vec<(String, u32)> {
        match &self.value {
            Yaml::Seq(items) => items
                .iter()
                .filter_map(|i| i.as_str().map(|s| (s.to_string(), i.line)))
                .collect(),
            Yaml::Map(entries) => entries.iter().map(|(k, v)| (k.clone(), v.line)).collect(),
            Yaml::Scalar(s) => vec![(s.clone(), self.line)],
            Yaml::Null => Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct Line<'a> {
    indent: usize,
    /// Content after the indentation, without a trailing comment.
    text: &'a str,
    /// Content after the indentation, comments kept (block scalars).
    raw: &'a str,
    number: u32,
}

/// Parses every document of a YAML stream. Documents that are empty are skipped.
pub fn parse_documents(source: &str) -> Vec<YamlNode> {
    let mut docs: Vec<Vec<Line<'_>>> = vec![Vec::new()];
    for (idx, raw_line) in source.lines().enumerate() {
        let number = (idx + 1) as u32;
        let line = raw_line.trim_end_matches('\r');
        let content = line.trim_start_matches(' ');
        let indent = line.len() - content.len();
        if indent == 0 && (content == "---" || content.starts_with("--- ") || content == "...") {
            docs.push(Vec::new());
            continue;
        }
        if indent == 0 && content.starts_with('%') {
            continue;
        }
        let text = strip_comment(content).trim_end();
        if text.is_empty() {
            continue;
        }
        if let Some(doc) = docs.last_mut() {
            doc.push(Line {
                indent,
                text,
                raw: content.trim_end(),
                number,
            });
        }
    }
    let mut out = Vec::new();
    for lines in docs {
        if lines.is_empty() {
            continue;
        }
        let mut parser = Parser {
            lines,
            pos: 0,
            anchors: HashMap::new(),
        };
        let node = parser.parse_block();
        out.push(node);
    }
    out
}

/// Removes a `#` comment that starts the line or follows whitespace outside quotes.
fn strip_comment(text: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_ws = true;
    for (i, c) in text.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && prev_ws => return &text[..i],
            _ => {}
        }
        prev_ws = c == ' ' || c == '\t';
    }
    text
}

fn is_seq_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// Splits `key: rest` (the key unquoted). `None` when the line is not a mapping entry.
fn split_key(text: &str) -> Option<(String, &str)> {
    if text.starts_with('[') || text.starts_with('{') || is_seq_item(text) {
        return None;
    }
    let bytes = text.as_bytes();
    if bytes.first() == Some(&b'"') || bytes.first() == Some(&b'\'') {
        let quote = bytes[0] as char;
        let close = text[1..].find(quote)? + 1;
        let after = &text[close + 1..];
        let after_trim = after.trim_start();
        if let Some(rest) = after_trim.strip_prefix(':') {
            if rest.is_empty() || rest.starts_with(' ') {
                return Some((text[1..close].to_string(), rest.trim()));
            }
        }
        return None;
    }
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in text.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ if in_single || in_double => {}
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ':' if depth == 0 => {
                let rest = &text[i + 1..];
                if rest.is_empty() || rest.starts_with(' ') {
                    let key = text[..i].trim();
                    if key.is_empty() {
                        return None;
                    }
                    return Some((key.to_string(), rest.trim()));
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits a leading `&anchor` from a value; returns (anchor, remaining text).
fn take_anchor(text: &str) -> (Option<String>, &str) {
    if let Some(rest) = text.strip_prefix('&') {
        let end = rest.find(' ').unwrap_or(rest.len());
        let name = rest[..end].to_string();
        return (Some(name), rest[end..].trim_start());
    }
    (None, text)
}

/// Drops a leading `!tag`.
fn strip_tag(text: &str) -> &str {
    if text.starts_with('!') {
        match text.find(' ') {
            Some(i) => text[i..].trim_start(),
            None => "",
        }
    } else {
        text
    }
}

fn unquote(text: &str) -> String {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    t.to_string()
}

fn scalar_node(text: &str, line: u32) -> YamlNode {
    let t = text.trim();
    if t.is_empty() || t == "~" || t == "null" || t == "Null" || t == "NULL" {
        return YamlNode::new(Yaml::Null, line);
    }
    YamlNode::new(Yaml::Scalar(unquote(t)), line)
}

/// Bracket balance of a flow text outside quotes.
fn flow_balance(text: &str) -> i32 {
    let mut depth = 0;
    let mut in_single = false;
    let mut in_double = false;
    for c in text.chars() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '[' | '{' if !in_single && !in_double => depth += 1,
            ']' | '}' if !in_single && !in_double => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Parser for flow collections (`[...]`, `{...}`) over one joined string.
struct Flow<'s> {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    anchors: &'s HashMap<String, YamlNode>,
}

impl<'s> Flow<'s> {
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn value(&mut self) -> YamlNode {
        self.skip_ws();
        match self.peek() {
            Some('[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some(']') => {
                            self.pos += 1;
                            break;
                        }
                        None => break,
                        Some(',') => {
                            self.pos += 1;
                        }
                        Some(_) => {
                            let before = self.pos;
                            items.push(self.value());
                            if self.pos == before {
                                self.pos += 1;
                            }
                        }
                    }
                }
                YamlNode::new(Yaml::Seq(items), self.line)
            }
            Some('{') => {
                self.pos += 1;
                let mut entries = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some('}') => {
                            self.pos += 1;
                            break;
                        }
                        None => break,
                        Some(',') => {
                            self.pos += 1;
                        }
                        Some(_) => {
                            let before = self.pos;
                            let key = self.scalar_text(true);
                            self.skip_ws();
                            let value = if self.peek() == Some(':') {
                                self.pos += 1;
                                self.value()
                            } else {
                                YamlNode::new(Yaml::Null, self.line)
                            };
                            entries.push((unquote(&key), value));
                            if self.pos == before {
                                self.pos += 1;
                            }
                        }
                    }
                }
                YamlNode::new(Yaml::Map(entries), self.line)
            }
            Some('*') => {
                self.pos += 1;
                let name = self.scalar_text(false);
                self.anchors
                    .get(name.trim())
                    .cloned()
                    .unwrap_or_else(|| YamlNode::new(Yaml::Null, self.line))
            }
            _ => {
                let text = self.scalar_text(false);
                scalar_node(&text, self.line)
            }
        }
    }

    /// Scalar text up to a flow delimiter (`,`, `]`, `}` and, for keys, `:`).
    fn scalar_text(&mut self, is_key: bool) -> String {
        self.skip_ws();
        let mut out = String::new();
        if let Some(q) = self.peek().filter(|c| *c == '"' || *c == '\'') {
            out.push(q);
            self.pos += 1;
            while let Some(c) = self.peek() {
                self.pos += 1;
                out.push(c);
                if c == '\\' && q == '"' {
                    if let Some(n) = self.peek() {
                        out.push(n);
                        self.pos += 1;
                    }
                    continue;
                }
                if c == q {
                    break;
                }
            }
            return out;
        }
        while let Some(c) = self.peek() {
            if c == ',' || c == ']' || c == '}' {
                break;
            }
            if is_key && c == ':' {
                break;
            }
            if c == ':' && self.chars.get(self.pos + 1).map(|n| n.is_whitespace()).unwrap_or(true) && !is_key {
                break;
            }
            out.push(c);
            self.pos += 1;
        }
        out.trim().to_string()
    }
}

struct Parser<'a> {
    lines: Vec<Line<'a>>,
    pos: usize,
    anchors: HashMap<String, YamlNode>,
}

impl<'a> Parser<'a> {
    fn parse_block(&mut self) -> YamlNode {
        let line = match self.lines.get(self.pos) {
            Some(l) => *l,
            None => return YamlNode::new(Yaml::Null, 0),
        };
        if is_seq_item(line.text) {
            self.parse_seq(line.indent)
        } else if split_key(line.text).is_some() {
            self.parse_map(line.indent)
        } else {
            self.pos += 1;
            self.inline_value(line.text, line.number)
        }
    }

    fn parse_seq(&mut self, indent: usize) -> YamlNode {
        let start = self.lines[self.pos].number;
        let mut items = Vec::new();
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            if line.indent != indent || !is_seq_item(line.text) {
                if line.indent > indent {
                    self.pos += 1;
                    continue;
                }
                break;
            }
            let after_dash = line.text[1..].trim_start();
            let (anchor, rest) = take_anchor(after_dash);
            let rest = strip_tag(rest);
            let item = if rest.is_empty() {
                self.pos += 1;
                if self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
                    self.parse_block()
                } else {
                    YamlNode::new(Yaml::Null, line.number)
                }
            } else if is_seq_item(rest) || split_key(rest).is_some() {
                // The item's first line continues at the column where `rest` starts.
                let offset = line.text.len() - rest.len();
                self.lines[self.pos].indent = indent + offset;
                self.lines[self.pos].text = rest;
                self.lines[self.pos].raw = rest;
                self.parse_block()
            } else {
                self.pos += 1;
                self.inline_value(rest, line.number)
            };
            if let Some(name) = anchor {
                self.anchors.insert(name, item.clone());
            }
            items.push(item);
        }
        YamlNode::new(Yaml::Seq(items), start)
    }

    fn parse_map(&mut self, indent: usize) -> YamlNode {
        let start = self.lines[self.pos].number;
        let mut entries: Vec<(String, YamlNode)> = Vec::new();
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            if line.indent != indent {
                if line.indent > indent {
                    self.pos += 1;
                    continue;
                }
                break;
            }
            let (key, rest) = match split_key(line.text) {
                Some(kv) => kv,
                None => break,
            };
            self.pos += 1;
            let (anchor, rest) = take_anchor(rest);
            let rest = strip_tag(rest);
            let value = if rest.is_empty() {
                match self.lines.get(self.pos) {
                    Some(next) if next.indent > indent => self.parse_block(),
                    Some(next) if next.indent == indent && is_seq_item(next.text) => {
                        self.parse_seq(indent)
                    }
                    _ => YamlNode::new(Yaml::Null, line.number),
                }
            } else if rest.starts_with('|') || rest.starts_with('>') {
                self.block_scalar(indent, rest.starts_with('|'), line.number)
            } else {
                self.inline_value(rest, line.number)
            };
            if let Some(name) = anchor {
                self.anchors.insert(name, value.clone());
            }
            if key == "<<" {
                let sources: Vec<YamlNode> = match &value.value {
                    Yaml::Seq(items) => items.clone(),
                    _ => vec![value.clone()],
                };
                for src in sources {
                    for (k, v) in src.as_map() {
                        if !entries.iter().any(|(e, _)| e == k) {
                            entries.push((k.clone(), v.clone()));
                        }
                    }
                }
                continue;
            }
            match entries.iter_mut().find(|(k, _)| *k == key) {
                // A later key overrides a merged one.
                Some(existing) => existing.1 = value,
                None => entries.push((key, value)),
            }
        }
        YamlNode::new(Yaml::Map(entries), start)
    }

    /// Literal (`|`) or folded (`>`) block scalar: every following line indented deeper
    /// than the key.
    fn block_scalar(&mut self, indent: usize, literal: bool, line: u32) -> YamlNode {
        let mut parts: Vec<&str> = Vec::new();
        while self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
            parts.push(self.lines[self.pos].raw);
            self.pos += 1;
        }
        let text = if literal {
            parts.join("\n")
        } else {
            parts.join(" ")
        };
        YamlNode::new(Yaml::Scalar(text), line)
    }

    /// A value written on the key's line: alias, flow collection (possibly continued on
    /// following lines) or scalar.
    fn inline_value(&mut self, text: &str, line: u32) -> YamlNode {
        let text = text.trim();
        if let Some(name) = text.strip_prefix('*') {
            return self
                .anchors
                .get(name.trim())
                .cloned()
                .unwrap_or_else(|| YamlNode::new(Yaml::Null, line));
        }
        if text.starts_with('[') || text.starts_with('{') {
            let mut joined = text.to_string();
            while flow_balance(&joined) > 0 && self.pos < self.lines.len() {
                joined.push(' ');
                joined.push_str(self.lines[self.pos].text);
                self.pos += 1;
            }
            let mut flow = Flow {
                chars: joined.chars().collect(),
                pos: 0,
                line,
                anchors: &self.anchors,
            };
            return flow.value();
        }
        scalar_node(text, line)
    }
}
