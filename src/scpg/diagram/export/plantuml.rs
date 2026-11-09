//! PlantUMLExporter — prints diagram view models as PlantUML (§10.4).
//!
//! Every diagram is rendered from the typed view IR built by `views::build`; this module only
//! decides PlantUML syntax. Elements are declared with view-unique aliases and quoted display
//! names (`class "Builder" as cls_com_a_Outer_Builder`), and relationships are resolved by
//! element key, so same-named types in different packages or nesting levels never merge.
//! Projections that the artifacts cannot support are printed as an explicit notice.

use std::collections::{BTreeMap, HashMap};

use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::diagram::common::DiagramUtils as U;
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::export::plantuml_optimizer::{
    PlantUMLOptimizationOptions, PlantUMLOptimizer, RawEdge,
};
use crate::scpg::diagram::model::*;
use crate::scpg::diagram::views;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Class diagrams with more relationships than this collapse repeated cross-package
/// dependencies into package imports through [`PlantUMLOptimizer`].
const OPTIMIZE_EDGE_THRESHOLD: usize = 150;
/// Omitted element names listed in the legend.
const LEGEND_OMITTED_NAMES: usize = 20;

/// Strategy Interface for PlantUML Diagram Generation (§10.4).
pub trait PlantUMLDiagramStrategy: Send + Sync {
    /// Returns the unique diagram type identifier (e.g. "class", "sequence").
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

pub struct PlantUMLExporter {
    strategies: BTreeMap<String, Box<dyn PlantUMLDiagramStrategy>>,
}

impl Default for PlantUMLExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl PlantUMLExporter {
    /// Initializes the registry with the 14 UML and 5 program-analysis strategies.
    pub fn new() -> Self {
        let mut exporter = Self {
            strategies: BTreeMap::new(),
        };
        exporter.register_builtin();
        exporter
    }

    /// Add or replace a diagram generation strategy.
    pub fn register_strategy(&mut self, strategy: Box<dyn PlantUMLDiagramStrategy>) {
        self.strategies
            .insert(strategy.diagram_type().to_string(), strategy);
    }

    /// Remove a diagram generation strategy.
    pub fn unregister_strategy(
        &mut self,
        diagram_type: &str,
    ) -> Option<Box<dyn PlantUMLDiagramStrategy>> {
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

    pub fn theme_header() -> &'static str {
        "skinparam backgroundColor transparent\n\
skinparam monochrome false\n\
skinparam shadowing false\n\
skinparam roundcorner 6\n\
skinparam defaultFontName \"Space Grotesk\", \"Segoe UI\", sans-serif\n\
skinparam defaultFontColor #e5e5e5\n\
skinparam defaultFontSize 12\n\
skinparam ArrowColor #38bdf8\n\
skinparam ArrowThickness 1.5\n\
skinparam packageStyle rectangle\n\
skinparam PackageBorderColor #facc15\n\
skinparam PackageBackgroundColor #0f141c\n\
skinparam PackageFontColor #facc15\n\
skinparam ClassBorderColor #38bdf8\n\
skinparam ClassBackgroundColor #14171f\n\
skinparam ClassHeaderBackgroundColor #1e2433\n\
skinparam ClassFontColor #ffffff\n\
skinparam ObjectBorderColor #38bdf8\n\
skinparam ObjectBackgroundColor #14171f\n\
skinparam ObjectFontColor #ffffff\n\
skinparam ComponentBorderColor #38bdf8\n\
skinparam ComponentBackgroundColor #14171f\n\
skinparam ComponentFontColor #ffffff\n\
skinparam NodeBorderColor #38bdf8\n\
skinparam NodeBackgroundColor #14171f\n\
skinparam NodeFontColor #ffffff\n\
skinparam StateBorderColor #38bdf8\n\
skinparam StateBackgroundColor #14171f\n\
skinparam StateFontColor #ffffff\n\
skinparam ActivityBorderColor #38bdf8\n\
skinparam ActivityBackgroundColor #14171f\n\
skinparam ActivityFontColor #ffffff\n\
skinparam SequenceLifeLineBorderColor #38bdf8\n\
skinparam SequenceLifeLineBackgroundColor #14171f\n\
skinparam SequenceGroupBorderColor #facc15\n\
skinparam SequenceGroupBackgroundColor #0f141c\n\
skinparam SequenceGroupFontColor #facc15\n\
skinparam ParticipantBorderColor #38bdf8\n\
skinparam ParticipantBackgroundColor #14171f\n\
skinparam ParticipantFontColor #ffffff\n\
skinparam ActorBorderColor #facc15\n\
skinparam ActorBackgroundColor #14171f\n\
skinparam ActorFontColor #ffffff\n\
skinparam UsecaseBorderColor #38bdf8\n\
skinparam UsecaseBackgroundColor #14171f\n\
skinparam UsecaseFontColor #ffffff\n\n"
    }

    /// Builds the view of `kind` from `ctx` and prints it.
    pub fn render(kind: ViewKind, ctx: &DiagramContext) -> String {
        let model = views::build(kind, ctx);
        Self::print(&model, &ctx.input_digest())
    }

    fn render_basic(
        kind: ViewKind,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        Self::render(kind, &DiagramContext::new(uma, sta, tca))
    }

    /// Prints a view model as one PlantUML document.
    pub fn print(model: &ViewModel, input_digest: &str) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str(&format!(
            "' OpenHeart {} view | provenance: {} | input sha256:{}\n",
            model.kind.key(),
            model.provenance.as_str(),
            input_digest
        ));
        if !model.sources.is_empty() {
            out.push_str(&format!("' derived from: {}\n", model.sources.join(", ")));
        }
        out.push_str(Self::theme_header());
        if model.is_unavailable() {
            Self::print_unavailable(model, &mut out);
        } else {
            let extra = match model.kind {
                ViewKind::Class | ViewKind::Profile | ViewKind::Object => {
                    Self::print_classifiers(model, &mut out)
                }
                ViewKind::Sequence => Self::print_sequence(model, &mut out),
                ViewKind::Activity
                | ViewKind::StateMachine
                | ViewKind::Cfg
                | ViewKind::Cdg
                | ViewKind::Dfg
                | ViewKind::Robdd
                | ViewKind::InteractionOverview => Self::print_flow(model, &mut out),
                _ => Self::print_boxes(model, &mut out),
            };
            Self::print_legend(model, &extra, &mut out);
        }
        out.push_str("@enduml\n");
        out
    }

    fn print_title(model: &ViewModel, out: &mut String) {
        out.push_str(&format!("title {}\n\n", U::plantuml_label(&model.title)));
    }

    fn print_unavailable(model: &ViewModel, out: &mut String) {
        Self::print_title(model, out);
        out.push_str("note as notice_unavailable\n");
        out.push_str(&format!("Not available: {}\n", U::single_line(model.kind.title())));
        for note in &model.notes {
            let line = U::single_line(note);
            if line != "end note" {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push_str("end note\n");
    }

    fn print_legend(model: &ViewModel, extra: &[String], out: &mut String) {
        let mut lines: Vec<String> = Vec::new();
        if model.provenance != Provenance::Derived {
            lines.push(format!("provenance: {}", model.provenance.as_str()));
        }
        lines.extend(model.notes.iter().cloned());
        lines.extend(extra.iter().cloned());
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
        if lines.is_empty() {
            return;
        }
        out.push_str("\nlegend bottom left\n");
        for line in lines {
            let line = U::single_line(&line);
            if line != "endlegend" {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push_str("endlegend\n");
    }

    fn stereotype_suffix(stereotypes: &[String]) -> String {
        stereotypes
            .iter()
            .map(|s| format!(" <<{}>>", U::plantuml_stereotype(s)))
            .collect()
    }

    // ── Classifier family (class, object, profile) ──────────────────────────

    fn print_classifiers(model: &ViewModel, out: &mut String) -> Vec<String> {
        Self::print_title(model, out);
        out.push_str("set separator none\nhide empty members\n\n");
        let children = model.children_by_parent();
        Self::print_classifier_tree(model, &children, None, 0, out);
        out.push('\n');

        let mut extra = Vec::new();
        let lines: Vec<String> = model
            .edges
            .iter()
            .filter_map(|e| Self::class_edge_line(model, e))
            .collect();
        if model.kind == ViewKind::Class && lines.len() > OPTIMIZE_EDGE_THRESHOLD {
            let optimized = Self::optimize_class_edges(model);
            if optimized.len() < lines.len() {
                extra.push(format!(
                    "Edge optimizer: {} relationships drawn as {} lines; repeated cross-package dependencies are collapsed into package imports (generalizations are never collapsed).",
                    lines.len(),
                    optimized.len()
                ));
                for line in optimized {
                    out.push_str(&line);
                    out.push('\n');
                }
                return extra;
            }
        }
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        extra
    }

    fn print_classifier_tree(
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
        let indent = "  ".repeat(depth);
        for &i in idxs {
            let node = &model.nodes[i];
            if node.kind.is_container() {
                let label = node.qualified_name.as_deref().unwrap_or(node.label.as_str());
                let style = if node.kind == NodeKind::Package {
                    ""
                } else {
                    " <<Frame>>"
                };
                out.push_str(&format!(
                    "{}package \"{}\" as {}{}{} {{\n",
                    indent,
                    U::plantuml_quoted(label),
                    node.alias,
                    style,
                    Self::stereotype_suffix(&node.stereotypes)
                ));
                Self::print_classifier_tree(model, children, Some(&node.key), depth + 1, out);
                out.push_str(&format!("{}}}\n", indent));
            } else {
                Self::print_classifier(node, &indent, out);
            }
        }
    }

    fn print_classifier(node: &ViewNode, indent: &str, out: &mut String) {
        let (keyword, implied): (&str, Option<&str>) = match node.kind {
            NodeKind::AbstractClass => ("abstract class", None),
            NodeKind::Interface | NodeKind::ProvidedInterface => ("interface", None),
            NodeKind::Enum => ("enum", None),
            NodeKind::Annotation => ("annotation", None),
            NodeKind::Stereotype => ("class", Some("stereotype")),
            NodeKind::Metaclass => ("class", Some("metaclass")),
            NodeKind::Object => ("object", None),
            _ => ("class", None),
        };
        let mut stereotypes: Vec<String> = implied.map(|s| vec![s.to_string()]).unwrap_or_default();
        stereotypes.extend(node.stereotypes.iter().cloned());
        out.push_str(&format!(
            "{}{} \"{}\" as {}{} {{\n",
            indent,
            keyword,
            U::plantuml_quoted(&node.label),
            node.alias,
            Self::stereotype_suffix(&stereotypes)
        ));
        for member in &node.members {
            out.push_str(&format!("{}  {}\n", indent, Self::member_line(member)));
        }
        if node.members.is_empty() {
            for detail in &node.details {
                out.push_str(&format!("{}  {}\n", indent, Self::member_text(detail)));
            }
        }
        out.push_str(&format!("{}}}\n", indent));
    }

    fn member_text(text: &str) -> String {
        U::single_line(text).replace(['{', '}'], "")
    }

    fn member_line(member: &MemberView) -> String {
        let text = Self::member_text(&member.signature());
        if member.kind == MemberKind::Literal {
            return text;
        }
        let mut line = format!("{}{}", member.visibility, text);
        if member.is_static {
            line.push_str(" {static}");
        }
        if member.is_abstract {
            line.push_str(" {abstract}");
        }
        line
    }

    fn class_arrow(kind: EdgeKind) -> &'static str {
        match kind {
            EdgeKind::Generalization => "--|>",
            EdgeKind::Realization => "..|>",
            EdgeKind::Aggregation => "o--",
            EdgeKind::Composition => "*--",
            EdgeKind::Nesting => "+--",
            EdgeKind::Dependency | EdgeKind::Import | EdgeKind::Requires => "..>",
            EdgeKind::Link | EdgeKind::Connector | EdgeKind::Provides => "--",
            _ => "-->",
        }
    }

    fn class_edge_line(model: &ViewModel, edge: &ViewEdge) -> Option<String> {
        let src = &model.node(&edge.source)?.alias;
        let dst = &model.node(&edge.target)?.alias;
        let mut line = src.clone();
        if let Some(m) = &edge.source_multiplicity {
            line.push_str(&format!(" \"{}\"", U::plantuml_quoted(m)));
        }
        line.push_str(&format!(" {} ", Self::class_arrow(edge.kind)));
        if let Some(m) = &edge.target_multiplicity {
            line.push_str(&format!("\"{}\" ", U::plantuml_quoted(m)));
        }
        line.push_str(dst);
        if let Some(text) = edge.label.as_ref().or(edge.role.as_ref()) {
            line.push_str(&format!(" : {}", U::plantuml_label(text)));
        }
        Some(line)
    }

    fn optimize_class_edges(model: &ViewModel) -> Vec<String> {
        let mut class_to_package: HashMap<String, String> = HashMap::new();
        let mut package_class_counts: HashMap<String, usize> = HashMap::new();
        for node in &model.nodes {
            if node.kind.is_container() {
                continue;
            }
            if let Some(parent) = node.parent.as_deref().and_then(|p| model.node(p)) {
                class_to_package.insert(node.alias.clone(), parent.alias.clone());
                *package_class_counts.entry(parent.alias.clone()).or_insert(0) += 1;
            }
        }
        let raw_edges: Vec<RawEdge> = model
            .edges
            .iter()
            .filter_map(|e| {
                let line = Self::class_edge_line(model, e)?;
                Some(RawEdge {
                    src_class: model.node(&e.source)?.alias.clone(),
                    dst_class: model.node(&e.target)?.alias.clone(),
                    rel_op: Self::class_arrow(e.kind).to_string(),
                    full_line: line,
                })
            })
            .collect();
        PlantUMLOptimizer::optimize(
            raw_edges,
            &class_to_package,
            &package_class_counts,
            &PlantUMLOptimizationOptions::default(),
        )
    }

    // ── Flow family (activity, state machine, CFG, CDG, DFG, ROBDD, overview) ─

    fn print_flow(model: &ViewModel, out: &mut String) -> Vec<String> {
        Self::print_title(model, out);
        out.push_str("hide empty description\n\n");
        let children = model.children_by_parent();
        let mut edges_by_scope: BTreeMap<Option<String>, Vec<usize>> = BTreeMap::new();
        for (idx, edge) in model.edges.iter().enumerate() {
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
            edges_by_scope.entry(scope).or_default().push(idx);
        }
        Self::print_flow_scope(model, &children, &edges_by_scope, None, 0, out);
        Vec::new()
    }

    fn print_flow_scope(
        model: &ViewModel,
        children: &BTreeMap<Option<String>, Vec<usize>>,
        edges_by_scope: &BTreeMap<Option<String>, Vec<usize>>,
        scope: Option<&String>,
        depth: usize,
        out: &mut String,
    ) {
        let indent = "  ".repeat(depth);
        let key = scope.cloned();
        if let Some(idxs) = children.get(&key) {
            for &i in idxs {
                let node = &model.nodes[i];
                if node.kind.is_container() {
                    let mut label = node.label.clone();
                    for detail in &node.details {
                        label.push('\n');
                        label.push_str(detail);
                    }
                    out.push_str(&format!(
                        "{}state \"{}\" as {} {{\n",
                        indent,
                        U::plantuml_quoted(&label),
                        node.alias
                    ));
                    Self::print_flow_scope(
                        model,
                        children,
                        edges_by_scope,
                        Some(&node.key),
                        depth + 1,
                        out,
                    );
                    out.push_str(&format!("{}}}\n", indent));
                } else {
                    Self::print_flow_node(node, &indent, out);
                }
            }
        }
        if let Some(edge_idxs) = edges_by_scope.get(&key) {
            for &i in edge_idxs {
                if let Some(line) = Self::flow_edge_line(model, &model.edges[i]) {
                    out.push_str(&indent);
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }

    fn print_flow_node(node: &ViewNode, indent: &str, out: &mut String) {
        match node.kind {
            NodeKind::Initial | NodeKind::Final => {}
            NodeKind::Decision | NodeKind::Merge => {
                out.push_str(&format!("{}state {} <<choice>>\n", indent, node.alias));
            }
            NodeKind::Fork => out.push_str(&format!("{}state {} <<fork>>\n", indent, node.alias)),
            NodeKind::Join => out.push_str(&format!("{}state {} <<join>>\n", indent, node.alias)),
            _ => {
                let mut label = node.label.clone();
                if node.kind == NodeKind::InteractionUse && !label.starts_with("ref") {
                    label = format!("ref: {}", label);
                }
                if !node.stereotypes.is_empty() {
                    label = format!("{} «{}»", label, node.stereotypes.join(", "));
                }
                out.push_str(&format!(
                    "{}state \"{}\" as {}\n",
                    indent,
                    U::plantuml_quoted(&label),
                    node.alias
                ));
                for detail in &node.details {
                    out.push_str(&format!(
                        "{}{} : {}\n",
                        indent,
                        node.alias,
                        U::plantuml_label(detail)
                    ));
                }
            }
        }
    }

    fn flow_ref(model: &ViewModel, key: &str) -> Option<String> {
        let node = model.node(key)?;
        Some(match node.kind {
            NodeKind::Initial | NodeKind::Final => "[*]".to_string(),
            _ => node.alias.clone(),
        })
    }

    fn flow_edge_line(model: &ViewModel, edge: &ViewEdge) -> Option<String> {
        let src = Self::flow_ref(model, &edge.source)?;
        let dst = Self::flow_ref(model, &edge.target)?;
        let arrow = if edge.kind.is_dashed() {
            "-[dashed]->"
        } else {
            "-->"
        };
        let mut line = format!("{} {} {}", src, arrow, dst);
        if let Some(label) = &edge.label {
            line.push_str(&format!(" : {}", U::plantuml_label(label)));
        }
        Some(line)
    }

    // ── Box family (package, component, composite, call graph, communication) ─

    fn print_boxes(model: &ViewModel, out: &mut String) -> Vec<String> {
        Self::print_title(model, out);
        let children = model.children_by_parent();
        Self::print_box_tree(model, &children, None, 0, out);
        out.push('\n');
        for edge in &model.edges {
            let (src, dst) = match (model.node(&edge.source), model.node(&edge.target)) {
                (Some(s), Some(d)) => (&s.alias, &d.alias),
                _ => continue,
            };
            let arrow = match edge.kind {
                EdgeKind::Generalization => "--|>",
                EdgeKind::Realization => "..|>",
                EdgeKind::Link | EdgeKind::Connector | EdgeKind::Provides => "--",
                kind if kind.is_dashed() => "..>",
                _ => "-->",
            };
            out.push_str(&format!("{} {} {}", src, arrow, dst));
            if let Some(label) = &edge.label {
                out.push_str(&format!(" : {}", U::plantuml_label(label)));
            }
            out.push('\n');
        }
        Vec::new()
    }

    fn print_box_tree(
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
        let indent = "  ".repeat(depth);
        for &i in idxs {
            let node = &model.nodes[i];
            let mut label = node.label.clone();
            for detail in &node.details {
                label.push('\n');
                label.push_str(detail);
            }
            let stereo = Self::stereotype_suffix(&node.stereotypes);
            let has_children = children.contains_key(&Some(node.key.clone()));
            if node.kind.is_container() || has_children {
                let keyword = if node.kind == NodeKind::Package {
                    "package"
                } else {
                    "rectangle"
                };
                out.push_str(&format!(
                    "{}{} \"{}\" as {}{} {{\n",
                    indent,
                    keyword,
                    U::plantuml_quoted(&label),
                    node.alias,
                    stereo
                ));
                Self::print_box_tree(model, children, Some(&node.key), depth + 1, out);
                out.push_str(&format!("{}}}\n", indent));
                continue;
            }
            let keyword = match node.kind {
                NodeKind::Component => "component",
                NodeKind::ProvidedInterface | NodeKind::Interface => "interface",
                NodeKind::Actor => "actor",
                NodeKind::Package => "package",
                _ => "rectangle",
            };
            if keyword == "package" {
                out.push_str(&format!(
                    "{}package \"{}\" as {}{} {{\n{}}}\n",
                    indent,
                    U::plantuml_quoted(&label),
                    node.alias,
                    stereo,
                    indent
                ));
            } else {
                out.push_str(&format!(
                    "{}{} \"{}\" as {}{}\n",
                    indent,
                    keyword,
                    U::plantuml_quoted(&label),
                    node.alias,
                    stereo
                ));
            }
        }
    }

    // ── Sequence family ──────────────────────────────────────────────────────

    fn print_sequence(model: &ViewModel, out: &mut String) -> Vec<String> {
        let mut declared: Vec<&str> = Vec::new();
        for interaction in &model.interactions {
            for lifeline in &interaction.lifelines {
                if !declared.contains(&lifeline.as_str()) {
                    declared.push(lifeline.as_str());
                }
            }
        }
        for (page, interaction) in model.interactions.iter().enumerate() {
            if page == 0 {
                out.push_str(&format!(
                    "title {}\n\n",
                    U::plantuml_label(&format!("{} — {}", model.title, interaction.title))
                ));
                for key in &declared {
                    if let Some(node) = model.node(key) {
                        let keyword = if node.kind == NodeKind::Actor {
                            "actor"
                        } else {
                            "participant"
                        };
                        out.push_str(&format!(
                            "{} \"{}\" as {}\n",
                            keyword,
                            U::plantuml_quoted(&node.label),
                            node.alias
                        ));
                    }
                }
                out.push('\n');
            } else {
                out.push_str(&format!(
                    "\nnewpage {}\n",
                    U::plantuml_label(&interaction.title)
                ));
            }
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
                        let arrow = if *is_async { "->>" } else { "->" };
                        out.push_str(&format!(
                            "{} {} {} : {}\n",
                            Self::seq_alias(model, from),
                            arrow,
                            Self::seq_alias(model, to),
                            U::plantuml_label(&format!("{} {}", number, label))
                        ));
                        if *activates {
                            out.push_str(&format!("activate {}\n", Self::seq_alias(model, to)));
                        }
                    }
                    InteractionEvent::Reply {
                        from,
                        to,
                        label,
                        deactivates,
                    } => {
                        out.push_str(&format!(
                            "{} --> {} : {}\n",
                            Self::seq_alias(model, from),
                            Self::seq_alias(model, to),
                            U::plantuml_label(label)
                        ));
                        if *deactivates {
                            out.push_str(&format!("deactivate {}\n", Self::seq_alias(model, from)));
                        }
                    }
                    InteractionEvent::Deactivate { lifeline } => {
                        out.push_str(&format!("deactivate {}\n", Self::seq_alias(model, lifeline)));
                    }
                    InteractionEvent::FragmentStart { kind, guard } => {
                        match guard {
                            Some(g) => out.push_str(&format!(
                                "{} {}\n",
                                kind.as_str(),
                                U::plantuml_label(g)
                            )),
                            None => out.push_str(&format!("{}\n", kind.as_str())),
                        }
                    }
                    // PlantUML separates the operands of every group kind (`alt`, `par`,
                    // `critical`, ...) with `else`.
                    InteractionEvent::FragmentElse { guard } => match guard {
                        Some(g) => out.push_str(&format!("else {}\n", U::plantuml_label(g))),
                        None => out.push_str("else\n"),
                    },
                    InteractionEvent::FragmentEnd => out.push_str("end\n"),
                    InteractionEvent::Note { over, text } => {
                        out.push_str(&format!(
                            "note over {} : {}\n",
                            Self::seq_alias(model, over),
                            U::plantuml_label(text)
                        ));
                    }
                }
            }
            if interaction.omitted_messages > 0 {
                if let Some(first) = interaction.lifelines.first() {
                    out.push_str(&format!(
                        "note over {} : {} further call(s) omitted by the message budget\n",
                        Self::seq_alias(model, first),
                        interaction.omitted_messages
                    ));
                }
            }
        }
        Vec::new()
    }

    fn seq_alias(model: &ViewModel, key: &str) -> String {
        model
            .node(key)
            .map(|n| n.alias.clone())
            .unwrap_or_else(|| U::sanitize(key))
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

    pub fn export_state_machine_diagram(
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

// ── Concrete Strategy Implementations ─────────────────────────────────────────

/// Declares one built-in strategy per view kind and the registry initializer.
macro_rules! plantuml_strategies {
    ($($name:ident => $key:literal, $kind:expr;)*) => {
        $(
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl PlantUMLDiagramStrategy for $name {
                fn diagram_type(&self) -> &'static str {
                    $key
                }

                fn export(
                    &self,
                    uma: &UMLMetadataArtifact,
                    sta: &SymbolTableArtifact,
                    tca: &TokenCorpusArtifact,
                ) -> String {
                    PlantUMLExporter::render($kind, &DiagramContext::new(uma, sta, tca))
                }

                fn export_with_context(&self, ctx: &DiagramContext) -> String {
                    PlantUMLExporter::render($kind, ctx)
                }
            }
        )*

        impl PlantUMLExporter {
            fn register_builtin(&mut self) {
                $( self.register_strategy(Box::new($name)); )*
            }
        }
    };
}

plantuml_strategies! {
    ClassDiagramStrategy => "class", ViewKind::Class;
    ObjectDiagramStrategy => "object", ViewKind::Object;
    ComponentDiagramStrategy => "component", ViewKind::Component;
    DeploymentDiagramStrategy => "deployment", ViewKind::Deployment;
    PackageDiagramStrategy => "package", ViewKind::Package;
    CompositeStructureDiagramStrategy => "composite", ViewKind::CompositeStructure;
    ProfileDiagramStrategy => "profile", ViewKind::Profile;
    UseCaseDiagramStrategy => "usecase", ViewKind::UseCase;
    ActivityDiagramStrategy => "activity", ViewKind::Activity;
    StateMachineDiagramStrategy => "statemachine", ViewKind::StateMachine;
    SequenceDiagramStrategy => "sequence", ViewKind::Sequence;
    CommunicationDiagramStrategy => "communication", ViewKind::Communication;
    InteractionOverviewDiagramStrategy => "interaction", ViewKind::InteractionOverview;
    TimingDiagramStrategy => "timing", ViewKind::Timing;
    CFGDiagramStrategy => "cfg", ViewKind::Cfg;
    ROBDDDiagramStrategy => "robdd", ViewKind::Robdd;
    DFGDiagramStrategy => "dfg", ViewKind::Dfg;
    CDGDiagramStrategy => "cdg", ViewKind::Cdg;
    CallGraphDiagramStrategy => "callgraph", ViewKind::CallGraph;
}
