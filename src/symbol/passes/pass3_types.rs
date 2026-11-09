//! Pass 3 — Type Reference Resolution
//!
//! Binds every NN_TYPE_REF node of a structurally processed file. Primitive keywords bind to
//! their pseudo-ids and `var` is recorded as a type-inference site (JLS 14.4.1). Every other
//! name is resolved through the scope graph in JLS 6.4.1/7.5 order (local and member types,
//! single-type imports, same package across files, on-demand and implicit imports) and only in
//! the type namespace, so constructors, fields, parameters and locals never capture a type
//! name. Names that cannot be bound become externals keyed by qualified name (or explicitly
//! unresolved externals with their candidate packages), never by bare simple name.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{FrontEnd, ResolutionStatus};
use crate::symbol::passes::type_lowering::TypeLowering;
use crate::symbol::syntax::TokenView;

pub struct Pass3Types;

impl Pass3Types {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let tv = TokenView::new(tca);
        let mut lowering = TypeLowering::new(&tv, adapter);
        let ranges: Vec<(u32, u32)> = builder
            .units
            .iter()
            .filter(|u| u.front_end == FrontEnd::Structural && u.node_range.0 != u32::MAX)
            .map(|u| u.node_range)
            .collect();

        for (start, end) in ranges {
            for node in start..end.min(bpa.node_count) {
                if bpa.node_type(node) != ASTNodeType::NN_TYPE_REF {
                    continue;
                }
                let (ft, _) = bpa.token_range(node);
                if !tv.valid(ft) {
                    builder.set_type_ref_resolution(node, u32::MAX, ResolutionStatus::Unresolved);
                    continue;
                }
                let text = tv.text_str(ft);
                if text.is_empty() {
                    builder.set_type_ref_resolution(node, u32::MAX, ResolutionStatus::Unresolved);
                    continue;
                }
                if let Some(prim) = adapter.primitive_type_id(text) {
                    builder.set_type_ref_resolution(node, prim, ResolutionStatus::Primitive);
                    continue;
                }
                if text == "var" {
                    // `var` is a reserved type name, not a type (JLS 3.9, 14.4.1).
                    builder.set_type_ref_resolution(node, u32::MAX, ResolutionStatus::TypeInference);
                    continue;
                }
                let scope = builder.scope_of_node(node);
                let (sym, status) = lowering.resolve_simple(builder, scope, ft);
                builder.set_type_ref_resolution(node, sym, status);
                lowering.record_reference(builder, ft, node, sym, status);
            }
        }
    }
}
