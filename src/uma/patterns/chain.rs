//! Chain of Responsibility & Strategy Patterns for GoF Pattern Detection (§9.2.5).
//!
//! Provides a composable chain of pattern detection rules allowing dynamic registration
//! and evaluation of architectural pattern hypotheses.

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::*;
use crate::uma::types::*;

/// Strategy / Rule Trait for an individual GoF Design Pattern Detector.
pub trait PatternDetectionRule: Send + Sync {
    /// The canonical name of the design pattern (e.g., "Strategy", "Decorator").
    fn name(&self) -> &'static str;

    /// The pattern kind identifier (e.g. PATTERN_STRATEGY).
    fn pattern_kind(&self) -> u8;

    /// Evaluates whether the symbol satisfies the pattern's structural and behavioral invariants.
    /// Returns Some((pattern_kind, confidence)) if matched.
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)>;
}

// ── Concrete Pattern Rules ───────────────────────────────────────────────────

pub struct SingletonRule;
impl PatternDetectionRule for SingletonRule {
    fn name(&self) -> &'static str {
        "Singleton"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_SINGLETON
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_singleton(sym_id, sta, tca);
        if matched {
            Some((PATTERN_SINGLETON, conf))
        } else {
            None
        }
    }
}

pub struct ObserverRule;
impl PatternDetectionRule for ObserverRule {
    fn name(&self) -> &'static str {
        "Observer"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_OBSERVER
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_observer_subject(sym_id, sta, tca, cga);
        if matched {
            Some((PATTERN_OBSERVER, conf))
        } else {
            None
        }
    }
}

pub struct FactoryRule;
impl PatternDetectionRule for FactoryRule {
    fn name(&self) -> &'static str {
        "Factory"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_FACTORY
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_factory(sym_id, sta, tca);
        if matched {
            Some((PATTERN_FACTORY, conf))
        } else {
            None
        }
    }
}

pub struct BuilderRule;
impl PatternDetectionRule for BuilderRule {
    fn name(&self) -> &'static str {
        "Builder"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_BUILDER
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_builder(sym_id, sta, tca);
        if matched {
            Some((PATTERN_BUILDER, conf))
        } else {
            None
        }
    }
}

pub struct StateRule;
impl PatternDetectionRule for StateRule {
    fn name(&self) -> &'static str {
        "State"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_STATE
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_state(sym_id, sta, tca);
        if matched {
            Some((PATTERN_STATE, conf))
        } else {
            None
        }
    }
}

pub struct TemplateMethodRule;
impl PatternDetectionRule for TemplateMethodRule {
    fn name(&self) -> &'static str {
        "TemplateMethod"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_TEMPLATE_METHOD
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_template_method(sym_id, sta);
        if matched {
            Some((PATTERN_TEMPLATE_METHOD, conf))
        } else {
            None
        }
    }
}

pub struct DecoratorRule;
impl PatternDetectionRule for DecoratorRule {
    fn name(&self) -> &'static str {
        "Decorator"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_DECORATOR
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_decorator(sym_id, sta, tca);
        if matched {
            Some((PATTERN_DECORATOR, conf))
        } else {
            None
        }
    }
}

pub struct StrategyRule;
impl PatternDetectionRule for StrategyRule {
    fn name(&self) -> &'static str {
        "Strategy"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_STRATEGY
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_strategy(sym_id, sta, tca);
        if matched {
            Some((PATTERN_STRATEGY, conf))
        } else {
            None
        }
    }
}

pub struct AdapterRule;
impl PatternDetectionRule for AdapterRule {
    fn name(&self) -> &'static str {
        "Adapter"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_ADAPTER
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_adapter(sym_id, sta, tca);
        if matched {
            Some((PATTERN_ADAPTER, conf))
        } else {
            None
        }
    }
}

pub struct FacadeRule;
impl PatternDetectionRule for FacadeRule {
    fn name(&self) -> &'static str {
        "Facade"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_FACADE
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_facade(sym_id, sta, tca);
        if matched {
            Some((PATTERN_FACADE, conf))
        } else {
            None
        }
    }
}

pub struct CompositeRule;
impl PatternDetectionRule for CompositeRule {
    fn name(&self) -> &'static str {
        "Composite"
    }
    fn pattern_kind(&self) -> u8 {
        PATTERN_COMPOSITE
    }
    fn evaluate(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        _cga: &CallGraphArtifact,
    ) -> Option<(u8, u16)> {
        let (matched, conf) = is_composite(sym_id, sta, tca);
        if matched {
            Some((PATTERN_COMPOSITE, conf))
        } else {
            None
        }
    }
}

// ── Chain of Responsibility Engine ───────────────────────────────────────────

/// Composable Chain of Responsibility evaluating pattern hypotheses across symbols.
pub struct PatternDetectionChain {
    rules: Vec<Box<dyn PatternDetectionRule>>,
}

impl Default for PatternDetectionChain {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternDetectionChain {
    /// Creates a chain with all 11 default GoF pattern rules pre-registered.
    pub fn new() -> Self {
        let mut chain = Self { rules: Vec::new() };
        chain.add_rule(Box::new(SingletonRule));
        chain.add_rule(Box::new(ObserverRule));
        chain.add_rule(Box::new(FactoryRule));
        chain.add_rule(Box::new(BuilderRule));
        chain.add_rule(Box::new(StateRule));
        chain.add_rule(Box::new(TemplateMethodRule));
        chain.add_rule(Box::new(DecoratorRule));
        chain.add_rule(Box::new(StrategyRule));
        chain.add_rule(Box::new(AdapterRule));
        chain.add_rule(Box::new(FacadeRule));
        chain.add_rule(Box::new(CompositeRule));
        chain
    }

    /// Adds a custom pattern detection rule to the chain.
    pub fn add_rule(&mut self, rule: Box<dyn PatternDetectionRule>) {
        self.rules.push(rule);
    }

    /// Evaluates the entire chain against a symbol and returns all matching pattern records.
    pub fn evaluate_symbol(
        &self,
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Vec<DesignPatternRecord> {
        let mut results = Vec::new();
        for rule in &self.rules {
            if let Some((kind, conf)) = rule.evaluate(sym_id, sta, tca, cga) {
                results.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: kind as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }
        }
        results
    }
}
