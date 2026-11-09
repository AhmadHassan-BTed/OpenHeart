//! Pass 2 — Import Resolution
//!
//! Phase 2 drops `import_declaration`, so the import declarations of every compilation unit are
//! parsed from the token stream (path segments joined with `.`, `static` and on-demand forms,
//! Kotlin `as` aliases) and recorded in that unit's own scope, never in the shared root. Single
//! type imports and static-import owners are bound immediately: to the project type of that
//! canonical name, to a library catalog entry, or to an external keyed by the qualified name.

use crate::ast::BPASTArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{Dialect, Occurrence, OccurrenceRole, ResolutionStatus};
use crate::symbol::scope_graph::{ImportDecl, ImportKind, NameResolver};
use crate::symbol::syntax::{parse_compilation_unit_header, TokenView};

pub struct Pass2Imports;

impl Pass2Imports {
    pub fn run(_bpa: &BPASTArtifact, tca: &TokenCorpusArtifact, builder: &mut SymbolTableBuilder) {
        let tv = TokenView::new(tca);
        let units: Vec<(u32, (u32, u32), Dialect)> = builder
            .units
            .iter()
            .filter(|u| u.token_range.0 != u32::MAX && u.dialect != Dialect::Other)
            .map(|u| (u.cu_scope, u.token_range, u.dialect))
            .collect();

        for (cu, (start, end), dialect) in units {
            let header = parse_compilation_unit_header(&tv, start, end, dialect);
            for imp in header.imports {
                if imp.segments.is_empty() {
                    continue;
                }
                let segments: Vec<&str> = imp.segments.iter().map(|&t| tv.text_str(t)).collect();
                let path = segments.join(".");
                let kind = match (imp.is_static, imp.on_demand) {
                    (false, false) => ImportKind::SingleType,
                    (false, true) => ImportKind::OnDemand,
                    (true, false) => ImportKind::SingleStatic,
                    (true, true) => ImportKind::StaticOnDemand,
                };
                let name_token = imp.segments[imp.segments.len() - 1];
                let visible_name = if imp.on_demand {
                    String::new()
                } else if imp.alias != u32::MAX {
                    tv.text_str(imp.alias).to_string()
                } else {
                    segments[segments.len() - 1].to_string()
                };
                builder.scope_graph.add_import(
                    cu,
                    ImportDecl {
                        kind,
                        path: path.clone(),
                        visible_name,
                        first_token: imp.first,
                        name_token,
                    },
                );

                // Bind the type each import names, when it names one.
                let target = match kind {
                    ImportKind::SingleType => {
                        // A Kotlin import may name a function or property; by convention
                        // only capitalized names are types.
                        let is_type = dialect != Dialect::Kotlin
                            || segments[segments.len() - 1]
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_uppercase());
                        is_type.then(|| (path.clone(), name_token))
                    }
                    ImportKind::SingleStatic if imp.segments.len() >= 2 => Some((
                        segments[..segments.len() - 1].join("."),
                        imp.segments[imp.segments.len() - 2],
                    )),
                    ImportKind::StaticOnDemand => Some((path.clone(), name_token)),
                    ImportKind::OnDemand => {
                        // `import a.Outer.*` imports member types of a type; `import a.b.*`
                        // imports a package.
                        if builder.types_by_fqn.contains_key(&path) {
                            Some((path.clone(), name_token))
                        } else {
                            if let Some(pkg) = builder.package_entry(&path) {
                                builder.add_occurrence(Occurrence {
                                    token: name_token,
                                    node: u32::MAX,
                                    symbol: pkg.sym,
                                    role: OccurrenceRole::Import as u8,
                                    status: ResolutionStatus::Project as u8,
                                    candidates: Vec::new(),
                                });
                            }
                            None
                        }
                    }
                    _ => None,
                };
                if let Some((fqn, token)) = target {
                    let binding = NameResolver::bind_qualified_name(builder, &fqn);
                    let (sym, status) = builder.materialize_binding(binding, tv.text_id(token), token);
                    builder.add_occurrence(Occurrence {
                        token,
                        node: u32::MAX,
                        symbol: sym,
                        role: OccurrenceRole::Import as u8,
                        status: status as u8,
                        candidates: Vec::new(),
                    });
                }
            }
        }
    }
}
