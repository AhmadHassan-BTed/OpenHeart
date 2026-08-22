//! ActorIdentifier — identifies entry points (fan-in = 0 methods) and external actors (§9.2.1, §9.2.4).

use crate::core::types::cg::CallGraphArtifact;
use crate::symbol::SymbolTableArtifact;

pub const EXTERNAL_ACTOR_ID: u32 = 0xFFFF_FFFE;

pub struct ActorIdentifier;

impl ActorIdentifier {
    /// Identify all entry point methods in the call graph (public methods with fan-in == 0).
    pub fn find_entry_points(sta: &SymbolTableArtifact, cga: &CallGraphArtifact) -> Vec<u32> {
        let mut entry_points = Vec::new();

        let method_count = cga.method_count as usize;
        let mut in_degrees = vec![0u32; method_count];

        for &callee in &cga.callee_csr.adj {
            if (callee as usize) < in_degrees.len() {
                in_degrees[callee as usize] += 1;
            }
        }

        for sym_id in 0..sta.symbol_count {
            if let Some(sym) = sta.symbol(sym_id) {
                let is_callable = sym.kind
                    == crate::core::types::symbol::SymbolKind::SK_METHOD as u8
                    || sym.kind == crate::core::types::symbol::SymbolKind::SK_CONSTRUCTOR as u8;
                let is_entry_vis = sym.visibility
                    == crate::core::types::symbol::SymbolVisibility::Public as u8
                    || sym.visibility
                        == crate::core::types::symbol::SymbolVisibility::Package as u8;

                if is_callable && is_entry_vis {
                    let method_idx = sym_id as usize;
                    let fan_in = if method_idx < in_degrees.len() {
                        in_degrees[method_idx]
                    } else {
                        0
                    };
                    if fan_in == 0 {
                        entry_points.push(sym_id);
                    }
                }
            }
        }

        entry_points
    }
}
