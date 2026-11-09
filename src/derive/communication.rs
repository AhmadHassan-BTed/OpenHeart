//! Communication diagrams: the interactions of the sequence walk drawn as messages over
//! links between participants.
//!
//! Messages carry the walk's decimal sequence numbers (a nested number is a call made
//! during the activation of its prefix), the UML recurrence `*` when they are sent inside
//! a loop, and the guard of the innermost conditional operand. Replies are folded into
//! their calls as `result := call(...)`. A link joins two participants that exchange at
//! least one message.

use std::collections::{BTreeMap, HashMap};

use crate::derive::model::{Derivation, DiagramEdge, DiagramKind, DiagramModel, EdgeKind, IdBuilder, MessageSort};
use crate::derive::provenance::Fact;
use crate::derive::sequence::{lifeline_element_id, lifeline_node, message_element_id, InteractionTrace, Traces};
use crate::derive::DeriveContext;
use crate::ir::types::Span;

/// The communication diagram of one interaction.
pub fn model_of(ctx: &DeriveContext, trace: &InteractionTrace) -> DiagramModel {
    let mut model = DiagramModel::new(DiagramKind::Communication, format!("comm {}", trace.title))
        .with_scope(trace.title.clone());
    for i in 0..trace.lifelines.len() {
        model.add_node(lifeline_node(trace, i));
    }
    // Reply labels by the number of the call they answer.
    let mut replies: HashMap<&str, &str> = HashMap::new();
    for m in &trace.messages {
        if m.sort == MessageSort::Reply && !m.label.is_empty() {
            replies.entry(m.number.as_str()).or_insert(m.label.as_str());
        }
    }
    // Links: one per unordered pair of participants.
    let mut links: BTreeMap<(usize, usize), (Vec<Span>, Vec<Fact>, Derivation)> = BTreeMap::new();
    for m in &trace.messages {
        if m.sort == MessageSort::Reply {
            continue;
        }
        let pair = (m.from.min(m.to), m.from.max(m.to));
        let e = links
            .entry(pair)
            .or_insert_with(|| (Vec::new(), Vec::new(), Derivation::Exact));
        e.0.extend(m.spans.iter().copied());
        e.1.extend(m.facts.iter().copied());
        e.2.weaken(&m.derivation);
    }
    let link_id = |a: usize, b: usize| {
        IdBuilder::new("comm_link")
            .text(&trace.key)
            .text(&trace.lifelines[a].key)
            .text(&trace.lifelines[b].key)
            .finish()
    };
    for ((a, b), (spans, facts, derivation)) in &links {
        model.add_edge(
            DiagramEdge::new(
                link_id(*a, *b),
                EdgeKind::Link,
                lifeline_element_id(trace, *a),
                lifeline_element_id(trace, *b),
            )
            .with_spans(spans.iter().copied())
            .with_facts(facts.iter().copied())
            .with_derivation(derivation.clone()),
        );
    }
    for m in &trace.messages {
        if m.sort == MessageSort::Reply {
            continue;
        }
        let label = match replies.get(m.number.as_str()) {
            Some(result) if m.sort == MessageSort::SynchCall => format!("{} := {}", result, m.label),
            _ => m.label.clone(),
        };
        let link = link_id(m.from.min(m.to), m.from.max(m.to));
        let mut edge = DiagramEdge::new(
            message_element_id(trace, m),
            EdgeKind::Message,
            lifeline_element_id(trace, m.from),
            lifeline_element_id(trace, m.to),
        )
        .with_label(label)
        .with_order(m.order)
        .with_sequence_number(m.number.clone())
        .with_message_sort(m.sort)
        .with_guard(m.guard.clone())
        .with_property("link", link.to_string())
        .with_spans(m.spans.iter().copied())
        .with_facts(m.facts.iter().copied())
        .with_derivation(m.derivation.clone());
        if m.in_loop {
            edge = edge.with_property("recurrence", "*");
        }
        model.add_edge(edge);
    }
    for n in &trace.notes {
        model.note(n.clone());
    }
    model.weaken(&trace.derivation);
    model.finalize(ctx.limits.max_spans);
    model
}

/// One communication diagram per entry point.
pub fn derive_from_traces(ctx: &DeriveContext, traces: &Traces) -> Vec<DiagramModel> {
    if traces.traces.is_empty() {
        return vec![DiagramModel::not_derivable(
            DiagramKind::Communication,
            DiagramKind::Communication.title(),
            "no entry point has a lowered body: interactions start at entry points",
        )];
    }
    let mut out: Vec<DiagramModel> = traces.traces.iter().map(|t| model_of(ctx, t)).collect();
    if traces.omitted > 0 {
        out.push(DiagramModel::omission(
            DiagramKind::Communication,
            traces.omitted,
            ctx.limits.max_diagrams,
        ));
    }
    out
}

pub fn derive(ctx: &DeriveContext) -> Vec<DiagramModel> {
    derive_from_traces(ctx, &crate::derive::sequence::trace_entries(ctx))
}
