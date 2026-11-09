//! UMASerializer — binary I/O for UMLMetadataArtifact (.uma format v2) (§9.5).
//!
//! Layout (little-endian):
//! ```text
//! HEADER (64 bytes)
//!   magic u64 | format_version u32 | tra_hash u64 | section_count u32 |
//!   payload_len u64 | reserved [u8; 32]
//! PAYLOAD: section_count sections, each
//!   section_id u16 | reserved u16 | record_count u32 | byte_len u64 | records
//! CHECKSUM: CRC-64/ECMA over header + payload (8 bytes)
//! ```
//! Every count is a u32 written through a checked conversion. The reader is
//! bounds-checked end to end: it validates the checksum, the payload length, each
//! section's byte length, caps every record count by the bytes that remain before
//! allocating, requires each section to be consumed exactly, and requires the
//! payload to end exactly at the checksum. Malformed input yields
//! `io::ErrorKind::InvalidData`, never a panic. Unknown section ids are skipped
//! by their byte length so newer writers stay readable.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Result as IoResult};
use std::path::Path;

use crate::ingestion::serializer::crc64_ecma;
use crate::tra::types::UMLLinkRecord;
use crate::uma::types::*;

const SECTION_CLASSES: u16 = 1;
const SECTION_OBJECTS: u16 = 2;
const SECTION_OBJECT_LINKS: u16 = 3;
const SECTION_ACTIVITIES: u16 = 4;
const SECTION_STATE_MACHINES: u16 = 5;
const SECTION_SEQUENCES: u16 = 6;
const SECTION_COMMUNICATIONS: u16 = 7;
const SECTION_INTERACTION_OVERVIEWS: u16 = 8;
const SECTION_PACKAGES: u16 = 9;
const SECTION_PACKAGE_DEPENDENCIES: u16 = 10;
const SECTION_PACKAGE_METRICS: u16 = 11;
const SECTION_COMPONENTS: u16 = 12;
const SECTION_COMPOSITE_STRUCTURES: u16 = 13;
const SECTION_DESIGN_PATTERNS: u16 = 14;
const SECTION_PATTERN_INSTANCES: u16 = 15;
const SECTION_LABELS: u16 = 16;

const SECTION_HEADER_SIZE: usize = 16;
const CHECKSUM_SIZE: usize = 8;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

// ── Writer ────────────────────────────────────────────────────────────────────

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Write a collection length as u32, failing instead of truncating.
    fn count(&mut self, len: usize, what: &str) -> IoResult<()> {
        let n = u32::try_from(len)
            .map_err(|_| invalid(format!("UMA: too many {} records ({})", what, len)))?;
        self.u32(n);
        Ok(())
    }

    fn u32_list(&mut self, items: &[u32], what: &str) -> IoResult<()> {
        self.count(items.len(), what)?;
        for &v in items {
            self.u32(v);
        }
        Ok(())
    }

    /// Start a section; returns the offset of its byte_len slot.
    fn begin_section(&mut self, id: u16, count: usize, what: &str) -> IoResult<usize> {
        self.u16(id);
        self.u16(0);
        self.count(count, what)?;
        let slot = self.buf.len();
        self.u64(0);
        Ok(slot)
    }

    fn end_section(&mut self, slot: usize) {
        let len = (self.buf.len() - slot - 8) as u64;
        self.buf[slot..slot + 8].copy_from_slice(&len.to_le_bytes());
    }
}

// ── Reader ────────────────────────────────────────────────────────────────────

struct Cursor<'b> {
    bytes: &'b [u8],
    pos: usize,
    end: usize,
}

impl<'b> Cursor<'b> {
    fn new(bytes: &'b [u8], pos: usize, end: usize) -> Self {
        Self { bytes, pos, end }
    }

    fn remaining(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> IoResult<&'b [u8]> {
        if n > self.remaining() {
            return Err(invalid(format!(
                "UMA truncated: need {} bytes at offset {}, {} remain",
                n,
                self.pos,
                self.remaining()
            )));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> IoResult<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> IoResult<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> IoResult<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> IoResult<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }
    fn f32(&mut self) -> IoResult<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Read a record count, rejecting counts the remaining bytes cannot hold.
    fn count(&mut self, min_record_size: usize, what: &str) -> IoResult<usize> {
        let n = self.u32()? as usize;
        let need = n.checked_mul(min_record_size.max(1));
        match need {
            Some(need) if need <= self.remaining() => Ok(n),
            _ => Err(invalid(format!(
                "UMA: {} count {} exceeds the {} remaining bytes",
                what,
                n,
                self.remaining()
            ))),
        }
    }

    fn u32_list(&mut self, what: &str) -> IoResult<Vec<u32>> {
        let n = self.count(4, what)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u32()?);
        }
        Ok(v)
    }

    fn expect_end(&self, what: &str) -> IoResult<()> {
        if self.pos != self.end {
            return Err(invalid(format!(
                "UMA: {} section has {} unread trailing bytes",
                what,
                self.end - self.pos
            )));
        }
        Ok(())
    }
}

fn saturating_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub struct UMASerializer;

impl UMASerializer {
    /// Serialize UMLMetadataArtifact to `.uma` file at `path`.
    pub fn write(artifact: &UMLMetadataArtifact, path: &Path) -> IoResult<()> {
        let bytes = Self::to_bytes(artifact)?;
        fs::write(path, &bytes)
    }

    /// Read and deserialize a UMLMetadataArtifact from `.uma` file.
    pub fn read(path: &Path) -> IoResult<UMLMetadataArtifact> {
        let bytes = fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Encode an artifact to the `.uma` byte layout.
    pub fn to_bytes(artifact: &UMLMetadataArtifact) -> IoResult<Vec<u8>> {
        let mut w = Writer { buf: Vec::new() };

        // ── HEADER (64 bytes) ──
        w.u64(UMA_MAGIC);
        w.u32(artifact.format_version);
        w.u64(artifact.tra_hash);
        w.u32(16); // section_count
        let payload_slot = w.buf.len();
        w.u64(0); // payload_len (patched below)
        w.buf.extend_from_slice(&[0u8; 32]);
        debug_assert_eq!(w.buf.len(), UMA_HEADER_SIZE);

        Self::write_classes(&mut w, &artifact.classes)?;
        Self::write_objects(&mut w, artifact)?;
        Self::write_activities(&mut w, &artifact.activities)?;
        Self::write_state_machines(&mut w, &artifact.state_machines)?;
        Self::write_sequences(&mut w, &artifact.sequences)?;
        Self::write_interactions(&mut w, artifact)?;
        Self::write_packages(&mut w, artifact)?;
        Self::write_components(&mut w, artifact)?;
        Self::write_patterns(&mut w, artifact)?;
        Self::write_labels(&mut w, &artifact.label_texts)?;

        let payload_len = (w.buf.len() - UMA_HEADER_SIZE) as u64;
        w.buf[payload_slot..payload_slot + 8].copy_from_slice(&payload_len.to_le_bytes());

        let crc = crc64_ecma(&w.buf);
        w.u64(crc);
        Ok(w.buf)
    }

    fn write_uml_link(w: &mut Writer, link: &UMLLinkRecord) {
        w.u32(link.sym_id);
        w.u16(link.file_id);
        w.u32(link.line_start);
        w.u16(link.col_start);
        w.u32(link.line_end);
        w.u16(link.col_end);
        w.u32(link.scpg_hash);
        w.u8(link.sym_kind);
        w.buf.extend_from_slice(&link._reserved);
    }

    fn write_classes(w: &mut Writer, classes: &[ClassRecord]) -> IoResult<()> {
        let slot = w.begin_section(SECTION_CLASSES, classes.len(), "class")?;
        for c in classes {
            w.u32(c.sym_id);
            w.u8(c.stereotype);
            w.u8(c.visibility);
            w.u16(c.modifiers);
            w.u32(c.extends_sym);
            w.u8(c.design_pattern);
            w.u8(c._reserved);
            w.u8(c.type_param_count);
            w.u8(c._pad);
            w.u32(c.qualified_name_id);
            Self::write_uml_link(w, &c.uml_link);

            w.count(c.fields.len(), "field")?;
            for f in &c.fields {
                w.u32(f.field_sym_id);
                w.u32(f.type_sym_id);
                w.u8(f.visibility);
                w.u16(f.modifiers);
                w.u8(f.is_collection);
                w.u8(f.field_kind);
                w.u8(f.assoc_kind);
                w.u16(f.mult_min);
                w.u16(f.mult_max);
                w.u32(f.element_type_sym_id);
                w.u32(f.uml_link_node);
            }

            w.count(c.methods.len(), "method")?;
            for m in &c.methods {
                w.u32(m.method_sym_id);
                w.u32(m.return_type_sym_id);
                w.u8(m.visibility);
                w.u16(m.modifiers);
                w.u16(m.param_count);
                w.u16(m.cyclomatic);
                w.u64(m.sat_count);
                w.u8(m.is_constructor);
                w.u8(m.metrics_known);
                w.u32_list(&m.param_syms, "parameter")?;
                w.u32_list(&m.param_type_syms, "parameter type")?;
            }

            w.u32_list(&c.inner_classes, "inner class")?;
            w.u32_list(&c.implements_syms, "realization")?;
            w.u32_list(&c.association_syms, "association")?;
            w.u32_list(&c.generalization_syms, "generalization")?;
            w.u32_list(&c.dependency_syms, "dependency")?;
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_objects(w: &mut Writer, artifact: &UMLMetadataArtifact) -> IoResult<()> {
        let slot = w.begin_section(SECTION_OBJECTS, artifact.objects.len(), "object")?;
        for o in &artifact.objects {
            w.u32(o.alloc_token);
            w.u32(o.type_sym_id);
            w.u32(o.label_text_id);
            w.u32(o.containing_method_sym);
            w.u32(o.assigned_field_sym);
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_OBJECT_LINKS,
            artifact.object_links.len(),
            "object link",
        )?;
        for l in &artifact.object_links {
            w.u32(l.from_object);
            w.u32(l.to_object);
            w.u32(l.via_field_sym);
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_activities(w: &mut Writer, activities: &[ActivityRecord]) -> IoResult<()> {
        let slot = w.begin_section(SECTION_ACTIVITIES, activities.len(), "activity")?;
        for a in activities {
            w.u32(a.function_sym_id);
            w.u32(a.start_node);
            w.u16(a.end_node_count);
            w.u16(a.swimlane_count);
            w.u16(a.cyclomatic);
            w.u8(a.max_loop_depth);
            w.u8(a._reserved);
            w.count(a.nodes.len(), "activity node")?;
            for n in &a.nodes {
                w.u32(n.node_id);
                w.u32(n.label_text_id);
                w.u8(n.node_kind);
                w.u8(n.loop_depth);
                w.u32(n.guard_text_id);
                w.u32(n.first_token);
            }
            w.count(a.edges.len(), "activity edge")?;
            for e in &a.edges {
                w.u32(e.from_node);
                w.u32(e.to_node);
                w.u8(e.edge_kind);
                w.u8(e.is_back_edge);
                w.u32(e.guard_text_id);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_state_machines(w: &mut Writer, machines: &[StateMachineRecord]) -> IoResult<()> {
        let slot = w.begin_section(SECTION_STATE_MACHINES, machines.len(), "state machine")?;
        for sm in machines {
            w.u32(sm.class_sym_id);
            w.u32(sm.state_field_sym);
            w.u16(sm.initial_state);
            w.u8(sm.final_state_count);
            w.u8(sm._reserved);
            w.count(sm.states.len(), "state")?;
            for s in &sm.states {
                w.u32(s.state_id);
                w.u32(s.state_name_id);
                w.u8(s.is_initial);
                w.u8(s.is_final);
                w.u16(s._pad);
            }
            w.count(sm.transitions.len(), "transition")?;
            for t in &sm.transitions {
                w.u16(t.from_state);
                w.u16(t.to_state);
                w.u32(t.trigger_method_sym);
                w.u32(t.guard_text_id);
                w.u32(t.action_text_id);
                w.u32(t.source_token);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_sequences(w: &mut Writer, sequences: &[SequenceDiagramRecord]) -> IoResult<()> {
        let slot = w.begin_section(SECTION_SEQUENCES, sequences.len(), "sequence")?;
        for s in sequences {
            w.u32(s.scenario_name);
            w.u32(s.entry_method_sym);
            w.u8(s.truncated);
            w.u8(s.entry_kind);
            w.count(s.lifelines.len(), "lifeline")?;
            for l in &s.lifelines {
                w.u32(l.sym_id);
                w.u32(l.name_id);
                w.u32(l.type_sym_id);
                w.u8(l.is_actor);
                w.buf.extend_from_slice(&l._pad);
            }
            w.count(s.messages.len(), "message")?;
            for m in &s.messages {
                w.u32(m.from_lifeline);
                w.u32(m.to_lifeline);
                w.u32(m.call_site_id);
                w.u32(m.method_sym_id);
                w.u8(m.message_kind);
                w.u8(m.call_edge_type);
                w.u16(m.ordinal);
                w.u16(m.nesting_depth);
                w.u32(m.uml_link_token);
            }
            w.count(s.combined_fragments.len(), "combined fragment")?;
            for f in &s.combined_fragments {
                w.u8(f.fragment_kind);
                w.u32(f.guard_text_id);
                w.u16(f.start_message_ordinal);
                w.u16(f.end_message_ordinal);
                w.u16(f.nesting_depth);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_interactions(w: &mut Writer, artifact: &UMLMetadataArtifact) -> IoResult<()> {
        let slot = w.begin_section(
            SECTION_COMMUNICATIONS,
            artifact.communications.len(),
            "communication diagram",
        )?;
        for c in &artifact.communications {
            w.u32(c.sequence_index);
            w.count(c.links.len(), "communication link")?;
            for l in &c.links {
                w.u32(l.from_lifeline);
                w.u32(l.to_lifeline);
                w.count(l.message_ordinals.len(), "link message")?;
                for &o in &l.message_ordinals {
                    w.u16(o);
                }
            }
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_INTERACTION_OVERVIEWS,
            artifact.interaction_overviews.len(),
            "interaction overview",
        )?;
        for io in &artifact.interaction_overviews {
            w.u32(io.entry_method_sym);
            w.u32(io.activity_index);
            w.count(io.references.len(), "interaction reference")?;
            for r in &io.references {
                w.u32(r.activity_node);
                w.u32(r.sequence_index);
                w.u32(r.call_site_id);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_packages(w: &mut Writer, artifact: &UMLMetadataArtifact) -> IoResult<()> {
        let slot = w.begin_section(SECTION_PACKAGES, artifact.packages.len(), "package")?;
        for p in &artifact.packages {
            w.u32(p.package_sym_id);
            w.u32(p.name_id);
            w.u32(p.parent_package_sym);
            w.u16(p.class_count);
            w.u16(p.subpackage_count);
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_PACKAGE_DEPENDENCIES,
            artifact.package_dependencies.len(),
            "package dependency",
        )?;
        for d in &artifact.package_dependencies {
            w.u32(d.from_package_sym);
            w.u32(d.to_package_sym);
            w.u32(d.reference_count);
            w.u8(d.in_cycle);
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_PACKAGE_METRICS,
            artifact.package_metrics.len(),
            "package metrics",
        )?;
        for m in &artifact.package_metrics {
            w.u32(m.package_sym_id);
            w.u32(m.afferent);
            w.u32(m.efferent);
            w.f32(m.instability);
            w.f32(m.abstractness);
            w.f32(m.distance);
            w.u32(m.cycle_id);
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_components(w: &mut Writer, artifact: &UMLMetadataArtifact) -> IoResult<()> {
        let slot = w.begin_section(SECTION_COMPONENTS, artifact.components.len(), "component")?;
        for c in &artifact.components {
            w.u32(c.component_sym_id);
            w.u32(c.name_id);
            w.u8(c.component_kind);
            w.u32_list(&c.provided_interfaces, "provided interface")?;
            w.u32_list(&c.required_interfaces, "required interface")?;
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_COMPOSITE_STRUCTURES,
            artifact.composite_structures.len(),
            "composite structure",
        )?;
        for cs in &artifact.composite_structures {
            w.u32(cs.class_sym_id);
            w.count(cs.parts.len(), "composite part")?;
            for p in &cs.parts {
                w.u32(p.part_sym);
                w.u32(p.type_sym_id);
                w.u16(p.mult_min);
                w.u16(p.mult_max);
                w.u8(p.is_nested_type);
            }
            w.count(cs.connectors.len(), "composite connector")?;
            for c in &cs.connectors {
                w.u32(c.from_part);
                w.u32(c.to_part);
                w.u32(c.via_method_sym);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_patterns(w: &mut Writer, artifact: &UMLMetadataArtifact) -> IoResult<()> {
        let slot = w.begin_section(
            SECTION_DESIGN_PATTERNS,
            artifact.design_patterns.len(),
            "design pattern",
        )?;
        for p in &artifact.design_patterns {
            w.u32(p.class_sym);
            w.u16(p.pattern_kind);
            w.u16(p.confidence);
            w.u32(p.instance_id);
            w.u8(p.role);
            w.buf.extend_from_slice(&[0u8; 3]);
        }
        w.end_section(slot);

        let slot = w.begin_section(
            SECTION_PATTERN_INSTANCES,
            artifact.pattern_instances.len(),
            "pattern instance",
        )?;
        for inst in &artifact.pattern_instances {
            w.u32(inst.instance_id);
            w.u8(inst.pattern_kind);
            w.u8(inst.score);
            w.u16(0);
            w.u32(inst.primary_sym);
            w.u64(inst.evidence_mask);
            w.count(inst.roles.len(), "role binding")?;
            for r in &inst.roles {
                w.u8(r.role);
                w.u32(r.sym_id);
            }
        }
        w.end_section(slot);
        Ok(())
    }

    fn write_labels(w: &mut Writer, labels: &HashMap<u32, String>) -> IoResult<()> {
        let mut ids: Vec<&u32> = labels.keys().collect();
        ids.sort_unstable();
        let slot = w.begin_section(SECTION_LABELS, ids.len(), "label")?;
        for id in ids {
            let text = labels[id].as_bytes();
            w.u32(*id);
            w.count(text.len(), "label byte")?;
            w.buf.extend_from_slice(text);
        }
        w.end_section(slot);
        Ok(())
    }

    // ── Decoding ──────────────────────────────────────────────────────────────

    /// Decode an artifact from `.uma` bytes; malformed input is `InvalidData`.
    pub fn from_bytes(bytes: &[u8]) -> IoResult<UMLMetadataArtifact> {
        if bytes.len() < UMA_HEADER_SIZE + CHECKSUM_SIZE {
            return Err(invalid("UMA file too small"));
        }
        let data_end = bytes.len() - CHECKSUM_SIZE;
        let mut tail = Cursor::new(bytes, data_end, bytes.len());
        let stored_crc = tail.u64()?;
        let computed_crc = crc64_ecma(&bytes[..data_end]);
        if stored_crc != computed_crc {
            return Err(invalid(format!(
                "UMA CRC mismatch: stored=0x{:016X}, computed=0x{:016X}",
                stored_crc, computed_crc
            )));
        }

        let mut hdr = Cursor::new(bytes, 0, UMA_HEADER_SIZE);
        let magic = hdr.u64()?;
        if magic != UMA_MAGIC {
            return Err(invalid(format!("UMA magic mismatch: got 0x{:016X}", magic)));
        }
        let format_version = hdr.u32()?;
        if format_version != UMA_FORMAT_VERSION {
            return Err(invalid(format!(
                "Unsupported UMA format version {} (this reader handles {})",
                format_version, UMA_FORMAT_VERSION
            )));
        }
        let tra_hash = hdr.u64()?;
        let section_count = hdr.u32()?;
        let payload_len = hdr.u64()?;
        if payload_len != (data_end - UMA_HEADER_SIZE) as u64 {
            return Err(invalid(format!(
                "UMA payload length mismatch: header says {}, file holds {}",
                payload_len,
                data_end - UMA_HEADER_SIZE
            )));
        }

        let mut artifact = UMLMetadataArtifact::new(tra_hash);
        artifact.format_version = format_version;

        let mut cur = Cursor::new(bytes, UMA_HEADER_SIZE, data_end);
        for _ in 0..section_count {
            if cur.remaining() < SECTION_HEADER_SIZE {
                return Err(invalid("UMA truncated section header"));
            }
            let id = cur.u16()?;
            let _reserved = cur.u16()?;
            let count = cur.u32()? as usize;
            let byte_len = cur.u64()?;
            let byte_len = usize::try_from(byte_len)
                .ok()
                .filter(|&l| l <= cur.remaining())
                .ok_or_else(|| invalid(format!("UMA section {} overruns the payload", id)))?;
            let start = cur.pos;
            let mut sec = Cursor::new(bytes, start, start + byte_len);
            Self::read_section(id, count, &mut sec, &mut artifact)?;
            cur.pos = start + byte_len;
        }
        cur.expect_end("payload")?;
        Ok(artifact)
    }

    fn read_section(
        id: u16,
        count: usize,
        sec: &mut Cursor<'_>,
        a: &mut UMLMetadataArtifact,
    ) -> IoResult<()> {
        // Cap every declared count by the section bytes before allocating.
        let check = |min: usize, what: &str, remaining: usize| -> IoResult<()> {
            match count.checked_mul(min) {
                Some(need) if need <= remaining => Ok(()),
                _ => Err(invalid(format!(
                    "UMA: {} count {} exceeds section length {}",
                    what, count, remaining
                ))),
            }
        };
        match id {
            SECTION_CLASSES => {
                check(40, "class", sec.remaining())?;
                a.classes = (0..count)
                    .map(|_| Self::read_class(sec))
                    .collect::<IoResult<_>>()?;
                sec.expect_end("class")
            }
            SECTION_OBJECTS => {
                check(20, "object", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(ObjectRecord {
                        alloc_token: sec.u32()?,
                        type_sym_id: sec.u32()?,
                        label_text_id: sec.u32()?,
                        containing_method_sym: sec.u32()?,
                        assigned_field_sym: sec.u32()?,
                    });
                }
                a.objects = v;
                sec.expect_end("object")
            }
            SECTION_OBJECT_LINKS => {
                check(12, "object link", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(ObjectLinkRecord {
                        from_object: sec.u32()?,
                        to_object: sec.u32()?,
                        via_field_sym: sec.u32()?,
                    });
                }
                a.object_links = v;
                sec.expect_end("object link")
            }
            SECTION_ACTIVITIES => {
                check(20, "activity", sec.remaining())?;
                a.activities = (0..count)
                    .map(|_| Self::read_activity(sec))
                    .collect::<IoResult<_>>()?;
                sec.expect_end("activity")
            }
            SECTION_STATE_MACHINES => {
                check(16, "state machine", sec.remaining())?;
                a.state_machines = (0..count)
                    .map(|_| Self::read_state_machine(sec))
                    .collect::<IoResult<_>>()?;
                sec.expect_end("state machine")
            }
            SECTION_SEQUENCES => {
                check(22, "sequence", sec.remaining())?;
                a.sequences = (0..count)
                    .map(|_| Self::read_sequence(sec))
                    .collect::<IoResult<_>>()?;
                sec.expect_end("sequence")
            }
            SECTION_COMMUNICATIONS => {
                check(8, "communication diagram", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let sequence_index = sec.u32()?;
                    let n = sec.count(12, "communication link")?;
                    let mut links = Vec::with_capacity(n);
                    for _ in 0..n {
                        let from_lifeline = sec.u32()?;
                        let to_lifeline = sec.u32()?;
                        let k = sec.count(2, "link message")?;
                        let mut message_ordinals = Vec::with_capacity(k);
                        for _ in 0..k {
                            message_ordinals.push(sec.u16()?);
                        }
                        links.push(CommunicationLinkRecord {
                            from_lifeline,
                            to_lifeline,
                            message_ordinals,
                        });
                    }
                    v.push(CommunicationDiagramRecord {
                        sequence_index,
                        links,
                    });
                }
                a.communications = v;
                sec.expect_end("communication diagram")
            }
            SECTION_INTERACTION_OVERVIEWS => {
                check(12, "interaction overview", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let entry_method_sym = sec.u32()?;
                    let activity_index = sec.u32()?;
                    let n = sec.count(12, "interaction reference")?;
                    let mut references = Vec::with_capacity(n);
                    for _ in 0..n {
                        references.push(InteractionReferenceRecord {
                            activity_node: sec.u32()?,
                            sequence_index: sec.u32()?,
                            call_site_id: sec.u32()?,
                        });
                    }
                    v.push(InteractionOverviewRecord {
                        entry_method_sym,
                        activity_index,
                        references,
                    });
                }
                a.interaction_overviews = v;
                sec.expect_end("interaction overview")
            }
            SECTION_PACKAGES => {
                check(16, "package", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(PackageRecord {
                        package_sym_id: sec.u32()?,
                        name_id: sec.u32()?,
                        parent_package_sym: sec.u32()?,
                        class_count: sec.u16()?,
                        subpackage_count: sec.u16()?,
                    });
                }
                a.packages = v;
                sec.expect_end("package")
            }
            SECTION_PACKAGE_DEPENDENCIES => {
                check(13, "package dependency", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(PackageDependencyRecord {
                        from_package_sym: sec.u32()?,
                        to_package_sym: sec.u32()?,
                        reference_count: sec.u32()?,
                        in_cycle: sec.u8()?,
                    });
                }
                a.package_dependencies = v;
                sec.expect_end("package dependency")
            }
            SECTION_PACKAGE_METRICS => {
                check(28, "package metrics", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(PackageMetricsRecord {
                        package_sym_id: sec.u32()?,
                        afferent: sec.u32()?,
                        efferent: sec.u32()?,
                        instability: sec.f32()?,
                        abstractness: sec.f32()?,
                        distance: sec.f32()?,
                        cycle_id: sec.u32()?,
                    });
                }
                a.package_metrics = v;
                sec.expect_end("package metrics")
            }
            SECTION_COMPONENTS => {
                check(17, "component", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let component_sym_id = sec.u32()?;
                    let name_id = sec.u32()?;
                    let component_kind = sec.u8()?;
                    let provided_interfaces = sec.u32_list("provided interface")?;
                    let required_interfaces = sec.u32_list("required interface")?;
                    v.push(ComponentRecord {
                        component_sym_id,
                        name_id,
                        provided_interface_count: saturating_u16(provided_interfaces.len()),
                        required_interface_count: saturating_u16(required_interfaces.len()),
                        component_kind,
                        provided_interfaces,
                        required_interfaces,
                    });
                }
                a.components = v;
                sec.expect_end("component")
            }
            SECTION_COMPOSITE_STRUCTURES => {
                check(12, "composite structure", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let class_sym_id = sec.u32()?;
                    let n = sec.count(13, "composite part")?;
                    let mut parts = Vec::with_capacity(n);
                    for _ in 0..n {
                        parts.push(CompositePartRecord {
                            part_sym: sec.u32()?,
                            type_sym_id: sec.u32()?,
                            mult_min: sec.u16()?,
                            mult_max: sec.u16()?,
                            is_nested_type: sec.u8()?,
                        });
                    }
                    let n = sec.count(12, "composite connector")?;
                    let mut connectors = Vec::with_capacity(n);
                    for _ in 0..n {
                        connectors.push(CompositeConnectorRecord {
                            from_part: sec.u32()?,
                            to_part: sec.u32()?,
                            via_method_sym: sec.u32()?,
                        });
                    }
                    v.push(CompositeStructureRecord {
                        class_sym_id,
                        parts,
                        connectors,
                    });
                }
                a.composite_structures = v;
                sec.expect_end("composite structure")
            }
            SECTION_DESIGN_PATTERNS => {
                check(16, "design pattern", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let class_sym = sec.u32()?;
                    let pattern_kind = sec.u16()?;
                    let confidence = sec.u16()?;
                    let instance_id = sec.u32()?;
                    let role = sec.u8()?;
                    sec.take(3)?;
                    v.push(DesignPatternRecord {
                        class_sym,
                        pattern_kind,
                        confidence,
                        instance_id,
                        role,
                    });
                }
                a.design_patterns = v;
                sec.expect_end("design pattern")
            }
            SECTION_PATTERN_INSTANCES => {
                check(24, "pattern instance", sec.remaining())?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let instance_id = sec.u32()?;
                    let pattern_kind = sec.u8()?;
                    let score = sec.u8()?;
                    let _pad = sec.u16()?;
                    let primary_sym = sec.u32()?;
                    let evidence_mask = sec.u64()?;
                    let n = sec.count(5, "role binding")?;
                    let mut roles = Vec::with_capacity(n);
                    for _ in 0..n {
                        roles.push(PatternRoleBinding {
                            role: sec.u8()?,
                            sym_id: sec.u32()?,
                        });
                    }
                    v.push(PatternInstanceRecord {
                        instance_id,
                        pattern_kind,
                        score,
                        primary_sym,
                        evidence_mask,
                        roles,
                    });
                }
                a.pattern_instances = v;
                sec.expect_end("pattern instance")
            }
            SECTION_LABELS => {
                check(8, "label", sec.remaining())?;
                let mut map = HashMap::with_capacity(count);
                for _ in 0..count {
                    let id = sec.u32()?;
                    let len = sec.count(1, "label byte")?;
                    let raw = sec.take(len)?;
                    let text = std::str::from_utf8(raw)
                        .map_err(|_| invalid(format!("UMA label {} is not UTF-8", id)))?;
                    if id == NO_LABEL {
                        return Err(invalid("UMA label id 0 is reserved"));
                    }
                    map.insert(id, text.to_string());
                }
                a.label_texts = map;
                sec.expect_end("label")
            }
            _ => {
                // Unknown section from a newer writer: skip by byte length.
                sec.pos = sec.end;
                Ok(())
            }
        }
    }

    fn read_uml_link(c: &mut Cursor<'_>) -> IoResult<UMLLinkRecord> {
        let sym_id = c.u32()?;
        let file_id = c.u16()?;
        let line_start = c.u32()?;
        let col_start = c.u16()?;
        let line_end = c.u32()?;
        let col_end = c.u16()?;
        let scpg_hash = c.u32()?;
        let sym_kind = c.u8()?;
        let r = c.take(3)?;
        Ok(UMLLinkRecord {
            sym_id,
            file_id,
            line_start,
            col_start,
            line_end,
            col_end,
            scpg_hash,
            sym_kind,
            _reserved: [r[0], r[1], r[2]],
        })
    }

    fn read_class(c: &mut Cursor<'_>) -> IoResult<ClassRecord> {
        let sym_id = c.u32()?;
        let stereotype = c.u8()?;
        let visibility = c.u8()?;
        let modifiers = c.u16()?;
        let extends_sym = c.u32()?;
        let design_pattern = c.u8()?;
        let reserved = c.u8()?;
        let type_param_count = c.u8()?;
        let pad = c.u8()?;
        let qualified_name_id = c.u32()?;
        let uml_link = Self::read_uml_link(c)?;

        let n = c.count(26, "field")?;
        let mut fields = Vec::with_capacity(n);
        for _ in 0..n {
            fields.push(FieldRecord {
                field_sym_id: c.u32()?,
                type_sym_id: c.u32()?,
                visibility: c.u8()?,
                modifiers: c.u16()?,
                is_collection: c.u8()?,
                field_kind: c.u8()?,
                assoc_kind: c.u8()?,
                mult_min: c.u16()?,
                mult_max: c.u16()?,
                element_type_sym_id: c.u32()?,
                uml_link_node: c.u32()?,
            });
        }

        let n = c.count(32, "method")?;
        let mut methods = Vec::with_capacity(n);
        for _ in 0..n {
            let method_sym_id = c.u32()?;
            let return_type_sym_id = c.u32()?;
            let visibility = c.u8()?;
            let modifiers = c.u16()?;
            let param_count = c.u16()?;
            let cyclomatic = c.u16()?;
            let sat_count = c.u64()?;
            let is_constructor = c.u8()?;
            let metrics_known = c.u8()?;
            let param_syms = c.u32_list("parameter")?;
            let param_type_syms = c.u32_list("parameter type")?;
            methods.push(MethodRecord {
                method_sym_id,
                return_type_sym_id,
                visibility,
                modifiers,
                param_count,
                cyclomatic,
                sat_count,
                is_constructor,
                metrics_known,
                param_syms,
                param_type_syms,
            });
        }

        let inner_classes = c.u32_list("inner class")?;
        let implements_syms = c.u32_list("realization")?;
        let association_syms = c.u32_list("association")?;
        let generalization_syms = c.u32_list("generalization")?;
        let dependency_syms = c.u32_list("dependency")?;

        Ok(ClassRecord {
            sym_id,
            stereotype,
            visibility,
            modifiers,
            extends_sym,
            field_count: saturating_u16(fields.len()),
            method_count: saturating_u16(methods.len()),
            inner_count: saturating_u16(inner_classes.len()),
            design_pattern,
            _reserved: reserved,
            type_param_count,
            _pad: pad,
            uml_link,
            fields,
            methods,
            inner_classes,
            implements_syms,
            association_syms,
            generalization_syms,
            dependency_syms,
            qualified_name_id,
        })
    }

    fn read_activity(c: &mut Cursor<'_>) -> IoResult<ActivityRecord> {
        let function_sym_id = c.u32()?;
        let start_node = c.u32()?;
        let end_node_count = c.u16()?;
        let swimlane_count = c.u16()?;
        let cyclomatic = c.u16()?;
        let max_loop_depth = c.u8()?;
        let reserved = c.u8()?;
        let n = c.count(18, "activity node")?;
        let mut nodes = Vec::with_capacity(n);
        for _ in 0..n {
            nodes.push(ActivityNode {
                node_id: c.u32()?,
                label_text_id: c.u32()?,
                node_kind: c.u8()?,
                loop_depth: c.u8()?,
                guard_text_id: c.u32()?,
                first_token: c.u32()?,
            });
        }
        let n = c.count(14, "activity edge")?;
        let mut edges = Vec::with_capacity(n);
        for _ in 0..n {
            edges.push(ActivityEdge {
                from_node: c.u32()?,
                to_node: c.u32()?,
                edge_kind: c.u8()?,
                is_back_edge: c.u8()?,
                guard_text_id: c.u32()?,
            });
        }
        Ok(ActivityRecord {
            function_sym_id,
            node_count: saturating_u32(nodes.len()),
            edge_count: saturating_u32(edges.len()),
            start_node,
            end_node_count,
            swimlane_count,
            cyclomatic,
            max_loop_depth,
            _reserved: reserved,
            nodes,
            edges,
        })
    }

    fn read_state_machine(c: &mut Cursor<'_>) -> IoResult<StateMachineRecord> {
        let class_sym_id = c.u32()?;
        let state_field_sym = c.u32()?;
        let initial_state = c.u16()?;
        let final_state_count = c.u8()?;
        let reserved = c.u8()?;
        let n = c.count(12, "state")?;
        let mut states = Vec::with_capacity(n);
        for _ in 0..n {
            states.push(StateRecord {
                state_id: c.u32()?,
                state_name_id: c.u32()?,
                is_initial: c.u8()?,
                is_final: c.u8()?,
                _pad: c.u16()?,
            });
        }
        let n = c.count(20, "transition")?;
        let mut transitions = Vec::with_capacity(n);
        for _ in 0..n {
            transitions.push(TransitionRecord {
                from_state: c.u16()?,
                to_state: c.u16()?,
                trigger_method_sym: c.u32()?,
                guard_text_id: c.u32()?,
                action_text_id: c.u32()?,
                source_token: c.u32()?,
            });
        }
        Ok(StateMachineRecord {
            class_sym_id,
            state_field_sym,
            state_count: saturating_u16(states.len()),
            transition_count: saturating_u16(transitions.len()),
            initial_state,
            final_state_count,
            _reserved: reserved,
            states,
            transitions,
        })
    }

    fn read_sequence(c: &mut Cursor<'_>) -> IoResult<SequenceDiagramRecord> {
        let scenario_name = c.u32()?;
        let entry_method_sym = c.u32()?;
        let truncated = c.u8()?;
        let entry_kind = c.u8()?;
        let n = c.count(16, "lifeline")?;
        let mut lifelines = Vec::with_capacity(n);
        for _ in 0..n {
            let sym_id = c.u32()?;
            let name_id = c.u32()?;
            let type_sym_id = c.u32()?;
            let is_actor = c.u8()?;
            let p = c.take(3)?;
            lifelines.push(LifelineRecord {
                sym_id,
                name_id,
                type_sym_id,
                is_actor,
                _pad: [p[0], p[1], p[2]],
            });
        }
        let n = c.count(24, "message")?;
        let mut messages = Vec::with_capacity(n);
        for _ in 0..n {
            messages.push(MessageRecord {
                from_lifeline: c.u32()?,
                to_lifeline: c.u32()?,
                call_site_id: c.u32()?,
                method_sym_id: c.u32()?,
                message_kind: c.u8()?,
                call_edge_type: c.u8()?,
                ordinal: c.u16()?,
                nesting_depth: c.u16()?,
                uml_link_token: c.u32()?,
            });
        }
        let n = c.count(11, "combined fragment")?;
        let mut combined_fragments = Vec::with_capacity(n);
        for _ in 0..n {
            combined_fragments.push(CombinedFragment {
                fragment_kind: c.u8()?,
                guard_text_id: c.u32()?,
                start_message_ordinal: c.u16()?,
                end_message_ordinal: c.u16()?,
                nesting_depth: c.u16()?,
            });
        }
        Ok(SequenceDiagramRecord {
            scenario_name,
            entry_method_sym,
            lifeline_count: saturating_u16(lifelines.len()),
            message_count: saturating_u16(messages.len()),
            fragment_count: saturating_u16(combined_fragments.len()),
            truncated,
            entry_kind,
            lifelines,
            messages,
            combined_fragments,
        })
    }
}
