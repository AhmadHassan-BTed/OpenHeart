//! Interpreter pattern rule (§9.2.5).
//!
//! An abstract expression declares an interpret operation taking a context;
//! terminal expressions implement it directly, nonterminal expressions hold
//! sub-expressions of the abstract type and interpret them recursively.
//!
//! Anchor: the AbstractExpression. Mandatory: (1) the anchor is an abstract class
//! or an interface declaring an abstract operation i with ≥1 parameter (the
//! context); (2) a concrete subtype holds ≥2 attributes typed by the anchor (or a
//! collection of it) and its override of i invokes i through them (nonterminal);
//! (3) a concrete subtype holds no attribute typed by the anchor (terminal).
//! Optional: ≥2 nonterminal kinds; i returns a value.

use crate::uma::index::UMAIndex;
use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct InterpreterRule;

impl PatternDetectionRule for InterpreterRule {
    fn name(&self) -> &'static str {
        "Interpreter"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_INTERPRETER
    }

    fn evaluate(&self, e: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(e) {
            return Vec::new();
        }
        let concrete = ctx.concrete_descendants(e);
        for i in ctx.abstract_methods(e) {
            if ctx.param_count(i) == 0 {
                continue;
            }
            let mut ev = Evidence::new();
            ev.require(EVIDENCE_SUPERTYPE, true);
            let mut nonterminals = Vec::new();
            let mut terminals = Vec::new();
            for &x in &concrete {
                let subexprs: Vec<u32> = ctx
                    .fields(x)
                    .iter()
                    .filter(|f| !f.is_static && f.referenced_type() == e)
                    .map(|f| f.name)
                    .collect();
                let has_collection = ctx
                    .fields(x)
                    .iter()
                    .any(|f| f.is_collection && f.referenced_type() == e);
                if subexprs.is_empty() {
                    terminals.push(x);
                    continue;
                }
                if subexprs.len() < 2 && !has_collection {
                    continue;
                }
                let recursive = match ctx.index.same_signature_in(i, x) {
                    Some(ov) => subexprs.iter().any(|&n| {
                        ctx.forwards_through(ov, n, e, true) || ctx.loops_over_and_calls(ov, n, e)
                    }),
                    None => false,
                };
                if recursive {
                    nonterminals.push(x);
                }
            }
            if !ev.require(EVIDENCE_RECURSIVE_STRUCTURE, !nonterminals.is_empty()) {
                continue;
            }
            if !ev.require(EVIDENCE_CONCRETE_PARTICIPANTS, !terminals.is_empty()) {
                continue;
            }
            ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, nonterminals.len() >= 2);
            let r = ctx.return_type(i);
            ev.optional(
                EVIDENCE_VALUE_RESULT,
                r != UNKNOWN_SYM && !UMAIndex::is_void(r),
            );
            return vec![PatternMatch::new(PATTERN_INTERPRETER, e, ev)
                .with(ROLE_ABSTRACT_EXPRESSION, e)
                .with_all(ROLE_TERMINAL_EXPRESSION, &terminals)
                .with_all(ROLE_NONTERMINAL_EXPRESSION, &nonterminals)];
        }
        Vec::new()
    }
}
