//! SemanticIndex — query-ready projection of the Phase 9 UML metadata (§10.5).

use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::types::ScpgFormatError;
use crate::uma::types::*;

pub fn stereotype_name(stereotype: u8) -> &'static str {
    match stereotype {
        STEREOTYPE_ABSTRACT => "abstract",
        STEREOTYPE_INTERFACE => "interface",
        STEREOTYPE_ENUM => "enum",
        STEREOTYPE_RECORD => "record",
        STEREOTYPE_ANNOTATION => "annotation",
        _ => "none",
    }
}

/// Canonical pattern name (Phase 9 naming), or `"none"` for unknown kinds.
pub fn pattern_name(pattern: u16) -> &'static str {
    match u8::try_from(pattern).map(pattern_kind_name) {
        Ok(name) if !name.is_empty() => name,
        _ => "none",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UmlClassRow {
    pub sym_id: u32,
    pub stereotype: u8,
    pub design_pattern: u8,
    pub extends_sym: u32,
    pub field_count: u16,
    pub method_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatternInstanceRow {
    pub class_sym: u32,
    pub pattern_kind: u16,
    pub confidence: u16,
    /// Pattern instance this participant belongs to.
    pub instance_id: u32,
    /// Role (`ROLE_*`) the class plays in the instance.
    pub role: u8,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticIndex {
    pub classes: Vec<UmlClassRow>,
    pub patterns: Vec<PatternInstanceRow>,
    pub activity_methods: Vec<u32>,
    pub state_machine_classes: Vec<u32>,
    pub sequence_count: u32,
}

impl SemanticIndex {
    pub fn from_uma(uma: &UMLMetadataArtifact) -> Self {
        let mut classes: Vec<UmlClassRow> = uma
            .classes
            .iter()
            .map(|c| UmlClassRow {
                sym_id: c.sym_id,
                stereotype: c.stereotype,
                design_pattern: c.design_pattern,
                extends_sym: c.extends_sym,
                field_count: c.field_count,
                method_count: c.method_count,
            })
            .collect();
        classes.sort_by_key(|c| c.sym_id);
        classes.dedup_by_key(|c| c.sym_id);

        let mut patterns: Vec<PatternInstanceRow> = uma
            .design_patterns
            .iter()
            .map(|p| PatternInstanceRow {
                class_sym: p.class_sym,
                pattern_kind: p.pattern_kind,
                confidence: p.confidence,
                instance_id: p.instance_id,
                role: p.role,
            })
            .collect();
        patterns.sort_by_key(|p| {
            (
                p.instance_id,
                p.role,
                p.class_sym,
                p.pattern_kind,
                p.confidence,
            )
        });
        patterns.dedup();

        let mut activity_methods: Vec<u32> =
            uma.activities.iter().map(|a| a.function_sym_id).collect();
        activity_methods.sort_unstable();
        activity_methods.dedup();

        let mut state_machine_classes: Vec<u32> =
            uma.state_machines.iter().map(|s| s.class_sym_id).collect();
        state_machine_classes.sort_unstable();
        state_machine_classes.dedup();

        Self {
            classes,
            patterns,
            activity_methods,
            state_machine_classes,
            sequence_count: uma.sequences.len() as u32,
        }
    }

    pub fn class(&self, sym_id: u32) -> Option<&UmlClassRow> {
        self.classes
            .binary_search_by_key(&sym_id, |c| c.sym_id)
            .ok()
            .map(|i| &self.classes[i])
    }

    pub fn has_activity(&self, method: u32) -> bool {
        self.activity_methods.binary_search(&method).is_ok()
    }

    pub fn has_state_machine(&self, class: u32) -> bool {
        self.state_machine_classes.binary_search(&class).is_ok()
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u32s(&self.classes.iter().map(|c| c.sym_id).collect::<Vec<_>>());
        enc.put_u8s(
            &self
                .classes
                .iter()
                .map(|c| c.stereotype)
                .collect::<Vec<_>>(),
        );
        enc.put_u8s(
            &self
                .classes
                .iter()
                .map(|c| c.design_pattern)
                .collect::<Vec<_>>(),
        );
        enc.put_u32s(
            &self
                .classes
                .iter()
                .map(|c| c.extends_sym)
                .collect::<Vec<_>>(),
        );
        enc.put_u16s(
            &self
                .classes
                .iter()
                .map(|c| c.field_count)
                .collect::<Vec<_>>(),
        );
        enc.put_u16s(
            &self
                .classes
                .iter()
                .map(|c| c.method_count)
                .collect::<Vec<_>>(),
        );
        enc.put_u32s(
            &self
                .patterns
                .iter()
                .map(|p| p.class_sym)
                .collect::<Vec<_>>(),
        );
        enc.put_u16s(
            &self
                .patterns
                .iter()
                .map(|p| p.pattern_kind)
                .collect::<Vec<_>>(),
        );
        enc.put_u16s(
            &self
                .patterns
                .iter()
                .map(|p| p.confidence)
                .collect::<Vec<_>>(),
        );
        enc.put_u32s(
            &self
                .patterns
                .iter()
                .map(|p| p.instance_id)
                .collect::<Vec<_>>(),
        );
        enc.put_u8s(&self.patterns.iter().map(|p| p.role).collect::<Vec<_>>());
        enc.put_u32s(&self.activity_methods);
        enc.put_u32s(&self.state_machine_classes);
        enc.put_u32(self.sequence_count);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let sym_ids = dec.u32s()?;
        let stereotypes = dec.u8s()?;
        let design_patterns = dec.u8s()?;
        let extends = dec.u32s()?;
        let field_counts = dec.u16s()?;
        let method_counts = dec.u16s()?;
        let n = sym_ids.len();
        dec.expect_len("stereotypes", stereotypes.len(), n)?;
        dec.expect_len("design_patterns", design_patterns.len(), n)?;
        dec.expect_len("extends", extends.len(), n)?;
        dec.expect_len("field_counts", field_counts.len(), n)?;
        dec.expect_len("method_counts", method_counts.len(), n)?;
        let classes = (0..n)
            .map(|i| UmlClassRow {
                sym_id: sym_ids[i],
                stereotype: stereotypes[i],
                design_pattern: design_patterns[i],
                extends_sym: extends[i],
                field_count: field_counts[i],
                method_count: method_counts[i],
            })
            .collect::<Vec<_>>();
        if classes.windows(2).any(|w| w[0].sym_id >= w[1].sym_id) {
            return Err(dec.malformed("UML classes are not sorted by sym_id"));
        }

        let class_syms = dec.u32s()?;
        let kinds = dec.u16s()?;
        let confidences = dec.u16s()?;
        let instance_ids = dec.u32s()?;
        let roles = dec.u8s()?;
        dec.expect_len("pattern_kinds", kinds.len(), class_syms.len())?;
        dec.expect_len("pattern_confidences", confidences.len(), class_syms.len())?;
        dec.expect_len("pattern_instance_ids", instance_ids.len(), class_syms.len())?;
        dec.expect_len("pattern_roles", roles.len(), class_syms.len())?;
        let patterns = (0..class_syms.len())
            .map(|i| PatternInstanceRow {
                class_sym: class_syms[i],
                pattern_kind: kinds[i],
                confidence: confidences[i],
                instance_id: instance_ids[i],
                role: roles[i],
            })
            .collect();

        let activity_methods = dec.u32s()?;
        let state_machine_classes = dec.u32s()?;
        let sequence_count = dec.u32()?;
        Ok(Self {
            classes,
            patterns,
            activity_methods,
            state_machine_classes,
            sequence_count,
        })
    }
}
