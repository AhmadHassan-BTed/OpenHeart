//! IntegrityVerifier — CRC-64/NVME integrity checksums and SHA-256 content digests (§10.2).
//!
//! Two distinct guarantees are provided:
//! - **Integrity** (accidental corruption): CRC-64/NVME over the header, the directory and
//!   each stored section. Checked structurally at open time and lazily per section.
//! - **Identity** (cache keys, link stamps, snapshot versioning): a SHA-256 digest over the
//!   canonical section payloads. It excludes the creation timestamp, so two builds of the same
//!   sources produce the same digest.

use sha2::{Digest, Sha256};

use crate::core::types::source::SourceFileRecord;
use crate::scpg::types::{SCPGSectionType, ScpgDigest, SCPG_FORMAT_VERSION};

/// CRC-64/NVME: poly 0xAD93D23594C93659, reflected, init and xorout all ones.
/// Check value for b"123456789" is 0xAE8B14860A799888.
const CRC64_NVME_POLY_REFLECTED: u64 = 0xAD93_D235_94C9_3659u64.reverse_bits();

static CRC64_NVME_TABLE: [u64; 256] = build_crc64_table(CRC64_NVME_POLY_REFLECTED);

const fn build_crc64_table(poly: u64) -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u64;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ poly } else { crc >> 1 };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Streaming CRC-64/NVME state.
#[derive(Debug, Clone, Copy)]
pub struct Crc64Nvme {
    state: u64,
}

impl Default for Crc64Nvme {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc64Nvme {
    pub fn new() -> Self {
        Self { state: u64::MAX }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.state;
        for &b in bytes {
            crc = CRC64_NVME_TABLE[((crc ^ b as u64) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.state = crc;
    }

    pub fn finish(&self) -> u64 {
        !self.state
    }
}

/// One-shot CRC-64/NVME.
pub fn crc64_nvme(bytes: &[u8]) -> u64 {
    let mut crc = Crc64Nvme::new();
    crc.update(bytes);
    crc.finish()
}

/// Incremental builder of the canonical SHA-256 content digest.
///
/// Definition: `SHA-256("OPENHEART-SCPG-CONTENT\0" ‖ format_version ‖ Σ(type ‖ len ‖ payload))`
/// over the sections in on-disk order. The digest therefore changes whenever any analysis
/// result changes and never depends on wall-clock time.
pub struct ContentDigestBuilder {
    hasher: Sha256,
}

impl Default for ContentDigestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentDigestBuilder {
    pub fn new() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"OPENHEART-SCPG-CONTENT\0");
        hasher.update(SCPG_FORMAT_VERSION.to_le_bytes());
        Self { hasher }
    }

    pub fn section(&mut self, section: SCPGSectionType, payload: &[u8]) {
        self.hasher.update((section as u32).to_le_bytes());
        self.hasher.update((payload.len() as u64).to_le_bytes());
        self.hasher.update(payload);
    }

    pub fn finish(self) -> ScpgDigest {
        self.hasher.finalize().into()
    }
}

/// The single short-hash definition shared by the header, the engine and every UMLLink the
/// engine hands out: the first four digest bytes, little-endian.
pub fn short_hash(digest: &ScpgDigest) -> u32 {
    u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
}

pub fn digest_hex(digest: &ScpgDigest) -> String {
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Source tree hash with the same definition Phase 1 uses: SHA-256 over the per-file content
/// SHA-256 values in file-id order.
pub fn source_tree_hash(files: &[SourceFileRecord]) -> [u8; 32] {
    let mut ordered: Vec<&SourceFileRecord> = files.iter().collect();
    ordered.sort_by_key(|f| f.file_id);
    let mut hasher = Sha256::new();
    for f in ordered {
        hasher.update(f.content_sha256);
    }
    hasher.finalize().into()
}

/// 64-bit entity fingerprint (truncated SHA-256) used for incremental change detection.
pub struct Fingerprinter {
    hasher: Sha256,
}

impl Fingerprinter {
    pub fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((domain.len() as u32).to_le_bytes());
        hasher.update(domain);
        Self { hasher }
    }

    pub fn u8(&mut self, v: u8) {
        self.hasher.update([v]);
    }

    pub fn u16(&mut self, v: u16) {
        self.hasher.update(v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.hasher.update(v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.hasher.update(v.to_le_bytes());
    }

    /// Length-prefixed bytes, so concatenations of different splits never collide.
    pub fn bytes(&mut self, v: &[u8]) {
        self.hasher.update((v.len() as u64).to_le_bytes());
        self.hasher.update(v);
    }

    pub fn finish(self) -> u64 {
        let out: [u8; 32] = self.hasher.finalize().into();
        u64::from_le_bytes([out[0], out[1], out[2], out[3], out[4], out[5], out[6], out[7]])
    }
}
