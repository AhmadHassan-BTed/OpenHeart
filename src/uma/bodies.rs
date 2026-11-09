//! MemberBodies — lexical body facts for every member of every user type,
//! scanned once per Phase 9 run and shared by the class, object, state-machine,
//! sequence and pattern extractors (§9.3).

use std::collections::HashMap;

use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::LangId;
use crate::uma::index::UMAIndex;
use crate::uma::source::BodyFacts;

pub struct MemberBodies {
    bodies: HashMap<u32, BodyFacts>,
}

impl MemberBodies {
    /// Scan the bodies of all methods, constructors and initializers of user types.
    pub fn build(index: &UMAIndex<'_>) -> Self {
        let mut bodies = HashMap::new();
        for sym_id in 0..index.symbol_count() {
            let id = sym_id as u32;
            let kind = match index.kind(id) {
                Some(k) => k,
                None => continue,
            };
            if !matches!(
                kind,
                SymbolKind::SK_METHOD
                    | SymbolKind::SK_CONSTRUCTOR
                    | SymbolKind::SK_STATIC_INIT
                    | SymbolKind::SK_INSTANCE_INIT
            ) {
                continue;
            }
            if index.enclosing_type(id) == u32::MAX {
                continue;
            }
            let rec = match index.symbol(id) {
                Some(r) if r.first_token_id != u32::MAX => r,
                _ => continue,
            };
            let range = match index.scanner.body_range(rec) {
                Some(r) => r,
                None => continue,
            };
            let lang = index.language_of_sym(id).unwrap_or(LangId::Unknown);
            bodies.insert(id, index.scanner.scan_body(range, lang));
        }
        Self { bodies }
    }

    /// An empty cache (used when only symbol-level facts are wanted).
    pub fn empty() -> Self {
        Self {
            bodies: HashMap::new(),
        }
    }

    pub fn get(&self, member: u32) -> Option<&BodyFacts> {
        self.bodies.get(&member)
    }

    pub fn has_body(&self, member: u32) -> bool {
        self.bodies.contains_key(&member)
    }

    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
}
