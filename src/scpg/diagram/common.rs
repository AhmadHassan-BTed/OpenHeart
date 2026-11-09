//! Shared Diagram Utilities (§10.4).
//!
//! The single home of the identifier, escaping, hashing and truncation rules used by every
//! format printer, so PlantUML, Mermaid, JSON, XMI, DOT and GraphML agree on them. No printer
//! keeps a private copy of these rules.

use crate::ingestion::interner::fnv1a_64;

/// Common utilities for diagram identifiers and text formatting.
pub struct DiagramUtils;

impl DiagramUtils {
    /// Maps arbitrary text to an ASCII identifier fragment (`[A-Za-z0-9_]`).
    ///
    /// Distinct inputs may map to the same fragment (`Foo$Bar` and `Foo_Bar`); callers that need
    /// unique identifiers must go through [`crate::scpg::diagram::model::AliasTable`].
    pub fn sanitize(name: &str) -> String {
        name.trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Stable 64-bit FNV-1a hash over `parts`, separated by the ASCII unit separator so that
    /// `["ab", "c"]` and `["a", "bc"]` hash differently.
    pub fn stable_hash(parts: &[&str]) -> u64 {
        let mut buf = Vec::with_capacity(parts.iter().map(|p| p.len() + 1).sum());
        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                buf.push(0x1F);
            }
            buf.extend_from_slice(part.as_bytes());
        }
        fnv1a_64(&buf)
    }

    /// Sixteen lowercase hex digits of [`Self::stable_hash`].
    pub fn short_hash(parts: &[&str]) -> String {
        format!("{:016x}", Self::stable_hash(parts))
    }

    /// Lowercase hex encoding of a byte slice.
    pub fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }

    /// Returns the standard UML visibility marker for a visibility code.
    pub fn visibility_symbol(vis: u8) -> &'static str {
        match vis {
            1 => "+",
            2 => "-",
            3 => "#",
            _ => "~",
        }
    }

    /// Returns the UML visibility keyword (XMI `visibility` attribute) for a visibility code.
    pub fn visibility_keyword(vis: u8) -> &'static str {
        match vis {
            1 => "public",
            2 => "private",
            3 => "protected",
            _ => "package",
        }
    }

    /// Collapses runs of whitespace and control characters into single spaces.
    pub fn single_line(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut pending_space = false;
        for c in text.chars() {
            if c.is_whitespace() || c.is_control() {
                pending_space = !out.is_empty();
            } else {
                if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(c);
            }
        }
        out
    }

    /// Truncates `text` to at most `max_chars` characters, always on a character boundary,
    /// marking the cut with an ellipsis.
    pub fn truncate(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }

    /// Text placed inside a PlantUML double-quoted name. PlantUML has no escape for `"`, so it
    /// becomes `'`; embedded newlines become the PlantUML line-break sequence `\n`.
    pub fn plantuml_quoted(text: &str) -> String {
        text.split('\n')
            .map(|line| Self::single_line(line).replace('"', "'"))
            .collect::<Vec<_>>()
            .join("\\n")
    }

    /// Text placed after a `:` (edge labels, description lines). Newlines become `\n`.
    pub fn plantuml_label(text: &str) -> String {
        text.split('\n')
            .map(Self::single_line)
            .collect::<Vec<_>>()
            .join("\\n")
    }

    /// Text placed inside a PlantUML stereotype `<<...>>`.
    pub fn plantuml_stereotype(text: &str) -> String {
        Self::single_line(text).replace(['<', '>', '"'], "")
    }

    /// Text placed inside a quoted Mermaid label. Characters with syntactic meaning are written
    /// as Mermaid entity codes; newlines become `<br/>`.
    pub fn mermaid_text(text: &str) -> String {
        text.split('\n')
            .map(|line| {
                let mut out = String::with_capacity(line.len());
                for c in Self::single_line(line).chars() {
                    match c {
                        '#' => out.push_str("#35;"),
                        '"' => out.push_str("#quot;"),
                        '<' => out.push_str("#lt;"),
                        '>' => out.push_str("#gt;"),
                        '|' => out.push_str("#124;"),
                        ';' => out.push_str("#59;"),
                        '`' => out.push_str("#96;"),
                        _ => out.push(c),
                    }
                }
                out
            })
            .collect::<Vec<_>>()
            .join("<br/>")
    }

    /// Text placed in a Mermaid class-diagram member or relation label, where generics are
    /// written `~T~` and quotes are not allowed.
    pub fn mermaid_class_text(text: &str) -> String {
        Self::single_line(text)
            .replace('<', "~")
            .replace('>', "~")
            .replace('"', "'")
            .replace(['{', '}', ';', '#'], "")
    }

    /// Escapes text for an XML attribute value or text node and drops characters that are not
    /// allowed in XML 1.0.
    pub fn xml_escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                '\t' => out.push_str("&#9;"),
                '\n' => out.push_str("&#10;"),
                '\r' => out.push_str("&#13;"),
                c if (c as u32) < 0x20 => {}
                '\u{FFFE}' | '\u{FFFF}' => {}
                _ => out.push(c),
            }
        }
        out
    }

    /// Escapes text for a Graphviz DOT double-quoted string.
    pub fn dot_escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if c.is_control() => {}
                _ => out.push(c),
            }
        }
        out
    }
}
