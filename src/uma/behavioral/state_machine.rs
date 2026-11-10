//! StateMachineExtractor — protocol state machines from enum-typed state fields
//! (§9.2.3).
//!
//! A class gets a state machine when it holds a non-static attribute typed by an
//! enumeration declared in the sources and at least one of its operations
//! assigns an enumeration constant to that attribute (Sen & Mall, SoSyM 2016):
//! - states are the constants the class assigns (plus the declared constants
//!   when STA models them);
//! - the initial state is the attribute's initializer constant, or a constant
//!   assigned unconditionally in a constructor; otherwise it is reported as
//!   `INITIAL_STATE_UNKNOWN`;
//! - each assignment in an operation is a transition triggered by that
//!   operation; its source states come from an enclosing guard comparing the
//!   attribute with a constant (`if (state == A)`, `switch (state) case A:`),
//!   and it is `TRANSITION_FROM_ANY` when no guard constrains the source;
//! - final states are not reported: termination is not derivable statically.
//! Classes without such an attribute get no state machine.

use std::collections::BTreeSet;

use crate::core::types::symbol::SymbolKind;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::{truncate_chars, LabelExtractor, LabelTable};
use crate::uma::source::{AssignedValue, BodyFacts};
use crate::uma::types::*;

pub struct StateMachineExtractor;

struct Transition {
    from: Option<u32>,
    to: u32,
    trigger: u32,
    guard: Option<String>,
    token: u32,
}

impl StateMachineExtractor {
    pub fn extract_all(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        class_syms: &[u32],
    ) -> Vec<StateMachineRecord> {
        let mut machines = Vec::new();
        for &c in class_syms {
            for f in index.fields(c) {
                if index.is_static(f) {
                    continue;
                }
                let enum_type = index.type_of(f);
                if index.kind(enum_type) != Some(SymbolKind::SK_ENUM) {
                    continue;
                }
                if let Some(m) = Self::extract_for_field(index, bodies, labels, c, f, enum_type) {
                    machines.push(m);
                }
            }
        }
        machines
    }

    fn extract_for_field(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        class_sym: u32,
        field: u32,
        enum_type: u32,
    ) -> Option<StateMachineRecord> {
        let field_name = index.name_id(field);
        let declared: Vec<u32> = index
            .children_of_kind(enum_type, SymbolKind::SK_ENUM_CONSTANT)
            .map(|k| index.name_id(k))
            .collect();
        let is_state_name = |text: u32| declared.is_empty() || declared.contains(&text);

        let mut initial: Option<u32> = None;
        if let Some(sym) = index.symbol(field) {
            if let Some((a, b)) = index.scanner.initializer_range(sym) {
                if let Some(text) = Self::trailing_constant(index, a, b) {
                    if is_state_name(text) {
                        initial = Some(text);
                    }
                }
            }
        }

        let mut transitions: Vec<Transition> = Vec::new();
        for &member in index.children(class_sym) {
            let kind = index.kind(member);
            let is_init = matches!(
                kind,
                Some(SymbolKind::SK_CONSTRUCTOR) | Some(SymbolKind::SK_INSTANCE_INIT)
            );
            if !is_init && kind != Some(SymbolKind::SK_METHOD) {
                continue;
            }
            let body = match bodies.get(member) {
                Some(b) => b,
                None => continue,
            };
            for asg in &body.assignments {
                if asg.lhs_text != field_name || asg.is_declaration || asg.via_other_receiver {
                    continue;
                }
                let to = match asg.value {
                    AssignedValue::Name { text } if is_state_name(text) && text != field_name => {
                        text
                    }
                    _ => continue,
                };
                let guards = Self::source_states(index, body, asg.lhs_token, field_name);
                if is_init {
                    if guards.is_empty() && initial.is_none() {
                        initial = Some(to);
                    }
                    continue;
                }
                if guards.is_empty() {
                    transitions.push(Transition {
                        from: None,
                        to,
                        trigger: member,
                        guard: None,
                        token: asg.lhs_token,
                    });
                } else {
                    for (from, guard) in guards {
                        transitions.push(Transition {
                            from: Some(from),
                            to,
                            trigger: member,
                            guard: Some(guard),
                            token: asg.lhs_token,
                        });
                    }
                }
            }
        }
        if transitions.is_empty() {
            return None;
        }

        // States in first-mention order: initial, declared, then assigned/guarded.
        let mut order: Vec<u32> = Vec::new();
        let push = |s: u32, order: &mut Vec<u32>| {
            if !order.contains(&s) {
                order.push(s);
            }
        };
        if let Some(i) = initial {
            push(i, &mut order);
        }
        for &d in &declared {
            push(d, &mut order);
        }
        for t in &transitions {
            if let Some(f) = t.from {
                push(f, &mut order);
            }
            push(t.to, &mut order);
        }
        if order.len() < 2 {
            return None;
        }
        if order.len() > u16::MAX as usize - 1 {
            return None;
        }

        let states: Vec<StateRecord> = order
            .iter()
            .enumerate()
            .map(|(i, &text)| {
                let name =
                    String::from_utf8_lossy(index.tca.interner.lookup_text(text)).into_owned();
                StateRecord {
                    state_id: i as u32,
                    state_name_id: labels.intern(&name),
                    is_initial: u8::from(Some(text) == initial),
                    is_final: 0,
                    _pad: 0,
                }
            })
            .collect();
        let pos = |text: u32| order.iter().position(|&s| s == text).map(|p| p as u16);

        let mut seen: BTreeSet<(u16, u16, u32)> = BTreeSet::new();
        let mut records = Vec::new();
        for t in transitions {
            let to = match pos(t.to) {
                Some(p) => p,
                None => continue,
            };
            let from = match t.from {
                Some(f) => match pos(f) {
                    Some(p) => p,
                    None => continue,
                },
                None => TRANSITION_FROM_ANY,
            };
            if !seen.insert((from, to, t.trigger)) {
                continue;
            }
            let guard_text_id = match &t.guard {
                Some(g) => labels.intern(&format!("[{}]", truncate_chars(g, 48))),
                None => NO_LABEL,
            };
            records.push(TransitionRecord {
                from_state: from,
                to_state: to,
                trigger_method_sym: t.trigger,
                guard_text_id,
                action_text_id: NO_LABEL,
                source_token: t.token,
            });
        }

        let initial_state = initial
            .and_then(|i| pos(i))
            .unwrap_or(INITIAL_STATE_UNKNOWN);
        Some(StateMachineRecord {
            class_sym_id: class_sym,
            state_field_sym: field,
            state_count: u16::try_from(states.len()).unwrap_or(u16::MAX),
            transition_count: u16::try_from(records.len()).unwrap_or(u16::MAX),
            initial_state,
            final_state_count: 0,
            _reserved: 0,
            states,
            transitions: records,
        })
    }

    /// Last identifier of a pure (qualified) name in `[a, b]`, e.g. `State.IDLE`.
    fn trailing_constant(index: &UMAIndex<'_>, a: u32, b: u32) -> Option<u32> {
        let scanner = &index.scanner;
        let file = scanner.file_of(a)?;
        let mut last = None;
        let mut t = a;
        while t <= b {
            if scanner.is_ident(t) {
                last = Some(scanner.text_id(t));
            } else if !scanner.is(t, b".") {
                return None;
            }
            t = match scanner.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
        }
        last
    }

    /// Source states implied by guards enclosing `tok`, with the guard text.
    fn source_states(
        index: &UMAIndex<'_>,
        body: &BodyFacts,
        tok: u32,
        field_name: u32,
    ) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        // Innermost guard wins: scan cases and ifs, keep the tightest region.
        let mut best_width = u32::MAX;
        for c in &body.cases {
            let width = c.body.1.saturating_sub(c.body.0);
            if c.body.0 <= tok && tok <= c.body.1 && width < best_width {
                if Self::span_mentions(index, c.selector.0, c.selector.1, field_name)
                    && !c.labels.is_empty()
                {
                    best_width = width;
                    out.clear();
                    for &l in &c.labels {
                        let name =
                            String::from_utf8_lossy(index.tca.interner.lookup_text(l)).into_owned();
                        out.push((l, format!("case {}", name)));
                    }
                }
            }
        }
        for r in &body.ifs {
            if !(r.then_branch.0 <= tok && tok <= r.then_branch.1) {
                continue;
            }
            let width = r.then_branch.1.saturating_sub(r.then_branch.0);
            if width >= best_width {
                continue;
            }
            let compared = Self::compared_constants(index, r.cond.0, r.cond.1, field_name);
            if compared.is_empty() {
                continue;
            }
            best_width = width;
            out.clear();
            let text = LabelExtractor::token_span_label(r.cond.0, r.cond.1, index.tca, 48)
                .unwrap_or_default();
            for k in compared {
                out.push((k, text.clone()));
            }
        }
        out
    }

    fn span_mentions(index: &UMAIndex<'_>, a: u32, b: u32, text: u32) -> bool {
        let scanner = &index.scanner;
        let file = match scanner.file_of(a) {
            Some(f) => f,
            None => return false,
        };
        let mut t = a;
        while t <= b {
            if scanner.text_id(t) == text {
                return true;
            }
            t = match scanner.next_code(t, file) {
                Some(n) => n,
                None => return false,
            };
        }
        false
    }

    /// Constants compared for equality with the field in a condition:
    /// atoms `field == K` / `K == field` (also through `this.` and qualified
    /// constants), joined by `||` or `&&`.
    fn compared_constants(index: &UMAIndex<'_>, a: u32, b: u32, field_name: u32) -> Vec<u32> {
        let scanner = &index.scanner;
        let file = match scanner.file_of(a) {
            Some(f) => f,
            None => return Vec::new(),
        };
        let mut atoms: Vec<Vec<u32>> = vec![Vec::new()];
        let mut t = a;
        while t <= b {
            let txt = scanner.text(t);
            if txt == b"&&" || txt == b"||" {
                atoms.push(Vec::new());
            } else if txt != b"(" && txt != b")" {
                if let Some(cur) = atoms.last_mut() {
                    cur.push(t);
                }
            }
            t = match scanner.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
        }
        let mut out = Vec::new();
        for atom in atoms {
            let eq = match atom.iter().position(|&tk| scanner.is(tk, b"==")) {
                Some(p) => p,
                None => continue,
            };
            let (lhs, rhs) = (&atom[..eq], &atom[eq + 1..]);
            let last_ident = |side: &[u32]| -> Option<u32> {
                if side.is_empty()
                    || !side.iter().all(|&tk| {
                        scanner.is_ident(tk) || scanner.is(tk, b".") || scanner.is(tk, b"this")
                    })
                {
                    return None;
                }
                side.iter()
                    .rev()
                    .find(|&&tk| scanner.is_ident(tk))
                    .map(|&tk| scanner.text_id(tk))
            };
            let (l, r) = (last_ident(lhs), last_ident(rhs));
            match (l, r) {
                (Some(x), Some(k)) if x == field_name && k != field_name => out.push(k),
                (Some(k), Some(x)) if x == field_name && k != field_name => out.push(k),
                _ => {}
            }
        }
        out
    }
}
