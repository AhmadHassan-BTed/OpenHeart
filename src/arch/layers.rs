//! Layer inference and layering conformance.
//!
//! **Inferred layers.** Strongly connected components are contracted (the members of a
//! dependency cycle cannot be layered apart, so they share a layer) and every component gets
//! its longest-path level over the condensation: level 0 depends on nothing outside itself,
//! level `k` depends on something at level `k - 1` and on nothing at level `k` or above. This
//! is the longest-path layering of the condensation: every acyclic dependency points strictly
//! down and the number of layers equals the longest dependency chain.
//!
//! **Violations of the inferred layering.** Acyclic dependencies never point up, so the
//! violations are the dependencies inside a cyclic component, each marked with whether the
//! DSM ordering selects it as feedback (one of the dependencies whose inversion or removal
//! breaks the cycle), and, in strict mode, the dependencies that skip one or more layers.
//!
//! **Declared layers.** When the configuration declares layers (listed top to bottom) with
//! name patterns, each node takes the layer of its most specific matching pattern, and every
//! dependency between assigned nodes is checked: a dependency to a higher layer is an upward
//! violation; one that skips layers is a violation unless skipping is allowed; one inside a
//! layer is a violation when intra-layer dependencies are forbidden. Nodes no pattern matches,
//! and nodes that equally specific patterns of different layers match, are reported instead
//! of being placed.
//!
//! **Patterns** match qualified names. `*` matches one name segment (a run of identifier
//! characters: alphanumerics, `_` and `$`), `**` matches any text and `?` one character. A
//! pattern matches a name when it matches the whole name or a prefix of it that ends before a
//! separator (a non-identifier character), so a namespace pattern covers the namespace's
//! members. Among matching patterns, a whole-name match beats a prefix match, then more
//! literal characters win.

use std::collections::BTreeMap;

use crate::arch::dsm::{self, component_levels, strongly_connected_components, DsmConfig};
use crate::arch::graph::{span_key, Certainty, DependencyGraph, KindSet, Level, NodeId};
use crate::ir::Span;

#[derive(Debug, Clone)]
pub struct DeclaredLayer {
    pub name: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeclaredLayers {
    /// Layers from top (index 0) to bottom.
    pub layers: Vec<DeclaredLayer>,
    /// A layer may use any lower layer, not only the one directly below.
    pub allow_skip: bool,
    /// Members of one layer may depend on each other.
    pub allow_intra: bool,
}

#[derive(Debug, Clone)]
pub struct LayerConfig {
    pub kinds: KindSet,
    pub include_external: bool,
    /// Report dependencies that skip inferred layers.
    pub strict: bool,
    pub declared: Option<DeclaredLayers>,
    /// Ordering parameters for selecting feedback dependencies inside cycles.
    pub improvement_rounds: u32,
    pub max_improvement_size: usize,
}

impl Default for LayerConfig {
    fn default() -> Self {
        let dsm = DsmConfig::default();
        Self {
            kinds: KindSet::STATIC,
            include_external: false,
            strict: false,
            declared: None,
            improvement_rounds: dsm.improvement_rounds,
            max_improvement_size: dsm.max_improvement_size,
        }
    }
}

/// One inferred layer.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Longest-path level (0 = bottom).
    pub level: u32,
    pub members: Vec<NodeId>,
    /// Cyclic components contracted into this layer.
    pub cyclic_components: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    /// A dependency inside a cyclic component; `feedback` marks the dependencies the DSM
    /// ordering selects to break the cycle.
    Cycle { component: u32, feedback: bool },
    /// A dependency that skips `skipped` layers.
    SkipLayer { skipped: u32 },
    /// A dependency on a higher declared layer.
    Upward,
    /// A dependency between members of one declared layer that forbids it.
    Intra,
}

#[derive(Debug, Clone)]
pub struct LayerViolation {
    pub kind: ViolationKind,
    pub from: NodeId,
    pub to: NodeId,
    /// Inferred levels for inferred-layer violations, declared layer indices (0 = top) for
    /// declared-layer violations.
    pub from_layer: u32,
    pub to_layer: u32,
    /// Distinct dependency sites over the selected kinds.
    pub weight: u32,
    pub kinds: KindSet,
    pub spans: Vec<Span>,
    pub certainty: Certainty,
}

#[derive(Debug, Clone)]
pub struct InvalidPattern {
    pub layer: u32,
    pub pattern: String,
    pub reason: String,
}

/// Conformance of the graph to declared layers.
#[derive(Debug, Clone)]
pub struct DeclaredConformance {
    /// Declared layer of each node (0 = top), `None` when unassigned or excluded.
    pub assignment: Vec<Option<u32>>,
    pub unassigned: Vec<NodeId>,
    /// Nodes matched with equal specificity by patterns of different layers.
    pub ambiguous: Vec<(NodeId, Vec<u32>)>,
    pub violations: Vec<LayerViolation>,
    /// Lowest and highest inferred level of the members of each declared layer.
    pub inferred_levels: Vec<Option<(u32, u32)>>,
    /// Declared layers that no node matches.
    pub empty_layers: Vec<u32>,
    pub invalid_patterns: Vec<InvalidPattern>,
}

#[derive(Debug, Clone)]
pub struct LayerResult {
    pub level: Level,
    /// Inferred level of each node, `None` for excluded nodes.
    pub layer_of: Vec<Option<u32>>,
    /// Inferred layers from the bottom (index = level).
    pub layers: Vec<Layer>,
    pub violations: Vec<LayerViolation>,
    pub declared: Option<DeclaredConformance>,
}

/// Infers the layers of `graph` and checks the declared layers, if any.
pub fn infer(graph: &DependencyGraph, config: &LayerConfig) -> LayerResult {
    let n = graph.node_count();
    let adjacency = graph.adjacency(config.kinds, config.include_external);
    let comps = strongly_connected_components(&adjacency);
    let levels = component_levels(&adjacency, &comps);
    let included = |v: usize| graph.included(NodeId(v as u32), config.include_external);

    let mut layer_of: Vec<Option<u32>> = vec![None; n];
    let mut by_level: BTreeMap<u32, Vec<NodeId>> = BTreeMap::new();
    let mut cyclic_per_level: BTreeMap<u32, u32> = BTreeMap::new();
    for (c, members) in comps.members.iter().enumerate() {
        let lvl = levels[c];
        let mut any = false;
        for &v in members {
            if included(v as usize) {
                layer_of[v as usize] = Some(lvl);
                by_level.entry(lvl).or_default().push(NodeId(v));
                any = true;
            }
        }
        if any && members.len() > 1 {
            *cyclic_per_level.entry(lvl).or_insert(0) += 1;
        }
    }
    let layers: Vec<Layer> = by_level
        .into_iter()
        .map(|(level, mut members)| {
            members.sort_unstable();
            Layer {
                level,
                members,
                cyclic_components: cyclic_per_level.get(&level).copied().unwrap_or(0),
            }
        })
        .collect();

    // Feedback selection inside cycles comes from the DSM ordering of the same graph.
    let has_cycles = comps.members.iter().any(|m| m.len() > 1);
    let position = if has_cycles {
        dsm::build(
            graph,
            &DsmConfig {
                kinds: config.kinds,
                include_external: config.include_external,
                improvement_rounds: config.improvement_rounds,
                max_improvement_size: config.max_improvement_size,
                max_closure_components: 0,
            },
        )
        .position
    } else {
        Vec::new()
    };

    let mut violations = Vec::new();
    for u in 0..n {
        let Some(lu) = layer_of[u] else {
            continue;
        };
        for &v in adjacency.targets(u as u32) {
            let Some(lv) = layer_of[v as usize] else {
                continue;
            };
            let cu = comps.component[u];
            let kind = if cu == comps.component[v as usize] {
                ViolationKind::Cycle {
                    component: cu,
                    feedback: position
                        .get(v as usize)
                        .zip(position.get(u))
                        .is_some_and(|(&pv, &pu)| pv > pu),
                }
            } else if config.strict && lu > lv + 1 {
                ViolationKind::SkipLayer {
                    skipped: lu - lv - 1,
                }
            } else {
                continue;
            };
            violations.push(violation(graph, kind, u as u32, v, lu, lv, config.kinds));
        }
    }

    let declared = config
        .declared
        .as_ref()
        .map(|d| check_declared(graph, d, &layer_of, config));

    LayerResult {
        level: graph.level,
        layer_of,
        layers,
        violations,
        declared,
    }
}

fn check_declared(
    graph: &DependencyGraph,
    declared: &DeclaredLayers,
    layer_of: &[Option<u32>],
    config: &LayerConfig,
) -> DeclaredConformance {
    let n = graph.node_count();
    let mut invalid_patterns = Vec::new();
    let mut compiled: Vec<(u32, Pattern)> = Vec::new();
    for (li, layer) in declared.layers.iter().enumerate() {
        for text in &layer.patterns {
            match Pattern::compile(text) {
                Ok(p) => compiled.push((li as u32, p)),
                Err(reason) => invalid_patterns.push(InvalidPattern {
                    layer: li as u32,
                    pattern: text.clone(),
                    reason,
                }),
            }
        }
    }

    let mut assignment: Vec<Option<u32>> = vec![None; n];
    let mut unassigned = Vec::new();
    let mut ambiguous = Vec::new();
    for v in 0..n {
        if layer_of[v].is_none() {
            continue;
        }
        let name: Vec<char> = graph.nodes[v].name.chars().collect();
        let mut best: Option<(Specificity, Vec<u32>)> = None;
        for (li, pattern) in &compiled {
            let Some(spec) = pattern.matches(&name) else {
                continue;
            };
            let replace = best.as_ref().is_none_or(|(s, _)| spec > *s);
            if replace {
                best = Some((spec, vec![*li]));
            } else if let Some((s, layers)) = best.as_mut() {
                if spec == *s && !layers.contains(li) {
                    layers.push(*li);
                }
            }
        }
        match best {
            None => unassigned.push(NodeId(v as u32)),
            Some((_, layers)) if layers.len() == 1 => assignment[v] = Some(layers[0]),
            Some((_, mut layers)) => {
                layers.sort_unstable();
                ambiguous.push((NodeId(v as u32), layers));
            }
        }
    }

    let layer_count = declared.layers.len();
    let mut inferred_levels: Vec<Option<(u32, u32)>> = vec![None; layer_count];
    for v in 0..n {
        if let (Some(a), Some(l)) = (assignment[v], layer_of[v]) {
            let slot = &mut inferred_levels[a as usize];
            *slot = Some(match *slot {
                Some((lo, hi)) => (lo.min(l), hi.max(l)),
                None => (l, l),
            });
        }
    }
    let empty_layers = (0..layer_count as u32)
        .filter(|&l| inferred_levels[l as usize].is_none())
        .collect();

    let adjacency = graph.adjacency(config.kinds, config.include_external);
    let mut violations = Vec::new();
    for u in 0..n {
        let Some(a) = assignment[u] else {
            continue;
        };
        for &v in adjacency.targets(u as u32) {
            let Some(b) = assignment[v as usize] else {
                continue;
            };
            let kind = if b < a {
                ViolationKind::Upward
            } else if b > a + 1 && !declared.allow_skip {
                ViolationKind::SkipLayer { skipped: b - a - 1 }
            } else if a == b && !declared.allow_intra {
                ViolationKind::Intra
            } else {
                continue;
            };
            violations.push(violation(graph, kind, u as u32, v, a, b, config.kinds));
        }
    }

    DeclaredConformance {
        assignment,
        unassigned,
        ambiguous,
        violations,
        inferred_levels,
        empty_layers,
        invalid_patterns,
    }
}

/// Aggregates the edges `from → to` over `kinds` into one violation record.
fn violation(
    graph: &DependencyGraph,
    kind: ViolationKind,
    from: u32,
    to: u32,
    from_layer: u32,
    to_layer: u32,
    kinds: KindSet,
) -> LayerViolation {
    let mut weight = 0u32;
    let mut found = KindSet::NONE;
    let mut spans: Vec<Span> = Vec::new();
    let mut certainty: Option<Certainty> = None;
    for edge in graph.edges_between(NodeId(from), NodeId(to)) {
        if !kinds.contains(edge.kind) {
            continue;
        }
        weight += edge.weight();
        found = found.with(edge.kind);
        spans.extend(edge.spans());
        certainty = Some(match certainty {
            Some(c) => c.strongest(edge.certainty.clone()),
            None => edge.certainty.clone(),
        });
    }
    spans.sort_by_key(span_key);
    spans.dedup_by_key(|s| span_key(s));
    LayerViolation {
        kind,
        from: NodeId(from),
        to: NodeId(to),
        from_layer,
        to_layer,
        weight,
        kinds: found,
        spans,
        certainty: certainty.unwrap_or(Certainty::Exact),
    }
}

// ── Name patterns ────────────────────────────────────────────────────────────

/// How specifically a pattern matched: whole-name matches first, then literal characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Specificity {
    whole: bool,
    literals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `*`: zero or more identifier characters.
    Segment,
    /// `**`: zero or more characters.
    Any,
    /// `?`: exactly one character.
    One,
}

#[derive(Debug, Clone)]
struct Pattern {
    tokens: Vec<Token>,
    literals: u32,
}

#[inline]
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl Pattern {
    fn compile(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("the pattern is empty".to_string());
        }
        let chars: Vec<char> = text.chars().collect();
        let mut tokens = Vec::with_capacity(chars.len());
        let mut literals = 0u32;
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '*' if chars.get(i + 1) == Some(&'*') => {
                    tokens.push(Token::Any);
                    i += 2;
                    while chars.get(i) == Some(&'*') {
                        i += 1;
                    }
                    continue;
                }
                '*' => tokens.push(Token::Segment),
                '?' => tokens.push(Token::One),
                c => {
                    tokens.push(Token::Literal(c));
                    literals += 1;
                }
            }
            i += 1;
        }
        Ok(Self { tokens, literals })
    }

    /// Specificity of the match of `name`, `None` when the pattern does not match it.
    fn matches(&self, name: &[char]) -> Option<Specificity> {
        let ends = self.prefix_matches(name);
        if ends[name.len()] {
            return Some(Specificity {
                whole: true,
                literals: self.literals,
            });
        }
        (0..name.len())
            .any(|k| ends[k] && k > 0 && !is_identifier_char(name[k]))
            .then_some(Specificity {
                whole: false,
                literals: self.literals,
            })
    }

    /// `ends[k]` is true when the pattern matches `name[..k]` exactly.
    fn prefix_matches(&self, name: &[char]) -> Vec<bool> {
        let len = name.len();
        // reach[k]: the tokens processed so far can match name[..k].
        let mut reach = vec![false; len + 1];
        reach[0] = true;
        for token in &self.tokens {
            let mut next = vec![false; len + 1];
            match *token {
                Token::Literal(c) => {
                    for k in 0..len {
                        if reach[k] && name[k] == c {
                            next[k + 1] = true;
                        }
                    }
                }
                Token::One => {
                    for k in 0..len {
                        if reach[k] {
                            next[k + 1] = true;
                        }
                    }
                }
                Token::Segment => {
                    for k in 0..=len {
                        if !reach[k] {
                            continue;
                        }
                        next[k] = true;
                        let mut j = k;
                        while j < len && is_identifier_char(name[j]) {
                            j += 1;
                            next[j] = true;
                        }
                    }
                }
                Token::Any => {
                    if let Some(first) = reach.iter().position(|&r| r) {
                        for slot in next.iter_mut().skip(first) {
                            *slot = true;
                        }
                    }
                }
            }
            reach = next;
        }
        reach
    }
}
