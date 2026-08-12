//! Traceability Artifact Binary Serializer & Deserializer (§7.6).

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::ingestion::serializer::crc64_ecma;
use crate::tra::types::*;

pub struct TraceabilitySerializer;

impl TraceabilitySerializer {
    pub fn write(artifact: &TraceabilityArtifact, out_path: &Path) -> std::io::Result<()> {
        let mut buf = Vec::new();

        // 64-byte Header
        buf.extend_from_slice(&TRA_MAGIC.to_le_bytes()); // 0..8
        buf.extend_from_slice(&artifact.format_version.to_le_bytes()); // 8..12
        buf.extend_from_slice(&(artifact.bi_ast.len() as u32).to_le_bytes()); // 12..16
        buf.extend_from_slice(&(artifact.bi_sym.len() as u32).to_le_bytes()); // 16..20
        buf.extend_from_slice(&(artifact.bi_blk.len() as u32).to_le_bytes()); // 20..24
        buf.extend_from_slice(&(artifact.bi_ssa.len() as u32).to_le_bytes()); // 24..28
        buf.extend_from_slice(&(artifact.bi_cs.len() as u32).to_le_bytes()); // 28..32
        buf.extend_from_slice(&(artifact.uml_links.len() as u32).to_le_bytes()); // 32..36
        buf.extend_from_slice(&artifact.hashes.scpg_hash.to_le_bytes()); // 36..40
        buf.extend_from_slice(&artifact.hashes.tca_hash.to_le_bytes()); // 40..48
        buf.extend_from_slice(&artifact.hashes.bpa_hash.to_le_bytes()); // 48..56
        buf.extend_from_slice(&artifact.hashes.sta_hash.to_le_bytes()); // 56..64

        // Remaining 3 hashes
        buf.extend_from_slice(&artifact.hashes.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.ssa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.hashes.cga_hash.to_le_bytes());

        // BI_AST section
        for entry in &artifact.bi_ast {
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }

        // BI_SYM section
        for entry in &artifact.bi_sym {
            buf.extend_from_slice(&entry.decl_first_tok.to_le_bytes());
            buf.extend_from_slice(&entry.decl_last_tok.to_le_bytes());
            buf.extend_from_slice(&entry.def_first_tok.to_le_bytes());
            buf.extend_from_slice(&entry.def_last_tok.to_le_bytes());
        }

        // BI_BLK section
        for entry in &artifact.bi_blk {
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }

        // BI_SSA section
        for entry in &artifact.bi_ssa {
            buf.extend_from_slice(&entry.def_stmt.to_le_bytes());
            buf.extend_from_slice(&entry.first_token_id.to_le_bytes());
            buf.extend_from_slice(&entry.last_token_id.to_le_bytes());
        }

        // BI_CS section
        for entry in &artifact.bi_cs {
            buf.extend_from_slice(&entry.call_token.to_le_bytes());
        }

        // Symbol Span Index section
        for rec in &artifact.sym_span {
            buf.extend_from_slice(&rec.first_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.last_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.sym_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.extend_from_slice(&rec.line_start.to_le_bytes());
            buf.extend_from_slice(&rec.col_start.to_le_bytes());
            buf.extend_from_slice(&rec.line_end.to_le_bytes());
        }

        // Call Site Span Index section
        for rec in &artifact.cs_span {
            buf.extend_from_slice(&rec.first_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.call_site_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.extend_from_slice(&rec.line_start.to_le_bytes());
        }

        // UMLLink Table section
        for rec in &artifact.uml_links {
            buf.extend_from_slice(&rec.sym_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            let line_start_bytes = rec.line_start.to_le_bytes();
            buf.extend_from_slice(&line_start_bytes[0..3]); // u24
            buf.extend_from_slice(&rec.col_start.to_le_bytes());
            let line_end_bytes = rec.line_end.to_le_bytes();
            buf.extend_from_slice(&line_end_bytes[0..3]); // u24
            buf.extend_from_slice(&rec.col_end.to_le_bytes());
            buf.extend_from_slice(&rec.scpg_hash.to_le_bytes());
            buf.push(rec.sym_kind);
            buf.extend_from_slice(&rec._reserved);
        }

        // Compute and append CRC-64 checksum
        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());

        let mut file = File::create(out_path)?;
        file.write_all(&buf)?;
        Ok(())
    }

    pub fn deserialize(path: &Path) -> std::io::Result<TraceabilityArtifact> {
        let mut file = File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() < 88 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TRA artifact file too small",
            ));
        }

        let payload_len = buf.len() - 8;
        let expected_crc = u64::from_le_bytes(buf[payload_len..].try_into().unwrap());
        let computed_crc = crc64_ecma(&buf[..payload_len]);

        if expected_crc != computed_crc {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("TRA CRC-64 mismatch: expected {:#X}, got {:#X}", expected_crc, computed_crc),
            ));
        }

        let magic = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        if magic != TRA_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid TRA magic bytes",
            ));
        }

        let format_version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let n_ast = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
        let n_sym = u32::from_le_bytes(buf[16..20].try_into().unwrap()) as usize;
        let n_blk = u32::from_le_bytes(buf[20..24].try_into().unwrap()) as usize;
        let n_ssa = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
        let n_cs = u32::from_le_bytes(buf[28..32].try_into().unwrap()) as usize;
        let n_uml = u32::from_le_bytes(buf[32..36].try_into().unwrap()) as usize;
        let scpg_hash = u32::from_le_bytes(buf[36..40].try_into().unwrap());
        let tca_hash = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let bpa_hash = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let sta_hash = u64::from_le_bytes(buf[56..64].try_into().unwrap());

        let cfa_hash = u64::from_le_bytes(buf[64..72].try_into().unwrap());
        let ssa_hash = u64::from_le_bytes(buf[72..80].try_into().unwrap());
        let cga_hash = u64::from_le_bytes(buf[80..88].try_into().unwrap());

        let hashes = ScpgHashChain {
            tca_hash,
            bpa_hash,
            sta_hash,
            cfa_hash,
            ssa_hash,
            cga_hash,
            scpg_hash,
        };

        let mut offset = 88;

        let mut bi_ast = Vec::with_capacity(n_ast);
        for _ in 0..n_ast {
            let ft = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let lt = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            bi_ast.push(BIAstEntry { first_token_id: ft, last_token_id: lt });
            offset += 8;
        }

        let mut bi_sym = Vec::with_capacity(n_sym);
        for _ in 0..n_sym {
            let dft = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let dlt = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            let bft = u32::from_le_bytes(buf[offset + 8..offset + 12].try_into().unwrap());
            let blt = u32::from_le_bytes(buf[offset + 12..offset + 16].try_into().unwrap());
            bi_sym.push(BISymEntry { decl_first_tok: dft, decl_last_tok: dlt, def_first_tok: bft, def_last_tok: blt });
            offset += 16;
        }

        let mut bi_blk = Vec::with_capacity(n_blk);
        for _ in 0..n_blk {
            let ft = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let lt = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            bi_blk.push(BIBlkEntry { first_token_id: ft, last_token_id: lt });
            offset += 8;
        }

        let mut bi_ssa = Vec::with_capacity(n_ssa);
        for _ in 0..n_ssa {
            let ds = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let ft = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            let lt = u32::from_le_bytes(buf[offset + 8..offset + 12].try_into().unwrap());
            bi_ssa.push(BISsaEntry { def_stmt: ds, first_token_id: ft, last_token_id: lt });
            offset += 12;
        }

        let mut bi_cs = Vec::with_capacity(n_cs);
        for _ in 0..n_cs {
            let ct = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            bi_cs.push(BICsEntry { call_token: ct });
            offset += 4;
        }

        let mut sym_span = Vec::with_capacity(n_sym);
        for _ in 0..n_sym {
            let ft = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let lt = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            let sid = u32::from_le_bytes(buf[offset + 8..offset + 12].try_into().unwrap());
            let fid = u16::from_le_bytes(buf[offset + 12..offset + 14].try_into().unwrap());
            let ls = u16::from_le_bytes(buf[offset + 14..offset + 16].try_into().unwrap());
            let cs = u16::from_le_bytes(buf[offset + 16..offset + 18].try_into().unwrap());
            let le = u16::from_le_bytes(buf[offset + 18..offset + 20].try_into().unwrap());
            sym_span.push(SymbolSpanRecord { first_token_id: ft, last_token_id: lt, sym_id: sid, file_id: fid, line_start: ls, col_start: cs, line_end: le });
            offset += 20;
        }

        let mut cs_span = Vec::with_capacity(n_cs);
        for _ in 0..n_cs {
            let ft = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let csid = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            let fid = u16::from_le_bytes(buf[offset + 8..offset + 10].try_into().unwrap());
            let ls = u16::from_le_bytes(buf[offset + 10..offset + 12].try_into().unwrap());
            cs_span.push(CallSiteSpanRecord { first_token_id: ft, call_site_id: csid, file_id: fid, line_start: ls });
            offset += 12;
        }

        let mut uml_links = Vec::with_capacity(n_uml);
        for _ in 0..n_uml {
            let sid = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let fid = u16::from_le_bytes(buf[offset + 4..offset + 6].try_into().unwrap());
            let mut ls_bytes = [0u8; 4];
            ls_bytes[0..3].copy_from_slice(&buf[offset + 6..offset + 9]);
            let ls = u32::from_le_bytes(ls_bytes);
            let cs = u16::from_le_bytes(buf[offset + 9..offset + 11].try_into().unwrap());
            let mut le_bytes = [0u8; 4];
            le_bytes[0..3].copy_from_slice(&buf[offset + 11..offset + 14]);
            let le = u32::from_le_bytes(le_bytes);
            let ce = u16::from_le_bytes(buf[offset + 14..offset + 16].try_into().unwrap());
            let sh = u32::from_le_bytes(buf[offset + 16..offset + 20].try_into().unwrap());
            let sk = buf[offset + 20];
            let res = [buf[offset + 21], buf[offset + 22], buf[offset + 23]];

            uml_links.push(UMLLinkRecord {
                sym_id: sid,
                file_id: fid,
                line_start: ls,
                col_start: cs,
                line_end: le,
                col_end: ce,
                scpg_hash: sh,
                sym_kind: sk,
                _reserved: res,
            });
            offset += 24;
        }

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
        })
    }
}
