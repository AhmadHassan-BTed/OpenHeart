//! Token positions of traceability records (§7.4).
//!
//! A token's sort key gives its *start*: file, 1-based line and 0-based byte column. The end
//! of a span is the end of its last token, which is not `start column + length` for a token
//! that contains line breaks (text blocks, block comments): its end lies on a later line.
//! [`token_end`] derives it from the token text, so span ends are exact.
//!
//! Spans are token-id ranges and token ids are corpus-global, so a range taken from another
//! artifact can run past the end of its file. [`clamp_span`] keeps every range inside the
//! file of its first token.

use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;

/// File id encoded in a token's sort key.
#[inline]
pub fn file_of(tca: &TokenCorpusArtifact, tok: u32) -> u16 {
    unpack_sort_key(tca.token_records[tok as usize].sort_key).0
}

/// `(file_id, line, column)` of the first byte of token `tok`.
#[inline]
pub fn token_start(tca: &TokenCorpusArtifact, tok: u32) -> (u16, u32, u16) {
    unpack_sort_key(tca.token_records[tok as usize].sort_key)
}

/// `(line, column)` just past the last byte of token `tok` (0-based, exclusive column),
/// accounting for line breaks inside the token. The token text is authoritative; its stored
/// length (saturated at `u16::MAX`) is used only when no text is recorded.
pub fn token_end(tca: &TokenCorpusArtifact, tok: u32) -> (u32, u32) {
    let rec = &tca.token_records[tok as usize];
    let (_, line, col) = unpack_sort_key(rec.sort_key);
    let text = tca.interner.lookup_text(rec.text_id);
    if text.is_empty() {
        return (line, col as u32 + rec.len as u32);
    }
    match text.iter().rposition(|&b| b == b'\n') {
        None => (line, (col as u32).saturating_add(text.len() as u32)),
        Some(last_newline) => {
            let breaks = text.iter().filter(|&&b| b == b'\n').count() as u32;
            let tail = (text.len() - 1 - last_newline) as u32;
            (line.saturating_add(breaks), tail)
        }
    }
}

/// The token range `(first, last)` clamped to the corpus and to the file of `first`, or
/// `None` when it is empty or `first` is not a token.
pub fn clamp_span(tca: &TokenCorpusArtifact, first: u32, last: u32) -> Option<(u32, u32)> {
    let n = tca.token_records.len();
    if n == 0 || first == u32::MAX || first as usize >= n {
        return None;
    }
    let mut last = last.min(n as u32 - 1);
    if last < first {
        return None;
    }
    let file = file_of(tca, first);
    if file_of(tca, last) != file {
        last = last_token_of_file(tca, file, first);
    }
    Some((first, last))
}

/// The last token of `file`, given that token `from` belongs to it. Tokens are sorted by
/// sort key, so a file's tokens are contiguous and the boundary is found by binary search.
fn last_token_of_file(tca: &TokenCorpusArtifact, file: u16, from: u32) -> u32 {
    let records = &tca.token_records;
    let upper = records.partition_point(|r| unpack_sort_key(r.sort_key).0 <= file);
    let candidate = (upper as u32).saturating_sub(1).max(from);
    if (candidate as usize) < records.len() && file_of(tca, candidate) == file {
        candidate
    } else {
        // The corpus is not sorted as Phase 1 guarantees; the single token is certain.
        from
    }
}

/// Saturating conversion of a byte column to the 16-bit column fields of TRA records.
#[inline]
pub fn column_u16(col: u32) -> u16 {
    u16::try_from(col).unwrap_or(u16::MAX)
}
