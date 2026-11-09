use crate::core::types::source::SourceFileRecord;
use crate::core::types::token::{TokenEntry, TokenRecord};
use crate::ingestion::interner::StringInterner;
use crate::ingestion::serializer::TokenCorpusArtifact;

pub struct TokenCorpusBuilder {
    token_records: Vec<TokenRecord>,
    token_entries: Vec<TokenEntry>,
}

impl TokenCorpusBuilder {
    pub fn new() -> Self {
        Self {
            token_records: Vec::new(),
            token_entries: Vec::new(),
        }
    }

    pub fn with_capacity(tokens: usize) -> Self {
        Self {
            token_records: Vec::with_capacity(tokens),
            token_entries: Vec::with_capacity(tokens),
        }
    }

    pub fn len(&self) -> usize {
        self.token_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.token_entries.is_empty()
    }

    pub fn push(&mut self, token_id: u32, record: TokenRecord) {
        debug_assert_eq!(token_id as usize, self.token_entries.len());

        let entry = TokenEntry {
            sort_key: record.sort_key,
            text_id: record.text_id,
            len: record.len,
            token_type: record.token_type,
            _padding: record._padding,
        };

        self.token_records.push(record);
        self.token_entries.push(entry);
    }

    pub fn sort_records(&mut self) {
        self.token_records.sort_unstable_by_key(|r| r.sort_key);
    }

    pub fn finalize(
        mut self,
        file_records: Vec<SourceFileRecord>,
        interner: StringInterner,
    ) -> Result<TokenCorpusArtifact, String> {
        let total_tokens = self.token_entries.len();

        // ── Invariant 1: Monotonicity ──
        // Token ids are allocated in (file, line, column) order, so the backward entry map
        // (indexed by token id) must already be strictly increasing by sort key. Consumers
        // index the sorted forward table by token id; this check is what makes that valid.
        for i in 1..total_tokens {
            let prev = self.token_entries[i - 1].sort_key;
            let cur = self.token_entries[i].sort_key;
            if cur <= prev {
                return Err(format!(
                    "Invariant 1 Violation (Monotonicity): token_id {} has sort_key 0x{:016X}, \
                     not above token_id {} at 0x{:016X}",
                    i,
                    cur,
                    i - 1,
                    prev
                ));
            }
        }

        self.sort_records();

        // ── Invariant 2: Injectivity ──
        for i in 1..self.token_records.len() {
            if self.token_records[i].sort_key == self.token_records[i - 1].sort_key {
                return Err(format!(
                    "Invariant 2 Violation (Injectivity): Duplicate sort_key 0x{:016X} found at index {}",
                    self.token_records[i].sort_key, i
                ));
            }
        }

        // ── Invariant 3: Completeness ──
        let mut expected_first: u64 = 0;
        for rec in &file_records {
            if rec.file_token_count > 0 && rec.first_token_id as u64 != expected_first {
                return Err(format!(
                    "Invariant 3 Violation (Completeness): file {} starts at token {} but the \
                     previous file ended at {}",
                    rec.file_id, rec.first_token_id, expected_first
                ));
            }
            expected_first += rec.file_token_count as u64;
        }
        if expected_first != total_tokens as u64 {
            return Err(format!(
                "Invariant 3 Violation (Completeness): Sum of file token counts ({}) != total tokens ({})",
                expected_first, total_tokens
            ));
        }

        // ── Invariant 4: Forward-Backward Consistency ──
        // With Invariant 1, sorting is the identity, so forward index i is token_id i.
        for (token_id, entry) in self.token_entries.iter().enumerate() {
            let matched = &self.token_records[token_id];
            if matched.sort_key != entry.sort_key
                || matched.text_id != entry.text_id
                || matched.token_type != entry.token_type
            {
                return Err(format!(
                    "Invariant 4 Violation: Backward token_id {} does not match forward record at sort_key 0x{:016X}",
                    token_id, entry.sort_key
                ));
            }
        }

        Ok(TokenCorpusArtifact {
            file_records,
            token_records: self.token_records,
            token_entries: self.token_entries,
            interner,
        })
    }
}

impl Default for TokenCorpusBuilder {
    fn default() -> Self {
        Self::new()
    }
}
