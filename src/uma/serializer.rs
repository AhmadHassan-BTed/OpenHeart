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
            buf.extend_from_slice(&(class_rec.fields.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(class_rec.methods.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(class_rec.inner_classes.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(class_rec.implements_syms.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(class_rec.association_syms.len() as u16).to_le_bytes());
            buf.push(class_rec.design_pattern);
            buf.push(class_rec._reserved);
            buf.push(class_rec.type_param_count);
            buf.push(class_rec._pad);

            // UMLLink (24 bytes)
            buf.extend_from_slice(&class_rec.uml_link.sym_id.to_le_bytes());
            buf.extend_from_slice(&class_rec.uml_link.file_id.to_le_bytes());
            let line_start_bytes = class_rec.uml_link.line_start.to_le_bytes();
            buf.extend_from_slice(&line_start_bytes[0..3]);
            buf.extend_from_slice(&class_rec.uml_link.col_start.to_le_bytes());
            let line_end_bytes = class_rec.uml_link.line_end.to_le_bytes();
            buf.extend_from_slice(&line_end_bytes[0..3]);
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

            // Implements sym_ids
            for &imp_sym in &class_rec.implements_syms {
                buf.extend_from_slice(&imp_sym.to_le_bytes());
            }

            // Association sym_ids
            for &assoc_sym in &class_rec.association_syms {
                buf.extend_from_slice(&assoc_sym.to_le_bytes());
            }
        }

        // ── 3. ACTIVITY RECORDS ─────────────────────────────────────────────
        for act in &artifact.activities {
            buf.extend_from_slice(&act.function_sym_id.to_le_bytes());
            buf.extend_from_slice(&(act.nodes.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(act.edges.len() as u16).to_le_bytes());
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
            buf.extend_from_slice(&(sm.states.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(sm.transitions.len() as u16).to_le_bytes());
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
            buf.extend_from_slice(&(seq.lifelines.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(seq.messages.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(seq.combined_fragments.len() as u16).to_le_bytes());
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
            buf.extend_from_slice(&(pat.pattern_kind as u32).to_le_bytes());
            buf.extend_from_slice(&(pat.confidence as u32).to_le_bytes());
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
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "UMA file too small",
            ));
        }

        let data_end = bytes.len() - 8;
        let stored_crc = u64::from_le_bytes(bytes[data_end..].try_into().unwrap());
        let computed_crc = crc64_ecma(&bytes[..data_end]);
        if stored_crc != computed_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "UMA CRC mismatch: stored=0x{:016X}, computed=0x{:016X}",
                    stored_crc, computed_crc
                ),
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
        let act_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let sm_count = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let seq_count = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
        let pkg_count = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
        let comp_count = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        let pat_count = u32::from_le_bytes(bytes[44..48].try_into().unwrap());

        let mut offset = UMA_HEADER_SIZE;

        // 1. Classes
        let mut classes = Vec::with_capacity(class_count as usize);
        for _ in 0..class_count {
            let sym_id = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let stereotype = bytes[offset + 4];
            let visibility = bytes[offset + 5];
            let modifiers = u16::from_le_bytes(bytes[offset + 6..offset + 8].try_into().unwrap());
            let extends_sym =
                u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            let field_count =
                u16::from_le_bytes(bytes[offset + 12..offset + 14].try_into().unwrap());
            let method_count =
                u16::from_le_bytes(bytes[offset + 14..offset + 16].try_into().unwrap());
            let inner_count =
                u16::from_le_bytes(bytes[offset + 16..offset + 18].try_into().unwrap());
            let implements_count =
                u16::from_le_bytes(bytes[offset + 18..offset + 20].try_into().unwrap());
            let association_count =
                u16::from_le_bytes(bytes[offset + 20..offset + 22].try_into().unwrap());
            let design_pattern = bytes[offset + 22];
            let res = bytes[offset + 23];
            let type_param_count = bytes[offset + 24];
            let pad = bytes[offset + 25];
            offset += 26;

            // UMLLink (24 bytes)
            let link_sym = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let link_fid = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
            let mut ls_bytes = [0u8; 4];
            ls_bytes[0..3].copy_from_slice(&bytes[offset + 6..offset + 9]);
            let link_ls = u32::from_le_bytes(ls_bytes);
            let link_cs = u16::from_le_bytes(bytes[offset + 9..offset + 11].try_into().unwrap());
            let mut le_bytes = [0u8; 4];
            le_bytes[0..3].copy_from_slice(&bytes[offset + 11..offset + 14]);
            let link_le = u32::from_le_bytes(le_bytes);
            let link_ce = u16::from_le_bytes(bytes[offset + 14..offset + 16].try_into().unwrap());
            let link_sh = u32::from_le_bytes(bytes[offset + 16..offset + 20].try_into().unwrap());
            let link_sk = bytes[offset + 20];
            let link_res = [bytes[offset + 21], bytes[offset + 22], bytes[offset + 23]];
            offset += 24;

            let uml_link = crate::tra::types::UMLLinkRecord {
                sym_id: link_sym,
                file_id: link_fid,
                line_start: link_ls,
                col_start: link_cs,
                line_end: link_le,
                col_end: link_ce,
                scpg_hash: link_sh,
                sym_kind: link_sk,
                _reserved: link_res,
            };

            let mut fields = Vec::with_capacity(field_count as usize);
            for _ in 0..field_count {
                let fsid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let tsid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let vis = bytes[offset + 8];
                let mods = bytes[offset + 9];
                let is_coll = bytes[offset + 10];
                let fpad = bytes[offset + 11];
                let uln = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
                let fres = u32::from_le_bytes(bytes[offset + 16..offset + 20].try_into().unwrap());
                fields.push(FieldRecord {
                    field_sym_id: fsid,
                    type_sym_id: tsid,
                    visibility: vis,
                    modifiers: mods,
                    is_collection: is_coll,
                    _pad: fpad,
                    uml_link_node: uln,
                    _reserved: fres,
                });
                offset += 20;
            }

            let mut methods = Vec::with_capacity(method_count as usize);
            for _ in 0..method_count {
                let msid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let rtsid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let vis = bytes[offset + 8];
                let mods = bytes[offset + 9];
                let pc = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
                let cyc = u16::from_le_bytes(bytes[offset + 12..offset + 14].try_into().unwrap());
                let sat = u64::from_le_bytes(bytes[offset + 14..offset + 22].try_into().unwrap());
                methods.push(MethodRecord {
                    method_sym_id: msid,
                    return_type_sym_id: rtsid,
                    visibility: vis,
                    modifiers: mods,
                    param_count: pc,
                    cyclomatic: cyc,
                    sat_count: sat,
                });
                offset += 22;
            }

            let mut inner_classes = Vec::with_capacity(inner_count as usize);
            for _ in 0..inner_count {
                let isid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                inner_classes.push(isid);
                offset += 4;
            }

            let mut implements_syms = Vec::with_capacity(implements_count as usize);
            for _ in 0..implements_count {
                let imp_id = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                implements_syms.push(imp_id);
                offset += 4;
            }

            let mut association_syms = Vec::with_capacity(association_count as usize);
            for _ in 0..association_count {
                let assoc_id = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                association_syms.push(assoc_id);
                offset += 4;
            }

            classes.push(ClassRecord {
                sym_id,
                stereotype,
                visibility,
                modifiers,
                extends_sym,
                field_count,
                method_count,
                inner_count,
                design_pattern,
                _reserved: res,
                type_param_count,
                _pad: pad,
                uml_link,
                fields,
                methods,
                inner_classes,
                implements_syms,
                association_syms,
            });
        }

        // 2. Activities
        let mut activities = Vec::with_capacity(act_count as usize);
        for _ in 0..act_count {
            let fsid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let nc = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
            let ec = u16::from_le_bytes(bytes[offset + 6..offset + 8].try_into().unwrap());
            let sn = u16::from_le_bytes(bytes[offset + 8..offset + 10].try_into().unwrap());
            let enc = bytes[offset + 10];
            let sc = bytes[offset + 11];
            let cyc = u16::from_le_bytes(bytes[offset + 12..offset + 14].try_into().unwrap());
            let res = u16::from_le_bytes(bytes[offset + 14..offset + 16].try_into().unwrap());
            offset += 16;

            let mut nodes = Vec::with_capacity(nc as usize);
            for _ in 0..nc {
                let nid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let ltid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let nk = bytes[offset + 8];
                let ld = bytes[offset + 9];
                let gtid = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
                let pad = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
                nodes.push(ActivityNode {
                    node_id: nid,
                    label_text_id: ltid,
                    node_kind: nk,
                    loop_depth: ld,
                    guard_text_id: gtid,
                    _pad: pad,
                });
                offset += 16;
            }

            let mut edges = Vec::with_capacity(ec as usize);
            for _ in 0..ec {
                let fn_ = u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                let tn = u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap());
                let ek = bytes[offset + 4];
                let ibe = bytes[offset + 5];
                let gtid = u32::from_le_bytes(bytes[offset + 6..offset + 10].try_into().unwrap());
                let pad = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
                edges.push(ActivityEdge {
                    from_node: fn_,
                    to_node: tn,
                    edge_kind: ek,
                    is_back_edge: ibe,
                    guard_text_id: gtid,
                    _pad: pad,
                });
                offset += 12;
            }

            activities.push(ActivityRecord {
                function_sym_id: fsid,
                node_count: nc,
                edge_count: ec,
                start_node: sn,
                end_node_count: enc,
                swimlane_count: sc,
                cyclomatic: cyc,
                _reserved: res,
                nodes,
                edges,
            });
        }

        // 3. State Machines
        let mut state_machines = Vec::with_capacity(sm_count as usize);
        for _ in 0..sm_count {
            let csid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let sc = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
            let tc = u16::from_le_bytes(bytes[offset + 6..offset + 8].try_into().unwrap());
            let is = u16::from_le_bytes(bytes[offset + 8..offset + 10].try_into().unwrap());
            let fsc = bytes[offset + 10];
            let res = bytes[offset + 11];
            let pad = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
            offset += 16;

            let mut states = Vec::with_capacity(sc as usize);
            for _ in 0..sc {
                let stid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let snid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let ii = bytes[offset + 8];
                let if_ = bytes[offset + 9];
                let spad = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
                states.push(StateRecord {
                    state_id: stid,
                    state_name_id: snid,
                    is_initial: ii,
                    is_final: if_,
                    _pad: spad,
                });
                offset += 12;
            }

            let mut transitions = Vec::with_capacity(tc as usize);
            for _ in 0..tc {
                let fs = u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                let ts = u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap());
                let tms = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let gtid = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
                let atid = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
                transitions.push(TransitionRecord {
                    from_state: fs,
                    to_state: ts,
                    trigger_method_sym: tms,
                    guard_text_id: gtid,
                    action_text_id: atid,
                });
                offset += 16;
            }

            state_machines.push(StateMachineRecord {
                class_sym_id: csid,
                state_count: sc,
                transition_count: tc,
                initial_state: is,
                final_state_count: fsc,
                _reserved: res,
                _pad: pad,
                states,
                transitions,
            });
        }

        // 4. Sequences
        let mut sequences = Vec::with_capacity(seq_count as usize);
        for _ in 0..seq_count {
            let sn = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let lc = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
            let mc = u16::from_le_bytes(bytes[offset + 6..offset + 8].try_into().unwrap());
            let fc = u16::from_le_bytes(bytes[offset + 8..offset + 10].try_into().unwrap());
            let res = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
            offset += 12;

            let mut lifelines = Vec::with_capacity(lc as usize);
            for _ in 0..lc {
                let sid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let nid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let tsid = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
                let ia = bytes[offset + 12];
                let lpad = [bytes[offset + 13], bytes[offset + 14], bytes[offset + 15]];
                lifelines.push(LifelineRecord {
                    sym_id: sid,
                    name_id: nid,
                    type_sym_id: tsid,
                    is_actor: ia,
                    _pad: lpad,
                });
                offset += 16;
            }

            let mut messages = Vec::with_capacity(mc as usize);
            for _ in 0..mc {
                let fl = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let tl = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                let csid = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
                let msid = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
                let mk = bytes[offset + 16];
                let ord = u16::from_le_bytes(bytes[offset + 17..offset + 19].try_into().unwrap());
                let mpad = u16::from_le_bytes(bytes[offset + 19..offset + 21].try_into().unwrap());
                let ult = u32::from_le_bytes(bytes[offset + 21..offset + 25].try_into().unwrap());
                messages.push(MessageRecord {
                    from_lifeline: fl,
                    to_lifeline: tl,
                    call_site_id: csid,
                    method_sym_id: msid,
                    message_kind: mk,
                    ordinal: ord,
                    _pad: mpad,
                    uml_link_token: ult,
                });
                offset += 25;
            }

            let mut combined_fragments = Vec::with_capacity(fc as usize);
            for _ in 0..fc {
                let fk = bytes[offset];
                let gtid = u32::from_le_bytes(bytes[offset + 1..offset + 5].try_into().unwrap());
                let smo = u16::from_le_bytes(bytes[offset + 5..offset + 7].try_into().unwrap());
                let emo = u16::from_le_bytes(bytes[offset + 7..offset + 9].try_into().unwrap());
                let fpad = u16::from_le_bytes(bytes[offset + 9..offset + 11].try_into().unwrap());
                combined_fragments.push(CombinedFragment {
                    fragment_kind: fk,
                    guard_text_id: gtid,
                    start_message_ordinal: smo,
                    end_message_ordinal: emo,
                    _pad: fpad,
                });
                offset += 11;
            }

            sequences.push(SequenceDiagramRecord {
                scenario_name: sn,
                lifeline_count: lc,
                message_count: mc,
                fragment_count: fc,
                _reserved: res,
                lifelines,
                messages,
                combined_fragments,
            });
        }

        // 5. Packages
        let mut packages = Vec::with_capacity(pkg_count as usize);
        for _ in 0..pkg_count {
            let psid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let nid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            let pps = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            let cc = u16::from_le_bytes(bytes[offset + 12..offset + 14].try_into().unwrap());
            let sc = u16::from_le_bytes(bytes[offset + 14..offset + 16].try_into().unwrap());
            packages.push(PackageRecord {
                package_sym_id: psid,
                name_id: nid,
                parent_package_sym: pps,
                class_count: cc,
                subpackage_count: sc,
            });
            offset += 16;
        }

        // 6. Components
        let mut components = Vec::with_capacity(comp_count as usize);
        for _ in 0..comp_count {
            let csid = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let nid = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            let pic = u16::from_le_bytes(bytes[offset + 8..offset + 10].try_into().unwrap());
            let ric = u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
            let cpad = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
            components.push(ComponentRecord {
                component_sym_id: csid,
                name_id: nid,
                provided_interface_count: pic,
                required_interface_count: ric,
                _pad: cpad,
            });
            offset += 16;
        }

        // 7. Design Patterns
        let mut design_patterns = Vec::with_capacity(pat_count as usize);
        for _ in 0..pat_count {
            let cs = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let pk = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
            let conf = u16::from_le_bytes(bytes[offset + 6..offset + 8].try_into().unwrap());
            let pres = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            design_patterns.push(DesignPatternRecord {
                class_sym: cs,
                pattern_kind: pk,
                confidence: conf,
                _reserved: pres,
            });
            offset += 12;
        }

        Ok(UMLMetadataArtifact {
            format_version,
            tra_hash,
            classes,
            objects: Vec::new(),
            activities,
            state_machines,
            sequences,
            packages,
            components,
            design_patterns,
            label_texts: std::collections::HashMap::new(),
        })
    }
}
