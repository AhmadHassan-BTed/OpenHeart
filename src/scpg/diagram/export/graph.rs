//! GraphExporter — Graphviz DOT and GraphML interchange of diagram view models (§10.4).
//!
//! Both formats are printed from the same view IR as PlantUML, Mermaid and JSON, so node
//! and edge sets agree across formats. DOT nests containers as clusters; GraphML is flat
//! with a `parent` attribute and carries keys, qualified names, provenance and source lines
//! for import into Gephi, yEd, Neo4j or NetworkX.

use crate::scpg::diagram::common::DiagramUtils as U;
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::model::*;
use crate::scpg::diagram::views;

pub struct GraphExporter;

impl GraphExporter {
    pub fn export_dot(kind: ViewKind, ctx: &DiagramContext) -> String {
        Self::to_dot(&views::build(kind, ctx), &ctx.input_digest())
    }

    pub fn export_graphml(kind: ViewKind, ctx: &DiagramContext) -> String {
        Self::to_graphml(&views::build(kind, ctx), &ctx.input_digest())
    }

    fn node_label(node: &ViewNode) -> String {
        let mut label = node.label.clone();
        if !node.stereotypes.is_empty() {
            label = format!("«{}»\n{}", node.stereotypes.join(", "), label);
        }
        for member in &node.members {
            label.push('\n');
            label.push_str(member.visibility);
            label.push_str(&member.signature());
        }
        for detail in &node.details {
            label.push('\n');
            label.push_str(detail);
        }
        label
    }

    fn dot_shape(kind: NodeKind) -> &'static str {
        match kind {
            NodeKind::Initial => "circle",
            NodeKind::Final => "doublecircle",
            NodeKind::Decision | NodeKind::Merge => "diamond",
            NodeKind::Fork | NodeKind::Join => "rect",
            NodeKind::ProvidedInterface | NodeKind::BddVariable | NodeKind::SsaValue => "ellipse",
            NodeKind::SsaPhi => "hexagon",
            NodeKind::Package => "folder",
            NodeKind::Component => "component",
            NodeKind::Actor => "house",
            NodeKind::InteractionUse => "box3d",
            NodeKind::ExceptionHandler => "parallelogram",
            _ => "box",
        }
    }

    fn dot_edge_attrs(edge: &ViewEdge) -> Vec<String> {
        let mut attrs = Vec::new();
        match edge.kind {
            EdgeKind::Generalization | EdgeKind::Extension => attrs.push("arrowhead=empty".to_string()),
            EdgeKind::Realization => attrs.push("arrowhead=empty".to_string()),
            EdgeKind::Aggregation => {
                attrs.push("dir=both".to_string());
                attrs.push("arrowtail=odiamond".to_string());
                attrs.push("arrowhead=vee".to_string());
            }
            EdgeKind::Composition => {
                attrs.push("dir=both".to_string());
                attrs.push("arrowtail=diamond".to_string());
                attrs.push("arrowhead=vee".to_string());
            }
            EdgeKind::Nesting => {
                attrs.push("dir=back".to_string());
                attrs.push("arrowtail=odot".to_string());
            }
            kind if kind.is_undirected() => attrs.push("dir=none".to_string()),
            _ => {}
        }
        if edge.kind.is_dashed() {
            attrs.push("style=dashed".to_string());
        }
        if let Some(label) = edge.label.as_ref().or(edge.role.as_ref()) {
            attrs.push(format!("label=\"{}\"", U::dot_escape(label)));
        }
        if let Some(m) = &edge.source_multiplicity {
            attrs.push(format!("taillabel=\"{}\"", U::dot_escape(m)));
        }
        if let Some(m) = &edge.target_multiplicity {
            attrs.push(format!("headlabel=\"{}\"", U::dot_escape(m)));
        }
        attrs
    }

    /// Graphviz DOT document of a view model.
    pub fn to_dot(model: &ViewModel, input_digest: &str) -> String {
        let mut out = format!("digraph \"{}\" {{\n", U::dot_escape(model.kind.key()));
        out.push_str(&format!(
            "  // OpenHeart {} view | provenance: {} | input sha256:{}\n",
            model.kind.key(),
            model.provenance.as_str(),
            input_digest
        ));
        out.push_str(&format!(
            "  graph [label=\"{}\", labelloc=t, compound=true, fontname=\"Helvetica\"];\n",
            U::dot_escape(&model.title)
        ));
        out.push_str("  node [shape=box, fontname=\"Helvetica\", fontsize=10];\n");
        out.push_str("  edge [fontname=\"Helvetica\", fontsize=9];\n");
        if model.is_unavailable() {
            out.push_str(&format!(
                "  notice_unavailable [label=\"{}\"];\n",
                U::dot_escape(&format!("Not available\n{}", model.notes.join("\n")))
            ));
            out.push_str("}\n");
            return out;
        }
        let children = model.children_by_parent();
        Self::dot_tree(model, &children, None, 1, &mut out);
        for edge in &model.edges {
            let (src, dst) = match (model.node(&edge.source), model.node(&edge.target)) {
                (Some(s), Some(d)) => (&s.alias, &d.alias),
                _ => continue,
            };
            let attrs = Self::dot_edge_attrs(edge);
            if attrs.is_empty() {
                out.push_str(&format!("  \"{}\" -> \"{}\";\n", src, dst));
            } else {
                out.push_str(&format!("  \"{}\" -> \"{}\" [{}];\n", src, dst, attrs.join(", ")));
            }
        }
        let mut legend: Vec<String> = Vec::new();
        if model.provenance != Provenance::Derived {
            legend.push(format!("provenance: {}", model.provenance.as_str()));
        }
        legend.extend(model.notes.iter().cloned());
        if model.omitted_count() > 0 {
            legend.push(format!("{} element(s) omitted", model.omitted_count()));
        }
        if !legend.is_empty() {
            out.push_str(&format!(
                "  legend_box [shape=note, label=\"{}\"];\n",
                U::dot_escape(&legend.join("\n"))
            ));
        }
        out.push_str("}\n");
        out
    }

    fn dot_tree(
        model: &ViewModel,
        children: &std::collections::BTreeMap<Option<String>, Vec<usize>>,
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
            if children.contains_key(&Some(node.key.clone())) {
                out.push_str(&format!("{}subgraph \"cluster_{}\" {{\n", indent, node.alias));
                out.push_str(&format!(
                    "{}  label=\"{}\";\n",
                    indent,
                    U::dot_escape(&Self::node_label(node))
                ));
                Self::dot_tree(model, children, Some(&node.key), depth + 1, out);
                out.push_str(&format!("{}}}\n", indent));
            } else {
                out.push_str(&format!(
                    "{}\"{}\" [label=\"{}\", shape={}];\n",
                    indent,
                    node.alias,
                    U::dot_escape(&Self::node_label(node)),
                    Self::dot_shape(node.kind)
                ));
            }
        }
    }

    /// GraphML document of a view model.
    pub fn to_graphml(model: &ViewModel, input_digest: &str) -> String {
        let x = |s: &str| U::xml_escape(s);
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">\n");
        let keys = [
            ("g_kind", "graph", "view_kind", "string"),
            ("g_provenance", "graph", "provenance", "string"),
            ("g_digest", "graph", "input_digest", "string"),
            ("g_notes", "graph", "notes", "string"),
            ("n_label", "node", "label", "string"),
            ("n_kind", "node", "kind", "string"),
            ("n_qname", "node", "qualified_name", "string"),
            ("n_parent", "node", "parent", "string"),
            ("n_stereotypes", "node", "stereotypes", "string"),
            ("n_details", "node", "details", "string"),
            ("n_file", "node", "file", "string"),
            ("n_line", "node", "line", "int"),
            ("n_provenance", "node", "provenance", "string"),
            ("e_kind", "edge", "kind", "string"),
            ("e_label", "edge", "label", "string"),
            ("e_source_mult", "edge", "source_multiplicity", "string"),
            ("e_target_mult", "edge", "target_multiplicity", "string"),
            ("e_order", "edge", "order", "int"),
            ("e_provenance", "edge", "provenance", "string"),
        ];
        for (id, domain, name, ty) in keys {
            out.push_str(&format!(
                "  <key id=\"{}\" for=\"{}\" attr.name=\"{}\" attr.type=\"{}\"/>\n",
                id, domain, name, ty
            ));
        }
        out.push_str(&format!(
            "  <graph id=\"{}\" edgedefault=\"directed\">\n",
            x(model.kind.key())
        ));
        let data = |out: &mut String, key: &str, value: &str| {
            out.push_str(&format!("      <data key=\"{}\">{}</data>\n", key, U::xml_escape(value)));
        };
        out.push_str(&format!("    <data key=\"g_kind\">{}</data>\n", x(model.kind.key())));
        out.push_str(&format!(
            "    <data key=\"g_provenance\">{}</data>\n",
            x(model.provenance.as_str())
        ));
        out.push_str(&format!("    <data key=\"g_digest\">{}</data>\n", x(input_digest)));
        if !model.notes.is_empty() {
            out.push_str(&format!(
                "    <data key=\"g_notes\">{}</data>\n",
                x(&model.notes.join("\n"))
            ));
        }
        for node in &model.nodes {
            out.push_str(&format!("    <node id=\"{}\">\n", x(&node.key)));
            data(&mut out, "n_label", &node.label);
            data(&mut out, "n_kind", node.kind.as_str());
            if let Some(q) = &node.qualified_name {
                data(&mut out, "n_qname", q);
            }
            if let Some(p) = &node.parent {
                data(&mut out, "n_parent", p);
            }
            if !node.stereotypes.is_empty() {
                data(&mut out, "n_stereotypes", &node.stereotypes.join(", "));
            }
            let mut details: Vec<String> = node
                .members
                .iter()
                .map(|m| format!("{}{}", m.visibility, m.signature()))
                .collect();
            details.extend(node.details.iter().cloned());
            if !details.is_empty() {
                data(&mut out, "n_details", &details.join("\n"));
            }
            if let Some(span) = &node.span {
                if let Some(path) = &span.path {
                    data(&mut out, "n_file", path);
                }
                data(&mut out, "n_line", &span.line_start.to_string());
            }
            data(&mut out, "n_provenance", node.provenance.as_str());
            out.push_str("    </node>\n");
        }
        for edge in &model.edges {
            out.push_str(&format!(
                "    <edge id=\"{}\" source=\"{}\" target=\"{}\">\n",
                x(&edge.key),
                x(&edge.source),
                x(&edge.target)
            ));
            data(&mut out, "e_kind", edge.kind.as_str());
            if let Some(label) = edge.label.as_ref().or(edge.role.as_ref()) {
                data(&mut out, "e_label", label);
            }
            if let Some(m) = &edge.source_multiplicity {
                data(&mut out, "e_source_mult", m);
            }
            if let Some(m) = &edge.target_multiplicity {
                data(&mut out, "e_target_mult", m);
            }
            if let Some(order) = edge.order {
                data(&mut out, "e_order", &order.to_string());
            }
            data(&mut out, "e_provenance", edge.provenance.as_str());
            out.push_str("    </edge>\n");
        }
        out.push_str("  </graph>\n</graphml>\n");
        out
    }
}
