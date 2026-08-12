//! Dispatch Resolution Engine (Direct, CHA, 1-CFA) for Phase 6.
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::core::types::symbol::*;
use crate::core::types::*;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use std::collections::{HashMap, HashSet, VecDeque};

pub const MOD_ABSTRACT: u16 = SymbolModifiers::ABSTRACT;

/// Dispatch Resolver for Call Sites (§6.2.3, §6.2.4)
pub struct DispatchResolver<'a> {
    pub sta: &'a SymbolTableArtifact,
    pub ssa: &'a SSAArtifact,
    pub pts: &'a HashMap<u32, HashSet<u32>>,
}

impl<'a> DispatchResolver<'a> {
    pub fn new(
        sta: &'a SymbolTableArtifact,
        ssa: &'a SSAArtifact,
        pts: &'a HashMap<u32, HashSet<u32>>,
    ) -> Self {
        Self { sta, ssa, pts }
    }

    /// Resolve targets for a CallSite
    pub fn resolve_call_site(&self, site: &CallSite, bpa: &BPASTArtifact) -> Vec<u32> {
        match site.call_type {
            CG_EDGE_DIRECT | CG_EDGE_SPECIAL | CG_EDGE_CONSTRUCTOR => {
                self.resolve_direct(site, bpa)
            }
            CG_EDGE_VIRTUAL | CG_EDGE_INTERFACE => {
                let cfa_targets = self.resolve_1cfa(site, bpa);
                if !cfa_targets.is_empty() {
                    cfa_targets
                } else {
                    self.resolve_cha(site, bpa)
                }
            }
            CG_EDGE_DYNAMIC => self.resolve_dynamic(site, bpa),
            _ => Vec::new(),
        }
    }

    /// Direct & Special monomorphic resolution
    fn resolve_direct(&self, site: &CallSite, bpa: &BPASTArtifact) -> Vec<u32> {
        if let Some(target) = resolve_method_target(site.call_node, bpa, self.sta) {
            vec![target]
        } else {
            Vec::new()
        }
    }

    /// Class Hierarchy Analysis (CHA) Resolution (§6.2.3)
    fn resolve_cha(&self, site: &CallSite, bpa: &BPASTArtifact) -> Vec<u32> {
        let mut targets = HashSet::new();
        let target_sym = match resolve_method_target(site.call_node, bpa, self.sta) {
            Some(s) => s,
            None => return Vec::new(),
        };

        let method_sym = match self.sta.symbol(target_sym) {
            Some(s) => s,
            None => return vec![target_sym],
        };
        let name_id = method_sym.name_id;
        let declared_type = method_sym.parent_sym;
        if declared_type == u32::MAX {
            return vec![target_sym];
        }

        let mut worklist = VecDeque::new();
        let mut visited = HashSet::new();
        worklist.push_back(declared_type);

        while let Some(t_sym) = worklist.pop_front() {
            if !visited.insert(t_sym) {
                continue;
            }

            for s in &self.sta.symbol_records {
                if s.parent_sym == t_sym
                    && s.name_id == name_id
                    && (s.kind == SymbolKind::SK_METHOD as u8
                        || s.kind == SymbolKind::SK_CONSTRUCTOR as u8)
                    && (s.modifiers & SymbolModifiers::ABSTRACT) == 0
                {
                    targets.insert(s.symbol_id);
                }
            }

            for edge in &self.sta.th_edges {
                if edge.to_sym == t_sym
                    && (edge.relation == THRelation::TH_EXTENDS
                        || edge.relation == THRelation::TH_IMPLEMENTS)
                {
                    worklist.push_back(edge.from_sym);
                }
            }
        }

        if targets.is_empty() {
            vec![target_sym]
        } else {
            let mut res: Vec<u32> = targets.into_iter().collect();
            res.sort_unstable();
            res
        }
    }

    /// 1-CFA Allocation-Site Sensitivity Resolution (§6.2.4)
    fn resolve_1cfa(&self, site: &CallSite, bpa: &BPASTArtifact) -> Vec<u32> {
        if site.receiver_ssa == u32::MAX {
            return Vec::new();
        }

        if let Some(alloc_types) = self.pts.get(&site.receiver_ssa) {
            if !alloc_types.is_empty() {
                let target_sym = match resolve_method_target(site.call_node, bpa, self.sta) {
                    Some(s) => s,
                    None => return Vec::new(),
                };
                let name_id = match self.sta.symbol(target_sym) {
                    Some(s) => s.name_id,
                    None => 0,
                };

                let mut targets = Vec::new();
                for &alloc_type in alloc_types {
                    for s in &self.sta.symbol_records {
                        if s.parent_sym == alloc_type
                            && s.name_id == name_id
                            && (s.kind == SymbolKind::SK_METHOD as u8
                                || s.kind == SymbolKind::SK_CONSTRUCTOR as u8)
                            && (s.modifiers & SymbolModifiers::ABSTRACT) == 0
                        {
                            targets.push(s.symbol_id);
                        }
                    }
                }
                if !targets.is_empty() {
                    targets.sort_unstable();
                    targets.dedup();
                    return targets;
                }
            }
        }

        for func in &self.ssa.functions {
            for rec in &func.ssa_records {
                if rec.ssa_id == site.receiver_ssa
                    && rec.def_stmt != u32::MAX
                    && bpa.node_type(rec.def_stmt) == ASTNodeType::NN_NEW_EXPR
                {
                    if let Some(target) = resolve_method_target(site.call_node, bpa, self.sta) {
                        return vec![target];
                    }
                }
            }
        }

        Vec::new()
    }

    /// Dynamic Resolution
    fn resolve_dynamic(&self, site: &CallSite, bpa: &BPASTArtifact) -> Vec<u32> {
        if let Some(target) = resolve_method_target(site.call_node, bpa, self.sta) {
            vec![target]
        } else {
            Vec::new()
        }
    }
}

/// Helper function to resolve target method symbol for an AST call node
pub fn resolve_method_target(
    call_node: u32,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) -> Option<u32> {
    // 1. Direct def/decl node match
    for sym in &sta.symbol_records {
        if sym.kind == SymbolKind::SK_METHOD as u8
            || sym.kind == SymbolKind::SK_CONSTRUCTOR as u8
            || sym.kind == SymbolKind::SK_STATIC_INIT as u8
            || sym.kind == SymbolKind::SK_LAMBDA as u8
        {
            if sym.def_node == call_node || sym.decl_node == call_node {
                return Some(sym.symbol_id);
            }
        }
    }

    // 2. Find method identifier token or first token of call node
    let mut child = bpa.first_child(call_node);
    let mut tok_id = u32::MAX;
    while let Some(c) = child {
        if bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR {
            tok_id = bpa.token_range(c).0;
            break;
        }
        child = bpa.next_sibling(c);
    }
    if tok_id == u32::MAX {
        tok_id = bpa.token_range(call_node).0;
    }

    // 3. Match symbol by token position
    for sym in &sta.symbol_records {
        if (sym.kind == SymbolKind::SK_METHOD as u8
            || sym.kind == SymbolKind::SK_CONSTRUCTOR as u8
            || sym.kind == SymbolKind::SK_STATIC_INIT as u8
            || sym.kind == SymbolKind::SK_LAMBDA as u8)
            && sym.first_token_id == tok_id
        {
            return Some(sym.symbol_id);
        }
    }

    // 4. Token range enclosing match
    for sym in &sta.symbol_records {
        if sym.kind == SymbolKind::SK_METHOD as u8
            || sym.kind == SymbolKind::SK_CONSTRUCTOR as u8
        {
            if tok_id >= sym.first_token_id && tok_id <= sym.last_token_id {
                return Some(sym.symbol_id);
            }
        }
    }

    None
}
