//! Binary Serializer & Deserializer for CallGraphArtifact (.cga) (§6.6).
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::core::io::binary::{BinaryReader, BinaryWriter};
use crate::core::types::*;
use crate::ingestion::serializer::crc64_ecma;
use std::fs::File;
use std::io::{Read, Result as IoResult, Write};
use std::path::Path;

pub const CGA_MAGIC: u64 = 0x4347410001000000; // "CGA\0\x01\0\0\0"
pub const CGA_FORMAT_VERSION: u32 = 1;

pub struct CGASerializer;

impl CGASerializer {
    /// Serialize CallGraphArtifact to `.cga` binary file
    pub fn serialize(artifact: &CallGraphArtifact, out_path: &Path) -> IoResult<()> {
        let mut buf = Vec::new();
        {
            let mut w = BinaryWriter::new(&mut buf);

            // Header (64 bytes)
            w.write_u64(CGA_MAGIC)?;
            w.write_u32(CGA_FORMAT_VERSION)?;
            w.write_u32(artifact.method_count)?;
            w.write_u32(artifact.call_site_count)?;
            w.write_u32(artifact.call_edge_count)?;
            w.write_u64(artifact.ssa_hash)?;
            w.write_u64(artifact.sta_hash)?;
            w.write_bytes(&[0u8; 24])?;

            // 1. Call Site Table (n_sites * 28B)
            for site in &artifact.call_sites {
                w.write_u32(site.call_site_id)?;
                w.write_u32(site.caller_sym)?;
                w.write_u32(site.call_node)?;
                w.write_u32(site.receiver_ssa)?;
                w.write_u32(site.call_block)?;
                w.write_u32(site.call_token)?;
                w.write_u8(site.call_type)?;
                w.write_u8(site.flags)?;
                w.write_u16(site.arg_count)?;
            }

            // 2. Callee CSR
            w.write_u32(artifact.callee_csr.offsets.len() as u32)?;
            for &off in &artifact.callee_csr.offsets {
                w.write_u32(off)?;
            }
            w.write_u32(artifact.callee_csr.adj.len() as u32)?;
            for &a in &artifact.callee_csr.adj {
                w.write_u32(a)?;
            }
            for &et in &artifact.callee_csr.edge_types {
                w.write_u8(et)?;
            }

            // 3. Caller CSR
            w.write_u32(artifact.caller_csr.offsets.len() as u32)?;
            for &off in &artifact.caller_csr.offsets {
                w.write_u32(off)?;
            }
            w.write_u32(artifact.caller_csr.adj.len() as u32)?;
            for &a in &artifact.caller_csr.adj {
                w.write_u32(a)?;
            }

            // 4. Site-to-Edge Map
            w.write_u32(artifact.site_to_edge_map.len() as u32)?;
            for &(caller, callee, site_id) in &artifact.site_to_edge_map {
                w.write_u32(caller)?;
                w.write_u32(callee)?;
                w.write_u32(site_id)?;
            }

            // 5. Points-To Table
            w.write_u32(artifact.points_to_table.len() as u32)?;
            for pt in &artifact.points_to_table {
                w.write_u32(pt.ssa_id)?;
                w.write_u32(pt.alloc_type_sym_id)?;
            }

            // 6. SCC Table & Members
            w.write_u32(artifact.sccs.len() as u32)?;
            for scc in &artifact.sccs {
                w.write_u32(scc.scc_id)?;
                w.write_u32(scc.member_offset)?;
                w.write_u16(scc.member_count)?;
                w.write_u8(scc.scc_class)?;
                w.write_u8(0)?;
            }
            w.write_u32(artifact.scc_members.len() as u32)?;
            for &m in &artifact.scc_members {
                w.write_u32(m)?;
            }
        }

        let checksum = crc64_ecma(&buf);
        let mut w = BinaryWriter::new(&mut buf);
        w.write_u64(checksum)?;

        let mut file = File::create(out_path)?;
        file.write_all(&buf)?;
        file.flush()?;

        Ok(())
    }

    /// Deserialize `.cga` binary file to CallGraphArtifact
    pub fn deserialize(in_path: &Path) -> IoResult<CallGraphArtifact> {
        let mut file = File::open(in_path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() < 72 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "CGA artifact too small",
            ));
        }

        let payload_len = buf.len() - 8;
        let expected_checksum = u64::from_le_bytes(buf[payload_len..].try_into().unwrap());
        let computed_checksum = crc64_ecma(&buf[..payload_len]);

        if expected_checksum != computed_checksum {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "CGA checksum mismatch: expected {:#X}, got {:#X}",
                    expected_checksum, computed_checksum
                ),
            ));
        }

        let mut r = BinaryReader::new(&buf[..payload_len]);

        let magic = r.read_u64()?;
        if magic != CGA_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid CGA magic header",
            ));
        }

        let format_version = r.read_u32()?;
        let method_count = r.read_u32()?;
        let call_site_count = r.read_u32()?;
        let call_edge_count = r.read_u32()?;
        let ssa_hash = r.read_u64()?;
        let sta_hash = r.read_u64()?;
        for _ in 0..24 {
            r.read_u8()?;
        }

        // 1. Call Site Table
        let mut call_sites = Vec::with_capacity(call_site_count as usize);
        for _ in 0..call_site_count {
            call_sites.push(CallSite::new(
                r.read_u32()?,
                r.read_u32()?,
                r.read_u32()?,
                r.read_u32()?,
                r.read_u32()?,
                r.read_u32()?,
                r.read_u8()?,
                r.read_u8()?,
                r.read_u16()?,
            ));
        }

        // 2. Callee CSR
        let off_len = r.read_u32()? as usize;
        let mut callee_offsets = Vec::with_capacity(off_len);
        for _ in 0..off_len {
            callee_offsets.push(r.read_u32()?);
        }

        let adj_len = r.read_u32()? as usize;
        let mut callee_adj = Vec::with_capacity(adj_len);
        for _ in 0..adj_len {
            callee_adj.push(r.read_u32()?);
        }

        let mut callee_edge_types = Vec::with_capacity(adj_len);
        for _ in 0..adj_len {
            callee_edge_types.push(r.read_u8()?);
        }

        let callee_csr = CGCSR {
            offsets: callee_offsets,
            adj: callee_adj,
            edge_types: callee_edge_types,
        };

        // 3. Caller CSR
        let caller_off_len = r.read_u32()? as usize;
        let mut caller_offsets = Vec::with_capacity(caller_off_len);
        for _ in 0..caller_off_len {
            caller_offsets.push(r.read_u32()?);
        }

        let caller_adj_len = r.read_u32()? as usize;
        let mut caller_adj = Vec::with_capacity(caller_adj_len);
        for _ in 0..caller_adj_len {
            caller_adj.push(r.read_u32()?);
        }

        let caller_csr = CGCSR {
            offsets: caller_offsets,
            adj: caller_adj,
            edge_types: Vec::new(),
        };

        // 4. Site-to-Edge Map
        let map_len = r.read_u32()? as usize;
        let mut site_to_edge_map = Vec::with_capacity(map_len);
        for _ in 0..map_len {
            site_to_edge_map.push((r.read_u32()?, r.read_u32()?, r.read_u32()?));
        }

        // 5. Points-To Table
        let pt_len = r.read_u32()? as usize;
        let mut points_to_table = Vec::with_capacity(pt_len);
        for _ in 0..pt_len {
            points_to_table.push(PointsToEntry {
                ssa_id: r.read_u32()?,
                alloc_type_sym_id: r.read_u32()?,
            });
        }

        // 6. SCC Table & Members
        let scc_len = r.read_u32()? as usize;
        let mut sccs = Vec::with_capacity(scc_len);
        for _ in 0..scc_len {
            let scc = SCCRecord::new(r.read_u32()?, r.read_u32()?, r.read_u16()?, r.read_u8()?);
            r.read_u8()?;
            sccs.push(scc);
        }

        let scc_mem_len = r.read_u32()? as usize;
        let mut scc_members = Vec::with_capacity(scc_mem_len);
        for _ in 0..scc_mem_len {
            scc_members.push(r.read_u32()?);
        }

        Ok(CallGraphArtifact {
            format_version,
            method_count,
            call_site_count,
            call_edge_count,
            ssa_hash,
            sta_hash,
            call_sites,
            callee_csr,
            caller_csr,
            site_to_edge_map,
            points_to_table,
            sccs,
            scc_members,
        })
    }
}
