//! Binary BPASTSerializer for writing and reading .bpa binary files with CRC-64 integrity.
//!
//! Version 2 layout (little-endian):
//! ```text
//! HEADER (64 B)    magic [8] | version u32 | node_count u32 | bit_count u64 |
//!                  root_count u32 | section_count u32 | reserved [24] | tca_hash u64
//! DIRECTORY        section_count x (kind u32 | elem_size u32 | offset u64 | byte_len u64)
//! SECTIONS         each starts on an 8-byte boundary (zero padding in between)
//! CHECKSUM         CRC-64/ECMA u64 over everything above
//! ```
//! Sections: BP words (u64), node types (u8), node attrs (u32), token ranges (u32 pairs),
//! parent map (u32), name tokens (u32). The rank/select and range min-max indexes are
//! rebuilt from the BP words in O(n / 64) word operations on load.
//!
//! Version 1 files (unaligned length-prefixed sections, a per-bit jump table, no name
//! tokens) are still readable. Every length and offset is validated against the bytes
//! present before anything is allocated or indexed, and the decoded tree is structurally
//! validated (balanced parentheses, matching array lengths, parents preceding children).

use std::fs;
use std::io;
use std::path::Path;

use super::bp_encoder::BPEncoder;
use super::builder::BPASTArtifact;
use super::preorder::PreorderArrays;
use super::rank_select::RankSelectIndex;
use super::rmq::RangeMinMaxTree;
use crate::core::io::binary::{invalid_data, pad_to_alignment, ByteCursor};
use crate::core::io::fs::write_atomic;
use crate::ingestion::serializer::crc64_ecma;

pub const BPA_MAGIC: &[u8; 8] = b"BPAST\x00\x01\x00";
pub const BPA_VERSION: u32 = 2;
pub const BPA_HEADER_SIZE: usize = 64;
const SECTION_ALIGN: usize = 8;
const DIRECTORY_ENTRY_SIZE: usize = 24;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    BpWords = 1,
    NodeTypes = 2,
    NodeAttrs = 3,
    TokenRanges = 4,
    ParentMap = 5,
    NameTokens = 6,
}

impl SectionKind {
    const ALL: [SectionKind; 6] = [
        SectionKind::BpWords,
        SectionKind::NodeTypes,
        SectionKind::NodeAttrs,
        SectionKind::TokenRanges,
        SectionKind::ParentMap,
        SectionKind::NameTokens,
    ];

    fn elem_size(self) -> usize {
        match self {
            SectionKind::BpWords => 8,
            SectionKind::NodeTypes => 1,
            SectionKind::NodeAttrs | SectionKind::ParentMap | SectionKind::NameTokens => 4,
            SectionKind::TokenRanges => 8,
        }
    }
}

/// Summary of a validated `.bpa` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpaSummary {
    pub version: u32,
    pub node_count: u32,
    pub bit_count: u64,
    pub root_count: u32,
    pub tca_hash: u64,
}

pub struct BPASTSerializer;

impl BPASTSerializer {
    /// Encodes the artifact in the version-2 layout.
    pub fn to_bytes(artifact: &BPASTArtifact) -> Vec<u8> {
        let p = &artifact.preorder;
        let sections: [(SectionKind, Vec<u8>); 6] = [
            (
                SectionKind::BpWords,
                artifact
                    .bp_encoder
                    .words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect(),
            ),
            (SectionKind::NodeTypes, p.node_types.clone()),
            (
                SectionKind::NodeAttrs,
                p.node_attrs.iter().flat_map(|a| a.to_le_bytes()).collect(),
            ),
            (
                SectionKind::TokenRanges,
                p.token_ranges
                    .iter()
                    .flat_map(|&(a, b)| {
                        let mut pair = [0u8; 8];
                        pair[..4].copy_from_slice(&a.to_le_bytes());
                        pair[4..].copy_from_slice(&b.to_le_bytes());
                        pair
                    })
                    .collect(),
            ),
            (
                SectionKind::ParentMap,
                p.parent_map.iter().flat_map(|v| v.to_le_bytes()).collect(),
            ),
            (
                SectionKind::NameTokens,
                p.name_tokens.iter().flat_map(|v| v.to_le_bytes()).collect(),
            ),
        ];

        let payload: usize = sections.iter().map(|(_, b)| b.len() + SECTION_ALIGN).sum();
        let mut buf =
            Vec::with_capacity(BPA_HEADER_SIZE + sections.len() * DIRECTORY_ENTRY_SIZE + payload + 8);

        // Header (64 bytes)
        buf.extend_from_slice(BPA_MAGIC);
        buf.extend_from_slice(&BPA_VERSION.to_le_bytes());
        buf.extend_from_slice(&artifact.node_count.to_le_bytes());
        buf.extend_from_slice(&(artifact.bp_encoder.bit_count as u64).to_le_bytes());
        buf.extend_from_slice(&artifact.root_count().to_le_bytes());
        buf.extend_from_slice(&(sections.len() as u32).to_le_bytes());
        buf.extend_from_slice(&[0u8; 24]);
        buf.extend_from_slice(&artifact.tca_hash.to_le_bytes());
        debug_assert_eq!(buf.len(), BPA_HEADER_SIZE);

        // Directory, then aligned sections.
        let dir_start = buf.len();
        buf.resize(dir_start + sections.len() * DIRECTORY_ENTRY_SIZE, 0);
        for (i, (kind, bytes)) in sections.iter().enumerate() {
            pad_to_alignment(&mut buf, SECTION_ALIGN);
            let offset = buf.len() as u64;
            buf.extend_from_slice(bytes);
            let entry = dir_start + i * DIRECTORY_ENTRY_SIZE;
            buf[entry..entry + 4].copy_from_slice(&(*kind as u32).to_le_bytes());
            buf[entry + 4..entry + 8].copy_from_slice(&(kind.elem_size() as u32).to_le_bytes());
            buf[entry + 8..entry + 16].copy_from_slice(&offset.to_le_bytes());
            buf[entry + 16..entry + 24].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        }
        pad_to_alignment(&mut buf, SECTION_ALIGN);

        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        buf
    }

    /// Writes the artifact atomically (temporary file + rename).
    pub fn write(artifact: &BPASTArtifact, out_path: &Path) -> io::Result<()> {
        write_atomic(out_path, &Self::to_bytes(artifact))
    }

    pub fn read(in_path: &Path) -> io::Result<BPASTArtifact> {
        let buf = fs::read(in_path)?;
        Self::from_bytes(&buf)
    }

    /// Validates checksum, header and structure without keeping the decoded artifact.
    pub fn verify_bytes(buf: &[u8]) -> io::Result<BpaSummary> {
        let artifact = Self::from_bytes(buf)?;
        let version = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        Ok(BpaSummary {
            version,
            node_count: artifact.node_count,
            bit_count: artifact.bp_encoder.bit_count as u64,
            root_count: artifact.root_count(),
            tca_hash: artifact.tca_hash,
        })
    }

    pub fn from_bytes(buf: &[u8]) -> io::Result<BPASTArtifact> {
        if buf.len() < BPA_HEADER_SIZE + 8 {
            return Err(invalid_data("BPA file too small"));
        }
        let checksum_pos = buf.len() - 8;
        let mut trailer = [0u8; 8];
        trailer.copy_from_slice(&buf[checksum_pos..]);
        let stored_checksum = u64::from_le_bytes(trailer);
        let computed_checksum = crc64_ecma(&buf[..checksum_pos]);
        if stored_checksum != computed_checksum {
            return Err(invalid_data("BPA CRC-64 Checksum mismatch"));
        }
        let payload = &buf[..checksum_pos];

        let mut cur = ByteCursor::new(payload);
        let magic: [u8; 8] = cur.read_array()?;
        if &magic != BPA_MAGIC {
            return Err(invalid_data("Invalid BPA Magic Header"));
        }
        let version = cur.read_u32()?;
        let (preorder_and_bp, node_count, tca_hash) = match version {
            1 => Self::decode_v1(&mut cur)?,
            2 => Self::decode_v2(payload, &mut cur)?,
            other => {
                return Err(invalid_data(format!(
                    "Unsupported BPA format version {} (this build reads 1..={})",
                    other, BPA_VERSION
                )))
            }
        };
        let (bp_encoder, preorder) = preorder_and_bp;

        let rank_select = RankSelectIndex::build(&bp_encoder);
        let rmq = RangeMinMaxTree::build(&bp_encoder, &rank_select);
        let artifact = BPASTArtifact {
            node_count,
            bp_encoder,
            rank_select,
            rmq,
            preorder,
            tca_hash,
        };
        artifact
            .validate()
            .map_err(|e| invalid_data(format!("Malformed BPA tree: {}", e)))?;
        Ok(artifact)
    }

    fn decode_v2(
        payload: &[u8],
        cur: &mut ByteCursor,
    ) -> io::Result<((BPEncoder, PreorderArrays), u32, u64)> {
        let node_count = cur.read_u32()?;
        let bit_count = cur.read_u64()?;
        let _root_count = cur.read_u32()?;
        let section_count = cur.read_u32()?;
        cur.skip(24)?;
        let tca_hash = cur.read_u64()?;

        let n = node_count as usize;
        let expected_bits = 2u64 * node_count as u64;
        if bit_count != expected_bits {
            return Err(invalid_data(format!(
                "bit count {} != 2 x node count {}",
                bit_count, node_count
            )));
        }
        let bit_count = usize::try_from(bit_count)
            .map_err(|_| invalid_data("bit count overflows usize"))?;

        let n_sections = cur.checked_count(section_count as u64, DIRECTORY_ENTRY_SIZE)?;
        let mut found: [Option<&[u8]>; 6] = [None; 6];
        for _ in 0..n_sections {
            let kind = cur.read_u32()?;
            let elem_size = cur.read_u32()? as usize;
            let offset = cur.read_u64()?;
            let byte_len = cur.read_u64()?;
            let Some(slot) = SectionKind::ALL.iter().position(|k| *k as u32 == kind) else {
                // Unknown sections from a newer minor revision are skipped.
                continue;
            };
            let expected_kind = SectionKind::ALL[slot];
            if elem_size != expected_kind.elem_size() {
                return Err(invalid_data(format!(
                    "section {:?} has element size {} (expected {})",
                    expected_kind,
                    elem_size,
                    expected_kind.elem_size()
                )));
            }
            let start = usize::try_from(offset)
                .map_err(|_| invalid_data("section offset overflows usize"))?;
            let len = usize::try_from(byte_len)
                .map_err(|_| invalid_data("section length overflows usize"))?;
            if start % SECTION_ALIGN != 0 {
                return Err(invalid_data(format!(
                    "section {:?} at offset {} is not {}-byte aligned",
                    expected_kind, start, SECTION_ALIGN
                )));
            }
            let end = start
                .checked_add(len)
                .filter(|&e| e <= payload.len())
                .ok_or_else(|| {
                    invalid_data(format!(
                        "section {:?} ({} bytes at {}) runs past the end of the file",
                        expected_kind, len, start
                    ))
                })?;
            if len % elem_size != 0 {
                return Err(invalid_data(format!(
                    "section {:?} length {} is not a multiple of {}",
                    expected_kind, len, elem_size
                )));
            }
            if found[slot].is_some() {
                return Err(invalid_data(format!(
                    "duplicate section {:?}",
                    expected_kind
                )));
            }
            found[slot] = Some(&payload[start..end]);
        }

        let words_bytes = take_section(&found, SectionKind::BpWords, bit_count.div_ceil(64))?;
        let words: Vec<u64> = words_bytes
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect();

        let types_bytes = take_section(&found, SectionKind::NodeTypes, n)?;
        let node_attrs = u32_section(&found, SectionKind::NodeAttrs, n)?;
        let parent_map = u32_section(&found, SectionKind::ParentMap, n)?;
        let name_tokens = u32_section(&found, SectionKind::NameTokens, n)?;

        let ranges_bytes = take_section(&found, SectionKind::TokenRanges, n)?;
        let token_ranges: Vec<(u32, u32)> = ranges_bytes
            .chunks_exact(8)
            .map(|c| {
                (
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                )
            })
            .collect();

        Ok((
            (
                BPEncoder { words, bit_count },
                PreorderArrays {
                    node_types: types_bytes.to_vec(),
                    node_attrs,
                    token_ranges,
                    parent_map,
                    name_tokens,
                },
            ),
            node_count,
            tca_hash,
        ))
    }

    /// Version 1: `node_count u32 | bit_count u32 | file_count u16 | reserved [34] |
    /// tca_hash u64`, then length-prefixed BP words, jump table (skipped), node types,
    /// attrs, token ranges and parent map.
    fn decode_v1(cur: &mut ByteCursor) -> io::Result<((BPEncoder, PreorderArrays), u32, u64)> {
        let node_count = cur.read_u32()?;
        let bit_count = cur.read_u32()? as usize;
        let _file_count = cur.read_u16()?;
        cur.skip(34)?;
        let tca_hash = cur.read_u64()?;
        let n = node_count as usize;
        if bit_count as u64 != 2 * node_count as u64 {
            return Err(invalid_data(format!(
                "bit count {} != 2 x node count {}",
                bit_count, node_count
            )));
        }

        let words_len = cur.read_u32()? as u64;
        if words_len != bit_count.div_ceil(64) as u64 {
            return Err(invalid_data(format!(
                "{} BP words for {} bits",
                words_len, bit_count
            )));
        }
        let words = cur.read_u64_vec(words_len)?;

        let match_len = cur.read_u32()? as u64;
        let match_len = cur.checked_count(match_len, 4)?;
        cur.skip(match_len * 4)?;

        let node_types = cur.read_bytes(n)?.to_vec();

        let attrs_len = cur.read_u32()? as u64;
        if attrs_len != n as u64 {
            return Err(invalid_data(format!("{} attrs for {} nodes", attrs_len, n)));
        }
        let node_attrs = cur.read_u32_vec(attrs_len)?;

        let ranges_len = cur.read_u32()? as u64;
        if ranges_len != n as u64 {
            return Err(invalid_data(format!(
                "{} token ranges for {} nodes",
                ranges_len, n
            )));
        }
        let flat = cur.read_u32_vec(ranges_len * 2)?;
        let token_ranges = flat.chunks_exact(2).map(|c| (c[0], c[1])).collect();

        let parents_len = cur.read_u32()? as u64;
        if parents_len != n as u64 {
            return Err(invalid_data(format!(
                "{} parents for {} nodes",
                parents_len, n
            )));
        }
        let parent_map = cur.read_u32_vec(parents_len)?;

        if !cur.is_at_end() {
            return Err(invalid_data(format!(
                "{} unexpected trailing bytes",
                cur.remaining()
            )));
        }

        Ok((
            (
                BPEncoder { words, bit_count },
                PreorderArrays {
                    node_types,
                    node_attrs,
                    token_ranges,
                    parent_map,
                    name_tokens: vec![u32::MAX; n],
                },
            ),
            node_count,
            tca_hash,
        ))
    }
}

/// The bytes of a required section, checked to hold exactly `count` elements.
fn take_section<'a>(
    found: &[Option<&'a [u8]>; 6],
    kind: SectionKind,
    count: usize,
) -> io::Result<&'a [u8]> {
    let bytes = found[kind as usize - 1]
        .ok_or_else(|| invalid_data(format!("missing section {:?}", kind)))?;
    let expected = count
        .checked_mul(kind.elem_size())
        .ok_or_else(|| invalid_data(format!("section {:?} size overflows", kind)))?;
    if bytes.len() != expected {
        return Err(invalid_data(format!(
            "section {:?} holds {} elements, expected {}",
            kind,
            bytes.len() / kind.elem_size(),
            count
        )));
    }
    Ok(bytes)
}

fn u32_section(found: &[Option<&[u8]>; 6], kind: SectionKind, count: usize) -> io::Result<Vec<u32>> {
    Ok(take_section(found, kind, count)?
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}
