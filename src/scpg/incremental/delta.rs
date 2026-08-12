//! SourceDelta & UpdateResult data structures for incremental rebuilds (§10.6.2).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildScope {
    None,
    Behavioral,
    Structural,
    Full,
}

#[derive(Debug, Clone)]
pub struct SourceDelta {
    pub file_id: u16,
    pub is_comment_or_ws_only: bool,
    pub is_method_body_only: bool,
    pub is_type_decl_changed: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub scope: RebuildScope,
    pub stale_link_count: usize,
    pub new_scpg_hash: u32,
}
