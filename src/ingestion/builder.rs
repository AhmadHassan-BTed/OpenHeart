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
        self.sort_records();

        // ── Invariant 1: Monotonicity ──
        let total_tokens = self.token_entries.len();

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
        let sum_file_tokens: u32 = file_records.iter().map(|f| f.file_token_count).sum();
        if sum_file_tokens as usize != total_tokens {
            return Err(format!(
                "Invariant 3 Violation (Completeness): Sum of file token counts ({}) != total tokens ({})",
                sum_file_tokens, total_tokens
            ));
        }

        // ── Invariant 4: Forward-Backward Consistency ──
        for (token_id, entry) in self.token_entries.iter().enumerate() {
            match self
                .token_records
                .binary_search_by_key(&entry.sort_key, |r| r.sort_key)
            {
                Ok(idx) => {
                    let matched = &self.token_records[idx];
                    if matched.text_id != entry.text_id || matched.token_type != entry.token_type {
                        return Err(format!(
                            "Invariant 4 Violation: Backward token_id {} does not match forward record at sort_key 0x{:016X}",
                            token_id, entry.sort_key
                        ));
                    }
                }
                Err(_) => {
                    return Err(format!(
                        "Invariant 4 Violation: Backward token_id {} sort_key 0x{:016X} not found in forward index",
                        token_id, entry.sort_key
                    ));
                }
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
