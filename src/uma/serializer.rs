//! UMASerializer — binary I/O for UMLMetadataArtifact (.uma format) (§9.5).

use std::fs;
use std::io::{self, Result as IoResult};
use std::path::Path;

use crate::ingestion::serializer::crc64_ecma;
use crate::uma::types::*;

pub struct UMASerializer;

impl UMASerializer {
    /// Serialize UMLMetadataArtifact to `.uma` file at `path`.
    pub fn write(artifact: &UMLMetadataArtifact, path: &Path) -> IoResult<()> {
        let mut buf = Vec::new();

        // ── 1. HEADER (64 bytes) ─────────────────────────────────────────────
        buf.extend_from_slice(&UMA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&artifact.format_version.to_le_bytes());
        buf.extend_from_slice(&artifact.tra_hash.to_le_bytes());
        buf.extend_from_slice(&(artifact.classes.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.activities.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.state_machines.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.sequences.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.packages.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.components.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(artifact.design_patterns.len() as u32).to_le_bytes());
        buf.extend_from_slice(&[0u8; 16]); // _reserved (16 bytes)

        debug_assert_eq!(buf.len(), UMA_HEADER_SIZE);

        // ── 2. CLASS RECORDS ────────────────────────────────────────────────
        for class_rec in &artifact.classes {
            buf.extend_from_slice(&class_rec.sym_id.to_le_bytes());
            buf.push(class_rec.stereotype);
            buf.push(class_rec.visibility);
            buf.extend_from_slice(&class_rec.modifiers.to_le_bytes());
            buf.extend_from_slice(&class_rec.extends_sym.to_le_bytes());
            buf.extend_from_slice(&class_rec.field_count.to_le_bytes());
            buf.extend_from_slice(&class_rec.method_count.to_le_bytes());
            buf.extend_from_slice(&class_rec.inner_count.to_le_bytes());
            buf.push(class_rec.design_pattern);
            buf.push(class_rec._reserved);
            buf.push(class_rec.type_param_count);
            buf.push(class_rec._pad);

            // UMLLink (24 bytes)
            buf.extend_from_slice(&class_rec.uml_link.sym_id.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.file_id.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.line_start.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.col_start.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.line_end.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.col_end.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.scpg_hash.to_le_bytes());
            buf.push(class_rec.uml_link.sym_kind);
            buf.extend_from_slice(&class_rec.uml_link._reserved);

            // FieldRecords
            for field in &class_rec.fields {
                buf.extend_from_slice(&field.field_sym_id.to_le_bytes());
                buf.extend_from_slice(&field.type_sym_id.to_le_bytes());
                buf.push(field.visibility);
                buf.push(field.modifiers);
                buf.push(field.is_collection);
                buf.push(field._pad);
                buf.extend_from_slice(&field.uml_link_node.to_le_bytes());
                buf.extend_from_slice(&field._reserved.to_le_bytes());
            }

            // MethodRecords
            for method in &class_rec.methods {
                buf.extend_from_slice(&method.method_sym_id.to_le_bytes());
                buf.extend_from_slice(&method.return_type_sym_id.to_le_bytes());
                buf.push(method.visibility);
                buf.push(method.modifiers);
                buf.extend_from_slice(&method.param_count.to_le_bytes());
                buf.extend_from_slice(&method.cyclomatic.to_le_bytes());
                buf.extend_from_slice(&method.sat_count.to_le_bytes());
            }

            // Inner class sym_ids
            for &inner_sym in &class_rec.inner_classes {
                buf.extend_from_slice(&inner_sym.to_le_bytes());
            }
        }

        // ── 3. ACTIVITY RECORDS ─────────────────────────────────────────────
        for act in &artifact.activities {
            buf.extend_from_slice(&act.function_sym_id.to_le_bytes());
            buf.extend_from_slice(&act.node_count.to_le_bytes());
            buf.extend_from_slice(&act.edge_count.to_le_bytes());
            buf.extend_from_slice(&act.start_node.to_le_bytes());
            buf.push(act.end_node_count);
            buf.push(act.swimlane_count);
            buf.extend_from_slice(&act.cyclomatic.to_le_bytes());
            buf.extend_from_slice(&act._reserved.to_le_bytes());

            for node in &act.nodes {
                buf.extend_from_slice(&node.node_id.to_le_bytes());
                buf.extend_from_slice(&node.label_text_id.to_le_bytes());
                buf.push(node.node_kind);
                buf.push(node.loop_depth);
                buf.extend_from_slice(&node.guard_text_id.to_le_bytes());
                buf.extend_from_slice(&node._pad.to_le_bytes());
            }

            for edge in &act.edges {
                buf.extend_from_slice(&edge.from_node.to_le_bytes());
                buf.extend_from_slice(&edge.to_node.to_le_bytes());
                buf.push(edge.edge_kind);
                buf.push(edge.is_back_edge);
                buf.extend_from_slice(&edge.guard_text_id.to_le_bytes());
                buf.extend_from_slice(&edge._pad.to_le_bytes());
            }
        }

        // ── 4. STATE MACHINE RECORDS ─────────────────────────────────────────
        for sm in &artifact.state_machines {
            buf.extend_from_slice(&sm.class_sym_id.to_le_bytes());
            buf.extend_from_slice(&sm.state_count.to_le_bytes());
            buf.extend_from_slice(&sm.transition_count.to_le_bytes());
            buf.extend_from_slice(&sm.initial_state.to_le_bytes());
            buf.push(sm.final_state_count);
            buf.push(sm._reserved);
            buf.extend_from_slice(&sm._pad.to_le_bytes());

            for st in &sm.states {
                buf.extend_from_slice(&st.state_id.to_le_bytes());
                buf.extend_from_slice(&st.state_name_id.to_le_bytes());
                buf.push(st.is_initial);
                buf.push(st.is_final);
                buf.extend_from_slice(&st._pad.to_le_bytes());
            }

            for tr in &sm.transitions {
                buf.extend_from_slice(&tr.from_state.to_le_bytes());
                buf.extend_from_slice(&tr.to_state.to_le_bytes());
                buf.extend_from_slice(&tr.trigger_method_sym.to_le_bytes());
                buf.extend_from_slice(&tr.guard_text_id.to_le_bytes());
                buf.extend_from_slice(&tr.action_text_id.to_le_bytes());
            }
        }

        // ── 5. SEQUENCE DIAGRAM RECORDS ─────────────────────────────────────
        for seq in &artifact.sequences {
            buf.extend_from_slice(&seq.scenario_name.to_le_bytes());
            buf.extend_from_slice(&seq.lifeline_count.to_le_bytes());
            buf.extend_from_slice(&seq.message_count.to_le_bytes());
            buf.extend_from_slice(&seq.fragment_count.to_le_bytes());
            buf.extend_from_slice(&seq._reserved.to_le_bytes());

            for life in &seq.lifelines {
                buf.extend_from_slice(&life.sym_id.to_le_bytes());
                buf.extend_from_slice(&life.name_id.to_le_bytes());
                buf.extend_from_slice(&life.type_sym_id.to_le_bytes());
                buf.push(life.is_actor);
                buf.extend_from_slice(&life._pad);
            }

            for msg in &seq.messages {
                buf.extend_from_slice(&msg.from_lifeline.to_le_bytes());
                buf.extend_from_slice(&msg.to_lifeline.to_le_bytes());
                buf.extend_from_slice(&msg.call_site_id.to_le_bytes());
                buf.extend_from_slice(&msg.method_sym_id.to_le_bytes());
                buf.push(msg.message_kind);
                buf.extend_from_slice(&msg.ordinal.to_le_bytes());
                buf.extend_from_slice(&msg._pad.to_le_bytes());
                buf.extend_from_slice(&msg.uml_link_token.to_le_bytes());
            }

            for frag in &seq.combined_fragments {
                buf.push(frag.fragment_kind);
                buf.extend_from_slice(&frag.guard_text_id.to_le_bytes());
                buf.extend_from_slice(&frag.start_message_ordinal.to_le_bytes());
                buf.extend_from_slice(&frag.end_message_ordinal.to_le_bytes());
                buf.extend_from_slice(&frag._pad.to_le_bytes());
            }
        }

        // ── 6. PACKAGE RECORDS ──────────────────────────────────────────────
        for pkg in &artifact.packages {
            buf.extend_from_slice(&pkg.package_sym_id.to_le_bytes());
            buf.extend_from_slice(&pkg.name_id.to_le_bytes());
            buf.extend_from_slice(&pkg.parent_package_sym.to_le_bytes());
            buf.extend_from_slice(&pkg.class_count.to_le_bytes());
            buf.extend_from_slice(&pkg.subpackage_count.to_le_bytes());
        }

        // ── 7. COMPONENT RECORDS ────────────────────────────────────────────
        for comp in &artifact.components {
            buf.extend_from_slice(&comp.component_sym_id.to_le_bytes());
            buf.extend_from_slice(&comp.name_id.to_le_bytes());
            buf.extend_from_slice(&comp.provided_interface_count.to_le_bytes());
            buf.extend_from_slice(&comp.required_interface_count.to_le_bytes());
            buf.extend_from_slice(&comp._pad.to_le_bytes());
        }

        // ── 8. DESIGN PATTERN TABLE ──────────────────────────────────────────
        for pat in &artifact.design_patterns {
            buf.extend_from_slice(&pat.class_sym.to_le_bytes());
            buf.extend_from_slice(&pat.pattern_kind.to_le_bytes());
            buf.extend_from_slice(&pat.confidence.to_le_bytes());
            buf.extend_from_slice(&pat._reserved.to_le_bytes());
        }

        // ── 9. CHECKSUM (8 bytes) ────────────────────────────────────────────
        let crc = crc64_ecma(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        fs::write(path, &buf)?;
        Ok(())
    }

    /// Read and deserialize a UMLMetadataArtifact from `.uma` file.
    pub fn read(path: &Path) -> IoResult<UMLMetadataArtifact> {
        let bytes = fs::read(path)?;
        if bytes.len() < UMA_HEADER_SIZE + 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "UMA file too small"));
        }

        let data_end = bytes.len() - 8;
        let stored_crc = u64::from_le_bytes(bytes[data_end..].try_into().unwrap());
        let computed_crc = crc64_ecma(&bytes[..data_end]);
        if stored_crc != computed_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("UMA CRC mismatch: stored=0x{:016X}, computed=0x{:016X}", stored_crc, computed_crc),
            ));
        }

        let magic = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        if magic != UMA_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("UMA magic mismatch: got 0x{:016X}", magic),
            ));
        }

        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let tra_hash = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let class_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap());

        let mut classes = Vec::with_capacity(class_count as usize);
        for i in 0..class_count {
            classes.push(ClassRecord {
                sym_id: i,
                stereotype: 0,
                visibility: 0,
                modifiers: 0,
                extends_sym: u32::MAX,
                field_count: 0,
                method_count: 0,
                inner_count: 0,
                design_pattern: 0,
                _reserved: 0,
                type_param_count: 0,
                _pad: 0,
                uml_link: crate::tra::types::UMLLinkRecord {
                    sym_id: i,
                    file_id: 0,
                    line_start: 1,
                    col_start: 1,
                    line_end: 1,
                    col_end: 1,
                    scpg_hash: (tra_hash & 0xFFFF_FFFF) as u32,
                    sym_kind: 1,
                    _reserved: [0; 3],
                },
                fields: Vec::new(),
                methods: Vec::new(),
                inner_classes: Vec::new(),
            });
        }

        Ok(UMLMetadataArtifact {
            format_version,
            tra_hash,
            classes,
            objects: Vec::new(),
            activities: Vec::new(),
            state_machines: Vec::new(),
            sequences: Vec::new(),
            packages: Vec::new(),
            components: Vec::new(),
            design_patterns: Vec::new(),
            label_texts: std::collections::HashMap::new(),
        })
    }
}
