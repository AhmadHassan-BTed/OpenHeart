//! Abstract Factory & Universal Diagram Engine (§10.4).
//!
//! Provides the Abstract Factory pattern for creating multi-format diagram exporters
//! (PlantUML, Mermaid, XMI, JSON) and a unified export dispatch engine.

use std::collections::HashMap;

use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::diagram::export::{
    json::JSONExporter, mermaid::MermaidExporter, plantuml::PlantUMLExporter, xmi::XMIExporter,
};
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Supported diagram serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagramFormat {
    PlantUML,
    Mermaid,
    XMI,
    JSON,
}

impl DiagramFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PlantUML => "plantuml",
            Self::Mermaid => "mermaid",
            Self::XMI => "xmi",
            Self::JSON => "json",
        }
    }

    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::PlantUML => "puml",
            Self::Mermaid => "mmd",
            Self::XMI => "xmi",
            Self::JSON => "json",
        }
    }
}

/// Abstract Factory Trait for creating diagram exporters.
pub trait DiagramExporterFactory: Send + Sync {
    /// Returns the target format created by this factory.
    fn format(&self) -> DiagramFormat;

    /// Exports a specific diagram type using the factory's concrete engine.
    fn export_diagram(
        &self,
        diagram_type: &str,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Option<String>;

    /// Exports all supported diagrams in the factory's format.
    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String>;
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
        let exporter = PlantUMLExporter::new();
        exporter.export(diagram_type, uma, sta, tca)
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        let exporter = PlantUMLExporter::new();
        exporter.export_all(uma, sta, tca)
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
        let exporter = MermaidExporter::new();
        exporter.export(diagram_type, uma, sta, tca)
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        let exporter = MermaidExporter::new();
        exporter.export_all(uma, sta, tca)
    }
}

/// Concrete Factory for XMI 2.5 Exporters.
#[derive(Default, Debug, Clone)]
pub struct XMIFactory;

impl DiagramExporterFactory for XMIFactory {
    fn format(&self) -> DiagramFormat {
        DiagramFormat::XMI
    }

    fn export_diagram(
        &self,
        _diagram_type: &str,
        uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> Option<String> {
        Some(XMIExporter::export_class_diagram(&uma.classes))
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert(
            "class".to_string(),
            XMIExporter::export_class_diagram(&uma.classes),
        );
        map
    }
}

/// Concrete Factory for JSON AST/Graph Exporters.
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
        let graph_ir = match diagram_type {
            "package" => JSONExporter::export_package_diagram(uma, sta, tca),
            "sequence" => JSONExporter::export_sequence_diagram(uma, sta, tca),
            "state" | "statemachine" | "state_machine" => JSONExporter::export_state_diagram(uma, sta, tca),
            "activity" => JSONExporter::export_activity_diagram(uma, sta, tca),
            "component" => JSONExporter::export_component_diagram(uma, sta, tca),
            "deployment" => JSONExporter::export_deployment_diagram(uma, sta, tca),
            "usecase" | "use_case" => JSONExporter::export_usecase_diagram(uma, sta, tca),
            "object" => JSONExporter::export_object_diagram(uma, sta, tca),
            "communication" => JSONExporter::export_sequence_diagram(uma, sta, tca),
            "timing" => JSONExporter::export_state_diagram(uma, sta, tca),
            _ => JSONExporter::export_class_diagram(uma, sta, tca),
        };
        serde_json::to_string_pretty(&graph_ir).ok()
    }

    fn export_all_diagrams(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let types = [
            ("class", JSONExporter::export_class_diagram(uma, sta, tca)),
            ("package", JSONExporter::export_package_diagram(uma, sta, tca)),
            ("sequence", JSONExporter::export_sequence_diagram(uma, sta, tca)),
            ("state", JSONExporter::export_state_diagram(uma, sta, tca)),
            ("activity", JSONExporter::export_activity_diagram(uma, sta, tca)),
            ("component", JSONExporter::export_component_diagram(uma, sta, tca)),
            ("deployment", JSONExporter::export_deployment_diagram(uma, sta, tca)),
            ("usecase", JSONExporter::export_usecase_diagram(uma, sta, tca)),
            ("object", JSONExporter::export_object_diagram(uma, sta, tca)),
        ];

        for (name, ir) in types {
            if let Ok(json) = serde_json::to_string_pretty(&ir) {
                map.insert(name.to_string(), json);
            }
        }
        map
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
        engine
    }

    /// Registers a custom format exporter factory.
    pub fn register_factory(&mut self, factory: Box<dyn DiagramExporterFactory>) {
        self.factories.insert(factory.format(), factory);
    }

    /// Exports a specific diagram in the requested format.
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

    /// Exports all diagrams in the requested format.
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
}
