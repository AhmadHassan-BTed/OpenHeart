//! Focused diagrams for the query protocol's `view` operation
//! (docs/engine-query-protocol.md, section 2.2).
//!
//! Without a symbol, a view is the whole-program diagram, the same Graph IR the analysis
//! exports as `<id>.json`. With a symbol, the symbol is resolved and the diagram context is
//! focused on its qualified name ([`DiagramContext::with_focus`]). Each view builder then
//! restricts itself to that element the way it already does for a focused export, so the
//! focused diagram is derived by the same builder as the whole-program one, never
//! assembled separately.
//!
//! `depth` is accepted for the protocol. The builders' focus selects an element and what
//! they relate to it directly, so depths above 1 do not widen a view yet; the response
//! metadata says so.

use serde_json::{json, Value};

use crate::facts::{self, FactsError};
use crate::scpg::diagram::export::json::JSONExporter;
use crate::session::AnalysisSession;

/// Graph IR of `view`, whole-program or focused on `symbol`.
pub fn view(
    s: &AnalysisSession,
    view: &str,
    symbol: Option<&str>,
    depth: u32,
) -> Result<Value, FactsError> {
    let base = s.diagram_context();
    let (ctx, focus) = match symbol {
        None => (base, None),
        Some(spec) => {
            let sym = facts::resolve_symbol(s, spec)?;
            let symbols = &s.artifacts.engine.snapshot().symbols;
            let qualified = symbols
                .qualified_name(sym)
                .or_else(|| symbols.name(sym))
                .map(str::to_string)
                .ok_or_else(|| {
                    FactsError::new(
                        422,
                        format!("symbol {} has no name to focus a view on", sym),
                    )
                })?;
            (base.with_focus(qualified.clone()), Some(qualified))
        }
    };
    let ir = JSONExporter::export_type(view, &ctx)
        .ok_or_else(|| FactsError::new(400, format!("unknown diagram '{}'", view)))?;
    let mut value = serde_json::to_value(&ir)
        .map_err(|err| FactsError::new(500, format!("Graph IR serialization failed: {}", err)))?;
    if let Some(qualified) = focus {
        if let Value::Object(map) = &mut value {
            let metadata = map.entry("metadata").or_insert_with(|| json!({}));
            if let Value::Object(meta) = metadata {
                meta.insert("focus".into(), json!(qualified));
                meta.insert("focus_depth".into(), json!(1));
                if depth > 1 {
                    meta.insert(
                        "focus_note".into(),
                        json!("focused views show the element and what is related to it directly; deeper neighbourhoods are not derived yet"),
                    );
                }
            }
        }
    }
    Ok(value)
}
