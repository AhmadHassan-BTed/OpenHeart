//! MermaidExporter — prints diagram view models as Mermaid (§10.4, §9.6).
//!
//! Every diagram is rendered from the typed view IR built by `views::build`. Node ids are
//! view-unique prefixed aliases (never a reserved word such as `end`), display names are
//! quoted labels with Mermaid entity escapes, class relationships are kept outside
//! `namespace` blocks and namespaces are never nested, so the output parses with Mermaid 10+.
//! Output larger than Mermaid's default `maxTextSize` is rebuilt with halved budgets.

use std::collections::{BTreeMap, HashMap};

use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::diagram::common::DiagramUtils as U;
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::model::*;
use crate::scpg::diagram::views;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Mermaid's default `maxTextSize`.
const MERMAID_MAX_TEXT: usize = 50_000;
/// Budget halvings attempted before giving up on fitting `maxTextSize`.
const MAX_SHRINK_ATTEMPTS: usize = 4;
/// Omitted element names listed in the legend.
const LEGEND_OMITTED_NAMES: usize = 20;

/// Strategy Interface for Mermaid Diagram Generation (§9.6).
pub trait MermaidDiagramStrategy: Send + Sync {
    /// Returns the unique diagram type identifier.
    fn diagram_type(&self) -> &'static str;

    /// Renders the diagram from the mandatory artifacts only.
    fn export(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String;

    /// Renders the diagram from the full artifact context. Strategies that only use the
    /// mandatory artifacts keep the default, which forwards to [`Self::export`].
    fn export_with_context(&self, ctx: &DiagramContext) -> String {
        self.export(ctx.uma, ctx.sta, ctx.tca)
    }
}

pub struct MermaidExporter {
    strategies: BTreeMap<String, Box<dyn MermaidDiagramStrategy>>,
}

impl Default for MermaidExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MermaidExporter {
    /// Initializes the registry with the 14 UML and 5 program-analysis strategies.
    pub fn new() -> Self {
        let mut exporter = Self {
            strategies: BTreeMap::new(),
        };
        exporter.register_builtin();
        exporter
    }

    /// Add or replace a diagram generation strategy.
    pub fn register_strategy(&mut self, strategy: Box<dyn MermaidDiagramStrategy>) {
        self.strategies
            .insert(strategy.diagram_type().to_string(), strategy);
    }

    /// Remove a diagram generation strategy.
    pub fn unregister_strategy(
        &mut self,
        diagram_type: &str,
    ) -> Option<Box<dyn MermaidDiagramStrategy>> {
        self.strategies.remove(diagram_type)
    }

    pub fn has_strategy(&self, diagram_type: &str) -> bool {
        self.strategies.contains_key(diagram_type)
    }

    /// Registered diagram type keys, sorted.
    pub fn strategy_types(&self) -> Vec<&str> {
        self.strategies.keys().map(|k| k.as_str()).collect()
    }

    /// Export one diagram from the mandatory artifacts.
    pub fn export(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        self.strategies
            .get(diagram_type)
            .map(|s| s.export(uma, sta, tca))
    }

    /// Export every registered diagram from the mandatory artifacts.
    pub fn export_all(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        self.strategies
            .iter()
            .map(|(k, s)| (k.clone(), s.export(uma, sta, tca)))
            .collect()
    }

    /// Export one diagram from the full artifact context.
    pub fn export_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        self.strategies
            .get(diagram_type)
            .map(|s| s.export_with_context(ctx))
    }

    /// Export every registered diagram from the full artifact context.
    pub fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        self.strategies
            .iter()
            .map(|(k, s)| (k.clone(), s.export_with_context(ctx)))
            .collect()
    }

    /// Builds the view of `kind` and prints it, halving the budgets while the text exceeds
    /// Mermaid's default `maxTextSize`.
    pub fn render(kind: ViewKind, ctx: &DiagramContext) -> String {
        let digest = ctx.input_digest();
        let mut text = Self::print(&views::build(kind, ctx), &digest);
        let mut narrowed = ctx.clone();
        let mut attempt = 0;
        while text.len() > MERMAID_MAX_TEXT
            && attempt < MAX_SHRINK_ATTEMPTS
            && narrowed.options.max_nodes > 8
        {
            attempt += 1;
            narrowed.options.max_nodes /= 2;
            narrowed.options.max_edges /= 2;
            narrowed.options.max_messages = (narrowed.options.max_messages / 2).max(8);
            let mut model = views::build(kind, &narrowed);
            model.note(format!(
                "Budgets were halved {} time(s) to stay within Mermaid's default maxTextSize of {} characters.",
                attempt, MERMAID_MAX_TEXT
            ));
            text = Self::print(&model, &digest);
        }
        if text.len() > MERMAID_MAX_TEXT {
            text.push_str(&format!(
                "    %% size {} exceeds Mermaid's default maxTextSize ({}); raise maxTextSize in the renderer configuration\n",
                text.len(),
                MERMAID_MAX_TEXT
            ));
        }
        text
    }

    /// One Mermaid sequence diagram per scenario: (scenario title, diagram text).
    pub fn export_sequence_scenarios(ctx: &DiagramContext) -> Vec<(String, String)> {
        let digest = ctx.input_digest();
        let model = views::build(ViewKind::Sequence, ctx);
        if model.is_unavailable() {
            return vec![(model.title.clone(), Self::print(&model, &digest))];
        }
        model
            .interactions
            .iter()
            .map(|interaction| {
                let mut single = model.clone();
                single.interactions = vec![interaction.clone()];
                (interaction.title.clone(), Self::print(&single, &digest))
            })
            .collect()
    }

    fn render_basic(
        kind: ViewKind,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render(kind, &DiagramContext::new(uma, sta, tca))
    }

    /// Prints a view model as one Mermaid diagram.
    pub fn print(model: &ViewModel, input_digest: &str) -> String {
        let header = match model.kind {
            ViewKind::Class | ViewKind::Profile | ViewKind::Object => "classDiagram",
            ViewKind::Sequence => "sequenceDiagram",
            ViewKind::StateMachine => "stateDiagram-v2",
            ViewKind::Communication => "graph LR",
            _ => "graph TD",
        };
        let mut out = format!("{}\n", header);
        out.push_str(&format!(
            "    %% OpenHeart {} view | provenance: {} | input sha256:{}\n",
            model.kind.key(),
            model.provenance.as_str(),
            input_digest
        ));
        out.push_str(&format!("    %% title: {}\n", U::single_line(&model.title)));
        if !model.sources.is_empty() {
            out.push_str(&format!("    %% derived from: {}\n", model.sources.join(", ")));
        }
        if model.is_unavailable() {
            Self::print_unavailable(model, &mut out);
            return out;
        }
        match model.kind {
            ViewKind::Class | ViewKind::Profile | ViewKind::Object => {
                Self::print_class(model, &mut out)
            }
            ViewKind::Sequence => Self::print_sequence(model, &mut out),
            ViewKind::StateMachine => Self::print_state(model, &mut out),
            _ => Self::print_flowchart(model, &mut out),
        }
        out
    }

    fn legend_lines(model: &ViewModel) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        if model.provenance != Provenance::Derived {
            lines.push(format!("provenance: {}", model.provenance.as_str()));
        }
        lines.extend(model.notes.iter().cloned());
        if model.omitted_count() > 0 {
            let names: Vec<&str> = model
                .omitted
                .iter()
                .take(LEGEND_OMITTED_NAMES)
                .map(|s| s.as_str())
                .collect();
            let mut text = format!("omitted ({}): {}", model.omitted_count(), names.join(", "));
            if model.omitted_count() > names.len() {
                text.push_str(", …");
            }
            lines.push(text);
        }
        lines
    }

    /// Plain text for contexts without entity support (state labels, notes, annotations).
    fn plain(text: &str) -> String {
        U::single_line(text)
            .replace('"', "'")
            .replace([';', '#', '{', '}'], "")
    }

    fn print_unavailable(model: &ViewModel, out: &mut String) {
        let reason = model.notes.join(" ");
        match model.kind {
            ViewKind::Class | ViewKind::Profile | ViewKind::Object => {
                out.push_str("    class notice_unavailable[\"Not available\"]\n");
                out.push_str(&format!("    note \"{}\"\n", Self::plain(&reason)));
            }
            ViewKind::Sequence => {
                out.push_str("    participant notice_unavailable as Not available\n");
                out.push_str(&format!(
                    "    Note over notice_unavailable: {}\n",
                    U::mermaid_text(&reason)
                ));
            }
            ViewKind::StateMachine => {
                out.push_str("    state \"Not available\" as notice_unavailable\n");
                out.push_str(&format!(
                    "    note right of notice_unavailable : {}\n",
                    Self::plain(&reason)
                ));
            }
            _ => {
                out.push_str(&format!(
                    "    notice_unavailable[\"{}\"]\n",
                    U::mermaid_text(&format!("Not available\n{}", reason))
                ));
            }
        }
    }

    // ── classDiagram (class, object, profile) ────────────────────────────────

    fn print_class(model: &ViewModel, out: &mut String) {
        let mut by_container: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut top: Vec<usize> = Vec::new();
        for (idx, node) in model.nodes.iter().enumerate() {
            if node.kind.is_container() {
                continue;
            }
            match node.parent.as_deref().and_then(|p| model.node(p)) {
                Some(parent) => by_container.entry(parent.alias.clone()).or_default().push(idx),
                None => top.push(idx),
            }
        }
        for (namespace, idxs) in &by_container {
            out.push_str(&format!("    namespace {} {{\n", namespace));
            for &i in idxs {
                let node = &model.nodes[i];
                out.push_str(&format!(
                    "        class {}[\"{}\"]\n",
                    node.alias,
                    U::mermaid_class_text(&node.label)
                ));
            }
            out.push_str("    }\n");
        }
        for &i in &top {
            let node = &model.nodes[i];
            out.push_str(&format!(
                "    class {}[\"{}\"]\n",
                node.alias,
                U::mermaid_class_text(&node.label)
            ));
        }

        for node in model.nodes.iter().filter(|n| !n.kind.is_container()) {
            let implied = match node.kind {
                NodeKind::Interface => Some("interface"),
                NodeKind::AbstractClass => Some("abstract"),
                NodeKind::Enum => Some("enumeration"),
                NodeKind::Annotation => Some("annotation"),
                NodeKind::Stereotype => Some("stereotype"),
                NodeKind::Metaclass => Some("metaclass"),
                NodeKind::Object => Some("object"),
                _ => None,
            };
            let mut stereotypes: Vec<String> = implied.map(|s| vec![s.to_string()]).unwrap_or_default();
            stereotypes.extend(node.stereotypes.iter().cloned());
            if !stereotypes.is_empty() {
                let text = stereotypes
                    .iter()
                    .map(|s| Self::plain(s).replace(['<', '>'], ""))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("    <<{}>> {}\n", text, node.alias));
            }
            for member in &node.members {
                out.push_str(&format!("    {} : {}\n", node.alias, Self::member_text(member)));
            }
            if node.members.is_empty() {
                for detail in &node.details {
                    out.push_str(&format!(
                        "    {} : {}\n",
                        node.alias,
                        U::mermaid_class_text(detail)
                    ));
                }
            }
        }

        for edge in &model.edges {
            if let Some(line) = Self::class_relation(model, edge) {
                out.push_str(&format!("    {}\n", line));
            }
        }
        for line in Self::legend_lines(model) {
            out.push_str(&format!("    note \"{}\"\n", Self::plain(&line)));
        }
    }

    fn member_text(member: &MemberView) -> String {
        match member.kind {
            MemberKind::Literal => U::mermaid_class_text(&member.name),
            MemberKind::Field => {
                let mut text = member.visibility.to_string();
                if let Some(ty) = &member.type_name {
                    text.push_str(&U::mermaid_class_text(ty));
                    text.push(' ');
                }
                text.push_str(&U::mermaid_class_text(&member.name));
                if let Some(m) = &member.multiplicity {
                    text.push_str(&format!(" [{}]", m));
                }
                if member.is_static {
                    text.push('$');
                }
                text
            }
            MemberKind::Method | MemberKind::Constructor => {
                let params = match &member.params {
                    Some(list) => list
                        .iter()
                        .map(|p| match (&p.type_name, &p.name) {
                            (Some(ty), Some(n)) => format!("{} {}", U::mermaid_class_text(ty), U::mermaid_class_text(n)),
                            (None, Some(n)) => U::mermaid_class_text(n),
                            (Some(ty), None) => U::mermaid_class_text(ty),
                            (None, None) => "?".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    None => member.param_list(),
                };
                let mut text = format!("{}{}({})", member.visibility, U::mermaid_class_text(&member.name), params);
                if member.is_abstract {
                    text.push('*');
                } else if member.is_static {
                    text.push('$');
                }
                if let Some(ty) = &member.type_name {
                    text.push(' ');
                    text.push_str(&U::mermaid_class_text(ty));
                }
                text
            }
        }
    }

    fn class_relation(model: &ViewModel, edge: &ViewEdge) -> Option<String> {
        let src = &model.node(&edge.source)?.alias;
        let dst = &model.node(&edge.target)?.alias;
        let arrow = match edge.kind {
            EdgeKind::Generalization => "--|>",
            EdgeKind::Realization => "..|>",
            EdgeKind::Aggregation => "o--",
            EdgeKind::Composition => "*--",
            EdgeKind::Dependency | EdgeKind::Import | EdgeKind::Requires => "..>",
            EdgeKind::Nesting | EdgeKind::Link | EdgeKind::Connector | EdgeKind::Provides => "--",
            _ => "-->",
        };
        let mut line = src.clone();
        if let Some(m) = &edge.source_multiplicity {
            line.push_str(&format!(" \"{}\"", Self::plain(m)));
        }
        line.push_str(&format!(" {} ", arrow));
        if let Some(m) = &edge.target_multiplicity {
            line.push_str(&format!("\"{}\" ", Self::plain(m)));
        }
        line.push_str(dst);
        let label = edge
            .label
            .clone()
            .or_else(|| edge.role.clone())
            .or_else(|| {
                if edge.kind == EdgeKind::Nesting {
                    Some("nested".to_string())
                } else {
                    None
                }
            });
        if let Some(text) = label {
            line.push_str(&format!(" : {}", U::mermaid_class_text(&text)));
        }
        Some(line)
    }

    // ── sequenceDiagram ──────────────────────────────────────────────────────

    fn seq_alias(model: &ViewModel, key: &str) -> String {
        model
            .node(key)
            .map(|n| n.alias.clone())
            .unwrap_or_else(|| U::sanitize(key))
    }

    fn fragment_text(kind: FragmentKind, guard: &Option<String>) -> String {
        match guard {
            Some(g) if !g.trim().is_empty() => U::mermaid_text(g),
            _ => match kind {
                FragmentKind::Alt => "alternative".to_string(),
                FragmentKind::Opt => "optional".to_string(),
                FragmentKind::Loop => "repeat".to_string(),
                FragmentKind::Par => "parallel".to_string(),
                FragmentKind::Break => "break".to_string(),
                FragmentKind::Critical => "critical region".to_string(),
            },
        }
    }

    /// Separator keyword and text that start the next operand of a `kind` fragment; `None`
    /// for fragments that have a single operand in Mermaid (`opt`, `loop`, `break`).
    fn operand_text(kind: FragmentKind, guard: &Option<String>) -> Option<(&'static str, String)> {
        let (keyword, fallback) = match kind {
            FragmentKind::Alt => ("else", "otherwise"),
            FragmentKind::Par => ("and", "parallel"),
            FragmentKind::Critical => ("option", "otherwise"),
            FragmentKind::Opt | FragmentKind::Loop | FragmentKind::Break => return None,
        };
        let text = match guard {
            Some(g) if !g.trim().is_empty() => U::mermaid_text(g),
            _ => fallback.to_string(),
        };
        Some((keyword, text))
    }

    fn print_sequence(model: &ViewModel, out: &mut String) {
        let mut declared: Vec<&str> = Vec::new();
        for interaction in &model.interactions {
            for lifeline in &interaction.lifelines {
                if !declared.contains(&lifeline.as_str()) {
                    declared.push(lifeline.as_str());
                }
            }
        }
        for key in &declared {
            if let Some(node) = model.node(key) {
                let keyword = if node.kind == NodeKind::Actor {
                    "actor"
                } else {
                    "participant"
                };
                out.push_str(&format!(
                    "    {} {} as {}\n",
                    keyword,
                    node.alias,
                    Self::plain(&node.label)
                ));
            }
        }
        let framed = model.interactions.len() > 1;
        for interaction in &model.interactions {
            let first = match interaction.lifelines.first() {
                Some(f) => Self::seq_alias(model, f),
                None => continue,
            };
            let indent = if framed { "        " } else { "    " };
            if framed {
                out.push_str("    rect rgba(56, 189, 248, 0.08)\n");
            }
            out.push_str(&format!(
                "{}Note over {}: {}\n",
                indent,
                first,
                U::mermaid_text(&format!("Scenario: {}", interaction.title))
            ));
            // Kinds of the currently open fragments, innermost last.
            let mut open_fragments: Vec<FragmentKind> = Vec::new();
            for event in &interaction.events {
                match event {
                    InteractionEvent::Call {
                        from,
                        to,
                        label,
                        number,
                        is_async,
                        activates,
                        ..
                    } => {
                        let arrow = if *is_async {
                            "-)"
                        } else if *activates {
                            "->>+"
                        } else {
                            "->>"
                        };
                        out.push_str(&format!(
                            "{}{}{}{}: {}\n",
                            indent,
                            Self::seq_alias(model, from),
                            arrow,
                            Self::seq_alias(model, to),
                            U::mermaid_text(&format!("{} {}", number, label))
                        ));
                    }
                    InteractionEvent::Reply {
                        from,
                        to,
                        label,
                        deactivates,
                    } => {
                        let arrow = if *deactivates { "-->>-" } else { "-->>" };
                        out.push_str(&format!(
                            "{}{}{}{}: {}\n",
                            indent,
                            Self::seq_alias(model, from),
                            arrow,
                            Self::seq_alias(model, to),
                            U::mermaid_text(label)
                        ));
                    }
                    InteractionEvent::Deactivate { lifeline } => {
                        out.push_str(&format!(
                            "{}deactivate {}\n",
                            indent,
                            Self::seq_alias(model, lifeline)
                        ));
                    }
                    InteractionEvent::FragmentStart { kind, guard } => {
                        open_fragments.push(*kind);
                        out.push_str(&format!(
                            "{}{} {}\n",
                            indent,
                            kind.as_str(),
                            Self::fragment_text(*kind, guard)
                        ));
                    }
                    InteractionEvent::FragmentElse { guard } => {
                        match open_fragments
                            .last()
                            .and_then(|kind| Self::operand_text(*kind, guard))
                        {
                            Some((keyword, text)) => {
                                out.push_str(&format!("{}{} {}\n", indent, keyword, text))
                            }
                            // No operand separator for the enclosing fragment (or none is
                            // open): keep the guard as a comment so the output stays valid.
                            None => out.push_str(&format!(
                                "{}%% next operand: {}\n",
                                indent,
                                guard
                                    .as_deref()
                                    .map(U::mermaid_text)
                                    .unwrap_or_else(|| "otherwise".to_string())
                            )),
                        }
                    }
                    InteractionEvent::FragmentEnd => {
                        open_fragments.pop();
                        out.push_str(&format!("{}end\n", indent));
                    }
                    InteractionEvent::Note { over, text } => {
                        out.push_str(&format!(
                            "{}Note over {}: {}\n",
                            indent,
                            Self::seq_alias(model, over),
                            U::mermaid_text(text)
                        ));
                    }
                }
            }
            if interaction.omitted_messages > 0 {
                out.push_str(&format!(
                    "{}Note over {}: {}\n",
                    indent,
                    first,
                    U::mermaid_text(&format!(
                        "{} further call(s) omitted by the message budget",
                        interaction.omitted_messages
                    ))
                ));
            }
            if framed {
                out.push_str("    end\n");
            }
        }
        let legend = Self::legend_lines(model);
        if let (false, Some(first)) = (legend.is_empty(), declared.first()) {
            out.push_str(&format!(
                "    Note over {}: {}\n",
                Self::seq_alias(model, first),
                U::mermaid_text(&legend.join("\n"))
            ));
        }
    }

    // ── stateDiagram-v2 (state machines) ─────────────────────────────────────

    fn print_state(model: &ViewModel, out: &mut String) {
        let mut ids = AliasTable::default();
        let children = model.children_by_parent();
        let state_ref = |key: &str| -> Option<String> {
            let node = model.node(key)?;
            Some(match node.kind {
                NodeKind::Initial | NodeKind::Final => "[*]".to_string(),
                _ => node.alias.clone(),
            })
        };
        let mut edges_by_scope: BTreeMap<Option<String>, Vec<&ViewEdge>> = BTreeMap::new();
        for edge in &model.edges {
            let a = model.ancestors(&edge.source);
            let b = model.ancestors(&edge.target);
            let mut scope: Option<String> = None;
            for (x, y) in a.iter().zip(b.iter()) {
                if x == y {
                    scope = Some(x.clone());
                } else {
                    break;
                }
            }
            edges_by_scope.entry(scope).or_default().push(edge);
        }
        let print_edges = |scope: Option<String>, indent: &str, out: &mut String| {
            if let Some(edges) = edges_by_scope.get(&scope) {
                for edge in edges {
                    if let (Some(s), Some(d)) = (state_ref(edge.source.as_str()), state_ref(edge.target.as_str())) {
                        match &edge.label {
                            Some(l) => out.push_str(&format!("{}{} --> {} : {}\n", indent, s, d, Self::plain(l))),
                            None => out.push_str(&format!("{}{} --> {}\n", indent, s, d)),
                        }
                    }
                }
            }
        };
        if let Some(top) = children.get(&None) {
            for &i in top {
                let group = &model.nodes[i];
                if !group.kind.is_container() {
                    if !matches!(group.kind, NodeKind::Initial | NodeKind::Final) {
                        out.push_str(&format!(
                            "    state \"{}\" as {}\n",
                            Self::plain(&group.label),
                            group.alias
                        ));
                    }
                    continue;
                }
                let id = ids.alias("sm_", &group.label, &group.key);
                out.push_str(&format!("    state {} {{\n", id));
                if let Some(members) = children.get(&Some(group.key.clone())) {
                    for &m in members {
                        let node = &model.nodes[m];
                        if matches!(node.kind, NodeKind::Initial | NodeKind::Final) {
                            continue;
                        }
                        out.push_str(&format!(
                            "        state \"{}\" as {}\n",
                            Self::plain(&node.label),
                            node.alias
                        ));
                    }
                }
                print_edges(Some(group.key.clone()), "        ", out);
                out.push_str("    }\n");
            }
        }
        print_edges(None, "    ", out);
        for line in Self::legend_lines(model) {
            out.push_str(&format!("    %% {}\n", U::single_line(&line)));
        }
    }

    // ── flowchart (activity, CFG, CDG, DFG, ROBDD, call graph, packages, ...) ─

    fn flow_shape(node: &ViewNode, is_leaf_container: bool) -> (&'static str, &'static str) {
        if is_leaf_container {
            return ("[/", "\\]");
        }
        match node.kind {
            NodeKind::Initial | NodeKind::Final | NodeKind::SsaValue | NodeKind::Actor => ("([", "])"),
            NodeKind::Decision | NodeKind::Merge => ("{", "}"),
            NodeKind::Fork | NodeKind::Join | NodeKind::InteractionUse => ("[[", "]]"),
            NodeKind::SsaPhi => ("{{", "}}"),
            NodeKind::BddVariable | NodeKind::ProvidedInterface => ("((", "))"),
            NodeKind::ExceptionHandler => ("[/", "/]"),
            NodeKind::State => ("(", ")"),
            _ => ("[", "]"),
        }
    }

    fn node_text(node: &ViewNode) -> String {
        let mut text = match node.kind {
            NodeKind::Component => format!("«component» {}", node.label),
            NodeKind::InteractionUse if !node.label.starts_with("ref") => format!("ref: {}", node.label),
            _ => node.label.clone(),
        };
        if !node.stereotypes.is_empty() {
            text = format!("«{}» {}", node.stereotypes.join(", "), text);
        }
        for detail in &node.details {
            text.push('\n');
            text.push_str(detail);
        }
        text
    }

    fn print_flowchart(model: &ViewModel, out: &mut String) {
        let children = model.children_by_parent();
        Self::print_flow_tree(model, &children, None, 1, out);
        for edge in &model.edges {
            let (src, dst) = match (model.node(&edge.source), model.node(&edge.target)) {
                (Some(s), Some(d)) => (&s.alias, &d.alias),
                _ => continue,
            };
            let arrow = if edge.kind.is_undirected() {
                "---"
            } else if edge.kind.is_dashed() {
                "-.->"
            } else {
                "-->"
            };
            match &edge.label {
                Some(label) => out.push_str(&format!(
                    "    {} {}|\"{}\"| {}\n",
                    src,
                    arrow,
                    U::mermaid_text(label),
                    dst
                )),
                None => out.push_str(&format!("    {} {} {}\n", src, arrow, dst)),
            }
        }
        let legend = Self::legend_lines(model);
        if !legend.is_empty() {
            out.push_str(&format!(
                "    legend_box[\"{}\"]:::legend\n",
                U::mermaid_text(&legend.join("\n"))
            ));
            out.push_str("    classDef legend fill:#0f141c,stroke:#facc15,color:#facc15\n");
        }
    }

    fn print_flow_tree(
        model: &ViewModel,
        children: &BTreeMap<Option<String>, Vec<usize>>,
        parent: Option<&String>,
        depth: usize,
        out: &mut String,
    ) {
        let key = parent.cloned();
        let idxs = match children.get(&key) {
            Some(v) => v,
            None => return,
        };
        let indent = "    ".repeat(depth);
        for &i in idxs {
            let node = &model.nodes[i];
            let has_children = children.contains_key(&Some(node.key.clone()));
            if has_children {
                out.push_str(&format!(
                    "{}subgraph {}[\"{}\"]\n",
                    indent,
                    node.alias,
                    U::mermaid_text(&Self::node_text(node))
                ));
                out.push_str(&format!("{}    direction TB\n", indent));
                Self::print_flow_tree(model, children, Some(&node.key), depth + 1, out);
                out.push_str(&format!("{}end\n", indent));
            } else {
                let (open, close) = Self::flow_shape(node, node.kind.is_container());
                out.push_str(&format!(
                    "{}{}{}\"{}\"{}\n",
                    indent,
                    node.alias,
                    open,
                    U::mermaid_text(&Self::node_text(node)),
                    close
                ));
            }
        }
    }

    // ── Static entry points (mandatory artifacts only) ───────────────────────

    pub fn export_class_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Class, uma, sta, tca)
    }

    pub fn export_object_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Object, uma, sta, tca)
    }

    pub fn export_component_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Component, uma, sta, tca)
    }

    pub fn export_deployment_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Deployment, uma, sta, tca)
    }

    pub fn export_package_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Package, uma, sta, tca)
    }

    pub fn export_composite_structure_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::CompositeStructure, uma, sta, tca)
    }

    pub fn export_profile_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Profile, uma, sta, tca)
    }

    pub fn export_use_case_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::UseCase, uma, sta, tca)
    }

    pub fn export_activity_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Activity, uma, sta, tca)
    }

    pub fn export_state_machine(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::StateMachine, uma, sta, tca)
    }

    pub fn export_sequence_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Sequence, uma, sta, tca)
    }

    pub fn export_communication_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Communication, uma, sta, tca)
    }

    pub fn export_interaction_overview_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::InteractionOverview, uma, sta, tca)
    }

    pub fn export_timing_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Timing, uma, sta, tca)
    }

    /// Unavailable unless rendered through [`Self::export_with_context`] with a CFA.
    pub fn export_cfg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Cfg, uma, sta, tca)
    }

    /// Unavailable unless rendered through [`Self::export_with_context`] with a PSA.
    pub fn export_robdd_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Robdd, uma, sta, tca)
    }

    /// Unavailable unless rendered through [`Self::export_with_context`] with an SSA artifact.
    pub fn export_dfg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Dfg, uma, sta, tca)
    }

    /// Unavailable unless rendered through [`Self::export_with_context`] with an SSA artifact.
    pub fn export_cdg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::Cdg, uma, sta, tca)
    }

    /// Unavailable unless rendered through [`Self::export_with_context`] with a CGA.
    pub fn export_callgraph_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render_basic(ViewKind::CallGraph, uma, sta, tca)
    }
}

// ── Concrete Mermaid Strategy Implementations ─────────────────────────────────

/// Declares one built-in strategy per view kind and the registry initializer.
macro_rules! mermaid_strategies {
    ($($name:ident => $key:literal, $kind:expr;)*) => {
        $(
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl MermaidDiagramStrategy for $name {
                fn diagram_type(&self) -> &'static str {
                    $key
                }

                fn export(
                    &self,
                    uma: &UMLMetadataArtifact,
                    sta: &SymbolTableArtifact,
                    tca: &TokenCorpusArtifact,
                ) -> String {
                    MermaidExporter::render($kind, &DiagramContext::new(uma, sta, tca))
                }

                fn export_with_context(&self, ctx: &DiagramContext) -> String {
                    MermaidExporter::render($kind, ctx)
                }
            }
        )*

        impl MermaidExporter {
            fn register_builtin(&mut self) {
                $( self.register_strategy(Box::new($name)); )*
            }
        }
    };
}

mermaid_strategies! {
    ClassMermaidStrategy => "class", ViewKind::Class;
    ObjectMermaidStrategy => "object", ViewKind::Object;
    ComponentMermaidStrategy => "component", ViewKind::Component;
    DeploymentMermaidStrategy => "deployment", ViewKind::Deployment;
    PackageMermaidStrategy => "package", ViewKind::Package;
    CompositeStructureMermaidStrategy => "composite", ViewKind::CompositeStructure;
    ProfileMermaidStrategy => "profile", ViewKind::Profile;
    UseCaseMermaidStrategy => "usecase", ViewKind::UseCase;
    ActivityMermaidStrategy => "activity", ViewKind::Activity;
    StateMachineMermaidStrategy => "statemachine", ViewKind::StateMachine;
    SequenceMermaidStrategy => "sequence", ViewKind::Sequence;
    CommunicationMermaidStrategy => "communication", ViewKind::Communication;
    InteractionOverviewMermaidStrategy => "interaction", ViewKind::InteractionOverview;
    TimingMermaidStrategy => "timing", ViewKind::Timing;
    CFGMermaidStrategy => "cfg", ViewKind::Cfg;
    ROBDDMermaidStrategy => "robdd", ViewKind::Robdd;
    DFGMermaidStrategy => "dfg", ViewKind::Dfg;
    CDGMermaidStrategy => "cdg", ViewKind::Cdg;
    CallGraphMermaidStrategy => "callgraph", ViewKind::CallGraph;
}

pub fn theme_init() -> &'static str {
    "%%{init: {'theme': 'dark', 'themeVariables': { 'darkMode': true, 'background': 'transparent', 'primaryColor': '#14171f', 'primaryTextColor': '#ffffff', 'primaryBorderColor': '#38bdf8', 'lineColor': '#38bdf8', 'secondaryColor': '#0f141c', 'tertiaryColor': '#1e2433', 'edgeLabelBackground': '#0c0c0c', 'actorBkg': '#14171f', 'actorBorder': '#facc15', 'actorTextColor': '#ffffff', 'signalColor': '#38bdf8', 'signalTextColor': '#ffffff' }}}%%\n"
}
