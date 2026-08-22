//! Template Method pattern query.

use crate::symbol::SymbolTableArtifact;

pub fn is_template_method(class_sym: u32, sta: &SymbolTableArtifact) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    if (sym.modifiers & crate::core::types::symbol::SymbolModifiers::ABSTRACT) == 0 {
        // Not abstract
        return (false, 0);
    }

    let mut has_abstract_method = false;
    let mut has_protected_method = false;

    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == crate::core::types::symbol::SymbolKind::SK_METHOD as u8 {
            if (child.modifiers & crate::core::types::symbol::SymbolModifiers::ABSTRACT) != 0 {
                has_abstract_method = true;
            }
            if child.visibility == crate::core::types::symbol::SymbolVisibility::Protected as u8 {
                has_protected_method = true;
            }
        }
        child_id = child.next_sibling;
    }

    if has_abstract_method && has_protected_method {
        (true, 80)
    } else {
        (false, 0)
    }
}
