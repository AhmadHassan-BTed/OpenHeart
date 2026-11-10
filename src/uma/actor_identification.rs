//! ActorIdentifier — selects scenario entry points from framework semantics
//! (§9.2.1, §9.2.4).
//!
//! Entry points, in order of evidence:
//! 1. `static main` methods;
//! 2. methods carrying a framework handler annotation (HTTP mappings, message
//!    listeners, schedulers, JUnit tests, JAX-RS resources);
//! 3. lifecycle callbacks that override an operation of a library supertype
//!    (e.g. Android `Activity.onCreate`), recognized structurally: the owning
//!    class has a library ancestor and the method overrides no user operation.
//! Only when none of these exist does the extractor fall back to visible methods
//! that no analyzed code calls and that make at least one call themselves.

use crate::core::types::symbol::{SymbolKind, SymbolVisibility};
use crate::uma::index::UMAIndex;
use crate::uma::types::*;

pub const EXTERNAL_ACTOR_ID: u32 = 0xFFFF_FFFE;

/// Framework annotations whose methods the runtime invokes directly.
const HANDLER_ANNOTATIONS: &[&[u8]] = &[
    b"RequestMapping",
    b"GetMapping",
    b"PostMapping",
    b"PutMapping",
    b"DeleteMapping",
    b"PatchMapping",
    b"MessageMapping",
    b"SubscribeMapping",
    b"ExceptionHandler",
    b"KafkaListener",
    b"RabbitListener",
    b"JmsListener",
    b"SqsListener",
    b"StreamListener",
    b"EventListener",
    b"TransactionalEventListener",
    b"Scheduled",
    b"Test",
    b"ParameterizedTest",
    b"RepeatedTest",
    b"TestFactory",
    b"TestTemplate",
    b"GET",
    b"POST",
    b"PUT",
    b"DELETE",
    b"PATCH",
    b"HEAD",
    b"OPTIONS",
    b"WebMethod",
    b"Incoming",
    b"Consumes",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPoint {
    pub method: u32,
    pub kind: u8,
}

pub struct ActorIdentifier;

impl ActorIdentifier {
    /// Identify scenario entry points (sorted by symbol id within each kind).
    pub fn find_entry_points(index: &UMAIndex<'_>, include_tests: bool) -> Vec<EntryPoint> {
        let mut framework = Vec::new();
        let mut unreferenced = Vec::new();

        for sym_id in 0..index.symbol_count() {
            let id = sym_id as u32;
            if index.kind(id) != Some(SymbolKind::SK_METHOD) {
                continue;
            }
            let owner = index.enclosing_type(id);
            if owner == UNKNOWN_SYM {
                continue;
            }
            if !include_tests {
                if let Some(fid) = index.file_of_sym(id) {
                    if index.is_test_file(fid) {
                        continue;
                    }
                }
            }
            let sym = match index.symbol(id) {
                Some(s) => s,
                None => continue,
            };

            if index.is_static(id) && index.name_bytes(id) == b"main" && sym.param_count <= 1 {
                framework.push(EntryPoint {
                    method: id,
                    kind: ENTRY_KIND_MAIN,
                });
                continue;
            }
            if sym.first_token_id != u32::MAX {
                let annotated = index
                    .scanner
                    .annotations(sym)
                    .iter()
                    .any(|&a| HANDLER_ANNOTATIONS.contains(&index.tca.interner.lookup_text(a)));
                if annotated {
                    framework.push(EntryPoint {
                        method: id,
                        kind: ENTRY_KIND_ANNOTATED,
                    });
                    continue;
                }
            }
            if Self::is_lifecycle_callback(index, id, owner) {
                framework.push(EntryPoint {
                    method: id,
                    kind: ENTRY_KIND_LIFECYCLE,
                });
                continue;
            }

            let visible = sym.visibility == SymbolVisibility::Public as u8
                || sym.visibility == SymbolVisibility::Package as u8;
            if visible && index.callers_of(id).is_empty() && !index.callees_of(id).is_empty() {
                unreferenced.push(EntryPoint {
                    method: id,
                    kind: ENTRY_KIND_UNREFERENCED,
                });
            }
        }

        if framework.is_empty() {
            unreferenced
        } else {
            framework
        }
    }

    /// Overrides an operation of a library supertype: the owner has a library
    /// ancestor, the method is visible and non-static, overrides no user
    /// operation, and is never called from analyzed code.
    fn is_lifecycle_callback(index: &UMAIndex<'_>, method: u32, owner: u32) -> bool {
        if index.is_static(method)
            || index.visibility(method) == SymbolVisibility::Private as u8
            || !index.callers_of(method).is_empty()
        {
            return false;
        }
        let has_library_ancestor = index
            .ancestors(owner)
            .into_iter()
            .any(|a| index.is_library_type(a) && !index.is_collection_type(a));
        if !has_library_ancestor {
            return false;
        }
        if index.overridden_method(method).is_some() {
            return false;
        }
        let name = index.name_bytes(method);
        // Framework callbacks follow the `onXxx` convention of the Android and
        // JavaFX/Swing listener APIs.
        name.len() > 2 && name.starts_with(b"on") && name[2].is_ascii_uppercase()
    }
}
