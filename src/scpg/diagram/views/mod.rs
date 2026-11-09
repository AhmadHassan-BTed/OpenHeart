//! View builders: compute one typed [`ViewModel`] per diagram kind from a
//! [`DiagramContext`] (§10.4).
//!
//! Builders only translate artifact records (UMA, CFA, SSA, CGA, PSA) into the view IR;
//! they never synthesise elements. A view whose inputs are missing or empty is returned as
//! [`ViewModel::unavailable`] with the reason.

pub mod analysis;
pub mod behavioral;
pub mod structural;

use std::collections::{BTreeMap, HashMap};

use crate::core::types::symbol::SymbolModifiers;
use crate::scpg::diagram::common::DiagramUtils;
use crate::scpg::diagram::context::{is_void_type_id, primitive_type_name, DiagramContext};
use crate::scpg::diagram::model::{
    AliasTable, MemberKind, MemberView, NodeKind, ParamView, Provenance, ViewKind, ViewModel,
    ViewNode,
};
use crate::uma::types::*;

/// Artifacts every view reads (names, spans and UMA records).
const BASE_ARTIFACTS: [&str; 3] = ["tca", "sta", "uma"];

/// Builds and finalizes the view of `kind`. When the attached artifacts come from different
/// analysis runs (their hash links disagree, see [`DiagramContext::consistency_issues`]) and
/// the view reads one of the disagreeing artifacts, the view is returned unavailable with
/// the mismatch as its reason instead of mixing symbol ids of different generations.
pub fn build(kind: ViewKind, ctx: &DiagramContext) -> ViewModel {
    let mut model = build_unchecked(kind, ctx);
    let issues = ctx.consistency_issues();
    if !issues.is_empty() {
        let affected: Vec<String> = issues
            .iter()
            .filter(|m| {
                BASE_ARTIFACTS.iter().any(|a| m.involves(a))
                    || model.sources.iter().any(|s| m.involves(s))
            })
            .map(|m| m.detail.clone())
            .collect();
        if !affected.is_empty() {
            let mut stale = ViewModel::unavailable(
                kind,
                "The attached artifacts come from different analysis runs; re-run the analysis so every artifact belongs to one snapshot.",
            );
            for detail in affected {
                stale.note(format!("Hash link mismatch: {}.", detail));
            }
            stale.sources = model.sources.clone();
            return stale;
        }
    }
    model.finalize(&ctx.options, kind.allows_degree_pruning());
    model
}

fn build_unchecked(kind: ViewKind, ctx: &DiagramContext) -> ViewModel {
    match kind {
        ViewKind::Class => structural::class_view(ctx),
        ViewKind::Object => structural::object_view(ctx),
        ViewKind::Package => structural::package_view(ctx),
        ViewKind::Component => structural::component_view(ctx),
        ViewKind::CompositeStructure => structural::composite_view(ctx),
        ViewKind::Deployment => structural::deployment_view(ctx),
        ViewKind::Profile => structural::profile_view(ctx),
        ViewKind::UseCase => behavioral::use_case_view(ctx),
        ViewKind::Activity => behavioral::activity_view(ctx),
        ViewKind::StateMachine => behavioral::state_machine_view(ctx),
        ViewKind::Sequence => behavioral::sequence_view(ctx),
        ViewKind::Communication => behavioral::communication_view(ctx),
        ViewKind::InteractionOverview => behavioral::interaction_overview_view(ctx),
        ViewKind::Timing => behavioral::timing_view(ctx),
        ViewKind::Cfg => analysis::cfg_view(ctx),
        ViewKind::Dfg => analysis::dfg_view(ctx),
        ViewKind::Cdg => analysis::cdg_view(ctx),
        ViewKind::CallGraph => analysis::callgraph_view(ctx),
        ViewKind::Robdd => analysis::robdd_view(ctx),
    }
}

/// Declared classifiers of the analyzed sources, keyed by symbol id (never by name).
pub struct TypeIndex {
    /// Symbols in key order.
    pub syms: Vec<u32>,
    key_by_sym: HashMap<u32, String>,
    fqn_by_sym: HashMap<u32, String>,
    package_by_sym: HashMap<u32, Option<String>>,
}

impl TypeIndex {
    pub fn build(ctx: &DiagramContext) -> Self {
        let mut fqn_by_sym: HashMap<u32, String> = HashMap::new();
        let mut package_by_sym: HashMap<u32, Option<String>> = HashMap::new();
        let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for class_rec in &ctx.uma.classes {
            if fqn_by_sym.contains_key(&class_rec.sym_id) {
                continue;
            }
            let fqn = ctx
                .uma
                .label(class_rec.qualified_name_id)
                .map(str::to_string)
                .or_else(|| ctx.type_fqn(class_rec.sym_id))
                .unwrap_or_else(|| format!("<unnamed type #{}>", class_rec.sym_id));
            groups.entry(fqn.clone()).or_default().push(class_rec.sym_id);
            fqn_by_sym.insert(class_rec.sym_id, fqn);
            package_by_sym.insert(
                class_rec.sym_id,
                ctx.package_of(class_rec.sym_id).filter(|p| !p.is_empty()),
            );
        }
        let mut key_by_sym = HashMap::new();
        let mut ordered: Vec<(String, u32)> = Vec::new();
        for (fqn, syms) in &groups {
            for &sym in syms {
                let key = if syms.len() > 1 {
                    format!("{}#{}", fqn, sym)
                } else {
                    fqn.clone()
                };
                ordered.push((key.clone(), sym));
                key_by_sym.insert(sym, key);
            }
        }
        ordered.sort();
        Self {
            syms: ordered.into_iter().map(|(_, s)| s).collect(),
            key_by_sym,
            fqn_by_sym,
            package_by_sym,
        }
    }

    pub fn contains(&self, sym_id: u32) -> bool {
        self.key_by_sym.contains_key(&sym_id)
    }

    /// Unique node key of a declared type.
    pub fn key(&self, sym_id: u32) -> Option<&str> {
        self.key_by_sym.get(&sym_id).map(|k| k.as_str())
    }

    pub fn fqn(&self, sym_id: u32) -> Option<&str> {
        self.fqn_by_sym.get(&sym_id).map(|k| k.as_str())
    }

    pub fn package(&self, sym_id: u32) -> Option<&str> {
        self.package_by_sym
            .get(&sym_id)
            .and_then(|p| p.as_deref())
    }

    pub fn is_empty(&self) -> bool {
        self.syms.is_empty()
    }
}

/// Node kind of a classifier record.
pub fn classifier_kind(class_rec: &ClassRecord) -> NodeKind {
    match class_rec.stereotype {
        STEREOTYPE_INTERFACE => NodeKind::Interface,
        STEREOTYPE_ABSTRACT => NodeKind::AbstractClass,
        STEREOTYPE_ENUM => NodeKind::Enum,
        STEREOTYPE_RECORD => NodeKind::Record,
        STEREOTYPE_ANNOTATION => NodeKind::Annotation,
        _ => NodeKind::Class,
    }
}

/// Node key of a package.
pub fn package_key(package: &str) -> String {
    format!("pkg:{}", package)
}

/// Segments of a package, namespace or module path written with `.`, `::` or `/`
/// separators (empty segments dropped).
pub fn package_segments(package: &str) -> Vec<&str> {
    package
        .split("::")
        .flat_map(|s| s.split(|c| c == '.' || c == '/'))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Ensures package nodes exist for every prefix of `package` (nested by prefix) and returns
/// the key of the innermost one. Prefixes keep the separator style of the input (`::` for
/// C++ namespaces, `.` otherwise), so one package has one key in every view.
pub fn ensure_package_chain(
    model: &mut ViewModel,
    aliases: &mut AliasTable,
    package: &str,
) -> Option<String> {
    let separator = if package.contains("::") { "::" } else { "." };
    let mut path = String::new();
    let mut parent: Option<String> = None;
    for part in package_segments(package) {
        if !path.is_empty() {
            path.push_str(separator);
        }
        path.push_str(part);
        let key = package_key(&path);
        if !model.has_node(&key) {
            let alias = aliases.alias("pkg_", &path, &key);
            model.add_node(
                ViewNode::new(key.clone(), alias, part, NodeKind::Package)
                    .with_parent(parent.clone())
                    .with_qualified_name(Some(path.clone())),
            );
        }
        parent = Some(key);
    }
    parent
}

/// UML multiplicity text of a (min, max) pair with the UMA unknown markers.
pub fn multiplicity(min: u16, max: u16) -> Option<String> {
    if max == MULTIPLICITY_UNKNOWN || min == MULTIPLICITY_UNKNOWN {
        return None;
    }
    let upper = if max == MULTIPLICITY_MANY {
        "*".to_string()
    } else {
        max.to_string()
    };
    if max != MULTIPLICITY_MANY && min == max {
        Some(upper)
    } else {
        Some(format!("{}..{}", min, upper))
    }
}

/// Pattern stereotypes of a class from the design-pattern participant records, with the
/// extractor's structural-evidence score. One entry per pattern kind (highest score); kinds
/// outside the known pattern range and detections below `ViewOptions::min_pattern_score`
/// are left out.
pub fn pattern_stereotypes(ctx: &DiagramContext, sym_id: u32) -> Vec<String> {
    let mut best: BTreeMap<u8, u16> = BTreeMap::new();
    for dp in &ctx.uma.design_patterns {
        if dp.class_sym != sym_id {
            continue;
        }
        let kind = match u8::try_from(dp.pattern_kind) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let entry = best.entry(kind).or_insert(0);
        *entry = (*entry).max(dp.confidence);
    }
    best.into_iter()
        .filter(|&(_, score)| score >= ctx.options.min_pattern_score)
        .filter_map(|(kind, score)| {
            let name = pattern_kind_name(kind);
            if name.is_empty() {
                None
            } else {
                Some(format!("{} {}%", name, score.min(100)))
            }
        })
        .collect()
}

/// Display name of a referenced type symbol (`None` when unresolved or `void`). Primitive
/// pseudo-ids of the symbol table are named by the primitive they stand for.
pub fn type_display(ctx: &DiagramContext, type_sym: u32) -> Option<String> {
    if type_sym == UNKNOWN_SYM || is_void_type_id(type_sym) {
        return None;
    }
    if let Some(primitive) = primitive_type_name(type_sym) {
        return Some(primitive.to_string());
    }
    ctx.nested_type_name(type_sym)
}

/// Declared type text of a member symbol, else the display name of the type symbol UMA
/// recorded for it. `void` results are `None` (an operation without a return type).
fn member_type(ctx: &DiagramContext, member_sym: u32, recorded_type: u32) -> Option<String> {
    if ctx.declared_type_is_void(member_sym) {
        return None;
    }
    ctx.declared_type(member_sym)
        .or_else(|| type_display(ctx, recorded_type))
}

/// Members of a classifier record, in record order.
pub fn class_members(ctx: &DiagramContext, class_rec: &ClassRecord) -> Vec<MemberView> {
    let mut members = Vec::new();
    for f in &class_rec.fields {
        let name = match ctx.simple_name(f.field_sym_id) {
            Some(n) => n,
            None => continue,
        };
        let kind = if f.field_kind == FIELD_KIND_ENUM_LITERAL {
            MemberKind::Literal
        } else {
            MemberKind::Field
        };
        let is_literal = kind == MemberKind::Literal;
        members.push(MemberView {
            kind,
            visibility: DiagramUtils::visibility_symbol(f.visibility),
            name,
            type_name: if is_literal {
                None
            } else {
                member_type(ctx, f.field_sym_id, f.type_sym_id)
            },
            params: None,
            param_count: 0,
            is_static: f.modifiers & SymbolModifiers::STATIC != 0,
            is_abstract: false,
            is_final: f.modifiers & SymbolModifiers::FINAL != 0,
            multiplicity: if is_literal {
                None
            } else {
                multiplicity(f.mult_min, f.mult_max)
            },
            is_collection: f.is_collection != 0,
            sym_id: Some(f.field_sym_id),
            span: ctx.symbol_span(f.field_sym_id),
        });
    }
    for m in &class_rec.methods {
        let name = match ctx.simple_name(m.method_sym_id) {
            Some(n) => n,
            None => continue,
        };
        let params = if m.param_syms.len() == m.param_count as usize {
            Some(
                m.param_syms
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| ParamView {
                        name: ctx.simple_name(p),
                        type_name: member_type(
                            ctx,
                            p,
                            m.param_type_syms.get(i).copied().unwrap_or(UNKNOWN_SYM),
                        ),
                    })
                    .collect(),
            )
        } else {
            None
        };
        let is_constructor = m.is_constructor != 0;
        members.push(MemberView {
            kind: if is_constructor {
                MemberKind::Constructor
            } else {
                MemberKind::Method
            },
            visibility: DiagramUtils::visibility_symbol(m.visibility),
            name,
            type_name: if is_constructor {
                None
            } else {
                member_type(ctx, m.method_sym_id, m.return_type_sym_id)
            },
            params,
            param_count: m.param_count,
            is_static: m.modifiers & SymbolModifiers::STATIC != 0,
            is_abstract: m.modifiers & SymbolModifiers::ABSTRACT != 0,
            is_final: m.modifiers & SymbolModifiers::FINAL != 0,
            multiplicity: None,
            is_collection: false,
            sym_id: Some(m.method_sym_id),
            span: ctx.symbol_span(m.method_sym_id),
        });
    }
    members
}

/// Classifier node for a declared type (members, stereotypes, span, package parent).
pub fn classifier_node(
    ctx: &DiagramContext,
    types: &TypeIndex,
    aliases: &mut AliasTable,
    class_rec: &ClassRecord,
    parent: Option<String>,
) -> Option<ViewNode> {
    let key = types.key(class_rec.sym_id)?.to_string();
    let fqn = types.fqn(class_rec.sym_id).map(str::to_string);
    let mut label = ctx
        .nested_type_name(class_rec.sym_id)
        .unwrap_or_else(|| key.clone());
    let type_params = ctx.type_parameters(class_rec.sym_id);
    let mut details = Vec::new();
    if !type_params.is_empty() {
        label = format!("{}<{}>", label, type_params.join(", "));
    } else if class_rec.type_param_count > 0 {
        details.push(format!(
            "{} type parameter(s); their names are not recorded",
            class_rec.type_param_count
        ));
    }
    let alias = aliases.alias("cls_", fqn.as_deref().unwrap_or(key.as_str()), &key);
    let span = ctx
        .span_of_link(&class_rec.uml_link)
        .or_else(|| ctx.symbol_span(class_rec.sym_id));
    let mut node = ViewNode::new(key, alias, label, classifier_kind(class_rec))
        .with_parent(parent)
        .with_qualified_name(fqn)
        .with_span(span)
        .with_details(details)
        .with_sym(class_rec.sym_id);
    if class_rec.stereotype == STEREOTYPE_RECORD {
        node = node.with_stereotype("record");
    }
    for stereotype in pattern_stereotypes(ctx, class_rec.sym_id) {
        node = node.with_stereotype(stereotype);
    }
    node.members = class_members(ctx, class_rec);
    Some(node)
}

/// Display label of a function symbol: `Owner.method()`.
pub fn function_label(ctx: &DiagramContext, sym_id: u32) -> String {
    match ctx.member_label(sym_id) {
        Some(label) => format!("{}()", label),
        None => format!("<function #{}>", sym_id),
    }
}

/// Qualified name of a function symbol (used for ordering and focus).
pub fn function_fqn(ctx: &DiagramContext, sym_id: u32) -> String {
    ctx.member_fqn(sym_id)
        .unwrap_or_else(|| format!("<function #{}>", sym_id))
}

/// Functions in qualified-name order, deduplicated and filtered by the view focus.
pub fn ordered_functions(ctx: &DiagramContext, syms: impl Iterator<Item = u32>) -> Vec<(String, u32)> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(String, u32)> = syms
        .filter(|s| seen.insert(*s))
        .map(|s| (function_fqn(ctx, s), s))
        .filter(|(fqn, _)| ctx.matches_focus(fqn))
        .collect();
    out.sort();
    out
}

/// Node budget shared by the per-function views: functions are admitted whole, in order,
/// until the budget is exhausted; the rest are recorded as omitted.
pub struct FunctionBudget {
    remaining: usize,
    pub omitted: Vec<String>,
}

impl FunctionBudget {
    pub fn new(max_nodes: usize) -> Self {
        Self {
            remaining: max_nodes,
            omitted: Vec::new(),
        }
    }

    pub fn admit(&mut self, name: &str, size: usize) -> bool {
        if size <= self.remaining {
            self.remaining -= size;
            true
        } else {
            self.omitted.push(name.to_string());
            false
        }
    }

    /// Records the omitted functions on the model.
    pub fn finish(self, model: &mut ViewModel, what: &str) {
        if self.omitted.is_empty() {
            return;
        }
        model.note(format!(
            "{} {} omitted by the node budget; narrow the view with a focus to see them.",
            self.omitted.len(),
            what
        ));
        for name in self.omitted {
            model.omit(name);
        }
    }
}

/// Marks a model as heuristic when any of its elements is heuristic.
pub fn settle_provenance(model: &mut ViewModel) {
    if model.is_unavailable() {
        return;
    }
    let weakest = model
        .nodes
        .iter()
        .map(|n| n.provenance)
        .chain(model.edges.iter().map(|e| e.provenance))
        .fold(model.provenance, Provenance::weakest);
    model.provenance = weakest.min(Provenance::Heuristic);
}

/// Iterative Tarjan SCC over a dense graph; returns components with more than one member
/// or a self loop.
pub fn cyclic_components(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); n];
    let mut self_loop = vec![false; n];
    for &(u, v) in edges {
        if u < n && v < n {
            adj[u].push(v);
            if u == v {
                self_loop[u] = true;
            }
        }
    }
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut result = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;
        loop {
            let (v, child) = match work.last() {
                Some(&(v, child)) => (v, child),
                None => break,
            };
            if child < adj[v].len() {
                let w = adj[v][child];
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                if index[w] == usize::MAX {
                    index[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut component = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if component.len() > 1 || self_loop[v] {
                        component.sort_unstable();
                        result.push(component);
                    }
                }
            }
        }
    }
    result.sort();
    result
}
