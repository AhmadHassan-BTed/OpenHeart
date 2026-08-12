//! JSONExporter — exports UMLMetadataArtifact records to JSON for web visualizers (§10.4).

use crate::uma::types::*;

pub struct JSONExporter;

impl JSONExporter {
    pub fn export_class_diagram(classes: &[ClassRecord]) -> String {
        let mut json = String::from("{\n  \"diagram\": \"class\",\n  \"classes\": [\n");
        for (i, class_rec) in classes.iter().enumerate() {
            let comma = if i + 1 < classes.len() { "," } else { "" };
            json.push_str(&format!(
                "    {{\"sym_id\": {}, \"methods\": {}}}{}\n",
                class_rec.sym_id, class_rec.method_count, comma
            ));
        }
        json.push_str("  ]\n}\n");
        json
    }
}
