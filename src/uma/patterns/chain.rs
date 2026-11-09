//! Pattern detection pipeline (§9.2.5): the rule trait, the evidence model and the
//! single chain that evaluates every registered GoF rule.
//!
//! Each rule is a specification of structural constraints (supertypes, typed
//! attributes, constructors) and behavioral constraints (forwarding calls, loops
//! over collections, allocations, overriding), in the style of PINOT (Shi &
//! Olsson, ASE 2006). A rule reports a match only when every mandatory constraint
//! holds; its score is the share of all specified constraints (mandatory and
//! optional) that held. Scores are explainable evidence ratios, **not**
//! calibrated probabilities — no labeled corpus is available to calibrate them.
//! Identifier spelling is never a constraint.

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::patterns::query::PatternContext;
use crate::uma::patterns::*;
use crate::uma::types::*;

/// Constraint bookkeeping for one candidate instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    pub satisfied: u32,
    pub specified: u32,
    pub mask: u64,
}

impl Evidence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a mandatory constraint; returns whether it held (callers stop on false).
    pub fn require(&mut self, fact: u64, holds: bool) -> bool {
        self.record(fact, holds);
        holds
    }

    /// Record an optional constraint that only raises the score.
    pub fn optional(&mut self, fact: u64, holds: bool) {
        self.record(fact, holds);
    }

    /// Note which alternative of a disjunctive constraint held (no score effect).
    pub fn note(&mut self, fact: u64) {
        self.mask |= fact;
    }

    fn record(&mut self, fact: u64, holds: bool) {
        self.specified += 1;
        if holds {
            self.satisfied += 1;
            self.mask |= fact;
        }
    }

    /// Share of specified constraints that held, in percent.
    pub fn score(&self) -> u8 {
        if self.specified == 0 {
            return 0;
        }
        let pct = (u64::from(self.satisfied) * 100) / u64::from(self.specified);
        u8::try_from(pct.min(100)).unwrap_or(100)
    }
}

/// One detected pattern instance with its participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub kind: u8,
    pub primary: u32,
    pub roles: Vec<PatternRoleBinding>,
    pub evidence: Evidence,
}

impl PatternMatch {
    pub fn new(kind: u8, primary: u32, evidence: Evidence) -> Self {
        Self {
            kind,
            primary,
            roles: Vec::new(),
            evidence,
        }
    }

    pub fn bind(&mut self, role: u8, sym_id: u32) {
        if sym_id == UNKNOWN_SYM {
            return;
        }
        let b = PatternRoleBinding { role, sym_id };
        if !self.roles.contains(&b) {
            self.roles.push(b);
        }
    }

    pub fn bind_all(&mut self, role: u8, syms: &[u32]) {
        for &s in syms {
            self.bind(role, s);
        }
    }

    pub fn with(mut self, role: u8, sym_id: u32) -> Self {
        self.bind(role, sym_id);
        self
    }

    pub fn with_all(mut self, role: u8, syms: &[u32]) -> Self {
        self.bind_all(role, syms);
        self
    }

    pub fn score(&self) -> u8 {
        self.evidence.score()
    }

    /// Syms bound to `role`.
    pub fn bound(&self, role: u8) -> impl Iterator<Item = u32> + '_ {
        self.roles
            .iter()
            .filter(move |b| b.role == role)
            .map(|b| b.sym_id)
    }

    /// Canonical form: role bindings sorted, for de-duplication and stable ids.
    pub fn normalize(&mut self) {
        self.roles.sort_by_key(|b| (b.role, b.sym_id));
        self.roles.dedup();
    }
}

/// A GoF pattern specification evaluated against one anchor type.
pub trait PatternDetectionRule: Send + Sync {
    /// The canonical name of the design pattern (e.g., "Strategy", "Decorator").
    fn name(&self) -> &'static str;

    /// The primary pattern kind (a rule may also report closely related kinds,
    /// e.g. the delegation rule reports Strategy, State or Bridge).
    fn pattern_kind(&self) -> u8;

    /// All instances anchored at `sym_id` whose mandatory constraints hold.
    fn evaluate(&self, sym_id: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch>;
}

/// The single pattern pipeline: every rule runs once per anchor type.
pub struct PatternDetectionChain {
    rules: Vec<Box<dyn PatternDetectionRule>>,
}

impl Default for PatternDetectionChain {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternDetectionChain {
    /// Creates a chain with the rules for all 23 GoF patterns registered.
    pub fn new() -> Self {
        let mut chain = Self { rules: Vec::new() };
        // Creational
        chain.add_rule(Box::new(singleton::SingletonRule));
        chain.add_rule(Box::new(factory::FactoryMethodRule));
        chain.add_rule(Box::new(abstract_factory::AbstractFactoryRule));
        chain.add_rule(Box::new(builder::BuilderRule));
        chain.add_rule(Box::new(prototype::PrototypeRule));
        // Structural
        chain.add_rule(Box::new(adapter::AdapterRule));
        chain.add_rule(Box::new(composite::CompositeRule));
        chain.add_rule(Box::new(decorator::DecoratorRule));
        chain.add_rule(Box::new(proxy::ProxyRule));
        chain.add_rule(Box::new(facade::FacadeRule));
        chain.add_rule(Box::new(flyweight::FlyweightRule));
        // Behavioral (Strategy/State/Bridge share one exclusive delegation rule)
        chain.add_rule(Box::new(strategy::DelegationFamilyRule));
        chain.add_rule(Box::new(observer::ObserverRule));
        chain.add_rule(Box::new(template_method::TemplateMethodRule));
        chain.add_rule(Box::new(visitor::VisitorRule));
        chain.add_rule(Box::new(command::CommandRule));
        chain.add_rule(Box::new(chain_of_responsibility::ChainOfResponsibilityRule));
        chain.add_rule(Box::new(iterator::IteratorRule));
        chain.add_rule(Box::new(mediator::MediatorRule));
        chain.add_rule(Box::new(memento::MementoRule));
        chain.add_rule(Box::new(interpreter::InterpreterRule));
        chain
    }

    /// Adds a custom pattern detection rule to the chain.
    pub fn add_rule(&mut self, rule: Box<dyn PatternDetectionRule>) {
        self.rules.push(rule);
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Evaluates every rule against one anchor type.
    pub fn evaluate(&self, sym_id: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        for rule in &self.rules {
            for mut m in rule.evaluate(sym_id, ctx) {
                m.normalize();
                out.push(m);
            }
        }
        out
    }

    /// One-off evaluation of a single symbol. Builds a fresh index and body cache
    /// (O(N) per call) and therefore suits ad-hoc queries, not whole-program runs;
    /// `PatternDetector::detect_all` shares one index across all classes.
    /// Returned rows carry per-call instance ordinals.
    pub fn evaluate_symbol(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Vec<DesignPatternRecord> {
        let index = UMAIndex::build(sta, tca, cga, None);
        let bodies = MemberBodies::build(&index);
        let ctx = PatternContext::new(&index, &bodies);
        let mut results = Vec::new();
        for (ordinal, m) in self.evaluate(sym_id, &ctx).into_iter().enumerate() {
            let instance_id = u32::try_from(ordinal).unwrap_or(u32::MAX);
            for b in &m.roles {
                if !index.is_user_type(b.sym_id) {
                    continue;
                }
                results.push(DesignPatternRecord {
                    class_sym: b.sym_id,
                    pattern_kind: u16::from(m.kind),
                    confidence: u16::from(m.score()),
                    instance_id,
                    role: b.role,
                });
            }
        }
        results
    }
}
