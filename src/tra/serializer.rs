//! Traceability Artifact Binary Serializer & Deserializer (§7.6).
//!
//! **TRA File Format v3 (all integers little-endian):**
//!
//! ```text
//! Header (304 bytes)
//!   0  magic u64 = b"OPENHTRA"        8  version u32 = 3     12  section_count u32 = 10
//!  16  scpg_hash u32                 20  reserved u32 = 0
//!  24  tca..cga hash tags 6 × u64    72  artifact digests 6 × 32 B
//! 264  chain digest 32 B            296  reserved u64 = 0
//! Section directory (section_count × 24 bytes), in section-id order:
//!   id u32, record_size u32, count u32, reserved u32 = 0, offset u64
//! Sections (contiguous, in id order):
//!   0 BI_AST 8 B   1 BI_SYM 16 B   2 BI_BLK 8 B   3 BI_SSA 12 B   4 BI_CS 4 B
//!   5 SYM_SPAN 28 B   6 CS_SPAN 14 B   7 UMLLINK 24 B (lines as u24)
//!   8 FUNCTION_INDEX 20 B   9 ENTITY 44 B
//! Checksum: CRC-64/ECMA-182 of everything before it (8 B)
//! ```
//!
//! The directory makes every section directly addressable: [`TraFileView`] validates a file
//! (in memory or memory-mapped) and hands out any section's bytes without parsing the ones
//! before it.
//!
//! The reader treats the file as untrusted: the checksum is verified first, the directory is
//! checked against the fixed layout, every read is bounds-checked, and the decoded tables
//! are checked for the orders and references the query API relies on (sorted spans, links
//! and function records; ids within their tables; parent links preceding their records;
//! zero reserved bytes). The hash chain is re-verified from its per-artifact digests.
//! Malformed input yields `InvalidData`, never a panic.
//!
//! Writes go through `core::io::fs::write_atomic` (temporary file, flush, rename), so a
//! crash or a full disk never leaves a truncated `.tra` in place.

use std::io;
use std::path::Path;

use crate::core::io::fs::write_atomic;
use crate::core::io::mmap::MemoryMappedFile;
use crate::ingestion::serializer::crc64_ecma;
use crate::tra::types::*;
use crate::tra::uml_link::UMLLinkRegistry;

/// Fixed header size of format v3.
const HEADER_SIZE: usize = 304;
/// Size of one section directory entry.
const DIR_ENTRY_SIZE: usize = 24;
const CHECKSUM_SIZE: usize = 8;

/// Largest value representable in the 24-bit UMLLink line fields.
const U24_MAX: u32 = 0x00FF_FFFF;

/// The sections of a `.tra` file, in file order (the discriminant is the section id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TraSection {
    BiAst = 0,
    BiSym = 1,
    BiBlk = 2,
    BiSsa = 3,
    BiCs = 4,
    SymSpan = 5,
    CsSpan = 6,
    UmlLink = 7,
    FunctionIndex = 8,
    Entity = 9,
}

impl TraSection {
    /// Every section in file order.
    pub const ALL: [TraSection; 10] = [
        TraSection::BiAst,
        TraSection::BiSym,
        TraSection::BiBlk,
        TraSection::BiSsa,
        TraSection::BiCs,
        TraSection::SymSpan,
        TraSection::CsSpan,
        TraSection::UmlLink,
        TraSection::FunctionIndex,
        TraSection::Entity,
    ];

    /// Serialized size of one record of the section.
    pub const fn record_size(self) -> usize {
        match self {
            TraSection::BiAst => 8,
            TraSection::BiSym => 16,
            TraSection::BiBlk => 8,
            TraSection::BiSsa => 12,
            TraSection::BiCs => 4,
            TraSection::SymSpan => 28,
            TraSection::CsSpan => 14,
            TraSection::UmlLink => 24,
            TraSection::FunctionIndex => 20,
            TraSection::Entity => 44,
        }
    }

    /// Section name for diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            TraSection::BiAst => "BI_AST",
            TraSection::BiSym => "BI_SYM",
            TraSection::BiBlk => "BI_BLK",
            TraSection::BiSsa => "BI_SSA",
            TraSection::BiCs => "BI_CS",
            TraSection::SymSpan => "SYM_SPAN",
            TraSection::CsSpan => "CS_SPAN",
            TraSection::UmlLink => "UMLLINK",
            TraSection::FunctionIndex => "FUNCTION_INDEX",
            TraSection::Entity => "ENTITY",
        }
    }
}

pub struct TraceabilitySerializer;

impl TraceabilitySerializer {
    /// Serialize `artifact` to `out_path` atomically.
    pub fn write(artifact: &TraceabilityArtifact, out_path: &Path) -> io::Result<()> {
        let buf = Self::to_bytes(artifact)?;
        write_atomic(out_path, &buf)
    }

    /// Serialize `artifact`. Fails, without producing bytes, when the artifact violates the
    /// orders and references the reader checks.
    pub fn to_bytes(artifact: &TraceabilityArtifact) -> io::Result<Vec<u8>> {
        validate_tables(artifact)?;

        let counts: [usize; 10] = [
            artifact.bi_ast.len(),
            artifact.bi_sym.len(),
            artifact.bi_blk.len(),
            artifact.bi_ssa.len(),
            artifact.bi_cs.len(),
            artifact.sym_span.len(),
            artifact.cs_span.len(),
            artifact.uml_links.len(),
            artifact.function_index.len(),
            artifact.entities.len(),
        ];
        let directory_end = HEADER_SIZE + TraSection::ALL.len() * DIR_ENTRY_SIZE;
        let mut offsets = [0u64; 10];
        let mut cursor = directory_end as u64;
        for (i, section) in TraSection::ALL.iter().enumerate() {
            if u32::try_from(counts[i]).is_err() {
                return Err(invalid(format!(
                    "too many {} records for the TRA format",
                    section.name()
                )));
            }
            offsets[i] = cursor;
            cursor += (counts[i] * section.record_size()) as u64;
        }
        let total = cursor as usize + CHECKSUM_SIZE;
        let mut buf = Vec::with_capacity(total);

        // Header.
        buf.extend_from_slice(&TRA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&artifact.format_version.to_le_bytes());
        buf.extend_from_slice(&(TraSection::ALL.len() as u32).to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.scpg_hash.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.tca_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.bpa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.sta_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.ssa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.cga_hash.to_le_bytes());
        for d in &artifact.hashes.artifact_digests {
            buf.extend_from_slice(d);
        }
        buf.extend_from_slice(&artifact.hashes.chain_digest);
        buf.extend_from_slice(&0u64.to_le_bytes());
        debug_assert_eq!(buf.len(), HEADER_SIZE);

        // Section directory.
        for (i, section) in TraSection::ALL.iter().enumerate() {
            buf.extend_from_slice(&(*section as u32).to_le_bytes());
            buf.extend_from_slice(&(section.record_size() as u32).to_le_bytes());
            buf.extend_from_slice(&(counts[i] as u32).to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&offsets[i].to_le_bytes());
        }
        debug_assert_eq!(buf.len(), directory_end);

        for entry in &artifact.bi_ast {
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }
        for entry in &artifact.bi_sym {
            buf.extend_from_slice(&entry.decl_first_tok.to_le_bytes());
            buf.extend_from_slice(&entry.decl_last_tok.to_le_bytes());
            buf.extend_from_slice(&entry.def_first_tok.to_le_bytes());
            buf.extend_from_slice(&entry.def_last_tok.to_le_bytes());
        }
        for entry in &artifact.bi_blk {
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }
        for entry in &artifact.bi_ssa {
            buf.extend_from_slice(&entry.def_stmt.to_le_bytes());
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }
        for entry in &artifact.bi_cs {
            buf.extend_from_slice(&entry.call_token.to_le_bytes());
        }
        for rec in &artifact.sym_span {
            buf.extend_from_slice(&rec.first_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.last_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.sym_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.extend_from_slice(&rec.col_start.to_le_bytes());
            buf.extend_from_slice(&rec.line_start.to_le_bytes());
            buf.extend_from_slice(&rec.line_end.to_le_bytes());
            buf.extend_from_slice(&rec.parent.to_le_bytes());
        }
        for rec in &artifact.cs_span {
            buf.extend_from_slice(&rec.first_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.call_site_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.extend_from_slice(&rec.line_start.to_le_bytes());
        }
        for rec in &artifact.uml_links {
            if rec.line_start > U24_MAX || rec.line_end > U24_MAX {
                return Err(invalid(format!(
                    "UMLLink of sym_id={} has a line beyond the 24-bit range",
                    rec.sym_id
                )));
            }
            buf.extend_from_slice(&rec.sym_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.extend_from_slice(&rec.line_start.to_le_bytes()[0..3]);
            buf.extend_from_slice(&rec.col_start.to_le_bytes());
            buf.extend_from_slice(&rec.line_end.to_le_bytes()[0..3]);
            buf.extend_from_slice(&rec.col_end.to_le_bytes());
            buf.extend_from_slice(&rec.scpg_hash.to_le_bytes());
            buf.push(rec.sym_kind);
            buf.extend_from_slice(&rec._reserved);
        }
        for rec in &artifact.function_index {
            buf.extend_from_slice(&rec.sym_id.to_le_bytes());
            buf.extend_from_slice(&rec.first_block.to_le_bytes());
            buf.extend_from_slice(&rec.block_count.to_le_bytes());
            buf.extend_from_slice(&rec.first_ssa.to_le_bytes());
            buf.extend_from_slice(&rec.ssa_count.to_le_bytes());
        }
        for e in &artifact.entities {
            buf.extend_from_slice(&e.stable_key);
            buf.extend_from_slice(&e.content_digest);
            buf.extend_from_slice(&e.file_id.to_le_bytes());
            buf.extend_from_slice(&e.col_start.to_le_bytes());
            buf.extend_from_slice(&e.line_start.to_le_bytes());
            buf.extend_from_slice(&e.line_end.to_le_bytes());
        }
        debug_assert_eq!(buf.len() as u64, cursor);

        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        Ok(buf)
    }

    /// Read and validate the `.tra` file at `path`.
    pub fn read(path: &Path) -> io::Result<TraceabilityArtifact> {
        let buf = std::fs::read(path)?;
        Self::from_bytes(&buf)
    }

    /// Same as [`TraceabilitySerializer::read`] (the name used by earlier callers).
    pub fn deserialize(path: &Path) -> io::Result<TraceabilityArtifact> {
        Self::read(path)
    }

    /// Memory-map a `.tra` file for [`TraFileView::parse`].
    pub fn map(path: &Path) -> io::Result<MemoryMappedFile> {
        MemoryMappedFile::open(path)
    }

    /// Decode and validate a whole `.tra` file.
    pub fn from_bytes(buf: &[u8]) -> io::Result<TraceabilityArtifact> {
        let view = TraFileView::parse(buf)?;

        let mut cur = view.cursor(TraSection::BiAst);
        let mut bi_ast = Vec::with_capacity(view.count(TraSection::BiAst));
        for _ in 0..view.count(TraSection::BiAst) {
            bi_ast.push(BIAstEntry {
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::BiSym);
        let mut bi_sym = Vec::with_capacity(view.count(TraSection::BiSym));
        for _ in 0..view.count(TraSection::BiSym) {
            bi_sym.push(BISymEntry {
                decl_first_tok: cur.u32()?,
                decl_last_tok: cur.u32()?,
                def_first_tok: cur.u32()?,
                def_last_tok: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::BiBlk);
        let mut bi_blk = Vec::with_capacity(view.count(TraSection::BiBlk));
        for _ in 0..view.count(TraSection::BiBlk) {
            bi_blk.push(BIBlkEntry {
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::BiSsa);
        let mut bi_ssa = Vec::with_capacity(view.count(TraSection::BiSsa));
        for _ in 0..view.count(TraSection::BiSsa) {
            bi_ssa.push(BISsaEntry {
                def_stmt: cur.u32()?,
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::BiCs);
        let mut bi_cs = Vec::with_capacity(view.count(TraSection::BiCs));
        for _ in 0..view.count(TraSection::BiCs) {
            bi_cs.push(BICsEntry {
                call_token: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::SymSpan);
        let mut sym_span = Vec::with_capacity(view.count(TraSection::SymSpan));
        for _ in 0..view.count(TraSection::SymSpan) {
            sym_span.push(SymbolSpanRecord {
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
                sym_id: cur.u32()?,
                file_id: cur.u16()?,
                col_start: cur.u16()?,
                line_start: cur.u32()?,
                line_end: cur.u32()?,
                parent: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::CsSpan);
        let mut cs_span = Vec::with_capacity(view.count(TraSection::CsSpan));
        for _ in 0..view.count(TraSection::CsSpan) {
            cs_span.push(CallSiteSpanRecord {
                first_token_id: cur.u32()?,
                call_site_id: cur.u32()?,
                file_id: cur.u16()?,
                line_start: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::UmlLink);
        let mut uml_links = Vec::with_capacity(view.count(TraSection::UmlLink));
        for _ in 0..view.count(TraSection::UmlLink) {
            let sym_id = cur.u32()?;
            let file_id = cur.u16()?;
            let line_start = cur.u24()?;
            let col_start = cur.u16()?;
            let line_end = cur.u24()?;
            let col_end = cur.u16()?;
            let link_hash = cur.u32()?;
            let sym_kind = cur.u8()?;
            let reserved = cur.array::<3>()?;
            uml_links.push(UMLLinkRecord {
                sym_id,
                file_id,
                line_start,
                col_start,
                line_end,
                col_end,
                scpg_hash: link_hash,
                sym_kind,
                _reserved: reserved,
            });
        }

        let mut cur = view.cursor(TraSection::FunctionIndex);
        let mut function_index = Vec::with_capacity(view.count(TraSection::FunctionIndex));
        for _ in 0..view.count(TraSection::FunctionIndex) {
            function_index.push(FunctionIndexRecord {
                sym_id: cur.u32()?,
                first_block: cur.u32()?,
                block_count: cur.u32()?,
                first_ssa: cur.u32()?,
                ssa_count: cur.u32()?,
            });
        }

        let mut cur = view.cursor(TraSection::Entity);
        let mut entities = Vec::with_capacity(view.count(TraSection::Entity));
        for _ in 0..view.count(TraSection::Entity) {
            entities.push(EntityRecord {
                stable_key: cur.array::<16>()?,
                content_digest: cur.array::<16>()?,
                file_id: cur.u16()?,
                col_start: cur.u16()?,
                line_start: cur.u32()?,
                line_end: cur.u32()?,
            });
        }

        let uml_slots = UMLLinkRegistry::slots(&uml_links, bi_sym.len());
        let artifact = TraceabilityArtifact {
            format_version: view.format_version,
            hashes: view.hashes,
            bi_ast,
            bi_sym,
            bi_blk,
            bi_ssa,
            bi_cs,
            sym_span,
            cs_span,
            uml_links,
            function_index,
            entities,
            uml_slots,
        };
        validate_tables(&artifact)?;
        Ok(artifact)
    }
}

/// Location of one section in a validated file.
#[derive(Clone, Copy, Debug)]
struct SectionSpan {
    offset: usize,
    count: usize,
}

/// A validated `.tra` file whose sections are addressable without decoding the others.
///
/// [`TraFileView::parse`] checks the checksum, the header (magic, version, reserved fields,
/// hash chain) and the section directory (ids in order, record sizes, contiguous offsets
/// that end exactly at the checksum). Record-level order and reference checks need the
/// decoded tables and are done by [`TraceabilitySerializer::from_bytes`].
pub struct TraFileView<'a> {
    data: &'a [u8],
    /// File format version (always [`TRA_FORMAT_VERSION`] after a successful parse).
    pub format_version: u32,
    /// The hash chain stored in the header (verified).
    pub hashes: ScpgHashChain,
    sections: [SectionSpan; 10],
}

impl<'a> TraFileView<'a> {
    /// Validate `buf` and index its sections.
    pub fn parse(buf: &'a [u8]) -> io::Result<Self> {
        let directory_end = HEADER_SIZE + TraSection::ALL.len() * DIR_ENTRY_SIZE;
        if buf.len() < directory_end + CHECKSUM_SIZE {
            return Err(invalid("TRA artifact file too small"));
        }

        // ── Validate CRC-64 before trusting any count ─────────────────────────
        let data_end = buf.len() - CHECKSUM_SIZE;
        let stored_crc = Cursor::new(&buf[data_end..]).u64()?;
        let computed_crc = crc64_ecma(&buf[..data_end]);
        if stored_crc != computed_crc {
            return Err(invalid(format!(
                "TRA CRC-64 mismatch: expected {:#X}, got {:#X}",
                stored_crc, computed_crc
            )));
        }
        let data = &buf[..data_end];

        let mut cur = Cursor::new(data);
        let magic = cur.u64()?;
        if magic != TRA_MAGIC {
            return Err(invalid("Invalid TRA magic bytes"));
        }
        let format_version = cur.u32()?;
        if format_version != TRA_FORMAT_VERSION {
            return Err(invalid(format!(
                "unsupported TRA format version {} (expected {})",
                format_version, TRA_FORMAT_VERSION
            )));
        }
        let section_count = cur.u32()? as usize;
        if section_count != TraSection::ALL.len() {
            return Err(invalid(format!(
                "TRA header declares {} sections (expected {})",
                section_count,
                TraSection::ALL.len()
            )));
        }
        let scpg_hash = cur.u32()?;
        if cur.u32()? != 0 {
            return Err(invalid("TRA reserved header field is not zero"));
        }
        let tca_hash = cur.u64()?;
        let bpa_hash = cur.u64()?;
        let sta_hash = cur.u64()?;
        let cfa_hash = cur.u64()?;
        let ssa_hash = cur.u64()?;
        let cga_hash = cur.u64()?;
        let mut artifact_digests = [[0u8; 32]; 6];
        for d in artifact_digests.iter_mut() {
            *d = cur.array::<32>()?;
        }
        let chain_digest = cur.array::<32>()?;
        if cur.u64()? != 0 {
            return Err(invalid("TRA reserved header field is not zero"));
        }

        let hashes = ScpgHashChain {
            tca_hash,
            bpa_hash,
            sta_hash,
            cfa_hash,
            ssa_hash,
            cga_hash,
            scpg_hash,
            artifact_digests,
            chain_digest,
        };
        hashes
            .verify()
            .map_err(|msg| invalid(format!("TRA hash chain invalid: {}", msg)))?;

        // ── Section directory ─────────────────────────────────────────────────
        let mut sections = [SectionSpan {
            offset: 0,
            count: 0,
        }; 10];
        let mut expected_offset = directory_end;
        for (i, section) in TraSection::ALL.iter().enumerate() {
            let id = cur.u32()?;
            let record_size = cur.u32()? as usize;
            let count = cur.u32()? as usize;
            let reserved = cur.u32()?;
            let offset = cur.u64()?;
            if id != *section as u32 || record_size != section.record_size() || reserved != 0 {
                return Err(invalid(format!(
                    "TRA directory entry {} is not the {} section of {}-byte records",
                    i,
                    section.name(),
                    section.record_size()
                )));
            }
            if offset != expected_offset as u64 {
                return Err(invalid(format!(
                    "TRA {} section at offset {} (expected {})",
                    section.name(),
                    offset,
                    expected_offset
                )));
            }
            let end = count
                .checked_mul(record_size)
                .and_then(|len| expected_offset.checked_add(len))
                .filter(|&end| end <= data.len())
                .ok_or_else(|| {
                    invalid(format!(
                        "TRA {} section declares {} records of {} bytes past the end of the file",
                        section.name(),
                        count,
                        record_size
                    ))
                })?;
            sections[i] = SectionSpan {
                offset: expected_offset,
                count,
            };
            expected_offset = end;
        }
        if expected_offset != data.len() {
            return Err(invalid(format!(
                "{} unexpected trailing bytes before the TRA checksum",
                data.len() - expected_offset
            )));
        }

        Ok(Self {
            data,
            format_version,
            hashes,
            sections,
        })
    }

    /// Number of records in `section`.
    #[inline]
    pub fn count(&self, section: TraSection) -> usize {
        self.sections[section as usize].count
    }

    /// The raw bytes of `section` (`count × record_size` bytes).
    pub fn section_bytes(&self, section: TraSection) -> &'a [u8] {
        let s = self.sections[section as usize];
        &self.data[s.offset..s.offset + s.count * section.record_size()]
    }

    fn cursor(&self, section: TraSection) -> Cursor<'a> {
        Cursor::new(self.section_bytes(section))
    }
}

/// Orders and references every consumer of the artifact relies on.
fn validate_tables(a: &TraceabilityArtifact) -> io::Result<()> {
    let n_sym = a.bi_sym.len();
    let n_cs = a.bi_cs.len();

    if a.entities.len() != n_sym {
        return Err(invalid(format!(
            "TRA entity table has {} records for {} symbols",
            a.entities.len(),
            n_sym
        )));
    }

    for (i, r) in a.sym_span.iter().enumerate() {
        if r.sym_id as usize >= n_sym || r.last_token_id < r.first_token_id {
            return Err(invalid(format!(
                "symbol span {} references sym_id={} or has an inverted range",
                i, r.sym_id
            )));
        }
        if r.parent != NO_PARENT && r.parent != PARENT_NOT_NESTED && r.parent as usize >= i {
            return Err(invalid(format!(
                "symbol span {} has parent {} that does not precede it",
                i, r.parent
            )));
        }
    }
    // Builder order: (file, first token, last token descending, sym_id).
    let span_key = |r: &SymbolSpanRecord| {
        (
            r.file_id,
            r.first_token_id,
            u32::MAX - r.last_token_id,
            r.sym_id,
        )
    };
    if a.sym_span.windows(2).any(|w| span_key(&w[0]) > span_key(&w[1])) {
        return Err(invalid("TRA symbol spans are not sorted"));
    }

    if let Some(r) = a.cs_span.iter().find(|r| r.call_site_id as usize >= n_cs) {
        return Err(invalid(format!(
            "call-site span references call_site_id={} of {} sites",
            r.call_site_id, n_cs
        )));
    }
    let cs_key = |r: &CallSiteSpanRecord| (r.file_id, r.first_token_id, r.call_site_id);
    if a.cs_span.windows(2).any(|w| cs_key(&w[0]) > cs_key(&w[1])) {
        return Err(invalid("TRA call-site spans are not sorted"));
    }

    if let Some(l) = a
        .uml_links
        .iter()
        .find(|l| l.sym_id as usize >= n_sym || l._reserved != [0; 3])
    {
        return Err(invalid(format!(
            "UMLLink of sym_id={} references an unknown symbol or has reserved bytes set",
            l.sym_id
        )));
    }
    if a.uml_links.windows(2).any(|w| w[0].sym_id >= w[1].sym_id) {
        return Err(invalid("TRA UMLLinks are not strictly sorted by sym_id"));
    }

    if a.function_index.windows(2).any(|w| w[0].sym_id >= w[1].sym_id) {
        return Err(invalid("TRA function index is not strictly sorted by sym_id"));
    }
    for r in &a.function_index {
        let blocks_ok = r.first_block == NO_OFFSET
            || (r.first_block as u64 + r.block_count as u64) <= a.bi_blk.len() as u64;
        let ssa_ok = r.first_ssa == NO_OFFSET
            || (r.first_ssa as u64 + r.ssa_count as u64) <= a.bi_ssa.len() as u64;
        if !blocks_ok || !ssa_ok {
            return Err(invalid(format!(
                "function record of sym_id={} points outside the block/SSA tables",
                r.sym_id
            )));
        }
    }
    Ok(())
}

/// `InvalidData` error with a message.
fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Bounds-checked little-endian reader over an untrusted byte slice.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(invalid(format!(
                "TRA truncated: needed {} bytes at offset {}, {} remain",
                n,
                self.pos,
                self.remaining()
            )));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let s = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.array::<2>()?))
    }

    fn u24(&mut self) -> io::Result<u32> {
        let b = self.array::<3>()?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array::<4>()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array::<8>()?))
    }
}
