//! Traceability Artifact Binary Serializer & Deserializer (§7.6).
//!
//! **TRA File Format v2 (all integers little-endian):**
//!
//! ```text
//! Header (328 bytes)
//!   0  magic u64            8  version u32
//!  12  n_ast u32           16  n_sym u32         20  n_blk u32        24  n_ssa u32
//!  28  n_cs u32            32  n_uml u32         36  n_sym_span u32   40  n_cs_span u32
//!  44  scpg_hash u32       48  tca..cga hash tags 6 × u64
//!  96  n_func_index u32   100  n_entities u32
//! 104  artifact digests 6 × 32 B               296  chain digest 32 B
//! Sections (in order): BI_AST 8 B, BI_SYM 16 B, BI_BLK 8 B, BI_SSA 12 B, BI_CS 4 B,
//!   SYM_SPAN 28 B, CS_SPAN 14 B, UMLLINK 24 B (lines as u24), FUNCTION_INDEX 20 B,
//!   ENTITY 44 B
//! Checksum: CRC-64/ECMA-182 of everything before it (8 B)
//! ```
//!
//! The reader treats the file as untrusted: the checksum is verified first, every declared
//! count is validated against the remaining bytes before anything is allocated, every read
//! is bounds-checked, and the hash chain is re-verified from its per-artifact digests.
//! Malformed input yields `InvalidData`, never a panic.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::ingestion::serializer::crc64_ecma;
use crate::tra::types::*;
use crate::tra::uml_link::UMLLinkRegistry;

/// Fixed header size of format v2.
const HEADER_SIZE: usize = 328;
const CHECKSUM_SIZE: usize = 8;

const BI_AST_SIZE: usize = 8;
const BI_SYM_SIZE: usize = 16;
const BI_BLK_SIZE: usize = 8;
const BI_SSA_SIZE: usize = 12;
const BI_CS_SIZE: usize = 4;
const SYM_SPAN_SIZE: usize = 28;
const CS_SPAN_SIZE: usize = 14;
const UML_LINK_SIZE: usize = 24;
const FUNC_INDEX_SIZE: usize = 20;
const ENTITY_SIZE: usize = 44;

/// Largest value representable in the 24-bit UMLLink line fields.
const U24_MAX: u32 = 0x00FF_FFFF;

pub struct TraceabilitySerializer;

impl TraceabilitySerializer {
    pub fn write(artifact: &TraceabilityArtifact, out_path: &Path) -> std::io::Result<()> {
        let buf = Self::to_bytes(artifact)?;
        let mut file = File::create(out_path)?;
        file.write_all(&buf)?;
        Ok(())
    }

    pub fn to_bytes(artifact: &TraceabilityArtifact) -> std::io::Result<Vec<u8>> {
        let count = |n: usize, what: &str| -> std::io::Result<u32> {
            u32::try_from(n).map_err(|_| invalid(format!("too many {} for the TRA format", what)))
        };
        let mut buf = Vec::new();

        // Header.
        buf.extend_from_slice(&TRA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&artifact.format_version.to_le_bytes());
        buf.extend_from_slice(&count(artifact.bi_ast.len(), "AST entries")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.bi_sym.len(), "symbols")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.bi_blk.len(), "blocks")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.bi_ssa.len(), "SSA entries")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.bi_cs.len(), "call sites")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.uml_links.len(), "UMLLinks")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.sym_span.len(), "symbol spans")?.to_le_bytes());
        buf.extend_from_slice(&count(artifact.cs_span.len(), "call-site spans")?.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.scpg_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.tca_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.bpa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.sta_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.ssa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.cga_hash.to_le_bytes());
        buf.extend_from_slice(
            &count(artifact.function_index.len(), "function records")?.to_le_bytes(),
        );
        buf.extend_from_slice(&count(artifact.entities.len(), "entities")?.to_le_bytes());
        for d in &artifact.hashes.artifact_digests {
            buf.extend_from_slice(d);
        }
        buf.extend_from_slice(&artifact.hashes.chain_digest);
        debug_assert_eq!(buf.len(), HEADER_SIZE);

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

        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        Ok(buf)
    }

    pub fn deserialize(path: &Path) -> std::io::Result<TraceabilityArtifact> {
        let mut file = File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Self::from_bytes(&buf)
    }

    pub fn from_bytes(buf: &[u8]) -> std::io::Result<TraceabilityArtifact> {
        if buf.len() < HEADER_SIZE + CHECKSUM_SIZE {
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

        let mut cur = Cursor::new(&buf[..data_end]);
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
        let n_ast = cur.u32()? as usize;
        let n_sym = cur.u32()? as usize;
        let n_blk = cur.u32()? as usize;
        let n_ssa = cur.u32()? as usize;
        let n_cs = cur.u32()? as usize;
        let n_uml = cur.u32()? as usize;
        let n_sym_span = cur.u32()? as usize;
        let n_cs_span = cur.u32()? as usize;
        let scpg_hash = cur.u32()?;
        let tca_hash = cur.u64()?;
        let bpa_hash = cur.u64()?;
        let sta_hash = cur.u64()?;
        let cfa_hash = cur.u64()?;
        let ssa_hash = cur.u64()?;
        let cga_hash = cur.u64()?;
        let n_func = cur.u32()? as usize;
        let n_entities = cur.u32()? as usize;
        let mut artifact_digests = [[0u8; 32]; 6];
        for d in artifact_digests.iter_mut() {
            *d = cur.array::<32>()?;
        }
        let chain_digest = cur.array::<32>()?;

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

        cur.require(n_ast, BI_AST_SIZE, "BI_AST")?;
        let mut bi_ast = Vec::with_capacity(n_ast);
        for _ in 0..n_ast {
            bi_ast.push(BIAstEntry {
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        cur.require(n_sym, BI_SYM_SIZE, "BI_SYM")?;
        let mut bi_sym = Vec::with_capacity(n_sym);
        for _ in 0..n_sym {
            bi_sym.push(BISymEntry {
                decl_first_tok: cur.u32()?,
                decl_last_tok: cur.u32()?,
                def_first_tok: cur.u32()?,
                def_last_tok: cur.u32()?,
            });
        }

        cur.require(n_blk, BI_BLK_SIZE, "BI_BLK")?;
        let mut bi_blk = Vec::with_capacity(n_blk);
        for _ in 0..n_blk {
            bi_blk.push(BIBlkEntry {
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        cur.require(n_ssa, BI_SSA_SIZE, "BI_SSA")?;
        let mut bi_ssa = Vec::with_capacity(n_ssa);
        for _ in 0..n_ssa {
            bi_ssa.push(BISsaEntry {
                def_stmt: cur.u32()?,
                first_token_id: cur.u32()?,
                last_token_id: cur.u32()?,
            });
        }

        cur.require(n_cs, BI_CS_SIZE, "BI_CS")?;
        let mut bi_cs = Vec::with_capacity(n_cs);
        for _ in 0..n_cs {
            bi_cs.push(BICsEntry {
                call_token: cur.u32()?,
            });
        }

        cur.require(n_sym_span, SYM_SPAN_SIZE, "symbol span index")?;
        let mut sym_span = Vec::with_capacity(n_sym_span);
        for _ in 0..n_sym_span {
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
        for (i, r) in sym_span.iter().enumerate() {
            if r.parent != NO_PARENT && r.parent != PARENT_NOT_NESTED && r.parent as usize >= i {
                return Err(invalid(format!(
                    "symbol span {} has parent {} that does not precede it",
                    i, r.parent
                )));
            }
        }

        cur.require(n_cs_span, CS_SPAN_SIZE, "call-site span index")?;
        let mut cs_span = Vec::with_capacity(n_cs_span);
        for _ in 0..n_cs_span {
            cs_span.push(CallSiteSpanRecord {
                first_token_id: cur.u32()?,
                call_site_id: cur.u32()?,
                file_id: cur.u16()?,
                line_start: cur.u32()?,
            });
        }

        cur.require(n_uml, UML_LINK_SIZE, "UMLLink table")?;
        let mut uml_links = Vec::with_capacity(n_uml);
        for _ in 0..n_uml {
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

        cur.require(n_func, FUNC_INDEX_SIZE, "function index")?;
        let mut function_index = Vec::with_capacity(n_func);
        for _ in 0..n_func {
            function_index.push(FunctionIndexRecord {
                sym_id: cur.u32()?,
                first_block: cur.u32()?,
                block_count: cur.u32()?,
                first_ssa: cur.u32()?,
                ssa_count: cur.u32()?,
            });
        }
        for r in &function_index {
            let blocks_ok = r.first_block == NO_OFFSET
                || (r.first_block as u64 + r.block_count as u64) <= n_blk as u64;
            let ssa_ok = r.first_ssa == NO_OFFSET
                || (r.first_ssa as u64 + r.ssa_count as u64) <= n_ssa as u64;
            if !blocks_ok || !ssa_ok {
                return Err(invalid(format!(
                    "function record of sym_id={} points outside the block/SSA tables",
                    r.sym_id
                )));
            }
        }

        cur.require(n_entities, ENTITY_SIZE, "entity table")?;
        let mut entities = Vec::with_capacity(n_entities);
        for _ in 0..n_entities {
            entities.push(EntityRecord {
                stable_key: cur.array::<16>()?,
                content_digest: cur.array::<16>()?,
                file_id: cur.u16()?,
                col_start: cur.u16()?,
                line_start: cur.u32()?,
                line_end: cur.u32()?,
            });
        }

        if cur.remaining() != 0 {
            return Err(invalid(format!(
                "{} unexpected trailing bytes before the TRA checksum",
                cur.remaining()
            )));
        }

        let uml_slots = UMLLinkRegistry::slots(&uml_links, bi_sym.len());

        Ok(TraceabilityArtifact {
            format_version,
            hashes,
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
        })
    }
}

/// `InvalidData` error with a message.
fn invalid(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
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

    /// Fail unless `count` records of `size` bytes remain (checked arithmetic).
    fn require(&self, count: usize, size: usize, what: &str) -> std::io::Result<()> {
        match count.checked_mul(size) {
            Some(needed) if needed <= self.remaining() => Ok(()),
            _ => Err(invalid(format!(
                "TRA {} declares {} records of {} bytes but only {} bytes remain",
                what,
                count,
                size,
                self.remaining()
            ))),
        }
    }

    fn take(&mut self, n: usize) -> std::io::Result<&'a [u8]> {
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

    fn array<const N: usize>(&mut self) -> std::io::Result<[u8; N]> {
        let s = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn u8(&mut self) -> std::io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> std::io::Result<u16> {
        Ok(u16::from_le_bytes(self.array::<2>()?))
    }

    fn u24(&mut self) -> std::io::Result<u32> {
        let b = self.array::<3>()?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }

    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.array::<4>()?))
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_le_bytes(self.array::<8>()?))
    }
}
