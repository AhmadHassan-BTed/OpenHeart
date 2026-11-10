//! Binary Serializer & Deserializer for CallGraphArtifact (.cga) (§6.6).
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::core::io::binary::{BinaryReader, BinaryWriter};
use crate::core::types::*;
use crate::ingestion::serializer::crc64_ecma;
use std::fs::File;
use std::io::{Read, Result as IoResult, Write};
use std::path::Path;

pub const CGA_MAGIC: u64 = 0x4347410001000000; // "CGA\0\x01\0\0\0"
/// Version 2: `receiver_ssa` and points-to `ssa_id` are global SSA ids, `call_block` is
/// `u32::MAX` for calls outside any CFG block, and SCCs cover only call-graph nodes.
/// Version 3: SCC records are the full 16-byte `SCCRecord` (`member_count` is a u32).
/// Version 4: two trailing sections, the dispatch/resolution kind of every
/// `site_to_edge_map` entry (2 bytes each) and the unresolved call sites (13 bytes each).
pub const CGA_FORMAT_VERSION: u32 = 4;

pub struct CGASerializer;

fn invalid(msg: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

impl CGASerializer {
    /// Encodes the artifact into the `.cga` byte layout (payload followed by its CRC-64).
    pub fn to_bytes(artifact: &CallGraphArtifact) -> IoResult<Vec<u8>> {
        if artifact.edge_kinds.len() != artifact.site_to_edge_map.len() {
            return Err(invalid(format!(
                "CGA edge kinds ({}) do not match the site-to-edge map ({})",
                artifact.edge_kinds.len(),
                artifact.site_to_edge_map.len()
            )));
        }
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
            for i in 0..artifact.caller_csr.adj.len() {
                w.write_u8(artifact.caller_csr.edge_types.get(i).copied().unwrap_or(0))?;
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
                w.write_u32(scc.member_count)?;
                w.write_u8(scc.scc_class)?;
                w.write_bytes(&[0u8; 3])?;
            }
            w.write_u32(artifact.scc_members.len() as u32)?;
            for &m in &artifact.scc_members {
                w.write_u32(m)?;
            }

            // 7. Edge kinds (one per site-to-edge entry, same order)
            w.write_u32(artifact.edge_kinds.len() as u32)?;
            for k in &artifact.edge_kinds {
                w.write_u8(k.dispatch)?;
                w.write_u8(k.resolution)?;
            }

            // 8. Unresolved call sites
            w.write_u32(artifact.unresolved_sites.len() as u32)?;
            for u in &artifact.unresolved_sites {
                w.write_u32(u.call_site_id)?;
                w.write_u8(u.reason)?;
                w.write_u32(u.name_id)?;
                w.write_u32(u.type_sym)?;
            }
        }

        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        Ok(buf)
    }

    /// Serialize CallGraphArtifact to `.cga` binary file
    pub fn serialize(artifact: &CallGraphArtifact, out_path: &Path) -> IoResult<()> {
        let buf = Self::to_bytes(artifact)?;
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
        Self::from_bytes(&buf)
    }

    /// Decodes a `.cga` byte buffer (checksum, magic and version are verified).
    pub fn from_bytes(buf: &[u8]) -> IoResult<CallGraphArtifact> {
        if buf.len() < 72 {
            return Err(invalid("CGA artifact too small".to_string()));
        }

        let payload_len = buf.len() - 8;
        let mut tail = [0u8; 8];
        tail.copy_from_slice(&buf[payload_len..]);
        let expected_checksum = u64::from_le_bytes(tail);
        let computed_checksum = crc64_ecma(&buf[..payload_len]);

        if expected_checksum != computed_checksum {
            return Err(invalid(format!(
                "CGA checksum mismatch: expected {:#X}, got {:#X}",
                expected_checksum, computed_checksum
            )));
        }

        let mut r = BinaryReader::new(&buf[..payload_len]);

        let magic = r.read_u64()?;
        if magic != CGA_MAGIC {
            return Err(invalid("Invalid CGA magic header".to_string()));
        }

        let format_version = r.read_u32()?;
        if format_version != CGA_FORMAT_VERSION {
            return Err(invalid(format!(
                "Unsupported CGA format version {} (expected {})",
                format_version, CGA_FORMAT_VERSION
            )));
        }
        let method_count = r.read_u32()?;
        let call_site_count = r.read_u32()?;
        let call_edge_count = r.read_u32()?;
        let ssa_hash = r.read_u64()?;
        let sta_hash = r.read_u64()?;
        for _ in 0..24 {
            r.read_u8()?;
        }

        // Every count is bounded by the payload length before anything is allocated.
        let bounded = |count: usize, elem: usize, what: &str| -> IoResult<usize> {
            match count.checked_mul(elem) {
                Some(bytes) if bytes <= payload_len => Ok(count),
                _ => Err(invalid(format!(
                    "CGA {} count {} exceeds the artifact size",
                    what, count
                ))),
            }
        };

        // 1. Call Site Table
        let n_sites = bounded(call_site_count as usize, 28, "call-site")?;
        let mut call_sites = Vec::with_capacity(n_sites);
        for _ in 0..n_sites {
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
        let off_len = bounded(r.read_u32()? as usize, 4, "callee offset")?;
        let mut callee_offsets = Vec::with_capacity(off_len);
        for _ in 0..off_len {
            callee_offsets.push(r.read_u32()?);
        }

        let adj_len = bounded(r.read_u32()? as usize, 5, "callee edge")?;
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
        let caller_off_len = bounded(r.read_u32()? as usize, 4, "caller offset")?;
        let mut caller_offsets = Vec::with_capacity(caller_off_len);
        for _ in 0..caller_off_len {
            caller_offsets.push(r.read_u32()?);
        }

        let caller_adj_len = bounded(r.read_u32()? as usize, 5, "caller edge")?;
        let mut caller_adj = Vec::with_capacity(caller_adj_len);
        for _ in 0..caller_adj_len {
            caller_adj.push(r.read_u32()?);
        }

        let mut caller_edge_types = Vec::with_capacity(caller_adj_len);
        for _ in 0..caller_adj_len {
            caller_edge_types.push(r.read_u8()?);
        }

        let caller_csr = CGCSR {
            offsets: caller_offsets,
            adj: caller_adj,
            edge_types: caller_edge_types,
        };

        // 4. Site-to-Edge Map
        let map_len = bounded(r.read_u32()? as usize, 12, "site-to-edge")?;
        let mut site_to_edge_map = Vec::with_capacity(map_len);
        for _ in 0..map_len {
            site_to_edge_map.push((r.read_u32()?, r.read_u32()?, r.read_u32()?));
        }

        // 5. Points-To Table
        let pt_len = bounded(r.read_u32()? as usize, 8, "points-to")?;
        let mut points_to_table = Vec::with_capacity(pt_len);
        for _ in 0..pt_len {
            points_to_table.push(PointsToEntry {
                ssa_id: r.read_u32()?,
                alloc_type_sym_id: r.read_u32()?,
            });
        }

        // 6. SCC Table & Members
        let scc_len = bounded(r.read_u32()? as usize, 16, "SCC")?;
        let mut sccs = Vec::with_capacity(scc_len);
        for _ in 0..scc_len {
            let scc = SCCRecord::new(r.read_u32()?, r.read_u32()?, r.read_u32()?, r.read_u8()?);
            for _ in 0..3 {
                r.read_u8()?;
            }
            sccs.push(scc);
        }

        let scc_mem_len = bounded(r.read_u32()? as usize, 4, "SCC member")?;
        let mut scc_members = Vec::with_capacity(scc_mem_len);
        for _ in 0..scc_mem_len {
            scc_members.push(r.read_u32()?);
        }

        // 7. Edge kinds
        let kinds_len = bounded(r.read_u32()? as usize, 2, "edge-kind")?;
        if kinds_len != site_to_edge_map.len() {
            return Err(invalid(format!(
                "CGA edge kinds ({}) do not match the site-to-edge map ({})",
                kinds_len,
                site_to_edge_map.len()
            )));
        }
        let mut edge_kinds = Vec::with_capacity(kinds_len);
        for _ in 0..kinds_len {
            edge_kinds.push(SiteEdgeKind {
                dispatch: r.read_u8()?,
                resolution: r.read_u8()?,
            });
        }

        // 8. Unresolved call sites
        let unresolved_len = bounded(r.read_u32()? as usize, 13, "unresolved-site")?;
        let mut unresolved_sites = Vec::with_capacity(unresolved_len);
        for _ in 0..unresolved_len {
            unresolved_sites.push(UnresolvedCallSite {
                call_site_id: r.read_u32()?,
                reason: r.read_u8()?,
                name_id: r.read_u32()?,
                type_sym: r.read_u32()?,
            });
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
            edge_kinds,
            unresolved_sites,
        })
    }
}
