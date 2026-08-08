//! Binary BPASTSerializer for writing and reading .bpa binary files with CRC-64 integrity.

use crate::phase1::serializer::crc64_ecma;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use super::bp_encoder::BPEncoder;
use super::builder::BPASTArtifact;
use super::jump_table::JumpTable;
use super::preorder::PreorderArrays;
use super::rank_select::RankSelectIndex;
use super::rmq::SparseTableRMQ;

pub const BPA_MAGIC: &[u8; 8] = b"BPAST\x00\x01\x00";

pub struct BPASTSerializer;

impl BPASTSerializer {
    pub fn write(artifact: &BPASTArtifact, out_path: &Path) -> std::io::Result<()> {
        let mut file = File::create(out_path)?;
        let mut buf = Vec::with_capacity(1024 * 1024);

        // Header (64 bytes)
        buf.extend_from_slice(BPA_MAGIC);
        buf.extend_from_slice(&1u32.to_le_bytes()); // format_version
        buf.extend_from_slice(&artifact.node_count.to_le_bytes());
        buf.extend_from_slice(&(artifact.bp_encoder.bit_count as u32).to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // file_count
        buf.extend_from_slice(&[0u8; 34]); // reserved
        buf.extend_from_slice(&artifact.tca_hash.to_le_bytes());

        // Section 1: BP Bitstring
        buf.extend_from_slice(&(artifact.bp_encoder.words.len() as u32).to_le_bytes());
        for &w in &artifact.bp_encoder.words {
            buf.extend_from_slice(&w.to_le_bytes());
        }

        // Section 2: Jump Table
        buf.extend_from_slice(&(artifact.jump_table.match_pos.len() as u32).to_le_bytes());
        for &m in &artifact.jump_table.match_pos {
            buf.extend_from_slice(&m.to_le_bytes());
        }

        // Section 3: Preorder Arrays
        buf.extend_from_slice(&artifact.preorder.node_types);

        buf.extend_from_slice(&(artifact.preorder.node_attrs.len() as u32).to_le_bytes());
        for &attr in &artifact.preorder.node_attrs {
            buf.extend_from_slice(&attr.to_le_bytes());
        }

        buf.extend_from_slice(&(artifact.preorder.token_ranges.len() as u32).to_le_bytes());
        for &(r0, r1) in &artifact.preorder.token_ranges {
            buf.extend_from_slice(&r0.to_le_bytes());
            buf.extend_from_slice(&r1.to_le_bytes());
        }

        buf.extend_from_slice(&(artifact.preorder.parent_map.len() as u32).to_le_bytes());
        for &p in &artifact.preorder.parent_map {
            buf.extend_from_slice(&p.to_le_bytes());
        }

        // Checksum
        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());

        file.write_all(&buf)?;
        Ok(())
    }

    pub fn read(in_path: &Path) -> std::io::Result<BPASTArtifact> {
        let mut file = File::open(in_path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() < 72 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BPA file too small",
            ));
        }

        let checksum_pos = buf.len() - 8;
        let stored_checksum = u64::from_le_bytes(buf[checksum_pos..].try_into().unwrap());
        let computed_checksum = crc64_ecma(&buf[..checksum_pos]);

        if stored_checksum != computed_checksum {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BPA CRC-64 Checksum mismatch",
            ));
        }

        let mut offset = 0;
        let magic = &buf[offset..offset + 8];
        if magic != BPA_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid BPA Magic Header",
            ));
        }
        offset += 8;

        let _format_ver = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let node_count = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let bit_count = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let _file_count = u16::from_le_bytes(buf[offset..offset + 2].try_into().unwrap());
        offset += 2;
        offset += 34; // skip reserved
        let tca_hash = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
        offset += 8;

        // BP Encoder Words
        let words_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut words = Vec::with_capacity(words_len);
        for _ in 0..words_len {
            words.push(u64::from_le_bytes(
                buf[offset..offset + 8].try_into().unwrap(),
            ));
            offset += 8;
        }
        let bp_encoder = BPEncoder { words, bit_count };

        // Jump Table
        let match_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut match_pos = Vec::with_capacity(match_len);
        for _ in 0..match_len {
            match_pos.push(u32::from_le_bytes(
                buf[offset..offset + 4].try_into().unwrap(),
            ));
            offset += 4;
        }
        let jump_table = JumpTable { match_pos };

        // Preorder Arrays
        let node_types = buf[offset..offset + node_count as usize].to_vec();
        offset += node_count as usize;

        let attrs_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut node_attrs = Vec::with_capacity(attrs_len);
        for _ in 0..attrs_len {
            node_attrs.push(u32::from_le_bytes(
                buf[offset..offset + 4].try_into().unwrap(),
            ));
            offset += 4;
        }

        let ranges_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut token_ranges = Vec::with_capacity(ranges_len);
        for _ in 0..ranges_len {
            let r0 = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            offset += 4;
            let r1 = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            offset += 4;
            token_ranges.push((r0, r1));
        }

        let parents_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut parent_map = Vec::with_capacity(parents_len);
        for _ in 0..parents_len {
            parent_map.push(u32::from_le_bytes(
                buf[offset..offset + 4].try_into().unwrap(),
            ));
            offset += 4;
        }

        let preorder = PreorderArrays {
            node_types,
            node_attrs,
            token_ranges,
            parent_map,
        };

        let rank_select = RankSelectIndex::build(&bp_encoder);
        let rmq = SparseTableRMQ::build(&bp_encoder, &rank_select);

        Ok(BPASTArtifact {
            node_count,
            bp_encoder,
            jump_table,
            rank_select,
            rmq,
            preorder,
            tca_hash,
        })
    }
}
