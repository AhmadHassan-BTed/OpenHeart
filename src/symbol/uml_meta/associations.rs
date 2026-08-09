//! AssociationDetector: implements assoc_kind() decision function and constructor assignment scanning.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{AssocKind, SymbolKind, SymbolModifiers};
use crate::symbol::builder::SymbolTableBuilder;

pub struct AssociationDetector;

impl AssociationDetector {
    /// Implements `assoc_kind(class C, field f: Type T)` decision logic
    pub fn detect_association(
        field_sym_id: u32,
        owner_sym_id: u32,
        type_sym_id: u32,
        builder: &SymbolTableBuilder,
        bpa: &BPASTArtifact,
    ) -> AssocKind {
        let field_sym = match builder.symbol(field_sym_id) {
            Some(s) => s,
            None => return AssocKind::None,
        };

        let type_sym = match builder.symbol(type_sym_id) {
            Some(s) => s,
            None => return AssocKind::Dependency,
        };

        // If type is external -> DEPENDENCY
        if type_sym.kind == SymbolKind::SK_EXTERNAL as u8 {
            return AssocKind::Dependency;
        }

        let qual_name = builder
            .qual_names
            .get_name(type_sym.qual_name_id)
            .unwrap_or("");

        // Collection types or arrays -> AGGREGATION (0..*)
        let is_collection = builder.std_lib.stubs.contains_key("List")
            || qual_name.contains("List")
            || qual_name.contains("Set")
            || qual_name.contains("Map")
            || qual_name.contains("Collection");

        let is_array = (field_sym.flags & 0x08) != 0; // is_record_component or array flag

        if is_collection || is_array {
            return AssocKind::Aggregation;
        }

        let is_final = (field_sym.modifiers & SymbolModifiers::FINAL) != 0;
        let is_created_in_ctor =
            Self::is_instantiated_in_constructor(field_sym_id, owner_sym_id, builder, bpa);

        if is_final && is_created_in_ctor {
            AssocKind::Composition
        } else {
            AssocKind::Association
        }
    }

    /// Scans constructor subtrees for `field = new Type(...)` instantiations
    fn is_instantiated_in_constructor(
        _field_sym_id: u32,
        owner_sym_id: u32,
        builder: &SymbolTableBuilder,
        bpa: &BPASTArtifact,
    ) -> bool {
        let owner_sym = match builder.symbol(owner_sym_id) {
            Some(s) => s,
            None => return false,
        };

        // Find constructor children of owner_sym
        let mut cur = owner_sym.first_child;
        while cur != u32::MAX {
            if let Some(child_sym) = builder.symbol(cur) {
                if child_sym.kind == SymbolKind::SK_CONSTRUCTOR as u8 {
                    let ctor_node = child_sym.decl_node;
                    if Self::has_new_expr_in_subtree(ctor_node, bpa) {
                        return true;
                    }
                }
            }
            cur = builder
                .symbol(cur)
                .map(|s| s.next_sibling)
                .unwrap_or(u32::MAX);
        }

        false
    }

    fn has_new_expr_in_subtree(root_node: u32, bpa: &BPASTArtifact) -> bool {
        let mut stack = vec![root_node];
        while let Some(node) = stack.pop() {
            if bpa.node_type(node) == ASTNodeType::NN_NEW_EXPR {
                return true;
            }
            if let Some(fc) = bpa.first_child(node) {
                stack.push(fc);
                let mut sib = bpa.next_sibling(fc);
                while let Some(s) = sib {
                    stack.push(s);
                    sib = bpa.next_sibling(s);
                }
            }
        }
        false
    }
}
