//! LabelExtractor — human-readable labels from statement token spans (§9.2.6),
//! and the LabelTable that interns them into `UMLMetadataArtifact.label_texts`.

use std::collections::HashMap;

use crate::ast::BPASTArtifact;
use crate::core::types::token::TokenType;
use crate::ingestion::TokenCorpusArtifact;
use crate::uma::types::NO_LABEL;

/// Interns label strings to dense ids starting at 1 (0 is `NO_LABEL`).
/// Ids follow first-insertion order, so identical inputs yield identical ids.
#[derive(Debug, Default)]
pub struct LabelTable {
    ids: HashMap<String, u32>,
    texts: Vec<String>,
}

impl LabelTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a label; the empty string maps to `NO_LABEL`.
    pub fn intern(&mut self, text: &str) -> u32 {
        if text.is_empty() {
            return NO_LABEL;
        }
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = match u32::try_from(self.texts.len() + 1) {
            Ok(id) if id != u32::MAX => id,
            _ => return NO_LABEL,
        };
        self.texts.push(text.to_string());
        self.ids.insert(text.to_string(), id);
        id
    }

    pub fn get(&self, id: u32) -> Option<&str> {
        if id == NO_LABEL {
            return None;
        }
        self.texts.get(id as usize - 1).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    pub fn into_map(self) -> HashMap<u32, String> {
        self.texts
            .into_iter()
            .enumerate()
            .map(|(i, t)| (i as u32 + 1, t))
            .collect()
    }
}

pub struct LabelExtractor;

impl LabelExtractor {
    /// Source text of an AST statement, rebuilt from its token span and truncated
    /// on a character boundary. `None` when the node has no tokens.
    pub fn statement_label(
        stmt_node: u32,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        max_chars: usize,
    ) -> Option<String> {
        if stmt_node == u32::MAX || stmt_node >= bpa.node_count {
            return None;
        }
        let (first, last) = bpa.token_range(stmt_node);
        Self::token_span_label(first, last, tca, max_chars)
    }

    /// Source text of the inclusive token span `[first, last]`.
    pub fn token_span_label(
        first: u32,
        last: u32,
        tca: &TokenCorpusArtifact,
        max_chars: usize,
    ) -> Option<String> {
        if first == u32::MAX || first > last || (first as usize) >= tca.token_records.len() {
            return None;
        }
        let last = last.min(tca.token_records.len() as u32 - 1);
        let mut out = String::new();
        let mut prev: &[u8] = b"";
        for tok in first..=last {
            let rec = &tca.token_records[tok as usize];
            if matches!(
                TokenType::from_u8(rec.token_type),
                TokenType::CommentLine
                    | TokenType::CommentBlock
                    | TokenType::CommentDoc
                    | TokenType::Whitespace
                    | TokenType::Newline
            ) {
                continue;
            }
            let bytes = tca.interner.lookup_text(rec.text_id);
            if bytes.is_empty() {
                continue;
            }
            if !out.is_empty() && needs_space(prev, bytes) {
                out.push(' ');
            }
            out.push_str(&String::from_utf8_lossy(bytes));
            prev = bytes;
            if out.chars().count() > max_chars.saturating_mul(4) {
                break;
            }
        }
        if out.is_empty() {
            return None;
        }
        Some(truncate_chars(&out, max_chars))
    }
}

/// Truncate to at most `max_chars` characters, appending `...` when cut.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let cut = s.char_indices().nth(keep).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}...", &s[..cut])
}

fn needs_space(prev: &[u8], next: &[u8]) -> bool {
    const NO_SPACE_BEFORE: &[&[u8]] = &[b".", b",", b";", b")", b"]", b"(", b"[", b"++", b"--", b"::", b"?."];
    const NO_SPACE_AFTER: &[&[u8]] = &[b".", b"(", b"[", b"@", b"!", b"::", b"?."];
    if NO_SPACE_BEFORE.contains(&next) || NO_SPACE_AFTER.contains(&prev) {
        return false;
    }
    true
}
