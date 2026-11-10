//! Source-layer indexes: file table, token positions and AST token ranges (§10.5).
//!
//! These are the columns the navigation, slicing and incremental layers need to turn entity
//! ids back into source coordinates without the original `.tca`/`.bpa` artifacts.

use crate::ast::BPASTArtifact;
use crate::core::types::token::{unpack_sort_key, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder, StringColumn};
use crate::scpg::types::ScpgFormatError;

/// A 1-based line / 0-based byte column position, as produced by Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePosition {
    pub file_id: u16,
    pub line: u32,
    pub col: u16,
}

/// Packs `(line, col)` into a totally ordered key.
pub fn position_key(line: u32, col: u16) -> u64 {
    ((line as u64) << 16) | col as u64
}

#[derive(Debug, Clone, Default)]
pub struct FileTable {
    file_ids: Vec<u16>,
    languages: Vec<u8>,
    content_hashes: Vec<u8>,
    sizes: Vec<u64>,
    first_tokens: Vec<u32>,
    token_counts: Vec<u32>,
    paths: StringColumn,
}

impl FileTable {
    pub fn from_tca(tca: &TokenCorpusArtifact) -> Self {
        let mut records: Vec<_> = tca.file_records.iter().collect();
        records.sort_by_key(|f| f.file_id);
        let mut table = Self {
            paths: StringColumn::new(),
            ..Default::default()
        };
        for f in records {
            table.file_ids.push(f.file_id);
            table.languages.push(f.language_id);
            table.content_hashes.extend_from_slice(&f.content_sha256);
            table.sizes.push(f.file_size_bytes);
            table.first_tokens.push(f.first_token_id);
            table.token_counts.push(f.file_token_count);
            let path =
                String::from_utf8_lossy(tca.interner.lookup_text(f.path_str_offset)).into_owned();
            table.paths.push(&path);
        }
        table
    }

    pub fn len(&self) -> usize {
        self.file_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.file_ids.is_empty()
    }

    fn slot(&self, file_id: u16) -> Option<usize> {
        self.file_ids.binary_search(&file_id).ok()
    }

    /// Path recorded by Phase 1 for `file_id`; `None` if the id is unknown or unrecorded.
    pub fn path(&self, file_id: u16) -> Option<&str> {
        self.slot(file_id)
            .and_then(|i| self.paths.get(i))
            .filter(|p| !p.is_empty())
    }

    pub fn content_hash(&self, file_id: u16) -> Option<[u8; 32]> {
        let i = self.slot(file_id)?;
        let bytes = self.content_hashes.get(i * 32..i * 32 + 32)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(bytes);
        Some(out)
    }

    pub fn language(&self, file_id: u16) -> Option<u8> {
        self.slot(file_id).map(|i| self.languages[i])
    }

    pub fn size(&self, file_id: u16) -> Option<u64> {
        self.slot(file_id).map(|i| self.sizes[i])
    }

    /// Looks up a file by its recorded path (exact match, or matching path suffix).
    pub fn find_by_path(&self, path: &str) -> Option<u16> {
        let normalized = path.replace('\\', "/");
        let mut suffix_match = None;
        for (i, &file_id) in self.file_ids.iter().enumerate() {
            let candidate = self.paths.get_or_empty(i).replace('\\', "/");
            if candidate == normalized {
                return Some(file_id);
            }
            if suffix_match.is_none() && !normalized.is_empty() && candidate.ends_with(&normalized)
            {
                suffix_match = Some(file_id);
            }
        }
        suffix_match
    }

    pub fn file_ids(&self) -> &[u16] {
        &self.file_ids
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u16s(&self.file_ids);
        enc.put_u8s(&self.languages);
        enc.put_u8s(&self.content_hashes);
        enc.put_u64s(&self.sizes);
        enc.put_u32s(&self.first_tokens);
        enc.put_u32s(&self.token_counts);
        self.paths.encode(enc);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let file_ids = dec.u16s()?;
        let n = file_ids.len();
        let languages = dec.u8s()?.to_vec();
        let content_hashes = dec.u8s()?.to_vec();
        let sizes = dec.u64s()?;
        let first_tokens = dec.u32s()?;
        let token_counts = dec.u32s()?;
        let paths = StringColumn::decode(dec)?;
        dec.expect_len("languages", languages.len(), n)?;
        dec.expect_len("content_hashes", content_hashes.len(), n * 32)?;
        dec.expect_len("sizes", sizes.len(), n)?;
        dec.expect_len("first_tokens", first_tokens.len(), n)?;
        dec.expect_len("token_counts", token_counts.len(), n)?;
        dec.expect_len("paths", paths.len(), n)?;
        if file_ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err(dec.malformed("file ids are not strictly increasing"));
        }
        Ok(Self {
            file_ids,
            languages,
            content_hashes,
            sizes,
            first_tokens,
            token_counts,
            paths,
        })
    }
}

/// Dense per-token positions (indexed by token id) plus a position-sorted permutation.
#[derive(Debug, Clone, Default)]
pub struct TokenIndex {
    sort_keys: Vec<u64>,
    lens: Vec<u16>,
    types: Vec<u8>,
    by_position: Vec<u32>,
}

impl TokenIndex {
    pub fn from_tca(tca: &TokenCorpusArtifact) -> Self {
        // `token_entries` is the dense backward index keyed by token id; fall back to the
        // forward records when a corpus was produced without it.
        let (sort_keys, lens, types): (Vec<u64>, Vec<u16>, Vec<u8>) =
            if !tca.token_entries.is_empty() {
                (
                    tca.token_entries.iter().map(|t| t.sort_key).collect(),
                    tca.token_entries.iter().map(|t| t.len).collect(),
                    tca.token_entries.iter().map(|t| t.token_type).collect(),
                )
            } else {
                (
                    tca.token_records.iter().map(|t| t.sort_key).collect(),
                    tca.token_records.iter().map(|t| t.len).collect(),
                    tca.token_records.iter().map(|t| t.token_type).collect(),
                )
            };
        let mut by_position: Vec<u32> = (0..sort_keys.len() as u32).collect();
        by_position.sort_by_key(|&t| (sort_keys[t as usize], t));
        Self {
            sort_keys,
            lens,
            types,
            by_position,
        }
    }

    pub fn len(&self) -> usize {
        self.sort_keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sort_keys.is_empty()
    }

    pub fn start(&self, token: u32) -> Option<SourcePosition> {
        let key = *self.sort_keys.get(token as usize)?;
        let (file_id, line, col) = unpack_sort_key(key);
        Some(SourcePosition { file_id, line, col })
    }

    /// Exclusive end position on the start line (tokens are measured in bytes).
    pub fn end(&self, token: u32) -> Option<SourcePosition> {
        let start = self.start(token)?;
        let len = self.lens[token as usize];
        Some(SourcePosition {
            col: start.col.saturating_add(len),
            ..start
        })
    }

    pub fn byte_len(&self, token: u32) -> Option<u16> {
        self.lens.get(token as usize).copied()
    }

    pub fn token_type(&self, token: u32) -> Option<TokenType> {
        self.types
            .get(token as usize)
            .map(|&t| TokenType::from_u8(t))
    }

    pub fn is_trivia(&self, token: u32) -> bool {
        matches!(
            self.token_type(token),
            Some(TokenType::CommentLine)
                | Some(TokenType::CommentBlock)
                | Some(TokenType::CommentDoc)
                | Some(TokenType::Whitespace)
                | Some(TokenType::Newline)
        )
    }

    /// Token ids of `file_id` whose start lies in `[from, to)` (position order).
    pub fn tokens_starting_in(&self, file_id: u16, from: (u32, u16), to: (u32, u16)) -> &[u32] {
        let key = |line: u32, col: u16| {
            ((file_id as u64) << 48) | (((line & 0x00FF_FFFF) as u64) << 24) | ((col as u64) << 8)
        };
        let lo_key = key(from.0, from.1);
        let hi_key = key(to.0, to.1);
        let lo = self
            .by_position
            .partition_point(|&t| self.sort_keys[t as usize] < lo_key);
        let hi = self
            .by_position
            .partition_point(|&t| self.sort_keys[t as usize] < hi_key);
        &self.by_position[lo..hi.max(lo)]
    }

    /// The last token of `file_id` starting at or before `(line, col)`.
    pub fn token_at_or_before(&self, file_id: u16, line: u32, col: u16) -> Option<u32> {
        let file_lo = (file_id as u64) << 48;
        let probe = file_lo | (((line & 0x00FF_FFFF) as u64) << 24) | ((col as u64) << 8) | 0xFF;
        let idx = self
            .by_position
            .partition_point(|&t| self.sort_keys[t as usize] <= probe);
        if idx == 0 {
            return None;
        }
        let t = self.by_position[idx - 1];
        if self.sort_keys[t as usize] >> 48 == file_id as u64 {
            Some(t)
        } else {
            None
        }
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u64s(&self.sort_keys);
        enc.put_u16s(&self.lens);
        enc.put_u8s(&self.types);
        enc.put_u32s(&self.by_position);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let sort_keys = dec.u64s()?;
        let lens = dec.u16s()?;
        let types = dec.u8s()?.to_vec();
        let by_position = dec.u32s()?;
        let n = sort_keys.len();
        dec.expect_len("lens", lens.len(), n)?;
        dec.expect_len("types", types.len(), n)?;
        dec.expect_len("by_position", by_position.len(), n)?;
        if by_position.iter().any(|&t| t as usize >= n) {
            return Err(dec.malformed("token permutation references an unknown token"));
        }
        if by_position
            .windows(2)
            .any(|w| sort_keys[w[0] as usize] > sort_keys[w[1] as usize])
        {
            return Err(dec.malformed("token permutation is not position-sorted"));
        }
        Ok(Self {
            sort_keys,
            lens,
            types,
            by_position,
        })
    }
}

/// Per-AST-node type and token range (pre-order indexed), for statement locations.
#[derive(Debug, Clone, Default)]
pub struct AstIndex {
    node_types: Vec<u8>,
    first_tokens: Vec<u32>,
    last_tokens: Vec<u32>,
}

impl AstIndex {
    pub fn from_bpa(bpa: &BPASTArtifact) -> Self {
        let n = bpa.node_count as usize;
        let mut index = Self {
            node_types: Vec::with_capacity(n),
            first_tokens: Vec::with_capacity(n),
            last_tokens: Vec::with_capacity(n),
        };
        for node in 0..bpa.node_count {
            let (first, last) = bpa.token_range(node);
            index.node_types.push(
                bpa.preorder
                    .node_types
                    .get(node as usize)
                    .copied()
                    .unwrap_or(0),
            );
            index.first_tokens.push(first);
            index.last_tokens.push(last);
        }
        index
    }

    pub fn len(&self) -> usize {
        self.node_types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.node_types.is_empty()
    }

    pub fn node_type(&self, node: u32) -> Option<u8> {
        self.node_types.get(node as usize).copied()
    }

    pub fn token_range(&self, node: u32) -> Option<(u32, u32)> {
        let first = *self.first_tokens.get(node as usize)?;
        let last = *self.last_tokens.get(node as usize)?;
        if first == u32::MAX || last == u32::MAX || last < first {
            None
        } else {
            Some((first, last))
        }
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u8s(&self.node_types);
        enc.put_u32s(&self.first_tokens);
        enc.put_u32s(&self.last_tokens);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let node_types = dec.u8s()?.to_vec();
        let first_tokens = dec.u32s()?;
        let last_tokens = dec.u32s()?;
        dec.expect_len("first_tokens", first_tokens.len(), node_types.len())?;
        dec.expect_len("last_tokens", last_tokens.len(), node_types.len())?;
        Ok(Self {
            node_types,
            first_tokens,
            last_tokens,
        })
    }
}
