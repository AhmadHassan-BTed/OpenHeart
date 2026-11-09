//! Abstract Factory & Universal Diagram Engine (§10.4).
//!
//! Provides the Abstract Factory pattern for creating multi-format diagram exporters
//! (PlantUML, Mermaid, XMI, JSON, DOT, GraphML) and a unified export dispatch engine.
//!
//! The `export_diagram`/`export_all_diagrams` entry points take only the mandatory
//! artifacts (`.uma`, `.sta`, `.tca`), so the compiler-IR views (CFG, DFG, CDG, call graph,
//! ROBDD) come out as explicit "not available" diagrams. Pass a [`DiagramContext`] holding
//! the `.cfa`, `.ssa`, `.cga`, `.psa` and `.tra` artifacts to the `*_with_context` entry
//! points to render them from the analysis results.

use std::collections::HashMap;

use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::export::{
    graph::GraphExporter, json::JSONExporter, mermaid::MermaidExporter, plantuml::PlantUMLExporter,
    xmi::XMIExporter,
};
use crate::scpg::diagram::model::ViewKind;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Supported diagram serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagramFormat {
    PlantUML,
    Mermaid,
    XMI,
    JSON,
    DOT,
    GraphML,
}

impl DiagramFormat {
    pub const ALL: [DiagramFormat; 6] = [
        DiagramFormat::PlantUML,
        DiagramFormat::Mermaid,
        DiagramFormat::XMI,
        DiagramFormat::JSON,
        DiagramFormat::DOT,
        DiagramFormat::GraphML,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PlantUML => "plantuml",
            Self::Mermaid => "mermaid",
            Self::XMI => "xmi",
            Self::JSON => "json",
            Self::DOT => "dot",
            Self::GraphML => "graphml",
        }
    }

    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::PlantUML => "puml",
            Self::Mermaid => "mmd",
            Self::XMI => "xmi",
            Self::JSON => "json",
            Self::DOT => "dot",
            Self::GraphML => "graphml",
        }
    }

    /// Parses a format name or file extension.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "plantuml" | "puml" => Some(Self::PlantUML),
            "mermaid" | "mmd" => Some(Self::Mermaid),
            "xmi" | "uml" => Some(Self::XMI),
            "json" => Some(Self::JSON),
            "dot" | "graphviz" | "gv" => Some(Self::DOT),
            "graphml" => Some(Self::GraphML),
            _ => None,
        }
    }
}

/// Abstract Factory Trait for creating diagram exporters.
pub trait DiagramExporterFactory: Send + Sync {
    /// Returns the target format created by this factory.
    fn format(&self) -> DiagramFormat;

    /// Exports a specific diagram type from the mandatory artifacts.
    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String>;

    /// Exports all supported diagrams from the mandatory artifacts.
    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String>;

    /// Exports a specific diagram type from the full artifact context. Factories that only
    /// use the mandatory artifacts keep the default, which forwards to `export_diagram`.
    fn export_diagram_with_context(
        &self,
        diagram_type: &str,
        ctx: &DiagramContext,
    ) -> Option<String> {
        self.export_diagram(diagram_type, ctx.uma, ctx.sta, ctx.tca)
    }

    /// Exports all supported diagrams from the full artifact context.
    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        self.export_all_diagrams(ctx.uma, ctx.sta, ctx.tca)
    }
}

// ── Concrete Factories ────────────────────────────────────────────────────────

/// Concrete Factory for PlantUML Exporters.
#[derive(Default, Debug, Clone)]
pub struct PlantUMLFactory;

impl DiagramExporterFactory for PlantUMLFactory {
    fn format(&self) -> DiagramFormat {
        DiagramFormat::PlantUML
    }

    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        PlantUMLExporter::new().export(diagram_type, uma, sta, tca)
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        PlantUMLExporter::new().export_all(uma, sta, tca)
    }

    fn export_diagram_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        PlantUMLExporter::new().export_with_context(diagram_type, ctx)
    }

    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        PlantUMLExporter::new().export_all_with_context(ctx)
    }
}

/// Concrete Factory for Mermaid Exporters.
#[derive(Default, Debug, Clone)]
pub struct MermaidFactory;

impl DiagramExporterFactory for MermaidFactory {
    fn format(&self) -> DiagramFormat {
        DiagramFormat::Mermaid
    }

    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        MermaidExporter::new().export(diagram_type, uma, sta, tca)
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        MermaidExporter::new().export_all(uma, sta, tca)
    }

    fn export_diagram_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        MermaidExporter::new().export_with_context(diagram_type, ctx)
    }

    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        MermaidExporter::new().export_all_with_context(ctx)
    }
}

/// Concrete Factory for OMG XMI 2.5.1 model interchange. XMI carries the whole UML class
/// model, so only the "class", "package" and "model" types are served; other diagram types
/// have no XMI projection and return `None`.
#[derive(Default, Debug, Clone)]
pub struct XMIFactory;

impl XMIFactory {
    fn serves(diagram_type: &str) -> bool {
        matches!(
            diagram_type.trim().to_ascii_lowercase().as_str(),
            "class" | "package" | "model" | "uml"
        )
    }
}

impl DiagramExporterFactory for XMIFactory {
    fn format(&self) -> DiagramFormat {
        DiagramFormat::XMI
    }

    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        self.export_diagram_with_context(diagram_type, &DiagramContext::new(uma, sta, tca))
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        self.export_all_with_context(&DiagramContext::new(uma, sta, tca))
    }

    fn export_diagram_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        if Self::serves(diagram_type) {
            Some(XMIExporter::export_model(ctx))
        } else {
            None
        }
    }

    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("class".to_string(), XMIExporter::export_model(ctx));
        map
    }
}

/// Concrete Factory for the versioned JSON Graph IR. Unknown diagram types return `None`.
#[derive(Default, Debug, Clone)]
pub struct JSONFactory;

impl DiagramExporterFactory for JSONFactory {
    fn format(&self) -> DiagramFormat {
        DiagramFormat::JSON
    }

    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        self.export_diagram_with_context(diagram_type, &DiagramContext::new(uma, sta, tca))
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        self.export_all_with_context(&DiagramContext::new(uma, sta, tca))
    }

    fn export_diagram_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        JSONExporter::export_type(diagram_type, ctx)
            .and_then(|ir| serde_json::to_string_pretty(&ir).ok())
    }

    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for kind in ViewKind::ALL {
            if let Ok(json) = serde_json::to_string_pretty(&JSONExporter::export(kind, ctx)) {
                map.insert(kind.json_type().to_string(), json);
            }
        }
        map
    }
}

/// Concrete Factory for graph interchange (Graphviz DOT or GraphML).
#[derive(Debug, Clone)]
pub struct GraphFactory {
    format: DiagramFormat,
}

impl GraphFactory {
    pub fn dot() -> Self {
        Self {
            format: DiagramFormat::DOT,
        }
    }

    pub fn graphml() -> Self {
        Self {
            format: DiagramFormat::GraphML,
        }
    }

    fn print(&self, kind: ViewKind, ctx: &DiagramContext) -> String {
        if self.format == DiagramFormat::GraphML {
            GraphExporter::export_graphml(kind, ctx)
        } else {
            GraphExporter::export_dot(kind, ctx)
        }
    }
}

impl DiagramExporterFactory for GraphFactory {
    fn format(&self) -> DiagramFormat {
        self.format
    }

    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        self.export_diagram_with_context(diagram_type, &DiagramContext::new(uma, sta, tca))
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        self.export_all_with_context(&DiagramContext::new(uma, sta, tca))
    }

    fn export_diagram_with_context(&self, diagram_type: &str, ctx: &DiagramContext) -> Option<String> {
        ViewKind::parse(diagram_type).map(|kind| self.print(kind, ctx))
    }

    fn export_all_with_context(&self, ctx: &DiagramContext) -> HashMap<String, String> {
        ViewKind::ALL
            .iter()
            .map(|&kind| (kind.key().to_string(), self.print(kind, ctx)))
            .collect()
    }
}

// ── Unified Universal Diagram Engine ──────────────────────────────────────────

/// Universal Diagram Engine orchestrating multi-format abstract factories.
pub struct UniversalDiagramEngine {
    factories: HashMap<DiagramFormat, Box<dyn DiagramExporterFactory>>,
}

impl Default for UniversalDiagramEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl UniversalDiagramEngine {
    /// Creates a new engine instance initialized with all standard format factories.
    pub fn new() -> Self {
        let mut engine = Self {
            factories: HashMap::new(),
        };
        engine.register_factory(Box::new(PlantUMLFactory));
        engine.register_factory(Box::new(MermaidFactory));
        engine.register_factory(Box::new(XMIFactory));
        engine.register_factory(Box::new(JSONFactory));
        engine.register_factory(Box::new(GraphFactory::dot()));
        engine.register_factory(Box::new(GraphFactory::graphml()));
        engine
    }

    /// Registers a custom format exporter factory.
    pub fn register_factory(&mut self, factory: Box<dyn DiagramExporterFactory>) {
        self.factories.insert(factory.format(), factory);
    }

    /// Registered formats in `DiagramFormat::ALL` order.
    pub fn formats(&self) -> Vec<DiagramFormat> {
        DiagramFormat::ALL
            .iter()
            .copied()
            .filter(|f| self.factories.contains_key(f))
            .collect()
    }

    /// Exports a specific diagram in the requested format from the mandatory artifacts.
    pub fn export_diagram(
        &self,
        format: DiagramFormat,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        self.factories
            .get(&format)
            .and_then(|f| f.export_diagram(diagram_type, uma, sta, tca))
    }

    /// Exports all diagrams in the requested format from the mandatory artifacts.
    pub fn export_all(
        &self,
        format: DiagramFormat,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        self.factories
            .get(&format)
            .map(|f| f.export_all_diagrams(uma, sta, tca))
            .unwrap_or_default()
    }

    /// Exports a specific diagram in the requested format from the full artifact context.
    pub fn export_diagram_with_context(
        &self,
        format: DiagramFormat,
        diagram_type: &str,
        ctx: &DiagramContext,
    ) -> Option<String> {
        self.factories
            .get(&format)
            .and_then(|f| f.export_diagram_with_context(diagram_type, ctx))
    }

    /// Exports all diagrams in the requested format from the full artifact context.
    pub fn export_all_with_context(
        &self,
        format: DiagramFormat,
        ctx: &DiagramContext,
    ) -> HashMap<String, String> {
        self.factories
            .get(&format)
            .map(|f| f.export_all_with_context(ctx))
            .unwrap_or_default()
    }
}
