//! AST Backward Index (§7.3, §7.4).

use crate::ast::BPASTArtifact;
use crate::tra::types::BIAstEntry;

pub struct ASTBackwardIndex;

impl ASTBackwardIndex {
    pub fn build(bpa: &BPASTArtifact) -> Vec<BIAstEntry> {
        let count = bpa.node_count as usize;
        let mut bi_ast = Vec::with_capacity(count);

        for i in 0..count {
            let (first_tok, last_tok) = bpa.token_range(i as u32);
            bi_ast.push(BIAstEntry {
                first_token_id: first_tok,
                last_token_id: last_tok,
            });
        }

        bi_ast
    }
}
