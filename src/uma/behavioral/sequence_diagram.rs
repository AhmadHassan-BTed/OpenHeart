//! SequenceDiagramExtractor — one SequenceDiagramRecord per scenario (§9.2.4).
//!
//! Scenarios are built by ordered, depth-bounded inlining of call sites
//! (Rountev, Volgin & Reddoch, PASTE 2005):
//! - call sites of a member are visited in evaluation order (a call is sent when
//!   its argument list completes, so nested argument calls precede it);
//! - each callee is expanded in place, right after its call message, so
//!   activations nest; a reply message closes every synchronous activation;
//! - recursion (a callee already on the activation stack), the depth bound and
//!   the message bound stop expansion — the latter two mark the scenario
//!   `truncated`;
//! - only validated call edges (callee name matches the invoked identifier) are
//!   followed, so unresolved library calls never become messages;
//! - calls inside loops, `if` branches and `switch` cases of the caller's body
//!   are wrapped in loop / opt / alt combined fragments whose guards are the
//!   source conditions;
//! - calls made inside the argument list of an asynchronous library entry point
//!   (`submit`, `execute`, `runAsync`, `supplyAsync`, ...) are asynchronous
//!   messages; constructor calls are create messages.
//!
//! Lifelines are named by the class that declares the receiving operation.

use std::collections::{BTreeMap, HashMap};

use crate::core::types::cg::CG_EDGE_CONSTRUCTOR;
use crate::core::types::symbol::SymbolKind;
use crate::uma::actor_identification::{ActorIdentifier, EntryPoint, EXTERNAL_ACTOR_ID};
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::{truncate_chars, LabelExtractor, LabelTable};
use crate::uma::source::BodyFacts;
use crate::uma::types::*;

/// Library operations whose functional argument runs asynchronously
/// (JDK executors and CompletableFuture, Kotlin coroutines, Android handlers).
const ASYNC_ENTRY_OPERATIONS: &[&[u8]] = &[
    b"submit",
    b"execute",
    b"invokeAll",
    b"invokeAny",
    b"schedule",
    b"scheduleAtFixedRate",
    b"scheduleWithFixedDelay",
    b"runAsync",
    b"supplyAsync",
    b"thenApplyAsync",
    b"thenAcceptAsync",
    b"thenRunAsync",
    b"thenComposeAsync",
    b"launch",
    b"async",
    b"post",
    b"postDelayed",
];

/// Bounds for scenario construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceOptions {
    /// Maximum activation nesting depth below the entry method.
    pub max_depth: u16,
    /// Maximum messages per scenario (≤ u16::MAX).
    pub max_messages: u16,
    /// Maximum dispatch targets drawn for one polymorphic call site.
    pub max_targets_per_site: usize,
    /// Maximum number of scenarios extracted.
    pub max_scenarios: usize,
}

impl Default for SequenceOptions {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_messages: 2000,
            max_targets_per_site: 4,
            max_scenarios: 1000,
        }
    }
}

/// A region of the caller's body that maps to a combined fragment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RegionKey {
    start_token: u32,
    kind: u8,
    operand: u8,
}

struct RegionSpan {
    guard: u32,
    first: u16,
    last: u16,
    depth: u16,
}

struct Scenario<'s> {
    lifelines: Vec<LifelineRecord>,
    lifeline_pos: HashMap<u32, usize>,
    messages: Vec<MessageRecord>,
    fragments: Vec<CombinedFragment>,
    stack: Vec<u32>,
    truncated: bool,
    options: SequenceOptions,
    call_cache: &'s mut HashMap<u32, Vec<(u32, Vec<u32>)>>,
}

pub struct SequenceDiagramExtractor;

impl SequenceDiagramExtractor {
    pub fn extract_all(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        include_tests: bool,
        options: SequenceOptions,
    ) -> Vec<SequenceDiagramRecord> {
        let entries = ActorIdentifier::find_entry_points(index, include_tests);
        if entries.len() > options.max_scenarios {
            crate::core::logger::log_warn(&format!(
                "[UMA] {} scenario entry points found; extracting the first {} (by kind, then symbol id).",
                entries.len(),
                options.max_scenarios
            ));
        }
        let mut call_cache: HashMap<u32, Vec<(u32, Vec<u32>)>> = HashMap::new();
        let mut sequences = Vec::new();
        for entry in entries.into_iter().take(options.max_scenarios) {
            if let Some(seq) =
                Self::extract_for_entry(entry, index, bodies, labels, options, &mut call_cache)
            {
                sequences.push(seq);
            }
        }
        sequences
    }

    fn extract_for_entry(
        entry: EntryPoint,
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        options: SequenceOptions,
        call_cache: &mut HashMap<u32, Vec<(u32, Vec<u32>)>>,
    ) -> Option<SequenceDiagramRecord> {
        let entry_rec = index.symbol(entry.method)?;
        let entry_class = index.enclosing_type(entry.method);
        if entry_class == UNKNOWN_SYM {
            return None;
        }

        let mut sc = Scenario {
            lifelines: Vec::new(),
            lifeline_pos: HashMap::new(),
            messages: Vec::new(),
            fragments: Vec::new(),
            stack: vec![entry.method],
            truncated: false,
            options,
            call_cache,
        };
        sc.lifelines.push(LifelineRecord {
            sym_id: EXTERNAL_ACTOR_ID,
            name_id: u32::MAX,
            type_sym_id: EXTERNAL_ACTOR_ID,
            is_actor: 1,
            _pad: [0; 3],
        });
        sc.lifeline_pos.insert(EXTERNAL_ACTOR_ID, 0);
        sc.lifeline(index, entry_class);

        sc.push_message(MessageRecord {
            from_lifeline: EXTERNAL_ACTOR_ID,
            to_lifeline: entry_class,
            call_site_id: u32::MAX,
            method_sym_id: entry.method,
            message_kind: MSG_KIND_FOUND,
            call_edge_type: 0,
            ordinal: 0,
            nesting_depth: 0,
            uml_link_token: entry_rec.first_token_id,
        });
        Self::expand(&mut sc, entry.method, entry_class, 1, index, bodies, labels);
        sc.push_message(MessageRecord {
            from_lifeline: entry_class,
            to_lifeline: EXTERNAL_ACTOR_ID,
            call_site_id: u32::MAX,
            method_sym_id: entry.method,
            message_kind: MSG_KIND_REPLY,
            call_edge_type: 0,
            ordinal: 0,
            nesting_depth: 0,
            uml_link_token: entry_rec.last_token_id,
        });

        let Scenario {
            lifelines,
            messages,
            mut fragments,
            truncated,
            ..
        } = sc;
        fragments.sort_by_key(|f| (f.start_message_ordinal, f.nesting_depth));
        Some(SequenceDiagramRecord {
            scenario_name: entry_rec.name_id,
            entry_method_sym: entry.method,
            lifeline_count: u16::try_from(lifelines.len()).unwrap_or(u16::MAX),
            message_count: u16::try_from(messages.len()).unwrap_or(u16::MAX),
            fragment_count: u16::try_from(fragments.len()).unwrap_or(u16::MAX),
            truncated: u8::from(truncated),
            entry_kind: entry.kind,
            lifelines,
            messages,
            combined_fragments: fragments,
        })
    }

    /// Inline the calls of `member` (running on lifeline `self_lifeline`).
    fn expand(
        sc: &mut Scenario<'_>,
        member: u32,
        self_lifeline: u32,
        depth: u16,
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
    ) {
        let calls = sc.calls_of(index, member);
        let body = bodies.get(member);
        let async_regions: Vec<(u32, u32)> = body
            .map(|b| {
                b.invocations
                    .iter()
                    .filter(|inv| {
                        ASYNC_ENTRY_OPERATIONS.contains(&index.scanner.text(inv.name_token))
                    })
                    .map(|inv| (inv.name_token, inv.close_token))
                    .collect()
            })
            .unwrap_or_default();
        let mut regions: BTreeMap<RegionKey, RegionSpan> = BTreeMap::new();

        for (site_index, callees) in calls {
            if sc.is_full() {
                sc.truncated = true;
                break;
            }
            let site = index.site(site_index);
            let first = sc.next_ordinal();
            let targets: Vec<u32> = callees
                .iter()
                .copied()
                .take(sc.options.max_targets_per_site)
                .collect();
            if callees.len() > targets.len() {
                sc.truncated = true;
            }
            let in_async = async_regions
                .iter()
                .any(|&(a, b)| a < site.call_token && site.call_token <= b);

            for &callee in &targets {
                if sc.is_full() {
                    sc.truncated = true;
                    break;
                }
                let target_class = match index.kind(callee) {
                    Some(k) if UMAIndex::is_type_kind(k) => callee,
                    _ => index.enclosing_type(callee),
                };
                if target_class == UNKNOWN_SYM {
                    continue;
                }
                sc.lifeline(index, target_class);
                let is_create = site.call_type == CG_EDGE_CONSTRUCTOR
                    || index.kind(callee) == Some(SymbolKind::SK_CONSTRUCTOR);
                let kind = if in_async {
                    MSG_KIND_ASYNC
                } else if is_create {
                    MSG_KIND_CREATE
                } else {
                    MSG_KIND_SYNC
                };
                sc.push_message(MessageRecord {
                    from_lifeline: self_lifeline,
                    to_lifeline: target_class,
                    call_site_id: site.call_site_id,
                    method_sym_id: callee,
                    message_kind: kind,
                    call_edge_type: site.call_type,
                    ordinal: 0,
                    nesting_depth: depth,
                    uml_link_token: site.call_token,
                });

                let recursive = sc.stack.contains(&callee);
                if !recursive && depth < sc.options.max_depth {
                    sc.stack.push(callee);
                    Self::expand(sc, callee, target_class, depth + 1, index, bodies, labels);
                    sc.stack.pop();
                } else if !recursive && !sc.calls_of(index, callee).is_empty() {
                    // Depth bound reached on a callee that would make further calls.
                    sc.truncated = true;
                }

                if kind == MSG_KIND_SYNC {
                    sc.push_message(MessageRecord {
                        from_lifeline: target_class,
                        to_lifeline: self_lifeline,
                        call_site_id: site.call_site_id,
                        method_sym_id: callee,
                        message_kind: MSG_KIND_REPLY,
                        call_edge_type: site.call_type,
                        ordinal: 0,
                        nesting_depth: depth,
                        uml_link_token: site.call_token,
                    });
                }
            }

            let last = sc.next_ordinal().saturating_sub(1);
            if sc.next_ordinal() > first {
                if let Some(b) = body {
                    Self::record_regions(
                        &mut regions,
                        b,
                        site.call_token,
                        first,
                        last,
                        depth,
                        index,
                        labels,
                    );
                }
                if targets.len() > 1 {
                    // Polymorphic dispatch: one operand per possible receiver class.
                    let guard = labels.intern("[dynamic dispatch]");
                    sc.fragments.push(CombinedFragment {
                        fragment_kind: FRAGMENT_ALT,
                        guard_text_id: guard,
                        start_message_ordinal: first,
                        end_message_ordinal: last,
                        nesting_depth: depth,
                    });
                }
            }
        }

        for (key, span) in regions {
            sc.fragments.push(CombinedFragment {
                fragment_kind: key.kind,
                guard_text_id: span.guard,
                start_message_ordinal: span.first,
                end_message_ordinal: span.last,
                nesting_depth: span.depth,
            });
        }
    }

    /// Widen the fragment regions of `body` that contain the call token.
    #[allow(clippy::too_many_arguments)]
    fn record_regions(
        regions: &mut BTreeMap<RegionKey, RegionSpan>,
        body: &BodyFacts,
        token: u32,
        first: u16,
        last: u16,
        depth: u16,
        index: &UMAIndex<'_>,
        labels: &mut LabelTable,
    ) {
        let mut hits: Vec<(RegionKey, String)> = Vec::new();
        for &(a, b) in &body.loops {
            if a <= token && token <= b {
                let guard = Self::loop_guard(a, b, index);
                hits.push((
                    RegionKey {
                        start_token: a,
                        kind: FRAGMENT_LOOP,
                        operand: 0,
                    },
                    guard,
                ));
            }
        }
        for r in &body.ifs {
            let cond = LabelExtractor::token_span_label(r.cond.0, r.cond.1, index.tca, 48)
                .unwrap_or_default();
            if r.then_branch.0 <= token && token <= r.then_branch.1 {
                let kind = if r.else_branch.is_some() {
                    FRAGMENT_ALT
                } else {
                    FRAGMENT_OPT
                };
                hits.push((
                    RegionKey {
                        start_token: r.if_token,
                        kind,
                        operand: 0,
                    },
                    format!("[{}]", cond),
                ));
            } else if let Some((a, b)) = r.else_branch {
                if a <= token && token <= b {
                    hits.push((
                        RegionKey {
                            start_token: r.if_token,
                            kind: FRAGMENT_ALT,
                            operand: 1,
                        },
                        "[else]".to_string(),
                    ));
                }
            }
        }
        for c in &body.cases {
            if c.body.0 <= token && token <= c.body.1 {
                let label = if c.is_default && c.labels.is_empty() {
                    "[default]".to_string()
                } else {
                    let names: Vec<String> = c
                        .labels
                        .iter()
                        .map(|&l| {
                            String::from_utf8_lossy(index.tca.interner.lookup_text(l)).into_owned()
                        })
                        .collect();
                    format!("[case {}]", names.join(", "))
                };
                hits.push((
                    RegionKey {
                        start_token: c.case_token,
                        kind: FRAGMENT_ALT,
                        operand: 0,
                    },
                    label,
                ));
            }
        }
        for (key, guard_text) in hits {
            match regions.get_mut(&key) {
                Some(span) => {
                    span.first = span.first.min(first);
                    span.last = span.last.max(last);
                }
                None => {
                    let guard = labels.intern(&truncate_chars(&guard_text, 56));
                    regions.insert(
                        key,
                        RegionSpan {
                            guard,
                            first,
                            last,
                            depth,
                        },
                    );
                }
            }
        }
    }

    /// Guard of a loop fragment: the loop header's condition text.
    fn loop_guard(start: u32, end: u32, index: &UMAIndex<'_>) -> String {
        let scanner = &index.scanner;
        let file = match scanner.file_of(start) {
            Some(f) => f,
            None => return "[loop]".to_string(),
        };
        let open = match scanner.next_code(start, file) {
            Some(t) if scanner.is(t, b"(") => t,
            _ => return "[loop]".to_string(),
        };
        let mut depth = 0i32;
        let mut t = open;
        while t <= end {
            match scanner.text(t) {
                b"(" => depth += 1,
                b")" => {
                    depth -= 1;
                    if depth == 0 {
                        let inner_first = scanner.next_code(open, file).unwrap_or(t);
                        let inner_last = scanner.prev_code(t, file).unwrap_or(open);
                        return match LabelExtractor::token_span_label(
                            inner_first,
                            inner_last,
                            index.tca,
                            48,
                        ) {
                            Some(text) if inner_first < t => format!("[{}]", text),
                            _ => "[loop]".to_string(),
                        };
                    }
                }
                _ => {}
            }
            t = match scanner.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
        }
        "[loop]".to_string()
    }
}

impl<'s> Scenario<'s> {
    fn lifeline(&mut self, index: &UMAIndex<'_>, class_sym: u32) {
        if self.lifeline_pos.contains_key(&class_sym) {
            return;
        }
        self.lifeline_pos.insert(class_sym, self.lifelines.len());
        self.lifelines.push(LifelineRecord {
            sym_id: class_sym,
            name_id: index.name_id(class_sym),
            type_sym_id: class_sym,
            is_actor: 0,
            _pad: [0; 3],
        });
    }

    fn next_ordinal(&self) -> u16 {
        u16::try_from(self.messages.len()).unwrap_or(u16::MAX)
    }

    fn is_full(&self) -> bool {
        // Keep one slot for the closing reply to the actor.
        self.messages.len() + 1 >= self.options.max_messages as usize
    }

    fn push_message(&mut self, mut msg: MessageRecord) {
        if self.messages.len() >= self.options.max_messages as usize {
            self.truncated = true;
            return;
        }
        msg.ordinal = self.next_ordinal();
        self.messages.push(msg);
    }

    /// Validated calls of a member grouped by call site, in evaluation order.
    fn calls_of(&mut self, index: &UMAIndex<'_>, member: u32) -> Vec<(u32, Vec<u32>)> {
        if let Some(cached) = self.call_cache.get(&member) {
            return cached.clone();
        }
        let mut grouped: Vec<(u32, Vec<u32>)> = Vec::new();
        for c in index.calls_of_member(member) {
            match grouped.last_mut() {
                Some((site, callees)) if *site == c.site_index => {
                    if !callees.contains(&c.callee) {
                        callees.push(c.callee);
                    }
                }
                _ => grouped.push((c.site_index, vec![c.callee])),
            }
        }
        self.call_cache.insert(member, grouped.clone());
        grouped
    }
}
