//! Token-text access shared by Phases 4-6.
//!
//! The BP AST stores token *ranges* only. Matching `break outer;` to its label, telling
//! a call's name apart from its receiver, splitting a `for` header at its semicolons and
//! resolving identifier occurrences to variables all need the token text held by the
//! token corpus (TCA). Every consumer receives an `Option<SourceText>`: when no corpus
//! is supplied the callers fall back to purely structural reasoning and record that the
//! result is approximate, instead of guessing names.

use crate::ast::BPASTArtifact;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::serializer::TokenCorpusArtifact;

/// Read-only view over the token corpus used to recover token text.
#[derive(Clone, Copy)]
pub struct SourceText<'a> {
    tca: &'a TokenCorpusArtifact,
}

/// Token positions of a parenthesised header such as `for (init; cond; update)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParenHeader {
    /// Token id of the opening `(`.
    pub open: u32,
    /// Token id of the matching `)`.
    pub close: u32,
    /// Token ids of the requested separator found at nesting depth 1, in order.
    pub separators: Vec<u32>,
}

impl<'a> SourceText<'a> {
    pub fn new(tca: &'a TokenCorpusArtifact) -> Self {
        Self { tca }
    }

    #[inline]
    pub fn token_count(&self) -> usize {
        self.tca.token_records.len()
    }

    /// Interned text id of token `tok`.
    #[inline]
    pub fn text_id(&self, tok: u32) -> Option<u32> {
        let tca: &'a TokenCorpusArtifact = self.tca;
        tca.token_records.get(tok as usize).map(|r| r.text_id)
    }

    /// Text of token `tok`.
    #[inline]
    pub fn text(&self, tok: u32) -> Option<&'a [u8]> {
        let tca: &'a TokenCorpusArtifact = self.tca;
        self.text_id(tok).map(|id| tca.interner.lookup_text(id))
    }

    /// Text of an interned string id.
    #[inline]
    pub fn text_of_id(&self, text_id: u32) -> &'a [u8] {
        let tca: &'a TokenCorpusArtifact = self.tca;
        tca.interner.lookup_text(text_id)
    }

    /// Interned id of `text`, if the corpus contains it.
    pub fn find_text_id(&self, text: &[u8]) -> Option<u32> {
        let id = self.tca.interner.find_id(text);
        if id == u32::MAX {
            None
        } else {
            Some(id)
        }
    }

    #[inline]
    pub fn token_is(&self, tok: u32, expected: &[u8]) -> bool {
        self.text(tok) == Some(expected)
    }

    /// File id of token `tok`.
    #[inline]
    pub fn file_of(&self, tok: u32) -> Option<u16> {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| unpack_sort_key(r.sort_key).0)
    }

    /// Text id of the first token of `node` (the identifier itself for leaf nodes).
    pub fn node_text_id(&self, bpa: &BPASTArtifact, node: u32) -> Option<u32> {
        let (ft, _) = bpa.token_range(node);
        if ft == u32::MAX {
            None
        } else {
            self.text_id(ft)
        }
    }

    /// Text of the first token of `node`.
    pub fn node_text(&self, bpa: &BPASTArtifact, node: u32) -> Option<&'a [u8]> {
        let (ft, _) = bpa.token_range(node);
        if ft == u32::MAX {
            None
        } else {
            self.text(ft)
        }
    }

    /// Text of the token immediately preceding the first token of `node`.
    pub fn text_before(&self, bpa: &BPASTArtifact, node: u32) -> Option<&'a [u8]> {
        let (ft, _) = bpa.token_range(node);
        if ft == u32::MAX || ft == 0 {
            return None;
        }
        if self.file_of(ft - 1) != self.file_of(ft) {
            return None;
        }
        self.text(ft - 1)
    }

    /// Text of the token immediately following the last token of `node`.
    pub fn text_after(&self, bpa: &BPASTArtifact, node: u32) -> Option<&'a [u8]> {
        let (ft, lt) = bpa.token_range(node);
        if ft == u32::MAX {
            return None;
        }
        let next = lt.checked_add(1)?;
        if self.file_of(next) != self.file_of(lt) {
            return None;
        }
        self.text(next)
    }

    /// Locates the first `(` at or after token `start` and its matching `)`, collecting
    /// every `separator` token found directly inside the parentheses (depth 1). Brackets
    /// and braces count towards nesting, so separators inside lambdas, array initialisers
    /// or nested calls are ignored. The scan never leaves the file of `start`.
    pub fn paren_header(&self, start: u32, separator: Option<&[u8]>) -> Option<ParenHeader> {
        let file = self.file_of(start)?;
        let count = self.token_count();
        let mut tok = start as usize;
        let mut open = None;
        while tok < count {
            if self.file_of(tok as u32) != Some(file) {
                return None;
            }
            if self.token_is(tok as u32, b"(") {
                open = Some(tok as u32);
                break;
            }
            tok += 1;
        }
        let open = open?;

        let mut depth = 0usize;
        let mut separators = Vec::new();
        let mut tok = open as usize;
        while tok < count {
            let t = tok as u32;
            if self.file_of(t) != Some(file) {
                return None;
            }
            match self.text(t) {
                Some(b"(") | Some(b"[") | Some(b"{") => depth += 1,
                Some(b")") | Some(b"]") | Some(b"}") => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(ParenHeader {
                            open,
                            close: t,
                            separators,
                        });
                    }
                }
                Some(text) => {
                    if depth == 1 && separator == Some(text) {
                        separators.push(t);
                    }
                }
                None => return None,
            }
            tok += 1;
        }
        None
    }

    /// True when any token in `[from, to]` (inclusive) has text `expected`.
    pub fn range_contains(&self, from: u32, to: u32, expected: &[u8]) -> bool {
        if from == u32::MAX || from > to {
            return false;
        }
        (from..=to).any(|t| self.token_is(t, expected))
    }
}
