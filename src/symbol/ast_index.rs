//! AstIndex: Phase-3-local structural view over the BP AST.
//!
//! Children are derived once from the explicit preorder parent map as a CSR table, so Phase 3
//! never depends on the succinct select/rank navigation of `BPASTArtifact` and every pass gets
//! O(1) child slices. The index also records, for every node, the root (compilation unit) it
//! belongs to, the preorder end of its subtree, and the source file and language of each root.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::token::{unpack_sort_key, LangId};
use crate::ingestion::TokenCorpusArtifact;
use std::collections::HashMap;

pub struct AstIndex {
    parents: Vec<u32>,
    child_offsets: Vec<u32>,
    children: Vec<u32>,
    subtree_end: Vec<u32>,
    root_of: Vec<u32>,
    roots: Vec<u32>,
    file_of_root: HashMap<u32, u16>,
    lang_of_file: HashMap<u16, LangId>,
}

impl AstIndex {
    pub fn build(bpa: &BPASTArtifact, tca: &TokenCorpusArtifact) -> Self {
        let n = bpa.node_count as usize;

        // Parents precede their children in preorder; any other value is a corrupt link and
        // the node is treated as a root instead of producing a cyclic structure.
        let mut parents = Vec::with_capacity(n);
        let mut child_counts = vec![0u32; n];
        for i in 0..n {
            let p = bpa.parent(i as u32);
            let p = if (p as usize) < i { p } else { u32::MAX };
            if p != u32::MAX {
                child_counts[p as usize] += 1;
            }
            parents.push(p);
        }

        let mut child_offsets = vec![0u32; n + 1];
        for i in 0..n {
            child_offsets[i + 1] = child_offsets[i] + child_counts[i];
        }
        let mut fill: Vec<u32> = child_offsets[..n].to_vec();
        let mut children = vec![0u32; child_offsets[n] as usize];
        for (i, &p) in parents.iter().enumerate() {
            if p != u32::MAX {
                let slot = &mut fill[p as usize];
                children[*slot as usize] = i as u32;
                *slot += 1;
            }
        }

        let mut subtree_end = vec![0u32; n];
        for i in (0..n).rev() {
            let (lo, hi) = (child_offsets[i] as usize, child_offsets[i + 1] as usize);
            subtree_end[i] = if hi > lo {
                subtree_end[children[hi - 1] as usize]
            } else {
                i as u32 + 1
            };
        }

        let mut root_of = vec![0u32; n];
        let mut roots = Vec::new();
        for i in 0..n {
            if parents[i] == u32::MAX {
                root_of[i] = i as u32;
                roots.push(i as u32);
            } else {
                root_of[i] = root_of[parents[i] as usize];
            }
        }

        let mut file_of_root = HashMap::new();
        for &r in &roots {
            let (ft, _) = bpa.token_range(r);
            if (ft as usize) < tca.token_records.len() {
                let fid = unpack_sort_key(tca.token_records[ft as usize].sort_key).0;
                file_of_root.insert(r, fid);
            }
        }

        let lang_of_file = tca
            .file_records
            .iter()
            .map(|rec| (rec.file_id, LangId::from(rec.language_id)))
            .collect();

        Self {
            parents,
            child_offsets,
            children,
            subtree_end,
            root_of,
            roots,
            file_of_root,
            lang_of_file,
        }
    }

    pub fn node_count(&self) -> u32 {
        self.parents.len() as u32
    }

    pub fn parent(&self, node: u32) -> u32 {
        self.parents.get(node as usize).copied().unwrap_or(u32::MAX)
    }

    pub fn children(&self, node: u32) -> &[u32] {
        let i = node as usize;
        if i + 1 >= self.child_offsets.len() {
            return &[];
        }
        &self.children[self.child_offsets[i] as usize..self.child_offsets[i + 1] as usize]
    }

    /// Exclusive preorder end of the subtree rooted at `node`.
    pub fn subtree_end(&self, node: u32) -> u32 {
        self.subtree_end
            .get(node as usize)
            .copied()
            .unwrap_or(node.saturating_add(1))
    }

    pub fn root_of(&self, node: u32) -> u32 {
        self.root_of.get(node as usize).copied().unwrap_or(u32::MAX)
    }

    pub fn roots(&self) -> &[u32] {
        &self.roots
    }

    /// Source file of a root node, or `None` when the root covers no token.
    pub fn file_of_root(&self, root: u32) -> Option<u16> {
        self.file_of_root.get(&root).copied()
    }

    pub fn language_of_file(&self, file_id: u16) -> Option<LangId> {
        self.lang_of_file.get(&file_id).copied()
    }

    pub fn first_child_of_type(
        &self,
        bpa: &BPASTArtifact,
        node: u32,
        node_type: ASTNodeType,
    ) -> Option<u32> {
        self.children(node)
            .iter()
            .copied()
            .find(|&c| bpa.node_type(c) == node_type)
    }

    pub fn children_of_type<'s>(
        &'s self,
        bpa: &'s BPASTArtifact,
        node: u32,
        node_type: ASTNodeType,
    ) -> impl Iterator<Item = u32> + 's {
        self.children(node)
            .iter()
            .copied()
            .filter(move |&c| bpa.node_type(c) == node_type)
    }

    /// True when `node` lies in the subtree of `ancestor` (inclusive).
    pub fn is_within(&self, node: u32, ancestor: u32) -> bool {
        node >= ancestor && node < self.subtree_end(ancestor)
    }
}
