//! Section column codec — little-endian, length-prefixed, 8-byte aligned columns (§10.2).
//!
//! Every section payload starts with `(section_type: u32, schema_version: u32)` followed by a
//! sequence of scalars and columns. A column is `pad-to-8 ‖ count: u64 ‖ elements ‖ pad-to-8`,
//! so every column body starts 8-byte aligned relative to the (64-byte aligned) section start.
//! The decoder validates every count against the bytes that remain before allocating, which
//! makes hostile length fields fail with `ScpgFormatError::Malformed` instead of aborting.

use crate::scpg::types::{SCPGSectionType, ScpgFormatError};

pub const SECTION_SCHEMA_VERSION: u32 = 1;
const COLUMN_ALIGN: usize = 8;

pub struct SectionEncoder {
    buf: Vec<u8>,
}

impl SectionEncoder {
    pub fn new(section: SCPGSectionType) -> Self {
        let mut enc = Self { buf: Vec::new() };
        enc.put_u32(section as u32);
        enc.put_u32(SECTION_SCHEMA_VERSION);
        enc
    }

    fn pad(&mut self) {
        while !self.buf.len().is_multiple_of(COLUMN_ALIGN) {
            self.buf.push(0);
        }
    }

    pub fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn put_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn put_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn put_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn column_header(&mut self, count: usize) {
        self.pad();
        self.put_u64(count as u64);
    }

    pub fn put_u8s(&mut self, v: &[u8]) {
        self.column_header(v.len());
        self.buf.extend_from_slice(v);
        self.pad();
    }

    pub fn put_u16s(&mut self, v: &[u16]) {
        self.column_header(v.len());
        self.buf.reserve(v.len() * 2);
        for &x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
        self.pad();
    }

    pub fn put_u32s(&mut self, v: &[u32]) {
        self.column_header(v.len());
        self.buf.reserve(v.len() * 4);
        for &x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
        self.pad();
    }

    pub fn put_u64s(&mut self, v: &[u64]) {
        self.column_header(v.len());
        self.buf.reserve(v.len() * 8);
        for &x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
        self.pad();
    }

    pub fn put_str(&mut self, s: &str) {
        self.put_u8s(s.as_bytes());
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.pad();
        self.buf
    }
}

pub struct SectionDecoder<'a> {
    section: SCPGSectionType,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> SectionDecoder<'a> {
    pub fn new(section: SCPGSectionType, bytes: &'a [u8]) -> Result<Self, ScpgFormatError> {
        let mut dec = Self {
            section,
            bytes,
            pos: 0,
        };
        let stored_type = dec.u32()?;
        if stored_type != section as u32 {
            return Err(dec.malformed(format!(
                "payload is tagged as section 0x{:X}",
                stored_type
            )));
        }
        let schema = dec.u32()?;
        if schema != SECTION_SCHEMA_VERSION {
            return Err(dec.malformed(format!(
                "unsupported section schema version {} (expected {})",
                schema, SECTION_SCHEMA_VERSION
            )));
        }
        Ok(dec)
    }

    pub fn section(&self) -> SCPGSectionType {
        self.section
    }

    pub fn malformed(&self, reason: impl Into<String>) -> ScpgFormatError {
        ScpgFormatError::Malformed {
            section: self.section,
            reason: reason.into(),
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ScpgFormatError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or_else(|| self.malformed(format!("unexpected end of payload at byte {}", self.pos)))?;
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn align(&mut self) -> Result<(), ScpgFormatError> {
        let rem = self.pos % COLUMN_ALIGN;
        if rem != 0 {
            self.take(COLUMN_ALIGN - rem)?;
        }
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8, ScpgFormatError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, ScpgFormatError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, ScpgFormatError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, ScpgFormatError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// Reads a column header and validates `count × elem_size` against the remaining bytes.
    fn column(&mut self, elem_size: usize) -> Result<&'a [u8], ScpgFormatError> {
        self.align()?;
        let count = self.u64()?;
        let count = usize::try_from(count)
            .map_err(|_| self.malformed(format!("column count {} overflows usize", count)))?;
        let byte_len = count
            .checked_mul(elem_size)
            .filter(|&len| len <= self.remaining())
            .ok_or_else(|| {
                self.malformed(format!(
                    "column of {} elements × {} bytes exceeds the {} remaining bytes",
                    count,
                    elem_size,
                    self.remaining()
                ))
            })?;
        let body = self.take(byte_len)?;
        self.align()?;
        Ok(body)
    }

    pub fn u8s(&mut self) -> Result<&'a [u8], ScpgFormatError> {
        self.column(1)
    }

    pub fn u16s(&mut self) -> Result<Vec<u16>, ScpgFormatError> {
        let body = self.column(2)?;
        Ok(body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect())
    }

    pub fn u32s(&mut self) -> Result<Vec<u32>, ScpgFormatError> {
        let body = self.column(4)?;
        Ok(body
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    pub fn u64s(&mut self) -> Result<Vec<u64>, ScpgFormatError> {
        let body = self.column(8)?;
        Ok(body
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    pub fn string(&mut self) -> Result<String, ScpgFormatError> {
        let body = self.u8s()?;
        std::str::from_utf8(body)
            .map(|s| s.to_string())
            .map_err(|e| self.malformed(format!("invalid UTF-8 string: {}", e)))
    }

    /// Fails unless every column has exactly `expected` elements.
    pub fn expect_len(&self, column: &str, actual: usize, expected: usize) -> Result<(), ScpgFormatError> {
        if actual != expected {
            return Err(self.malformed(format!(
                "column '{}' has {} elements, expected {}",
                column, actual, expected
            )));
        }
        Ok(())
    }

    /// Validates a CSR pair: `offsets.len() == rows + 1`, monotone, ending at `adj_len`, and
    /// (when `target_bound` is given) every adjacency target `< target_bound`.
    pub fn check_csr(
        &self,
        name: &str,
        offsets: &[u32],
        adj: &[u32],
        rows: usize,
        target_bound: Option<usize>,
    ) -> Result<(), ScpgFormatError> {
        if offsets.len() != rows + 1 {
            return Err(self.malformed(format!(
                "CSR '{}' has {} offsets for {} rows",
                name,
                offsets.len(),
                rows
            )));
        }
        if offsets[0] != 0 || offsets[rows] as usize != adj.len() {
            return Err(self.malformed(format!("CSR '{}' offsets do not span the adjacency", name)));
        }
        if offsets.windows(2).any(|w| w[0] > w[1]) {
            return Err(self.malformed(format!("CSR '{}' offsets are not monotone", name)));
        }
        if let Some(bound) = target_bound {
            if adj.iter().any(|&t| t as usize >= bound) {
                return Err(self.malformed(format!(
                    "CSR '{}' references a node outside [0, {})",
                    name, bound
                )));
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), ScpgFormatError> {
        // The encoder pads the payload to 8 bytes after trailing scalars.
        let rem = self.pos % COLUMN_ALIGN;
        if rem != 0 {
            self.pos += (COLUMN_ALIGN - rem).min(self.remaining());
        }
        if self.pos != self.bytes.len() {
            return Err(self.malformed(format!(
                "{} trailing bytes after the last column",
                self.bytes.len() - self.pos
            )));
        }
        Ok(())
    }
}

/// Compact UTF-8 string column: one blob plus `len + 1` byte offsets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringColumn {
    offsets: Vec<u64>,
    blob: String,
}

impl StringColumn {
    pub fn new() -> Self {
        Self {
            offsets: vec![0],
            blob: String::new(),
        }
    }

    pub fn push(&mut self, s: &str) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.blob.push_str(s);
        self.offsets.push(self.blob.len() as u64);
    }

    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        let start = *self.offsets.get(index)? as usize;
        let end = *self.offsets.get(index + 1)? as usize;
        self.blob.get(start..end)
    }

    /// Returns the string at `index`, or `""` when out of range.
    pub fn get_or_empty(&self, index: usize) -> &str {
        self.get(index).unwrap_or("")
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        if self.offsets.is_empty() {
            enc.put_u64s(&[0]);
        } else {
            enc.put_u64s(&self.offsets);
        }
        enc.put_u8s(self.blob.as_bytes());
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let offsets = dec.u64s()?;
        let blob_bytes = dec.u8s()?;
        let blob = std::str::from_utf8(blob_bytes)
            .map_err(|e| dec.malformed(format!("string column is not UTF-8: {}", e)))?
            .to_string();
        if offsets.first() != Some(&0) || *offsets.last().unwrap_or(&1) as usize != blob.len() {
            return Err(dec.malformed("string column offsets do not span the blob"));
        }
        for w in offsets.windows(2) {
            if w[0] > w[1] || !blob.is_char_boundary(w[1] as usize) {
                return Err(dec.malformed("string column offsets are not monotone char boundaries"));
            }
        }
        Ok(Self { offsets, blob })
    }
}
