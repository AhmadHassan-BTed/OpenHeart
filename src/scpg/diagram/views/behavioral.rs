//! Behavioral view builders: use case, activity, state machine, sequence, communication,
//! interaction overview and timing diagrams (§10.4, from the Phase 9 UMA records).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::scpg::diagram::context::{DiagramContext, EXTERNAL_ACTOR_SYM};
use crate::scpg::diagram::model::{
    AliasTable, EdgeKind, FragmentKind, Interaction, InteractionEvent, NodeKind, Provenance,
    ViewEdge, ViewKind, ViewModel, ViewNode,
};
use crate::scpg::diagram::views::analysis::{block_condition, block_lines, edge_type_label};
use crate::scpg::diagram::views::{
    function_fqn, function_label, ordered_functions, FunctionBudget,
};
use crate::core::types::cfg::CFGEdgeType;
use crate::uma::types::*;

/// Maximum message lines listed on one communication link.
const MAX_LINK_LINES: usize = 8;
/// Maximum entry points listed in the use-case note.
const MAX_LISTED_ENTRY_POINTS: usize = 25;

// ── 8. USE CASE DIAGRAM ──────────────────────────────────────────────────────

pub fn use_case_view(ctx: &DiagramContext) -> ViewModel {
    let mut model = ViewModel::unavailable(
        ViewKind::UseCase,
        "Use cases describe externally observable goals of actors; they are not derivable from code structure, and no requirements or use-case artifacts were analyzed.",
    );
    let entries: BTreeSet<String> = ctx
        .uma
        .sequences
        .iter()
        .map(|s| scenario_title(ctx, s))
        .collect();
    if !entries.is_empty() {
        let listed: Vec<String> = entries.iter().take(MAX_LISTED_ENTRY_POINTS).cloned().collect();
        let mut text = format!(
            "Entry points identified for sequence scenarios (candidate use-case realizations): {}",
            listed.join(", ")
        );
        if entries.len() > MAX_LISTED_ENTRY_POINTS {
            text.push_str(&format!(" and {} more", entries.len() - MAX_LISTED_ENTRY_POINTS));
        }
        model.note(text);
    }
    model
}

// ── 9. ACTIVITY DIAGRAM ──────────────────────────────────────────────────────

fn activity_node_kind(kind: u8) -> NodeKind {
    match kind {
        NODE_KIND_INITIAL => NodeKind::Initial,
        NODE_KIND_FINAL => NodeKind::Final,
        NODE_KIND_DECISION => NodeKind::Decision,
        NODE_KIND_MERGE => NodeKind::Merge,
        NODE_KIND_FORK => NodeKind::Fork,
        NODE_KIND_JOIN => NodeKind::Join,
        NODE_KIND_EXCEPTION => NodeKind::ExceptionHandler,
        _ => NodeKind::Action,
    }
}

pub fn activity_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.activities.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Activity,
            "Phase 9 recorded no activities (no analyzed method bodies).",
        );
    }
    let mut model = ViewModel::new(ViewKind::Activity);
    model.source("uma");
    if ctx.cfa.is_some() {
        model.source("cfa");
    }
    let mut by_sym: HashMap<u32, &ActivityRecord> = HashMap::new();
    for act in &ctx.uma.activities {
        by_sym.entry(act.function_sym_id).or_insert(act);
    }
    let order = ordered_functions(ctx, ctx.uma.activities.iter().map(|a| a.function_sym_id));
    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    let mut unlabeled = false;
    for (fqn, sym) in order {
        let act = match by_sym.get(&sym) {
            Some(a) => *a,
            None => continue,
        };
        if !budget.admit(&fqn, act.nodes.len() + 1) {
            continue;
        }
        unlabeled |= add_activity(ctx, &mut model, act, None);
    }
    budget.finish(&mut model, "function(s)");
    if unlabeled {
        model.note("Some actions are labelled by block id: Phase 9 recorded no label and no statement text is available for them.");
    }
    model
}

/// Adds one activity as a function partition. Nodes listed in `references` become
/// interaction uses labelled with the referenced scenario. Returns true when any node had to
/// be labelled by its block id.
fn add_activity(
    ctx: &DiagramContext,
    model: &mut ViewModel,
    act: &ActivityRecord,
    references: Option<&HashMap<u32, String>>,
) -> bool {
    let sym = act.function_sym_id;
    let group_key = format!("fn:{}", sym);
    let mut group_details = vec![format!("V(G)={}", act.cyclomatic)];
    if act.max_loop_depth > 0 {
        group_details.push(format!("max loop depth {}", act.max_loop_depth));
    }
    model.add_node(
        ViewNode::new(
            group_key.clone(),
            format!("fn_{}", sym),
            function_label(ctx, sym),
            NodeKind::Group,
        )
        .with_qualified_name(Some(function_fqn(ctx, sym)))
        .with_span(ctx.symbol_span(sym))
        .with_details(group_details)
        .with_sym(sym),
    );

    let cfg = ctx.function_cfg(sym);
    let mut unlabeled = false;
    let mut kinds: HashMap<u32, NodeKind> = HashMap::new();
    for node in &act.nodes {
        let mut kind = activity_node_kind(node.node_kind);
        let mut label = match kind {
            NodeKind::Initial => "start".to_string(),
            NodeKind::Final => "end".to_string(),
            _ => {
                let from_record = ctx.uma.label(node.label_text_id).map(str::to_string);
                let from_cfg = match (kind, cfg) {
                    (NodeKind::Decision, Some(c)) => block_condition(ctx, c, node.node_id)
                        .or_else(|| block_lines(ctx, c, node.node_id, 1).into_iter().next()),
                    (_, Some(c)) => block_lines(ctx, c, node.node_id, 1).into_iter().next(),
                    _ => None,
                };
                match from_record.or(from_cfg) {
                    Some(l) => l,
                    None => {
                        unlabeled = true;
                        format!("block {}", node.node_id)
                    }
                }
            }
        };
        if let Some(refs) = references {
            if let Some(reference) = refs.get(&node.node_id) {
                kind = NodeKind::InteractionUse;
                label = reference.clone();
            }
        }
        let mut details = Vec::new();
        if node.loop_depth > 0 {
            details.push(format!("loop depth {}", node.loop_depth));
        }
        let mut span = ctx.span_of_tokens(node.first_token, node.first_token);
        if let Some(c) = cfg {
            if let Some(blk) = c.blocks.get(node.node_id as usize) {
                if blk.first_token != u32::MAX {
                    span = ctx.span_of_tokens(blk.first_token, blk.last_token).or(span);
                }
            }
            if matches!(kind, NodeKind::Action | NodeKind::InteractionUse) {
                for line in block_lines(ctx, c, node.node_id, 3).into_iter().skip(1) {
                    details.push(line);
                }
            }
        }
        kinds.insert(node.node_id, kind);
        model.add_node(
            ViewNode::new(
                format!("act:{}:{}", sym, node.node_id),
                format!("act_{}_{}", sym, node.node_id),
                label,
                kind,
            )
            .with_parent(Some(group_key.clone()))
            .with_span(span)
            .with_details(details),
        );
    }

    let cfg_aligned = cfg
        .map(|c| c.edges.len() == act.edges.len())
        .unwrap_or(false);
    for (idx, edge) in act.edges.iter().enumerate() {
        let from = format!("act:{}:{}", sym, edge.from_node);
        let to = format!("act:{}:{}", sym, edge.to_node);
        let mut kind = if edge.edge_kind == EDGE_KIND_EXCEPTION {
            EdgeKind::ExceptionFlow
        } else {
            EdgeKind::ControlFlow
        };
        let mut label = ctx.uma.label(edge.guard_text_id).map(str::to_string);
        if label.is_none() && cfg_aligned {
            if let Some(c) = cfg {
                if let Some(&(_, _, etype)) = c.edges.get(idx) {
                    if etype == CFGEdgeType::Except {
                        kind = EdgeKind::ExceptionFlow;
                    }
                    let source_is_decision =
                        kinds.get(&edge.from_node) == Some(&NodeKind::Decision);
                    label = match etype {
                        CFGEdgeType::True if source_is_decision => Some(
                            match block_condition(ctx, c, edge.from_node) {
                                Some(cond) => format!("[{}]", cond),
                                None => "[true]".to_string(),
                            },
                        ),
                        CFGEdgeType::False if source_is_decision => Some("[else]".to_string()),
                        CFGEdgeType::Uncond | CFGEdgeType::LoopBack => None,
                        other => edge_type_label(other).map(|l| format!("[{}]", l)),
                    };
                }
            }
        }
        if label.is_none() && edge.is_back_edge != 0 {
            label = Some("loop".to_string());
        }
        model.add_edge(ViewEdge::new(&from, &to, kind, &idx.to_string()).with_label(label));
    }
    unlabeled
}

// ── 10. STATE MACHINE DIAGRAM ────────────────────────────────────────────────

pub fn state_machine_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.state_machines.is_empty() {
        return ViewModel::unavailable(
            ViewKind::StateMachine,
            "No class keeps an enum-typed state field with assignments, so Phase 9 recorded no state machine.",
        );
    }
    let mut model = ViewModel::new(ViewKind::StateMachine);
    model.source("uma");
    let mut ordered: Vec<(String, &StateMachineRecord)> = ctx
        .uma
        .state_machines
        .iter()
        .map(|sm| {
            let owner = ctx
                .type_fqn(sm.class_sym_id)
                .unwrap_or_else(|| format!("<type #{}>", sm.class_sym_id));
            let field = ctx
                .simple_name(sm.state_field_sym)
                .unwrap_or_else(|| format!("#{}", sm.state_field_sym));
            (format!("{}.{}", owner, field), sm)
        })
        .filter(|(name, _)| ctx.matches_focus(name))
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    let mut unnamed_states = 0usize;
    let mut unknown_initial = Vec::new();
    for (name, sm) in ordered {
        if !budget.admit(&name, sm.states.len() + 3) {
            continue;
        }
        let base = format!("{}:{}", sm.class_sym_id, sm.state_field_sym);
        let group_key = format!("sm:{}", base);
        let group_label = ctx
            .member_label(sm.state_field_sym)
            .unwrap_or_else(|| name.clone());
        model.add_node(
            ViewNode::new(
                group_key.clone(),
                format!("sm_{}_{}", sm.class_sym_id, sm.state_field_sym),
                group_label,
                NodeKind::Group,
            )
            .with_qualified_name(Some(name.clone()))
            .with_span(ctx.symbol_span(sm.state_field_sym))
            .with_details(vec![format!("{} states, {} transitions", sm.states.len(), sm.transitions.len())])
            .with_sym(sm.class_sym_id),
        );

        let state_key = |id: u32| format!("st:{}:{}", base, id);
        for st in &sm.states {
            let label = match ctx.uma.label(st.state_name_id) {
                Some(l) => l.to_string(),
                None => {
                    unnamed_states += 1;
                    format!("state {}", st.state_id)
                }
            };
            model.add_node(
                ViewNode::new(
                    state_key(st.state_id),
                    format!("st_{}_{}_{}", sm.class_sym_id, sm.state_field_sym, st.state_id),
                    label,
                    NodeKind::State,
                )
                .with_parent(Some(group_key.clone())),
            );
        }

        if sm.initial_state != INITIAL_STATE_UNKNOWN {
            let init_key = format!("sm:{}:initial", base);
            model.add_node(
                ViewNode::new(
                    init_key.clone(),
                    format!("sminit_{}_{}", sm.class_sym_id, sm.state_field_sym),
                    "initial",
                    NodeKind::Initial,
                )
                .with_parent(Some(group_key.clone())),
            );
            model.add_edge(ViewEdge::new(
                &init_key,
                &state_key(sm.initial_state as u32),
                EdgeKind::Transition,
                "initial",
            ));
        } else {
            unknown_initial.push(name.clone());
        }

        let finals: Vec<u32> = sm
            .states
            .iter()
            .filter(|s| s.is_final != 0)
            .map(|s| s.state_id)
            .collect();
        if !finals.is_empty() {
            let final_key = format!("sm:{}:final", base);
            model.add_node(
                ViewNode::new(
                    final_key.clone(),
                    format!("smfinal_{}_{}", sm.class_sym_id, sm.state_field_sym),
                    "final",
                    NodeKind::Final,
                )
                .with_parent(Some(group_key.clone())),
            );
            for id in finals {
                model.add_edge(ViewEdge::new(&state_key(id), &final_key, EdgeKind::Transition, "final"));
            }
        }

        let all_states: Vec<u32> = sm.states.iter().map(|s| s.state_id).collect();
        for (idx, t) in sm.transitions.iter().enumerate() {
            let mut label = String::new();
            if t.trigger_method_sym != UNKNOWN_SYM {
                if let Some(trigger) = ctx.simple_name(t.trigger_method_sym) {
                    label.push_str(&format!("{}()", trigger));
                }
            }
            if let Some(guard) = ctx.uma.label(t.guard_text_id) {
                let guard = guard.trim();
                if !label.is_empty() {
                    label.push(' ');
                }
                if guard.starts_with('[') {
                    label.push_str(guard);
                } else {
                    label.push_str(&format!("[{}]", guard));
                }
            }
            if let Some(action) = ctx.uma.label(t.action_text_id) {
                label.push_str(&format!(" / {}", action.trim()));
            }
            let label = if label.trim().is_empty() {
                None
            } else {
                Some(label.trim().to_string())
            };
            let sources: Vec<u32> = if t.from_state == TRANSITION_FROM_ANY {
                all_states.clone()
            } else {
                vec![t.from_state as u32]
            };
            let span = ctx.span_of_tokens(t.source_token, t.source_token);
            for src in sources {
                model.add_edge(
                    ViewEdge::new(
                        &state_key(src),
                        &state_key(t.to_state as u32),
                        EdgeKind::Transition,
                        &idx.to_string(),
                    )
                    .with_label(label.clone())
                    .with_span(span.clone()),
                );
            }
        }
    }
    budget.finish(&mut model, "state machine(s)");
    if unnamed_states > 0 {
        model.note(format!("{} state(s) have no recorded name and are labelled by id.", unnamed_states));
    }
    if !unknown_initial.is_empty() {
        model.note(format!(
            "No initializer establishes the start state of: {}.",
            unknown_initial.join(", ")
        ));
    }
    if ctx
        .uma
        .state_machines
        .iter()
        .any(|sm| sm.transitions.iter().any(|t| t.from_state == TRANSITION_FROM_ANY))
    {
        model.note("Transitions whose source state is not constrained by a guard are drawn from every state.");
    }
    model
}

// ── 11. SEQUENCE DIAGRAM ─────────────────────────────────────────────────────

/// Title of a scenario: its entry method, else the recorded scenario name (interned text).
pub fn scenario_title(ctx: &DiagramContext, seq: &SequenceDiagramRecord) -> String {
    if seq.entry_method_sym != UNKNOWN_SYM {
        if let Some(label) = ctx.member_label(seq.entry_method_sym) {
            return format!("{}()", label);
        }
    }
    ctx.text(seq.scenario_name)
        .map(|name| format!("{}()", name))
        .unwrap_or_else(|| "<unnamed scenario>".to_string())
}

fn scenario_order(ctx: &DiagramContext) -> Vec<(String, usize)> {
    let mut order: Vec<(String, usize)> = ctx
        .uma
        .sequences
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.entry_method_sym == UNKNOWN_SYM
                || ctx.matches_focus(&function_fqn(ctx, s.entry_method_sym))
        })
        .map(|(i, s)| (scenario_title(ctx, s), i))
        .collect();
    order.sort();
    order
}

/// Lifeline node keys of a scenario, keyed by lifeline symbol.
struct Lifelines {
    by_sym: HashMap<u32, String>,
}

impl Lifelines {
    fn new(
        ctx: &DiagramContext,
        model: &mut ViewModel,
        aliases: &mut AliasTable,
        seq: &SequenceDiagramRecord,
    ) -> Self {
        let mut lifelines = Self {
            by_sym: HashMap::new(),
        };
        for ll in &seq.lifelines {
            let key = lifeline_node(ctx, model, aliases, ll.sym_id, ll.type_sym_id, ll.is_actor != 0);
            lifelines.by_sym.insert(ll.sym_id, key);
        }
        lifelines
    }

    fn key(
        &mut self,
        ctx: &DiagramContext,
        model: &mut ViewModel,
        aliases: &mut AliasTable,
        sym: u32,
    ) -> String {
        if let Some(k) = self.by_sym.get(&sym) {
            return k.clone();
        }
        let is_actor = sym == EXTERNAL_ACTOR_SYM || sym == UNKNOWN_SYM;
        let key = lifeline_node(ctx, model, aliases, sym, UNKNOWN_SYM, is_actor);
        self.by_sym.insert(sym, key.clone());
        key
    }
}

fn lifeline_node(
    ctx: &DiagramContext,
    model: &mut ViewModel,
    aliases: &mut AliasTable,
    sym: u32,
    type_sym: u32,
    is_actor: bool,
) -> String {
    if is_actor || sym == EXTERNAL_ACTOR_SYM {
        let key = "ll:actor".to_string();
        if !model.has_node(&key) {
            model.add_node(
                ViewNode::new(key.clone(), "ll_actor", "Actor", NodeKind::Actor)
                    .with_provenance(Provenance::Heuristic),
            );
        }
        return key;
    }
    let type_sym = if type_sym != UNKNOWN_SYM { type_sym } else { sym };
    let type_qn = ctx
        .qualified_name(type_sym)
        .unwrap_or_else(|| format!("<lifeline #{}>", sym));
    let type_label = ctx
        .display_name(type_sym)
        .unwrap_or_else(|| type_qn.clone());
    let (key, label) = if type_sym != sym {
        let name = ctx
            .simple_name(sym)
            .unwrap_or_else(|| format!("#{}", sym));
        (
            format!("ll:{}#{}", type_qn, sym),
            format!("{} : {}", name, type_label),
        )
    } else {
        (format!("ll:{}", type_qn), type_label)
    };
    if !model.has_node(&key) {
        let alias = aliases.alias("ll_", &label, &key);
        model.add_node(
            ViewNode::new(key.clone(), alias, label, NodeKind::Lifeline)
                .with_qualified_name(Some(type_qn))
                .with_span(ctx.symbol_span(type_sym))
                .with_sym(type_sym),
        );
    }
    key
}

fn fragment_kind(kind: u8) -> FragmentKind {
    match kind {
        FRAGMENT_LOOP => FragmentKind::Loop,
        FRAGMENT_OPT => FragmentKind::Opt,
        FRAGMENT_PAR => FragmentKind::Par,
        FRAGMENT_BREAK => FragmentKind::Break,
        _ => FragmentKind::Alt,
    }
}

/// Hierarchical UML message numbers (`1`, `1.1`, `1.2`, `2`) of the non-reply messages,
/// derived from each message's activation nesting depth.
fn message_numbers(messages: &[&MessageRecord]) -> HashMap<u16, String> {
    let mut counters: Vec<u32> = Vec::new();
    let mut numbers = HashMap::new();
    for m in messages {
        if m.message_kind == MSG_KIND_REPLY {
            continue;
        }
        let depth = m.nesting_depth as usize;
        counters.truncate(depth + 1);
        while counters.len() < depth + 1 {
            counters.push(0);
        }
        counters[depth] += 1;
        let number = counters
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(".");
        numbers.insert(m.ordinal, number);
    }
    numbers
}

fn message_label(ctx: &DiagramContext, m: &MessageRecord) -> String {
    match ctx.simple_name(m.method_sym_id) {
        Some(name) if m.message_kind == MSG_KIND_CREATE => format!("«create» {}()", name),
        Some(name) => format!("{}()", name),
        None => "<unresolved call>".to_string(),
    }
}

pub fn sequence_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.sequences.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Sequence,
            "Phase 9 identified no entry-point scenarios (no call sites reachable from an entry point).",
        );
    }
    let mut model = ViewModel::new(ViewKind::Sequence);
    model.source("uma");
    let mut aliases = AliasTable::default();
    let mut order_counter = 0u32;
    let mut without_replies = 0usize;
    for (title, idx) in scenario_order(ctx) {
        let seq = &ctx.uma.sequences[idx];
        let (interaction, has_replies) =
            build_interaction(ctx, &mut model, &mut aliases, seq, idx, &title, &mut order_counter);
        if !has_replies {
            without_replies += 1;
        }
        model.interactions.push(interaction);
    }
    if model.interactions.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Sequence,
            "No sequence scenario matches the view focus.",
        );
    }
    if without_replies > 0 {
        model.note(format!(
            "{} scenario(s) carry no reply messages, so their activations are not drawn.",
            without_replies
        ));
    }
    let truncated: Vec<String> = model
        .interactions
        .iter()
        .filter(|i| i.truncated || i.omitted_messages > 0)
        .map(|i| i.title.clone())
        .collect();
    if !truncated.is_empty() {
        model.note(format!(
            "Scenarios cut short by the depth or message bound: {}.",
            truncated.join(", ")
        ));
    }
    model
}

/// Converts one UMA scenario into an ordered interaction. Returns the interaction and
/// whether the scenario carries reply messages.
fn build_interaction(
    ctx: &DiagramContext,
    model: &mut ViewModel,
    aliases: &mut AliasTable,
    seq: &SequenceDiagramRecord,
    seq_index: usize,
    title: &str,
    order_counter: &mut u32,
) -> (Interaction, bool) {
    let mut lifelines = Lifelines::new(ctx, model, aliases, seq);
    let mut messages: Vec<&MessageRecord> = seq.messages.iter().collect();
    messages.sort_by_key(|m| m.ordinal);
    let numbers = message_numbers(&messages);
    let has_replies = messages.iter().any(|m| m.message_kind == MSG_KIND_REPLY);

    let mut starts: BTreeMap<u16, Vec<&CombinedFragment>> = BTreeMap::new();
    let mut ends: BTreeMap<u16, usize> = BTreeMap::new();
    for frag in &seq.combined_fragments {
        starts.entry(frag.start_message_ordinal).or_default().push(frag);
        *ends.entry(frag.end_message_ordinal).or_insert(0) += 1;
    }
    for list in starts.values_mut() {
        list.sort_by_key(|f| f.nesting_depth);
    }

    let mut events: Vec<InteractionEvent> = Vec::new();
    let mut active: Vec<String> = Vec::new();
    let mut open_fragments = 0usize;
    let mut rendered = 0usize;
    let mut omitted = 0usize;
    for m in &messages {
        if rendered >= ctx.options.max_messages {
            if m.message_kind != MSG_KIND_REPLY {
                omitted += 1;
            }
            continue;
        }
        if let Some(frags) = starts.get(&m.ordinal) {
            for frag in frags {
                events.push(InteractionEvent::FragmentStart {
                    kind: fragment_kind(frag.fragment_kind),
                    guard: ctx.uma.label(frag.guard_text_id).map(str::to_string),
                });
                open_fragments += 1;
            }
        }
        let from = lifelines.key(ctx, model, aliases, m.from_lifeline);
        let to = lifelines.key(ctx, model, aliases, m.to_lifeline);
        let label = message_label(ctx, m);
        if m.message_kind == MSG_KIND_REPLY {
            let deactivates = has_replies && active.last() == Some(&from);
            if deactivates {
                active.pop();
            }
            events.push(InteractionEvent::Reply {
                from,
                to,
                label,
                deactivates,
            });
        } else {
            let number = numbers
                .get(&m.ordinal)
                .cloned()
                .unwrap_or_else(|| (m.ordinal as u32 + 1).to_string());
            let is_async = m.message_kind == MSG_KIND_ASYNC;
            let is_create = m.message_kind == MSG_KIND_CREATE;
            let activates = has_replies && !is_async;
            if activates {
                active.push(to.clone());
            }
            let span = ctx.span_of_tokens(m.uml_link_token, m.uml_link_token);
            *order_counter += 1;
            model.add_edge(
                ViewEdge::new(
                    &from,
                    &to,
                    EdgeKind::Message,
                    &format!("{}:{}", seq_index, m.ordinal),
                )
                .with_label(Some(format!("{} {}", number, label)))
                .with_order(*order_counter)
                .with_span(span.clone()),
            );
            events.push(InteractionEvent::Call {
                from,
                to,
                label,
                number,
                is_async,
                is_create,
                activates,
                span,
            });
            rendered += 1;
        }
        if let Some(&count) = ends.get(&m.ordinal) {
            for _ in 0..count {
                if open_fragments > 0 {
                    events.push(InteractionEvent::FragmentEnd);
                    open_fragments -= 1;
                }
            }
        }
    }
    for _ in 0..open_fragments {
        events.push(InteractionEvent::FragmentEnd);
    }
    while let Some(lifeline) = active.pop() {
        events.push(InteractionEvent::Deactivate { lifeline });
    }

    let mut order: Vec<String> = Vec::new();
    for event in &events {
        let (a, b) = match event {
            InteractionEvent::Call { from, to, .. } | InteractionEvent::Reply { from, to, .. } => {
                (Some(from), Some(to))
            }
            _ => (None, None),
        };
        for k in [a, b].into_iter().flatten() {
            if !order.contains(k) {
                order.push(k.clone());
            }
        }
    }
    (
        Interaction {
            key: format!("sd:{}", seq_index),
            title: title.to_string(),
            lifelines: order,
            events,
            omitted_messages: omitted,
            truncated: seq.truncated != 0,
        },
        has_replies,
    )
}

// ── 12. COMMUNICATION DIAGRAM ────────────────────────────────────────────────

pub fn communication_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.sequences.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Communication,
            "No sequence scenarios were recorded, so no communication diagram can be derived.",
        );
    }
    let mut model = ViewModel::new(ViewKind::Communication);
    model.source("uma");
    let mut aliases = AliasTable::default();
    let scenarios = scenario_order(ctx);
    let multiple = scenarios.len() > 1;

    for (scenario_no, (title, idx)) in scenarios.iter().enumerate() {
        let seq = &ctx.uma.sequences[*idx];
        let mut lifelines = Lifelines::new(ctx, &mut model, &mut aliases, seq);
        let mut messages: Vec<&MessageRecord> = seq.messages.iter().collect();
        messages.sort_by_key(|m| m.ordinal);
        let numbers = message_numbers(&messages);
        let prefix = if multiple {
            format!("S{} ", scenario_no + 1)
        } else {
            String::new()
        };
        let line_of = |m: &MessageRecord| -> Option<String> {
            numbers
                .get(&m.ordinal)
                .map(|n| format!("{}{}: {}", prefix, n, message_label(ctx, m)))
        };

        let mut links: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        match ctx
            .uma
            .communications
            .iter()
            .find(|c| c.sequence_index as usize == *idx)
        {
            Some(comm) => {
                let by_ordinal: HashMap<u16, &MessageRecord> =
                    messages.iter().map(|m| (m.ordinal, *m)).collect();
                for link in &comm.links {
                    let from = lifelines.key(ctx, &mut model, &mut aliases, link.from_lifeline);
                    let to = lifelines.key(ctx, &mut model, &mut aliases, link.to_lifeline);
                    let lines: Vec<String> = link
                        .message_ordinals
                        .iter()
                        .filter_map(|o| by_ordinal.get(o).and_then(|m| line_of(*m)))
                        .collect();
                    links.entry((from, to)).or_default().extend(lines);
                }
            }
            None => {
                for m in &messages {
                    if m.message_kind == MSG_KIND_REPLY {
                        continue;
                    }
                    let from = lifelines.key(ctx, &mut model, &mut aliases, m.from_lifeline);
                    let to = lifelines.key(ctx, &mut model, &mut aliases, m.to_lifeline);
                    if let Some(line) = line_of(*m) {
                        links.entry((from, to)).or_default().push(line);
                    }
                }
            }
        }

        for ((from, to), lines) in links {
            if lines.is_empty() {
                continue;
            }
            let mut shown: Vec<String> = lines.iter().take(MAX_LINK_LINES).cloned().collect();
            if lines.len() > MAX_LINK_LINES {
                shown.push(format!("… +{} more", lines.len() - MAX_LINK_LINES));
            }
            model.add_edge(
                ViewEdge::new(&from, &to, EdgeKind::Link, &idx.to_string())
                    .with_label(Some(shown.join("\n"))),
            );
        }
        if multiple {
            model.note(format!("S{} = {}", scenario_no + 1, title));
        }
    }
    model.note("Message numbers follow the activation nesting of the sequence scenarios (1, 1.1, 1.2, 2, ...).");
    model
}

// ── 13. INTERACTION OVERVIEW DIAGRAM ─────────────────────────────────────────

pub fn interaction_overview_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.interaction_overviews.is_empty() {
        return ViewModel::unavailable(
            ViewKind::InteractionOverview,
            "Phase 9 recorded no interaction overviews (no entry method whose control flow references sequence scenarios).",
        );
    }
    let mut model = ViewModel::new(ViewKind::InteractionOverview);
    model.source("uma");
    let mut ordered: Vec<(String, &InteractionOverviewRecord)> = ctx
        .uma
        .interaction_overviews
        .iter()
        .map(|o| (function_fqn(ctx, o.entry_method_sym), o))
        .filter(|(fqn, _)| ctx.matches_focus(fqn))
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    let mut missing = 0usize;
    for (fqn, overview) in ordered {
        let act = match ctx.uma.activities.get(overview.activity_index as usize) {
            Some(a) => a,
            None => {
                missing += 1;
                continue;
            }
        };
        if !budget.admit(&fqn, act.nodes.len() + 1) {
            continue;
        }
        let mut refs: HashMap<u32, Vec<String>> = HashMap::new();
        for r in &overview.references {
            let title = ctx
                .uma
                .sequences
                .get(r.sequence_index as usize)
                .map(|s| scenario_title(ctx, s))
                .unwrap_or_else(|| format!("<scenario #{}>", r.sequence_index));
            let list = refs.entry(r.activity_node).or_default();
            if !list.contains(&title) {
                list.push(title);
            }
        }
        let labels: HashMap<u32, String> = refs
            .into_iter()
            .map(|(node, titles)| (node, format!("ref: {}", titles.join(", "))))
            .collect();
        add_activity(ctx, &mut model, act, Some(&labels));
    }
    budget.finish(&mut model, "interaction overview(s)");
    if missing > 0 {
        model.note(format!(
            "{} interaction overview(s) reference an activity that is not recorded.",
            missing
        ));
    }
    model
}

// ── 14. TIMING DIAGRAM ───────────────────────────────────────────────────────

pub fn timing_view(_ctx: &DiagramContext) -> ViewModel {
    ViewModel::unavailable(
        ViewKind::Timing,
        "Timing constraints and durations cannot be derived by static analysis; no execution traces are part of the analyzed artifacts.",
    )
}
