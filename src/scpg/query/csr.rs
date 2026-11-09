//! Compressed-sparse-row helpers shared by every SCPG graph index (§10.5).

/// Builds a deduplicated CSR from `(row, target)` pairs. Rows `>= rows` are dropped.
pub fn csr_from_pairs(rows: usize, mut pairs: Vec<(u32, u32)>) -> (Vec<u32>, Vec<u32>) {
    pairs.retain(|&(r, _)| (r as usize) < rows);
    pairs.sort_unstable();
    pairs.dedup();
    let mut offsets = vec![0u32; rows + 1];
    for &(r, _) in &pairs {
        offsets[r as usize + 1] += 1;
    }
    for i in 0..rows {
        offsets[i + 1] += offsets[i];
    }
    let adj = pairs.into_iter().map(|(_, t)| t).collect();
    (offsets, adj)
}

/// Builds a CSR with a parallel kind column from `(row, target, kind)` triples.
///
/// With `one_kind_per_target`, duplicate `(row, target)` pairs collapse to the smallest kind;
/// otherwise each distinct `(row, target, kind)` triple is kept.
pub fn csr_from_triples(
    rows: usize,
    mut triples: Vec<(u32, u32, u8)>,
    one_kind_per_target: bool,
) -> (Vec<u32>, Vec<u32>, Vec<u8>) {
    triples.retain(|&(r, _, _)| (r as usize) < rows);
    triples.sort_unstable();
    if one_kind_per_target {
        triples.dedup_by_key(|t| (t.0, t.1));
    } else {
        triples.dedup();
    }
    let mut offsets = vec![0u32; rows + 1];
    for &(r, _, _) in &triples {
        offsets[r as usize + 1] += 1;
    }
    for i in 0..rows {
        offsets[i + 1] += offsets[i];
    }
    let mut adj = Vec::with_capacity(triples.len());
    let mut kinds = Vec::with_capacity(triples.len());
    for (_, t, k) in triples {
        adj.push(t);
        kinds.push(k);
    }
    (offsets, adj, kinds)
}

/// Returns row `r` of a CSR, or an empty slice for an unknown or malformed row.
pub fn row<'a, T>(offsets: &[u32], adj: &'a [T], r: u32) -> &'a [T] {
    let i = r as usize;
    if i + 1 >= offsets.len() {
        return &[];
    }
    let (start, end) = (offsets[i] as usize, offsets[i + 1] as usize);
    adj.get(start..end).unwrap_or(&[])
}
