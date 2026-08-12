//! IntegrityVerifier — CRC-32 per-section integrity and composite scpg_hash calculation (§10.2).

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

pub fn compose_scpg_hash(
    tca_hash: u64,
    bpa_hash: u64,
    sta_hash: u64,
    cfa_hash: u64,
    ssa_hash: u64,
    cga_hash: u64,
    tra_hash: u64,
    uma_hash: u64,
    psa_hash: u64,
) -> u32 {
    let combined = tca_hash
        ^ bpa_hash
        ^ sta_hash
        ^ cfa_hash
        ^ ssa_hash
        ^ cga_hash
        ^ tra_hash
        ^ uma_hash
        ^ psa_hash;
    (combined & 0xFFFF_FFFF) as u32 ^ ((combined >> 32) as u32)
}
