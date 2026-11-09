//! System boundaries: entry points, public surfaces, external dependencies and the
//! dependencies that cross subsystem boundaries.
//!
//! **Entry points** come from four sources, each recorded with the entry:
//! - the entry points the points-to solve started from (`CallGraph::entries`);
//! - the callbacks the solve found from external code: module functions that external code
//!   can invoke because an object or closure implementing them reached it;
//! - declarations a language frontend recognized in the program text (program entry
//!   functions, exported functions, handler or consumer annotations), handed in as
//!   [`DeclaredEntry`] data;
//! - registration calls: a frontend names the registration functions of a framework as
//!   [`RegistrationRule`]s, and every IR call of one of them registers the function values
//!   that reach its handler argument, with the route, method or channel read from
//!   compile-time string constants. A handler or route that the IR does not determine is
//!   reported as not derivable, never named by guess.
//!
//! Entries of different sources that denote the same function merge. For every entry, the
//! functions and partition groups reachable over the call facts are computed.
//!
//! **Public surface.** A declaration is exposed when it and every enclosing type are public,
//! or protected (the extension surface); declarations inside function bodies never are. Its
//! uses from other groups come from the call, field-access and dependency facts.
//! Visibility is read from the symbol table as each frontend records it.
//!
//! **External dependencies** are the external units the project depends on, grouped by
//! namespace, with their dependents and evidence.
//!
//! **Crossings** are the type-level dependencies whose endpoints lie in different groups of
//! a [`Partition`] (packages, projects or recovered subsystems), aggregated per ordered pair
//! of groups, and the function-level calls that cross; they give each group its provided and
//! required interfaces.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::arch::cluster::ClusterResult;
use crate::arch::dsm::strongly_connected_components;
use crate::arch::graph::{
    span_key, ArchModel, CallbackKind, Certainty, DepKind, KindSet, Level, NodeId, NodeKind,
    Origin, Strength, WeightedDigraph,
};
use crate::core::types::symbol::{SymbolKind, SymbolVisibility};
use crate::ir::func::{FuncIr, FuncKind};
use crate::ir::inst::{Callee, Inst};
use crate::ir::module::ModuleIr;
use crate::ir::types::{CallSiteId, FuncId, Span, ValueId};
use crate::ir::value::{ConstVal, Operand};
use crate::symbol::SymbolTableArtifact;

// ── Vocabulary ───────────────────────────────────────────────────────────────

/// What kind of entry into the system a function is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    /// The program's start function.
    Main,
    /// A function exported to other programs (library API, exported symbol).
    ExportedFunction,
    /// A handler of requests (HTTP routes, RPC methods).
    RequestHandler,
    /// A consumer of messages (queues, topics, event streams).
    MessageConsumer,
    /// A function run on a schedule.
    ScheduledTask,
    /// A listener of in-process events or UI callbacks.
    EventListener,
    TestCase,
    /// Initialization the runtime performs (static, module or instance initializers).
    Initializer,
    /// A function external code can call back.
    Callback,
    /// An entry point of the points-to solve whose role no other source states.
    SolverEntry,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::ExportedFunction => "exported_function",
            Self::RequestHandler => "request_handler",
            Self::MessageConsumer => "message_consumer",
            Self::ScheduledTask => "scheduled_task",
            Self::EventListener => "event_listener",
            Self::TestCase => "test_case",
            Self::Initializer => "initializer",
            Self::Callback => "callback",
            Self::SolverEntry => "solver_entry",
        }
    }
}

/// Where an entry point was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntrySource {
    CallGraphEntry,
    ExternalCallback,
    Declared,
    Registration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Route {
    pub path: Option<String>,
    /// Request methods the route accepts (empty when the facts state none).
    pub methods: Vec<String>,
}

/// An entry point a language frontend recognized in the program text.
#[derive(Debug, Clone)]
pub struct DeclaredEntry {
    /// The entry declaration (function, method or handler type).
    pub symbol: u32,
    pub kind: EntryKind,
    pub route: Option<Route>,
    pub channel: Option<String>,
    /// Spans of the text that establishes the entry (signature, annotation, export).
    pub evidence: Vec<Span>,
    /// Name of the frontend rule that recognized the entry.
    pub rule: String,
}

/// How a registration rule names its registration function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalleeMatcher {
    Symbol(u32),
    /// Canonical qualified name (`SymbolTableArtifact::qualified_name_of`).
    QualifiedName(String),
    /// Stable symbol descriptor (`SymbolTableArtifact::descriptor_of`).
    Descriptor(String),
}

/// A framework registration function: every call of it registers an entry. Argument
/// indices index `Inst::Call::args`, which start with the receiver of an instance call.
#[derive(Debug, Clone)]
pub struct RegistrationRule {
    pub callee: CalleeMatcher,
    pub kind: EntryKind,
    pub handler_arg: usize,
    pub route_arg: Option<usize>,
    /// Argument holding the request method name.
    pub method_arg: Option<usize>,
    /// Request methods implied by the registration function itself.
    pub fixed_methods: Vec<String>,
    pub channel_arg: Option<usize>,
    pub rule: String,
}

#[derive(Debug, Clone, Default)]
pub struct BoundaryInput<'m> {
    /// The IR the model was built from; registration rules need it.
    pub module: Option<&'m ModuleIr>,
    pub declared: Vec<DeclaredEntry>,
    pub registrations: Vec<RegistrationRule>,
}

#[derive(Debug, Clone)]
pub struct BoundaryConfig {
    /// Dependency kinds that count for crossings, interfaces and external dependencies.
    pub kinds: KindSet,
    /// Spans kept per aggregated record (the rest are counted in `omitted_spans`).
    pub max_spans: usize,
    /// Entries for which the reachable function count is computed (one traversal each).
    pub max_reach_entries: usize,
}

impl Default for BoundaryConfig {
    fn default() -> Self {
        Self {
            kinds: KindSet::ALL,
            max_spans: 64,
            max_reach_entries: 4096,
        }
    }
}

// ── Partitions ───────────────────────────────────────────────────────────────

/// Where the groups of a partition come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionSource {
    /// The nodes of a level of the model.
    Level(Level),
    /// The communities of a clustering of a level.
    Clusters(Level),
}

/// A grouping of the type-level units into subsystems.
#[derive(Debug, Clone)]
pub struct Partition {
    /// Group of each type-level node, `None` outside every group.
    pub group_of: Vec<Option<u32>>,
    pub names: Vec<String>,
    pub source: PartitionSource,
}

impl Partition {
    /// One group per project node of `level` (external containers form no group).
    pub fn from_level(model: &ArchModel, level: Level) -> Self {
        let graph = model.graph(level);
        let group_of = (0..model.types.node_count() as u32)
            .map(|u| {
                let g = model.container_of(NodeId(u), level);
                (!graph.node(g).external).then_some(g.0)
            })
            .collect();
        Self {
            group_of,
            names: graph.nodes.iter().map(|n| n.name.clone()).collect(),
            source: PartitionSource::Level(level),
        }
    }

    /// One group per community of `clusters`.
    pub fn from_clusters(model: &ArchModel, clusters: &ClusterResult) -> Self {
        let group_of = (0..model.types.node_count() as u32)
            .map(|u| clusters.community_of(model.container_of(NodeId(u), clusters.level)))
            .collect();
        Self {
            group_of,
            names: clusters.communities.iter().map(|c| c.label.clone()).collect(),
            source: PartitionSource::Clusters(clusters.level),
        }
    }

    pub fn group_count(&self) -> usize {
        self.names.len()
    }

    pub fn group(&self, unit: NodeId) -> Option<u32> {
        self.group_of.get(unit.index()).copied().flatten()
    }
}

// ── Results ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EntryPoint {
    pub kind: EntryKind,
    pub sources: Vec<EntrySource>,
    pub func: Option<FuncId>,
    pub symbol: Option<u32>,
    /// Type-level unit of the entry.
    pub unit: Option<NodeId>,
    pub route: Option<Route>,
    pub channel: Option<String>,
    pub evidence: Vec<Span>,
    /// Frontend rules that recognized the entry.
    pub rules: Vec<String>,
    /// Functions reachable from the entry over the call facts, the entry included; `None`
    /// when the entry has no lowered body or the count was not computed (see the notes).
    pub reaches_functions: Option<u32>,
    /// Groups of the partition whose functions the entry reaches, sorted.
    pub reaches_groups: Vec<u32>,
    pub certainty: Certainty,
}

/// How far a declaration is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Exposure {
    Public,
    /// Visible to subtypes only (the extension surface).
    Protected,
}

#[derive(Debug, Clone)]
pub struct ApiElement {
    pub symbol: u32,
    pub kind: SymbolKind,
    pub unit: NodeId,
    pub group: Option<u32>,
    pub exposure: Exposure,
    pub span: Option<Span>,
    /// Other groups whose code uses the element, sorted.
    pub used_by_groups: Vec<u32>,
    /// Whether the uses are completely known.
    pub usage: Certainty,
}

#[derive(Debug, Clone)]
pub struct ExternalDependency {
    /// Package-level node of the external namespace.
    pub namespace: NodeId,
    pub name: String,
    /// External type-level units the project uses, sorted.
    pub units: Vec<NodeId>,
    /// Project units that depend on them, sorted.
    pub dependents: Vec<NodeId>,
    pub dependent_groups: Vec<u32>,
    /// Distinct dependency sites.
    pub weight: u32,
    pub kinds: KindSet,
    pub spans: Vec<Span>,
    pub omitted_spans: u32,
    pub certainty: Certainty,
}

/// Dependencies from one group to another.
#[derive(Debug, Clone)]
pub struct Crossing {
    pub from_group: u32,
    pub to_group: u32,
    /// Distinct dependency sites.
    pub weight: u32,
    pub weight_by_kind: [u32; DepKind::COUNT],
    /// Type-level edges behind the crossing, sorted.
    pub edges: Vec<(NodeId, NodeId, DepKind)>,
    pub spans: Vec<Span>,
    pub omitted_spans: u32,
    pub certainty: Certainty,
}

/// A function-level call from one group into another.
#[derive(Debug, Clone, Copy)]
pub struct CallCrossing {
    pub site: CallSiteId,
    pub caller: FuncId,
    pub callee: FuncId,
    pub from_group: u32,
    pub to_group: u32,
    pub span: Option<Span>,
    pub strength: Strength,
}

/// What a group offers to and needs from the others.
#[derive(Debug, Clone)]
pub struct GroupInterface {
    pub group: u32,
    /// Members used from other groups, sorted.
    pub provided: Vec<NodeId>,
    /// The abstract ones among them (interfaces, abstract classes).
    pub provided_abstract: Vec<NodeId>,
    /// Project units of other groups that members use, sorted.
    pub required: Vec<NodeId>,
    /// External units that members use, sorted.
    pub required_external: Vec<NodeId>,
}

#[derive(Debug, Clone)]
pub struct Boundaries {
    pub entries: Vec<EntryPoint>,
    pub surface: Vec<ApiElement>,
    pub externals: Vec<ExternalDependency>,
    pub crossings: Vec<Crossing>,
    pub call_crossings: Vec<CallCrossing>,
    pub interfaces: Vec<GroupInterface>,
    /// Statements about what could not be derived or computed.
    pub notes: Vec<String>,
}

/// Computes the boundaries of `model` against `partition`.
pub fn analyze(
    model: &ArchModel,
    sta: &SymbolTableArtifact,
    partition: &Partition,
    input: &BoundaryInput<'_>,
    config: &BoundaryConfig,
) -> Boundaries {
    let mut notes = Vec::new();
    let symbol_units: HashMap<u32, NodeId> = model
        .types
        .nodes
        .iter()
        .filter_map(|n| n.symbol.map(|s| (s, n.id)))
        .collect();
    let func_of_symbol: HashMap<u32, FuncId> = model
        .functions
        .iter()
        .filter_map(|f| f.symbol.map(|s| (s, f.func)))
        .collect();

    let mut entries = collect_entries(
        model,
        sta,
        input,
        &symbol_units,
        &func_of_symbol,
        &mut notes,
    );
    compute_reach(model, partition, &mut entries, config, &mut notes);

    let surface = public_surface(model, sta, partition, &symbol_units, &func_of_symbol);
    let externals = external_dependencies(model, partition, config);
    let (crossings, interfaces) = group_crossings(model, partition, config);
    let call_crossings = function_call_crossings(model, partition);

    if !model.coverage.call_graph {
        notes.push(
            "no points-to call graph was supplied: solver entry points, external callbacks and runtime dispatch crossings are not derivable"
                .to_string(),
        );
    }
    if !input.registrations.is_empty() && input.module.is_none() {
        notes.push("registration rules were given without the IR module, so no registration call could be examined".to_string());
    }

    Boundaries {
        entries,
        surface,
        externals,
        crossings,
        call_crossings,
        interfaces,
        notes,
    }
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Key under which entries of different sources merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum EntryKey {
    Func(FuncId),
    Symbol(u32),
    /// A registration whose handler is not derivable, identified by its call site.
    Site(CallSiteId),
}

fn collect_entries(
    model: &ArchModel,
    sta: &SymbolTableArtifact,
    input: &BoundaryInput<'_>,
    symbol_units: &HashMap<u32, NodeId>,
    func_of_symbol: &HashMap<u32, FuncId>,
    notes: &mut Vec<String>,
) -> Vec<EntryPoint> {
    let mut entries: BTreeMap<EntryKey, EntryPoint> = BTreeMap::new();
    let facts_of = |f: FuncId| model.function(f);

    let mut add = |key: EntryKey, entry: EntryPoint| {
        match entries.get_mut(&key) {
            Some(existing) => merge_entry(existing, entry),
            None => {
                entries.insert(key, entry);
            }
        }
    };

    if let Some(solver_entries) = &model.entries {
        for &func in solver_entries {
            let facts = facts_of(func);
            let kind = match facts.map(|f| f.kind) {
                Some(FuncKind::StaticInit | FuncKind::InstanceInit | FuncKind::ModuleInit) => {
                    EntryKind::Initializer
                }
                _ => EntryKind::SolverEntry,
            };
            add(
                EntryKey::Func(func),
                EntryPoint {
                    kind,
                    sources: vec![EntrySource::CallGraphEntry],
                    func: Some(func),
                    symbol: facts.and_then(|f| f.symbol),
                    unit: facts.map(|f| f.owner),
                    route: None,
                    channel: None,
                    evidence: facts.and_then(|f| f.span).into_iter().collect(),
                    rules: Vec::new(),
                    reaches_functions: None,
                    reaches_groups: Vec::new(),
                    certainty: Certainty::Exact,
                },
            );
        }
    }

    for callback in &model.callbacks {
        let facts = facts_of(callback.func);
        let reason = match callback.kind {
            CallbackKind::ExternalOverride { .. } => {
                "external code may invoke the override on an object it received"
            }
            CallbackKind::ExternalFunctionValue => {
                "external code may invoke the function value it received"
            }
            CallbackKind::Intrinsic { .. } => "an intrinsic invokes the function value",
        };
        let certainty = match callback.kind {
            CallbackKind::Intrinsic { .. } => Certainty::Exact,
            _ => Certainty::Partial {
                reason: reason.to_string(),
            },
        };
        add(
            EntryKey::Func(callback.func),
            EntryPoint {
                kind: EntryKind::Callback,
                sources: vec![EntrySource::ExternalCallback],
                func: Some(callback.func),
                symbol: facts.and_then(|f| f.symbol),
                unit: facts.map(|f| f.owner),
                route: None,
                channel: None,
                evidence: callback.span.into_iter().collect(),
                rules: Vec::new(),
                reaches_functions: None,
                reaches_groups: Vec::new(),
                certainty,
            },
        );
    }

    for declared in &input.declared {
        let func = func_of_symbol.get(&declared.symbol).copied();
        let unit = func
            .and_then(|f| facts_of(f).map(|x| x.owner))
            .or_else(|| unit_of_symbol(sta, symbol_units, declared.symbol));
        let key = func.map_or(EntryKey::Symbol(declared.symbol), EntryKey::Func);
        add(
            key,
            EntryPoint {
                kind: declared.kind,
                sources: vec![EntrySource::Declared],
                func,
                symbol: Some(declared.symbol),
                unit,
                route: declared.route.clone(),
                channel: declared.channel.clone(),
                evidence: declared.evidence.clone(),
                rules: vec![declared.rule.clone()],
                reaches_functions: None,
                reaches_groups: Vec::new(),
                certainty: Certainty::Exact,
            },
        );
    }

    if let Some(module) = input.module {
        for (key, entry) in registrations(model, sta, module, &input.registrations, notes) {
            add(key, entry);
        }
    }

    entries.into_values().collect()
}

/// Merges two records of the same entry: declared and registered kinds win over the
/// solver's, evidence, rules and routes are united, and the stronger certainty holds.
fn merge_entry(existing: &mut EntryPoint, other: EntryPoint) {
    let specific = |k: EntryKind| !matches!(k, EntryKind::SolverEntry | EntryKind::Callback);
    if !specific(existing.kind) && specific(other.kind) {
        existing.kind = other.kind;
    }
    for s in other.sources {
        if !existing.sources.contains(&s) {
            existing.sources.push(s);
        }
    }
    existing.sources.sort_unstable();
    existing.symbol = existing.symbol.or(other.symbol);
    existing.unit = existing.unit.or(other.unit);
    existing.route = match (existing.route.take(), other.route) {
        (Some(mut a), Some(b)) => {
            if a.path.is_none() {
                a.path = b.path;
            }
            for m in b.methods {
                if !a.methods.contains(&m) {
                    a.methods.push(m);
                }
            }
            a.methods.sort();
            Some(a)
        }
        (a, b) => a.or(b),
    };
    existing.channel = existing.channel.take().or(other.channel);
    existing.evidence.extend(other.evidence);
    existing.evidence.sort_by_key(span_key);
    existing.evidence.dedup_by_key(|s| span_key(s));
    for r in other.rules {
        if !existing.rules.contains(&r) {
            existing.rules.push(r);
        }
    }
    existing.certainty = existing.certainty.clone().strongest(other.certainty);
}

/// Type-level unit of a declaration: the nearest enclosing declaration that is a unit.
fn unit_of_symbol(
    sta: &SymbolTableArtifact,
    symbol_units: &HashMap<u32, NodeId>,
    sym: u32,
) -> Option<NodeId> {
    let mut cur = sym;
    let mut steps = 0usize;
    while (cur as usize) < sta.symbol_records.len() && steps <= sta.symbol_records.len() {
        if let Some(&unit) = symbol_units.get(&cur) {
            return Some(unit);
        }
        cur = sta.symbol_records[cur as usize].parent_sym;
        steps += 1;
    }
    None
}

/// Symbols each rule's matcher denotes.
fn rule_symbols(sta: &SymbolTableArtifact, rules: &[RegistrationRule]) -> Vec<HashSet<u32>> {
    let mut sets: Vec<HashSet<u32>> = vec![HashSet::new(); rules.len()];
    // Several rules may share a name; each name is resolved once, then fanned out.
    let mut name_rules: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut descriptor_rules: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, rule) in rules.iter().enumerate() {
        match &rule.callee {
            CalleeMatcher::Symbol(s) => {
                sets[i].insert(*s);
            }
            CalleeMatcher::QualifiedName(q) => name_rules.entry(q.as_str()).or_default().push(i),
            CalleeMatcher::Descriptor(d) => {
                descriptor_rules.entry(d.as_str()).or_default().push(i)
            }
        }
    }
    if name_rules.is_empty() && descriptor_rules.is_empty() {
        return sets;
    }
    for sym in 0..sta.symbol_records.len() as u32 {
        if let Some(rule_ids) = sta.qualified_name_of(sym).and_then(|q| name_rules.get(q)) {
            for &i in rule_ids {
                sets[i].insert(sym);
            }
        }
        if let Some(rule_ids) = sta.descriptor_of(sym).and_then(|d| descriptor_rules.get(d)) {
            for &i in rule_ids {
                sets[i].insert(sym);
            }
        }
    }
    sets
}

/// Entries registered by calls of the rules' registration functions.
fn registrations(
    model: &ArchModel,
    sta: &SymbolTableArtifact,
    module: &ModuleIr,
    rules: &[RegistrationRule],
    notes: &mut Vec<String>,
) -> Vec<(EntryKey, EntryPoint)> {
    if rules.is_empty() {
        return Vec::new();
    }
    let symbols = rule_symbols(sta, rules);
    for (i, set) in symbols.iter().enumerate() {
        if set.is_empty() {
            notes.push(format!(
                "registration rule `{}` names no declaration of the symbol table",
                rules[i].rule
            ));
        }
    }
    let mut site_targets: HashMap<CallSiteId, Vec<FuncId>> = HashMap::new();
    for c in &model.calls {
        site_targets.entry(c.site).or_default().push(c.callee);
    }
    let func_symbol: HashMap<FuncId, u32> = module
        .funcs
        .iter()
        .filter_map(|f| f.symbol.map(|s| (f.id, s)))
        .collect();

    let mut out = Vec::new();
    for func in &module.funcs {
        let defs = definitions(func);
        for block in &func.blocks {
            for (i, inst) in block.insts.iter().enumerate() {
                let Inst::Call {
                    site, callee, args, ..
                } = inst
                else {
                    continue;
                };
                let mut callee_symbols: Vec<u32> = Vec::new();
                match callee {
                    Callee::Static(f) => callee_symbols.extend(func_symbol.get(f).copied()),
                    Callee::Symbol(s)
                    | Callee::Virtual { method: s, .. }
                    | Callee::Interface { method: s, .. } => callee_symbols.push(*s),
                    Callee::Indirect(_) | Callee::Unresolved(_) => {}
                }
                if let Some(targets) = site_targets.get(site) {
                    callee_symbols.extend(targets.iter().filter_map(|f| func_symbol.get(f).copied()));
                }
                let span = block
                    .inst_spans
                    .get(i)
                    .copied()
                    .filter(|s| !s.is_none())
                    .or_else(|| (!func.span.is_none()).then_some(func.span));

                for (r, rule) in rules.iter().enumerate() {
                    if !callee_symbols.iter().any(|s| symbols[r].contains(s)) {
                        continue;
                    }
                    out.extend(registration_entries(
                        model, module, func, &defs, *site, args, span, rule,
                    ));
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn registration_entries(
    model: &ArchModel,
    module: &ModuleIr,
    func: &FuncIr,
    defs: &HashMap<ValueId, &Inst>,
    site: CallSiteId,
    args: &[Operand],
    span: Option<Span>,
    rule: &RegistrationRule,
) -> Vec<(EntryKey, EntryPoint)> {
    let mut problems: Vec<String> = Vec::new();

    let read_strings = |index: Option<usize>, what: &str, problems: &mut Vec<String>| {
        let index = index?;
        match args.get(index) {
            Some(op) => {
                let (values, complete) = string_values(module, defs, op);
                if !complete {
                    problems.push(format!("the {} argument is not a compile-time string constant", what));
                }
                Some(values)
            }
            None => {
                problems.push(format!("the call has no {} argument at index {}", what, index));
                None
            }
        }
    };
    let paths = read_strings(rule.route_arg, "route", &mut problems);
    let methods = read_strings(rule.method_arg, "method", &mut problems);
    let channels = read_strings(rule.channel_arg, "channel", &mut problems);

    let route = if rule.route_arg.is_some() || rule.method_arg.is_some() || !rule.fixed_methods.is_empty() {
        let mut all_methods: Vec<String> = rule.fixed_methods.clone();
        all_methods.extend(methods.into_iter().flatten());
        all_methods.sort();
        all_methods.dedup();
        let path = match paths.as_deref() {
            Some([one]) => Some(one.clone()),
            Some([]) | None => None,
            Some(several) => {
                problems.push(format!(
                    "the route argument takes {} different constant values on different paths",
                    several.len()
                ));
                None
            }
        };
        Some(Route {
            path,
            methods: all_methods,
        })
    } else {
        None
    };
    let channel = match channels.as_deref() {
        Some([one]) => Some(one.clone()),
        Some(several) if several.len() > 1 => {
            problems.push(format!(
                "the channel argument takes {} different constant values on different paths",
                several.len()
            ));
            None
        }
        _ => None,
    };

    let (handlers, handlers_complete) = match args.get(rule.handler_arg) {
        Some(op) => function_values(defs, op),
        None => {
            problems.push(format!(
                "the call has no handler argument at index {}",
                rule.handler_arg
            ));
            (Vec::new(), false)
        }
    };
    if !handlers_complete {
        problems.push(
            "the handler argument is not (only) a function value created in the registering function"
                .to_string(),
        );
    }

    let certainty = |extra: bool| {
        if !problems.is_empty() {
            Certainty::Partial {
                reason: problems.join("; "),
            }
        } else if extra {
            Certainty::OverApproximate
        } else {
            Certainty::Exact
        }
    };

    let registering = model.function(func.id);
    if handlers.is_empty() {
        return vec![(
            EntryKey::Site(site),
            EntryPoint {
                kind: rule.kind,
                sources: vec![EntrySource::Registration],
                func: None,
                symbol: None,
                unit: registering.map(|f| f.owner),
                route,
                channel,
                evidence: span.into_iter().collect(),
                rules: vec![rule.rule.clone()],
                reaches_functions: None,
                reaches_groups: Vec::new(),
                certainty: Certainty::Partial {
                    reason: if problems.is_empty() {
                        "the registered handler is not derivable".to_string()
                    } else {
                        problems.join("; ")
                    },
                },
            },
        )];
    }
    // Several handlers reach the argument on different paths: each one may be registered.
    let several = handlers.len() > 1;
    handlers
        .into_iter()
        .map(|handler| {
            let facts = model.function(handler);
            (
                EntryKey::Func(handler),
                EntryPoint {
                    kind: rule.kind,
                    sources: vec![EntrySource::Registration],
                    func: Some(handler),
                    symbol: facts.and_then(|f| f.symbol),
                    unit: facts.map(|f| f.owner),
                    route: route.clone(),
                    channel: channel.clone(),
                    evidence: span.into_iter().collect(),
                    rules: vec![rule.rule.clone()],
                    reaches_functions: None,
                    reaches_groups: Vec::new(),
                    certainty: certainty(several),
                },
            )
        })
        .collect()
}

/// Defining instruction of every value of `func` (SSA: one definition per value).
fn definitions(func: &FuncIr) -> HashMap<ValueId, &Inst> {
    let mut defs = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let Some(dst) = inst.dst() {
                defs.insert(dst, inst);
            }
        }
    }
    defs
}

/// Functions whose closures reach `op` through copies, casts and phis, and whether every
/// path ends in a closure creation.
fn function_values(defs: &HashMap<ValueId, &Inst>, op: &Operand) -> (Vec<FuncId>, bool) {
    let mut found = BTreeSet::new();
    let mut complete = true;
    let mut stack: Vec<ValueId> = Vec::new();
    let mut seen: HashSet<ValueId> = HashSet::new();
    match op {
        Operand::Value(v) => stack.push(*v),
        Operand::Const(_) | Operand::Undef => complete = false,
    }
    while let Some(v) = stack.pop() {
        if !seen.insert(v) {
            continue;
        }
        match defs.get(&v) {
            Some(Inst::MakeClosure { func, .. }) => {
                found.insert(*func);
            }
            Some(Inst::Assign { src, .. }) | Some(Inst::Cast { src, .. }) => match src {
                Operand::Value(w) => stack.push(*w),
                Operand::Const(_) | Operand::Undef => complete = false,
            },
            Some(Inst::Phi { incoming, .. }) => {
                for (_, operand) in incoming {
                    match operand {
                        Operand::Value(w) => stack.push(*w),
                        Operand::Const(_) | Operand::Undef => complete = false,
                    }
                }
            }
            // A parameter, a load or a call result: the function value comes from elsewhere.
            _ => complete = false,
        }
    }
    (found.into_iter().collect(), complete)
}

/// String constants that reach `op` through copies, casts and phis, and whether every path
/// ends in a string constant.
fn string_values(
    module: &ModuleIr,
    defs: &HashMap<ValueId, &Inst>,
    op: &Operand,
) -> (Vec<String>, bool) {
    let mut found = BTreeSet::new();
    let mut complete = true;
    let mut operands: Vec<Operand> = vec![*op];
    let mut seen: HashSet<ValueId> = HashSet::new();
    while let Some(operand) = operands.pop() {
        match operand {
            Operand::Const(ConstVal::Str(id)) => {
                found.insert(String::from_utf8_lossy(module.strings.lookup_text(id)).into_owned());
            }
            Operand::Const(_) | Operand::Undef => complete = false,
            Operand::Value(v) => {
                if !seen.insert(v) {
                    continue;
                }
                match defs.get(&v) {
                    Some(Inst::Assign { src, .. }) | Some(Inst::Cast { src, .. }) => {
                        operands.push(*src)
                    }
                    Some(Inst::Phi { incoming, .. }) => {
                        operands.extend(incoming.iter().map(|&(_, o)| o))
                    }
                    _ => complete = false,
                }
            }
        }
    }
    (found.into_iter().collect(), complete)
}

/// Functions and groups reachable from every entry over the call facts.
fn compute_reach(
    model: &ArchModel,
    partition: &Partition,
    entries: &mut [EntryPoint],
    config: &BoundaryConfig,
    notes: &mut Vec<String>,
) {
    let n = model.functions.len();
    let index = |f: FuncId| model.functions.binary_search_by_key(&f, |x| x.func).ok();
    let mut edges = Vec::with_capacity(model.calls.len());
    for c in &model.calls {
        if let (Some(a), Some(b)) = (index(c.caller), index(c.callee)) {
            edges.push((a as u32, b as u32, 1u64));
        }
    }
    let g = WeightedDigraph::from_edges(n, edges);

    // Groups reached, by a bit-set dynamic program over the condensation.
    let groups = partition.group_count();
    let words = groups.div_ceil(64).max(1);
    let comps = strongly_connected_components(&g);
    let mut reach: Vec<Vec<u64>> = vec![vec![0u64; words]; comps.count()];
    for c in 0..comps.count() {
        let mut bits = vec![0u64; words];
        for &v in &comps.members[c] {
            if let Some(group) = partition.group(model.functions[v as usize].owner) {
                bits[group as usize / 64] |= 1u64 << (group % 64);
            }
            for &w in g.targets(v) {
                let d = comps.component[w as usize] as usize;
                if d != c {
                    for (b, &r) in bits.iter_mut().zip(reach[d].iter()) {
                        *b |= r;
                    }
                }
            }
        }
        reach[c] = bits;
    }

    let traversals = entries.iter().filter(|e| e.func.is_some()).count();
    let count_functions = traversals <= config.max_reach_entries;
    if !count_functions {
        notes.push(format!(
            "{} entry points exceed the limit of {} traversals; reachable function counts were not computed",
            traversals, config.max_reach_entries
        ));
    }
    let mut stamp = vec![u32::MAX; n];
    let mut stack: Vec<u32> = Vec::new();
    for (e, entry) in entries.iter_mut().enumerate() {
        let Some(start) = entry.func.and_then(index) else {
            continue;
        };
        let bits = &reach[comps.component[start] as usize];
        entry.reaches_groups = (0..groups as u32)
            .filter(|&grp| bits[grp as usize / 64] & (1u64 << (grp % 64)) != 0)
            .collect();
        if !count_functions {
            continue;
        }
        let mut count = 0u32;
        stamp[start] = e as u32;
        stack.push(start as u32);
        while let Some(v) = stack.pop() {
            count += 1;
            for &w in g.targets(v) {
                if stamp[w as usize] != e as u32 {
                    stamp[w as usize] = e as u32;
                    stack.push(w);
                }
            }
        }
        entry.reaches_functions = Some(count);
    }
}

// ── Public surface ───────────────────────────────────────────────────────────

fn public_surface(
    model: &ArchModel,
    sta: &SymbolTableArtifact,
    partition: &Partition,
    symbol_units: &HashMap<u32, NodeId>,
    func_of_symbol: &HashMap<u32, FuncId>,
) -> Vec<ApiElement> {
    // Uses of members from other groups: calls and field accesses.
    let group_of_func = |f: FuncId| {
        model
            .function(f)
            .and_then(|x| partition.group(x.owner))
    };
    let mut function_users: HashMap<FuncId, BTreeSet<u32>> = HashMap::new();
    for c in &model.calls {
        if let Some(g) = group_of_func(c.caller) {
            function_users.entry(c.callee).or_default().insert(g);
        }
    }
    let field_symbol: HashMap<_, u32> = model
        .fields
        .iter()
        .filter_map(|f| f.symbol.map(|s| (f.field, s)))
        .collect();
    let mut symbol_users: HashMap<u32, BTreeSet<u32>> = HashMap::new();
    for f in &model.functions {
        let Some(g) = partition.group(f.owner) else {
            continue;
        };
        for sym in &f.symbol_callees {
            symbol_users.entry(*sym).or_default().insert(g);
        }
        let fields = f
            .instance_fields
            .iter()
            .chain(f.static_fields.iter().map(|(_, field)| field));
        for field in fields {
            if let Some(&sym) = field_symbol.get(field) {
                symbol_users.entry(sym).or_default().insert(g);
            }
        }
    }
    // Uses of units from other groups: any incoming dependency.
    let mut unit_users: HashMap<NodeId, BTreeSet<u32>> = HashMap::new();
    for e in &model.types.edges {
        if let Some(g) = partition.group(e.from) {
            unit_users.entry(e.to).or_default().insert(g);
        }
    }
    let body_facts = model.coverage.ir;
    let token_files = TokenFiles::new(model);

    let mut out = Vec::new();
    for sym in 0..sta.symbol_records.len() as u32 {
        let rec = &sta.symbol_records[sym as usize];
        let kind = SymbolKind::from(rec.kind);
        let is_unit_kind = matches!(
            kind,
            SymbolKind::SK_CLASS
                | SymbolKind::SK_INTERFACE
                | SymbolKind::SK_ENUM
                | SymbolKind::SK_RECORD
                | SymbolKind::SK_ANNOTATION_TYPE
        );
        let is_member_kind = matches!(
            kind,
            SymbolKind::SK_METHOD
                | SymbolKind::SK_CONSTRUCTOR
                | SymbolKind::SK_FIELD
                | SymbolKind::SK_ENUM_CONSTANT
        );
        if !is_unit_kind && !is_member_kind {
            continue;
        }
        let Some(unit) = unit_of_symbol(sta, symbol_units, sym) else {
            continue;
        };
        let unit_node = model.types.node(unit);
        if unit_node.external {
            continue;
        }
        let Some(exposure) = exposure(sta, sym) else {
            continue;
        };
        let group = partition.group(unit);
        let is_unit = symbol_units.get(&sym) == Some(&unit);
        let mut users: BTreeSet<u32> = BTreeSet::new();
        let usage;
        if is_unit {
            users.extend(unit_users.get(&unit).into_iter().flatten().copied());
            usage = Certainty::Exact;
        } else {
            if let Some(f) = func_of_symbol.get(&sym) {
                users.extend(function_users.get(f).into_iter().flatten().copied());
            }
            users.extend(symbol_users.get(&sym).into_iter().flatten().copied());
            usage = if body_facts {
                Certainty::Exact
            } else {
                Certainty::NotDerivable {
                    reason: "member-level uses need the IR".to_string(),
                }
            };
        }
        if let Some(g) = group {
            users.remove(&g);
        }
        let span = unit_node
            .spans
            .first()
            .copied()
            .filter(|_| is_unit)
            .or_else(|| token_files.symbol_span(sta, sym));
        out.push(ApiElement {
            symbol: sym,
            kind,
            unit,
            group,
            exposure,
            span,
            used_by_groups: users.into_iter().collect(),
            usage,
        });
    }
    out
}

/// Effective exposure of a declaration: every declaration from it up to its namespace must
/// be public or protected; a declaration nested in a function body is never exposed.
fn exposure(sta: &SymbolTableArtifact, sym: u32) -> Option<Exposure> {
    let mut protected = false;
    let mut cur = sym;
    let mut steps = 0usize;
    while (cur as usize) < sta.symbol_records.len() && steps <= sta.symbol_records.len() {
        let rec = &sta.symbol_records[cur as usize];
        let kind = SymbolKind::from(rec.kind);
        match kind {
            SymbolKind::SK_PACKAGE | SymbolKind::SK_MODULE => break,
            SymbolKind::SK_METHOD
            | SymbolKind::SK_CONSTRUCTOR
            | SymbolKind::SK_LAMBDA
            | SymbolKind::SK_STATIC_INIT
            | SymbolKind::SK_INSTANCE_INIT
            | SymbolKind::SK_ANON_CLASS
                if cur != sym =>
            {
                return None
            }
            _ => {}
        }
        match SymbolVisibility::from(rec.visibility) {
            SymbolVisibility::Public => {}
            SymbolVisibility::Protected => protected = true,
            SymbolVisibility::Private | SymbolVisibility::Package => return None,
        }
        cur = rec.parent_sym;
        steps += 1;
    }
    Some(if protected {
        Exposure::Protected
    } else {
        Exposure::Public
    })
}

/// Token ranges of the analysed files, from the file-level nodes.
struct TokenFiles(Vec<(u32, u32, u32)>);

impl TokenFiles {
    fn new(model: &ArchModel) -> Self {
        let mut ranges: Vec<(u32, u32, u32)> = model
            .files
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .flat_map(|n| n.spans.iter().map(|s| (s.first_token, s.last_token, s.file_id)))
            .collect();
        ranges.sort_unstable();
        Self(ranges)
    }

    /// Span of the declaration `sym`, in the file whose tokens contain it.
    fn symbol_span(&self, sta: &SymbolTableArtifact, sym: u32) -> Option<Span> {
        let rec = sta.symbol_records.get(sym as usize)?;
        let first = rec.first_token_id;
        if first == u32::MAX {
            return None;
        }
        let i = self.0.partition_point(|r| r.0 <= first);
        let &(_, end, file) = self.0.get(i.checked_sub(1)?)?;
        if first > end {
            return None;
        }
        let last = if rec.last_token_id == u32::MAX {
            first
        } else {
            rec.last_token_id.clamp(first, end)
        };
        Some(Span::new(file, first, last))
    }
}

// ── External dependencies ────────────────────────────────────────────────────

fn external_dependencies(
    model: &ArchModel,
    partition: &Partition,
    config: &BoundaryConfig,
) -> Vec<ExternalDependency> {
    struct Acc {
        units: BTreeSet<NodeId>,
        dependents: BTreeSet<NodeId>,
        groups: BTreeSet<u32>,
        sites: HashMap<Origin, Strength>,
        spans: BTreeMap<(u32, u32, u32), Span>,
        kinds: KindSet,
        partial: Option<String>,
    }
    let mut by_namespace: BTreeMap<NodeId, Acc> = BTreeMap::new();
    let types = &model.types;
    for e in &types.edges {
        if !config.kinds.contains(e.kind) {
            continue;
        }
        let (from, to) = (types.node(e.from), types.node(e.to));
        if from.external || !to.external {
            continue;
        }
        let namespace = model.container_of(e.to, Level::Package);
        let acc = by_namespace.entry(namespace).or_insert_with(|| Acc {
            units: BTreeSet::new(),
            dependents: BTreeSet::new(),
            groups: BTreeSet::new(),
            sites: HashMap::new(),
            spans: BTreeMap::new(),
            kinds: KindSet::NONE,
            partial: None,
        });
        acc.units.insert(e.to);
        acc.dependents.insert(e.from);
        if let Some(g) = partition.group(e.from) {
            acc.groups.insert(g);
        }
        acc.kinds = acc.kinds.with(e.kind);
        for ev in &e.evidence {
            let slot = acc.sites.entry(ev.origin).or_insert(ev.strength);
            if ev.strength < *slot {
                *slot = ev.strength;
            }
            if let Some(span) = ev.span {
                acc.spans.entry(span_key(&span)).or_insert(span);
            }
        }
        if let Certainty::Partial { reason } = &e.certainty {
            acc.partial.get_or_insert_with(|| reason.clone());
        }
    }

    by_namespace
        .into_iter()
        .map(|(namespace, acc)| {
            let best = acc.sites.values().copied().min().unwrap_or(Strength::Partial);
            let total_spans = acc.spans.len();
            let spans: Vec<Span> = acc.spans.into_values().take(config.max_spans).collect();
            ExternalDependency {
                namespace,
                name: model.packages.node(namespace).name.clone(),
                units: acc.units.into_iter().collect(),
                dependents: acc.dependents.into_iter().collect(),
                dependent_groups: acc.groups.into_iter().collect(),
                weight: acc.sites.len() as u32,
                kinds: acc.kinds,
                omitted_spans: (total_spans - spans.len()) as u32,
                spans,
                certainty: Certainty::from_strength(best, acc.partial.as_deref()),
            }
        })
        .collect()
}

// ── Crossings and interfaces ─────────────────────────────────────────────────

fn group_crossings(
    model: &ArchModel,
    partition: &Partition,
    config: &BoundaryConfig,
) -> (Vec<Crossing>, Vec<GroupInterface>) {
    struct Acc {
        sites: HashMap<Origin, Strength>,
        by_kind: [HashSet<Origin>; DepKind::COUNT],
        edges: Vec<(NodeId, NodeId, DepKind)>,
        spans: BTreeMap<(u32, u32, u32), Span>,
        partial: Option<String>,
    }
    let groups = partition.group_count();
    let mut provided: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); groups];
    let mut required: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); groups];
    let mut required_external: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); groups];
    let mut by_pair: BTreeMap<(u32, u32), Acc> = BTreeMap::new();
    let types = &model.types;

    for e in &types.edges {
        if !config.kinds.contains(e.kind) {
            continue;
        }
        let Some(gf) = partition.group(e.from) else {
            continue;
        };
        if types.node(e.to).external {
            required_external[gf as usize].insert(e.to);
            continue;
        }
        let Some(gt) = partition.group(e.to) else {
            continue;
        };
        if gf == gt {
            continue;
        }
        provided[gt as usize].insert(e.to);
        required[gf as usize].insert(e.to);
        let acc = by_pair.entry((gf, gt)).or_insert_with(|| Acc {
            sites: HashMap::new(),
            by_kind: std::array::from_fn(|_| HashSet::new()),
            edges: Vec::new(),
            spans: BTreeMap::new(),
            partial: None,
        });
        acc.edges.push((e.from, e.to, e.kind));
        for ev in &e.evidence {
            let slot = acc.sites.entry(ev.origin).or_insert(ev.strength);
            if ev.strength < *slot {
                *slot = ev.strength;
            }
            acc.by_kind[e.kind.index()].insert(ev.origin);
            if let Some(span) = ev.span {
                acc.spans.entry(span_key(&span)).or_insert(span);
            }
        }
        if let Certainty::Partial { reason } = &e.certainty {
            acc.partial.get_or_insert_with(|| reason.clone());
        }
    }

    let crossings = by_pair
        .into_iter()
        .map(|((from_group, to_group), mut acc)| {
            acc.edges.sort();
            let best = acc.sites.values().copied().min().unwrap_or(Strength::Partial);
            let total_spans = acc.spans.len();
            let spans: Vec<Span> = acc.spans.into_values().take(config.max_spans).collect();
            let mut weight_by_kind = [0u32; DepKind::COUNT];
            for (k, set) in acc.by_kind.iter().enumerate() {
                weight_by_kind[k] = set.len() as u32;
            }
            Crossing {
                from_group,
                to_group,
                weight: acc.sites.len() as u32,
                weight_by_kind,
                edges: acc.edges,
                omitted_spans: (total_spans - spans.len()) as u32,
                spans,
                certainty: Certainty::from_strength(best, acc.partial.as_deref()),
            }
        })
        .collect();

    let interfaces = (0..groups)
        .map(|g| {
            let provided_list: Vec<NodeId> = provided[g].iter().copied().collect();
            let provided_abstract = provided_list
                .iter()
                .copied()
                .filter(|&n| types.node(n).is_abstract)
                .collect();
            GroupInterface {
                group: g as u32,
                provided: provided_list,
                provided_abstract,
                required: required[g].iter().copied().collect(),
                required_external: required_external[g].iter().copied().collect(),
            }
        })
        .collect();

    (crossings, interfaces)
}

fn function_call_crossings(model: &ArchModel, partition: &Partition) -> Vec<CallCrossing> {
    let group_of = |f: FuncId| model.function(f).and_then(|x| partition.group(x.owner));
    model
        .calls
        .iter()
        .filter_map(|c| {
            let (from_group, to_group) = (group_of(c.caller)?, group_of(c.callee)?);
            (from_group != to_group).then_some(CallCrossing {
                site: c.site,
                caller: c.caller,
                callee: c.callee,
                from_group,
                to_group,
                span: c.span,
                strength: c.strength,
            })
        })
        .collect()
}
