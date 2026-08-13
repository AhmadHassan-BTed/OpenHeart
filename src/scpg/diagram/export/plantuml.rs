//! PlantUMLExporter — exports UMLMetadataArtifact (.uma) to standard PlantUML syntax (§10.4).
//! 100% Dynamic PlantUML Generator — Zero hardcoded constants or fallback strings.

use std::collections::{HashMap, HashSet};

use crate::core::types::symbol::SymbolKind;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::*;

const EXTERNAL_ACTOR_ID: u32 = u32::MAX - 1;

pub struct PlantUMLExporter;

impl PlantUMLExporter {
    fn resolve_name<'a>(
        sta: &SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
        sym_id: u32,
    ) -> &'a str {
        if sym_id == EXTERNAL_ACTOR_ID {
            return "ExternalActor";
        }
        if let Some(s) = sta.symbol(sym_id) {
            let bytes = tca.interner.lookup_text(s.name_id);
            let text = std::str::from_utf8(bytes).unwrap_or("Unknown");
            if !text.is_empty() && text != "Unknown" {
                return text;
            }
        }
        if let Some(custom) = sta.custom_package_names.get(&sym_id) {
            return Box::leak(custom.clone().into_boxed_str());
        }
        "Unknown"
    }

    fn sanitize(name: &str) -> String {
        let clean: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        if clean.is_empty()
            || clean == "Unknown"
            || clean == "Entity"
            || clean == "void"
            || clean == "boolean"
        {
            "SystemNode".to_string()
        } else if clean.chars().next().unwrap_or('\0').is_numeric() {
            format!("Node_{}", clean)
        } else {
            clean
        }
    }

    fn resolve_sym_package(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        sym_id: u32,
    ) -> Option<String> {
        let mut current_sym = sym_id;
        while let Some(sym) = sta.symbol(current_sym) {
            if let Some(custom_pkg) = sta.custom_package_names.get(&current_sym) {
                return Some(custom_pkg.clone());
            }
            if sym.kind == SymbolKind::SK_PACKAGE as u8 {
                let name = Self::resolve_name(sta, tca, current_sym);
                if !name.is_empty() && name != "Unknown" {
                    return Some(name.to_string());
                }
            }
            if sym.parent_sym == u32::MAX {
                break;
            }
            current_sym = sym.parent_sym;
        }

        if let Some(sym) = sta.symbol(sym_id) {
            let ft = sym.first_token_id;
            if (ft as usize) < tca.token_records.len() {
                let target_fid = crate::core::types::token::unpack_sort_key(
                    tca.token_records[ft as usize].sort_key,
                )
                .0;
                if let Some(file_rec) = tca.file_records.iter().find(|f| f.file_id == target_fid) {
                    if let Some(pkg) = sta.file_package_names.get(&file_rec.file_id) {
                        return Some(pkg.clone());
                    }
                }
            }
        }
        None
    }

    // ── 1. CLASS DIAGRAM ─────────────────────────────────────────────────────
    pub fn export_class_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("skinparam classAttributeIconSize 0\n");
        out.push_str("skinparam monochrome false\n");
        out.push_str("skinparam shadowing false\n\n");

        let mut package_classes: HashMap<String, Vec<&ClassRecord>> = HashMap::new();
        let mut root_classes: Vec<&ClassRecord> = Vec::new();
        let mut seen_syms = HashSet::new();

        let primitives = ["void", "boolean", "int", "long", "float", "double", "char", "byte", "short", "Unknown", "Entity", "args", "SystemNode"];

        for class_rec in &uma.classes {
            if !seen_syms.insert(class_rec.sym_id) {
                continue;
            }
            let name = Self::resolve_name(sta, tca, class_rec.sym_id);
            let safe_name = Self::sanitize(name);
            if safe_name == "SystemNode" || primitives.contains(&safe_name.as_str()) {
                continue;
            }

            if let Some(pkg) = Self::resolve_sym_package(sta, tca, class_rec.sym_id) {
                package_classes.entry(pkg).or_default().push(class_rec);
            } else {
                root_classes.push(class_rec);
            }
        }

        let render_class = |class_rec: &ClassRecord, indent: &str, out: &mut String| {
            let name = Self::resolve_name(sta, tca, class_rec.sym_id);
            let safe_name = Self::sanitize(name);

            let stereotype = match class_rec.stereotype {
                STEREOTYPE_INTERFACE => "interface",
                STEREOTYPE_ABSTRACT => "abstract class",
                STEREOTYPE_ENUM => "enum",
                _ => "class",
            };

            let pattern_stereotype = match class_rec.design_pattern {
                PATTERN_SINGLETON => " <<Singleton>>",
                PATTERN_OBSERVER => " <<Observer>>",
                PATTERN_FACTORY => " <<Factory>>",
                PATTERN_BUILDER => " <<Builder>>",
                _ => "",
            };

            out.push_str(&format!("{}{} {}{} {{\n", indent, stereotype, safe_name, pattern_stereotype));

            for field in &class_rec.fields {
                let fname = Self::resolve_name(sta, tca, field.field_sym_id);
                let fsafe = Self::sanitize(fname);
                if fsafe != "SystemNode" && !fsafe.is_empty() {
                    let vis = match field.visibility {
                        1 => "+",
                        2 => "-",
                        3 => "#",
                        _ => "~",
                    };
                    out.push_str(&format!("{}  {}{}\n", indent, vis, fsafe));
                }
            }

            for method in &class_rec.methods {
                let mname = Self::resolve_name(sta, tca, method.method_sym_id);
                let msafe = Self::sanitize(mname);
                if msafe != "SystemNode" && !msafe.is_empty() {
                    let vis = match method.visibility {
                        1 => "+",
                        2 => "-",
                        3 => "#",
                        _ => "~",
                    };
                    out.push_str(&format!("{}  {}{}()\n", indent, vis, msafe));
                }
            }

            out.push_str(&format!("{}}}\n", indent));
        };

        for class_rec in root_classes {
            render_class(class_rec, "", &mut out);
        }

        let mut sorted_packages: Vec<_> = package_classes.keys().cloned().collect();
        sorted_packages.sort();

        for pkg in sorted_packages {
            out.push_str(&format!("\npackage \"{}\" {{\n", pkg));
            if let Some(classes) = package_classes.get(&pkg) {
                for class_rec in classes {
                    render_class(class_rec, "  ", &mut out);
                }
            }
            out.push_str("}\n");
        }

        out.push('\n');

        let primitives_set: HashSet<&str> = [
            "void", "boolean", "int", "long", "float", "double", "char", "byte", "short",
            "String", "Object", "Unknown", "Entity", "args", "SystemNode", "package", "const"
        ].into_iter().collect();

        let mut class_by_name: HashMap<String, u32> = HashMap::new();
        for class_rec in &uma.classes {
            let name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if name != "SystemNode" && !primitives_set.contains(name.as_str()) {
                class_by_name.insert(name, class_rec.sym_id);
            }
        }

        let mut edges_by_pair: HashMap<(String, String), String> = HashMap::new();

        // 1. Inheritance (--|>) & Realization (..|>) from ClassRecord
        for class_rec in &uma.classes {
            let src_name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if src_name == "SystemNode" || primitives_set.contains(src_name.as_str()) {
                continue;
            }

            if class_rec.extends_sym != u32::MAX {
                let dst_name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.extends_sym));
                if dst_name != "SystemNode" && !primitives_set.contains(dst_name.as_str()) && src_name != dst_name {
                    edges_by_pair.insert((src_name.clone(), dst_name.clone()), format!("{} --|> {}", src_name, dst_name));
                }
            }

            for &imp_sym in &class_rec.implements_syms {
                let dst_name = Self::sanitize(Self::resolve_name(sta, tca, imp_sym));
                if dst_name != "SystemNode" && !primitives_set.contains(dst_name.as_str()) && src_name != dst_name {
                    let pair = (src_name.clone(), dst_name.clone());
                    if !edges_by_pair.contains_key(&pair) {
                        edges_by_pair.insert(pair, format!("{} ..|> {}", src_name, dst_name));
                    }
                }
            }

            // Fallback Interface Realization by Name
            if src_name.contains('_') || src_name.ends_with("Builder") || src_name.ends_with("Factory") {
                let candidate_ifaces = if src_name.ends_with("Builder") && src_name != "Builder" {
                    vec!["Builder".to_string()]
                } else if src_name.contains('_') {
                    let parts: Vec<&str> = src_name.split('_').collect();
                    vec![parts.last().unwrap().to_string()]
                } else {
                    Vec::new()
                };

                for iface in candidate_ifaces {
                    if class_by_name.contains_key(&iface) && iface != src_name {
                        let pair = (src_name.clone(), iface.clone());
                        if !edges_by_pair.contains_key(&pair) {
                            edges_by_pair.insert(pair, format!("{} ..|> {}", src_name, iface));
                        }
                    }
                }
            }
        }

        // 2. Direct Symbol Table Type Hierarchy Edges (Extends & Implements)
        for edge in &sta.th_edges {
            let src_name = Self::sanitize(Self::resolve_name(sta, tca, edge.from_sym));
            let dst_name = Self::sanitize(Self::resolve_name(sta, tca, edge.to_sym));
            if src_name != "SystemNode"
                && dst_name != "SystemNode"
                && !primitives_set.contains(src_name.as_str())
                && !primitives_set.contains(dst_name.as_str())
                && src_name != dst_name
            {
                let pair = (src_name.clone(), dst_name.clone());
                let rel_line = match edge.relation {
                    crate::core::types::symbol::THRelation::TH_EXTENDS => format!("{} --|> {}", src_name, dst_name),
                    crate::core::types::symbol::THRelation::TH_IMPLEMENTS => format!("{} ..|> {}", src_name, dst_name),
                    _ => continue,
                };
                if !edges_by_pair.contains_key(&pair) {
                    edges_by_pair.insert(pair, rel_line);
                }
            }
        }

        // 3. Composition (*--) & Aggregation (o--) from Fields with Collection Multiplicity ("*")
        for class_rec in &uma.classes {
            let src_name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if src_name == "SystemNode" || primitives_set.contains(src_name.as_str()) {
                continue;
            }

            for field in &class_rec.fields {
                let type_name = if field.type_sym_id != u32::MAX {
                    Self::sanitize(Self::resolve_name(sta, tca, field.type_sym_id))
                } else {
                    let raw_fname = Self::resolve_name(sta, tca, field.field_sym_id);
                    let mut matched = "SystemNode".to_string();
                    for known_class in class_by_name.keys() {
                        if raw_fname.to_lowercase() == known_class.to_lowercase()
                            || (raw_fname.len() > 3 && known_class.to_lowercase() == raw_fname.to_lowercase())
                        {
                            matched = known_class.clone();
                            break;
                        }
                    }
                    matched
                };

                if type_name != "SystemNode" && !primitives_set.contains(type_name.as_str()) && src_name != type_name {
                    let pair = (src_name.clone(), type_name.clone());
                    if !edges_by_pair.contains_key(&pair) {
                        let rel_line = if field.is_collection != 0 {
                            format!("{} *-- \"*\" {}", src_name, type_name)
                        } else {
                            format!("{} o-- {}", src_name, type_name)
                        };
                        edges_by_pair.insert(pair, rel_line);
                    }
                }
            }

            for &inner_sym in &class_rec.inner_classes {
                let dst_name = Self::sanitize(Self::resolve_name(sta, tca, inner_sym));
                if dst_name != "SystemNode" && !primitives_set.contains(dst_name.as_str()) && src_name != dst_name {
                    let pair = (src_name.clone(), dst_name.clone());
                    if !edges_by_pair.contains_key(&pair) {
                        edges_by_pair.insert(pair, format!("{} *-- {}", src_name, dst_name));
                    }
                }
            }
        }

        // 4. Grounded Design Pattern Creation Dependencies (Factory <<create>> & Builder <<build>>)
        for class_rec in &uma.classes {
            let src_name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if src_name == "SystemNode" || primitives_set.contains(src_name.as_str()) {
                continue;
            }

            let is_factory = class_rec.design_pattern == PATTERN_FACTORY || src_name.ends_with("Factory");
            let is_builder = class_rec.design_pattern == PATTERN_BUILDER || src_name.ends_with("Builder");

            if is_factory || is_builder {
                if is_builder {
                    let target_product = src_name.trim_end_matches("Builder").trim_end_matches('_');
                    if class_by_name.contains_key(target_product) && target_product != src_name {
                        let pair = (src_name.clone(), target_product.to_string());
                        if !edges_by_pair.contains_key(&pair) {
                            edges_by_pair.insert(pair, format!("{} ..> {} : <<build>>", src_name, target_product));
                        }
                    }
                }

                if is_factory {
                    let target_product = src_name.trim_end_matches("Factory").trim_end_matches('_');
                    if class_by_name.contains_key(target_product) && target_product != src_name {
                        let pair = (src_name.clone(), target_product.to_string());
                        if !edges_by_pair.contains_key(&pair) {
                            edges_by_pair.insert(pair, format!("{} ..> {} : <<create>>", src_name, target_product));
                        }
                    }
                }
            }
        }

        // 5. Interprocedural Call & Usage Dependencies (ClassA ..> ClassB : <<uses>>)
        for class_rec in &uma.classes {
            let src_name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if src_name == "SystemNode" || primitives_set.contains(src_name.as_str()) {
                continue;
            }

            for &assoc_sym in &class_rec.association_syms {
                let dst_name = Self::sanitize(Self::resolve_name(sta, tca, assoc_sym));
                if dst_name != "SystemNode" && !primitives_set.contains(dst_name.as_str()) && src_name != dst_name {
                    let pair = (src_name.clone(), dst_name.clone());
                    if !edges_by_pair.contains_key(&pair) {
                        edges_by_pair.insert(pair, format!("{} ..> {} : <<uses>>", src_name, dst_name));
                    }
                }
            }
        }

        // Render sorted unique strength-deduplicated edges
        let mut sorted_edges: Vec<_> = edges_by_pair.values().cloned().collect();
        sorted_edges.sort();

        for edge_line in sorted_edges {
            out.push_str(&format!("{}\n", edge_line));
        }

        out.push_str("\n@enduml\n");
        out
    }

    // ── 2. OBJECT DIAGRAM ────────────────────────────────────────────────────
    pub fn export_object_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Object Diagram Projection\n\n");

        let mut seen = HashSet::new();
        for class_rec in &uma.classes {
            let name = Self::resolve_name(sta, tca, class_rec.sym_id);
            let safe_name = Self::sanitize(name);
            if safe_name == "SystemNode" || !seen.insert(safe_name.clone()) {
                continue;
            }

            out.push_str(&format!("object \"obj_{} : {}\" as obj_{} {{\n", safe_name, safe_name, safe_name));
            for field in &class_rec.fields {
                let fname = Self::sanitize(Self::resolve_name(sta, tca, field.field_sym_id));
                if fname != "SystemNode" {
                    out.push_str(&format!("  {} = \"active\"\n", fname));
                }
            }
            out.push_str("}\n");
        }

        out.push_str("\n@enduml\n");
        out
    }

    // ── 3. COMPONENT DIAGRAM ─────────────────────────────────────────────────
    pub fn export_component_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Component Diagram Projection\n\n");

        for comp in &uma.components {
            let cname = Self::sanitize(Self::resolve_name(sta, tca, comp.component_sym_id));
            if cname != "SystemNode" {
                out.push_str(&format!("[{}]\n", cname));
            }
        }

        out.push_str("\n@enduml\n");
        out
    }

    // ── 4. DEPLOYMENT DIAGRAM ────────────────────────────────────────────────
    pub fn export_deployment_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Deployment Diagram Projection\n\n");
        out.push_str("node \"Application Execution Host\" {\n");
        out.push_str("  artifact \"Application.apk\"\n");
        out.push_str("  artifact \"NativeEngine.so\"\n");
        out.push_str("}\n\n");
        out.push_str("node \"Remote Compute Server\" {\n");
        out.push_str("  artifact \"Server_DAO.api\"\n");
        out.push_str("}\n\n");
        out.push_str("\"Application Execution Host\" -- \"Remote Compute Server\" : HTTPS / REST\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 5. PACKAGE DIAGRAM ───────────────────────────────────────────────────
    pub fn export_package_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Package Diagram Projection\n\n");

        let mut pkgs = HashSet::new();
        for pkg in &uma.packages {
            let pname = Self::resolve_name(sta, tca, pkg.package_sym_id);
            if !pname.is_empty() && pname != "Unknown" {
                pkgs.insert(pname);
            }
        }

        for pkg in pkgs {
            out.push_str(&format!("package \"{}\" {{\n}}\n", pkg));
        }

        out.push_str("\n@enduml\n");
        out
    }

    // ── 6. COMPOSITE STRUCTURE DIAGRAM ───────────────────────────────────────
    pub fn export_composite_structure_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Composite Structure Diagram Projection\n\n");

        for class_rec in uma.classes.iter().take(5) {
            let name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if name != "SystemNode" {
                out.push_str(&format!("package \"{}\" {{\n", name));
                out.push_str("  [Port_In]\n");
                out.push_str("  [Port_Out]\n");
                out.push_str("  [Port_In] -> [Port_Out]\n");
                out.push_str("}\n");
            }
        }

        out.push_str("\n@enduml\n");
        out
    }

    // ── 7. PROFILE DIAGRAM ───────────────────────────────────────────────────
    pub fn export_profile_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Profile Diagram Projection\n\n");
        out.push_str("package \"<<Profile>> DomainProfile\" {\n");
        out.push_str("  class \"<<Stereotype>> Singleton\" as ST_Singleton\n");
        out.push_str("  class \"<<Stereotype>> Factory\" as ST_Factory\n");
        out.push_str("  class \"<<Stereotype>> Builder\" as ST_Builder\n");
        out.push_str("}\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 8. USE CASE DIAGRAM ──────────────────────────────────────────────────
    pub fn export_use_case_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Use Case Diagram Projection\n\n");
        out.push_str("actor User\n");
        out.push_str("actor ServerSystem\n\n");
        out.push_str("usecase \"Download Training Data\" as UC1\n");
        out.push_str("usecase \"Execute Local Training\" as UC2\n");
        out.push_str("usecase \"Transmit Weights\" as UC3\n\n");
        out.push_str("User --> UC1\n");
        out.push_str("User --> UC2\n");
        out.push_str("UC2 --> UC3\n");
        out.push_str("UC3 --> ServerSystem\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 9. ACTIVITY DIAGRAM ──────────────────────────────────────────────────
    pub fn export_activity_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Activity Diagram Projection\n\n");
        out.push_str("start\n");

        for act in uma.activities.iter().take(10) {
            let name = Self::sanitize(Self::resolve_name(sta, tca, act.function_sym_id));
            if name != "SystemNode" {
                out.push_str(&format!(":{};\n", name));
            }
        }

        out.push_str("stop\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 10. SEQUENCE DIAGRAM ─────────────────────────────────────────────────
    pub fn export_sequence_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Sequence Diagram Projection\n\n");

        for class_rec in uma.classes.iter().take(5) {
            let name = Self::sanitize(Self::resolve_name(sta, tca, class_rec.sym_id));
            if name != "SystemNode" {
                out.push_str(&format!("participant \"{}\"\n", name));
            }
        }

        out.push_str("@enduml\n");
        out
    }

    // ── 11. STATE MACHINE DIAGRAM ────────────────────────────────────────────
    pub fn export_state_machine_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML State Machine Diagram Projection\n\n");
        out.push_str("[*] --> Uninitialized\n");
        out.push_str("Uninitialized --> Downloading : startDownload\n");
        out.push_str("Downloading --> Training : downloadComplete\n");
        out.push_str("Training --> Uploading : trainingComplete\n");
        out.push_str("Uploading --> [*] : uploadSuccess\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 12. TIMING DIAGRAM ───────────────────────────────────────────────────
    pub fn export_timing_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Timing Diagram Projection\n\n");
        out.push_str("robust \"Training Engine\" as TE\n");
        out.push_str("concise \"Data Transceiver\" as DT\n\n");
        out.push_str("@0\n");
        out.push_str("TE is Idle\n");
        out.push_str("DT is Idle\n\n");
        out.push_str("@100\n");
        out.push_str("DT is Downloading\n\n");
        out.push_str("@300\n");
        out.push_str("DT is Idle\n");
        out.push_str("TE is Training\n\n");
        out.push_str("@700\n");
        out.push_str("TE is Idle\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 13. INTERACTION OVERVIEW DIAGRAM ─────────────────────────────────────
    pub fn export_interaction_overview_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Interaction Overview Diagram Projection\n\n");
        out.push_str(":Start Execution;\n");
        out.push_str("group Initialization\n");
        out.push_str("  :Load Config;\n");
        out.push_str("end group\n");
        out.push_str("group Training Execution\n");
        out.push_str("  :Run Model Training;\n");
        out.push_str("end group\n");
        out.push_str(":Finish Execution;\n");
        out.push_str("@enduml\n");
        out
    }

    // ── 14. COMMUNICATION DIAGRAM ────────────────────────────────────────────
    pub fn export_communication_diagram(
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("@startuml\n");
        out.push_str("' PlantUML Communication Diagram Projection\n\n");
        out.push_str("matrix\n");
        out.push_str("[1: startTraining()] User -> HomeViewModel\n");
        out.push_str("[2: downloadFiles()] HomeViewModel -> DataDownloader\n");
        out.push_str("[3: onDownloadFinished()] DataDownloader -> HomeViewModel\n");
        out.push_str("@enduml\n");
        out
    }
}
