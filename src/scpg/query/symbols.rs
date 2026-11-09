//! SymbolIndex & TypeHierarchyIndex — columnar symbol layer with stable symbol ids (§10.5).
//!
//! Positional `sym_id`s change whenever a declaration is inserted earlier in the corpus, so
//! every symbol additionally carries a **stable id** in SCIP symbol syntax
//! (`openheart . . . com/acme/Foo#bar(1).`). Stable ids are derived only from names, kinds,
//! nesting and arity, which keeps them identical across builds of unchanged declarations and
//! makes them usable for cross-build diffs and external links.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

use crate::core::types::symbol::{SymbolKind, SymbolModifiers};
use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::query::csr::{csr_from_pairs, row};
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder, StringColumn};
use crate::scpg::types::ScpgFormatError;
use crate::symbol::qual_name_table::QualifiedNameTable;
use crate::symbol::SymbolTableArtifact;

pub const NO_SYMBOL: u32 = u32::MAX;

/// Scheme and (empty) package fields of every global stable id.
pub const STABLE_ID_PREFIX: &str = "openheart . . . ";
/// Prefix of document-local stable ids (locals are not stable across builds by design).
pub const LOCAL_ID_PREFIX: &str = "local ";

const KIND_NAMES: [(u8, &str); 19] = [
    (SymbolKind::SK_PACKAGE as u8, "package"),
    (SymbolKind::SK_CLASS as u8, "class"),
    (SymbolKind::SK_INTERFACE as u8, "interface"),
    (SymbolKind::SK_ENUM as u8, "enum"),
    (SymbolKind::SK_RECORD as u8, "record"),
    (SymbolKind::SK_ANNOTATION_TYPE as u8, "annotation"),
    (SymbolKind::SK_METHOD as u8, "method"),
    (SymbolKind::SK_CONSTRUCTOR as u8, "constructor"),
    (SymbolKind::SK_FIELD as u8, "field"),
    (SymbolKind::SK_ENUM_CONSTANT as u8, "enum_constant"),
    (SymbolKind::SK_PARAM as u8, "param"),
    (SymbolKind::SK_LOCAL_VAR as u8, "local"),
    (SymbolKind::SK_TYPE_PARAM as u8, "type_param"),
    (SymbolKind::SK_LAMBDA as u8, "lambda"),
    (SymbolKind::SK_ANON_CLASS as u8, "anon_class"),
    (SymbolKind::SK_STATIC_INIT as u8, "static_init"),
    (SymbolKind::SK_INSTANCE_INIT as u8, "instance_init"),
    (SymbolKind::SK_MODULE as u8, "module"),
    (SymbolKind::SK_EXTERNAL as u8, "external"),
];

pub const MODIFIER_NAMES: [(u16, &str); 16] = [
    (SymbolModifiers::STATIC, "static"),
    (SymbolModifiers::FINAL, "final"),
    (SymbolModifiers::ABSTRACT, "abstract"),
    (SymbolModifiers::SYNCHRONIZED, "synchronized"),
    (SymbolModifiers::NATIVE, "native"),
    (SymbolModifiers::VOLATILE, "volatile"),
    (SymbolModifiers::TRANSIENT, "transient"),
    (SymbolModifiers::STRICTFP, "strictfp"),
    (SymbolModifiers::DEFAULT, "default"),
    (SymbolModifiers::SEALED, "sealed"),
    (SymbolModifiers::NON_SEALED, "non_sealed"),
    (SymbolModifiers::VARARGS, "varargs"),
    (SymbolModifiers::BRIDGE, "bridge"),
    (SymbolModifiers::SYNTHETIC, "synthetic"),
    (SymbolModifiers::DEPRECATED, "deprecated"),
    (SymbolModifiers::RECORD_COMPONENT, "record_component"),
];

pub fn kind_name(kind: u8) -> &'static str {
    KIND_NAMES
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, n)| *n)
        .unwrap_or("external")
}

pub fn kind_from_name(name: &str) -> Option<u8> {
    KIND_NAMES.iter().find(|(_, n)| *n == name).map(|(k, _)| *k)
}

pub fn visibility_name(visibility: u8) -> &'static str {
    match visibility {
        1 => "public",
        2 => "private",
        3 => "protected",
        _ => "package",
    }
}

pub fn is_type_kind(kind: u8) -> bool {
    kind == SymbolKind::SK_CLASS as u8
        || kind == SymbolKind::SK_INTERFACE as u8
        || kind == SymbolKind::SK_ENUM as u8
        || kind == SymbolKind::SK_RECORD as u8
        || kind == SymbolKind::SK_ANNOTATION_TYPE as u8
        || kind == SymbolKind::SK_ANON_CLASS as u8
}

pub fn is_callable_kind(kind: u8) -> bool {
    kind == SymbolKind::SK_METHOD as u8
        || kind == SymbolKind::SK_CONSTRUCTOR as u8
        || kind == SymbolKind::SK_LAMBDA as u8
        || kind == SymbolKind::SK_STATIC_INIT as u8
        || kind == SymbolKind::SK_INSTANCE_INIT as u8
}

fn is_member_variable_kind(kind: u8) -> bool {
    kind == SymbolKind::SK_LOCAL_VAR as u8 || kind == SymbolKind::SK_PARAM as u8
}

/// SCIP simple-identifier escaping: identifiers outside `[A-Za-z0-9_+$-]+` are backticked.
fn escape_identifier(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '$'))
    {
        name.to_string()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}

/// Returns the sub-slice of `order` whose elements compare `Equal`; `cmp(i)` compares the
/// element at index `i` against the probe. `order` must be sorted consistently with `cmp`.
fn equal_range<F: Fn(u32) -> Ordering>(order: &[u32], cmp: F) -> &[u32] {
    let lo = order.partition_point(|&i| cmp(i) == Ordering::Less);
    let hi = order.partition_point(|&i| cmp(i) != Ordering::Greater);
    &order[lo..hi.max(lo)]
}

#[derive(Debug, Clone, Default)]
pub struct SymbolIndex {
    kinds: Vec<u8>,
    parents: Vec<u32>,
    modifiers: Vec<u16>,
    visibilities: Vec<u8>,
    param_counts: Vec<u16>,
    decl_nodes: Vec<u32>,
    def_nodes: Vec<u32>,
    first_tokens: Vec<u32>,
    last_tokens: Vec<u32>,
    names: StringColumn,
    qualified_names: StringColumn,
    stable_ids: StringColumn,
    signature_fps: Vec<u64>,
    body_fps: Vec<u64>,

    // Derived lookup structures (rebuilt on decode, never serialized).
    stable_order: Vec<u32>,
    qualified_order: Vec<u32>,
    name_order: Vec<u32>,
    parent_name_order: Vec<u32>,
    child_offsets: Vec<u32>,
    child_adj: Vec<u32>,
}

impl SymbolIndex {
    pub fn from_artifacts(sta: &SymbolTableArtifact, tca: &TokenCorpusArtifact) -> Self {
        let n = sta.symbol_records.len();
        let mut kinds = Vec::with_capacity(n);
        let mut parents = Vec::with_capacity(n);
        let mut modifiers = Vec::with_capacity(n);
        let mut visibilities = Vec::with_capacity(n);
        let mut param_counts = Vec::with_capacity(n);
        let mut decl_nodes = Vec::with_capacity(n);
        let mut def_nodes = Vec::with_capacity(n);
        let mut first_tokens = Vec::with_capacity(n);
        let mut last_tokens = Vec::with_capacity(n);
        let mut raw_names = Vec::with_capacity(n);

        for (i, rec) in sta.symbol_records.iter().enumerate() {
            kinds.push(rec.kind);
            let parent = rec.parent_sym;
            parents.push(if (parent as usize) < n && parent as usize != i {
                parent
            } else {
                NO_SYMBOL
            });
            modifiers.push(rec.modifiers);
            visibilities.push(rec.visibility);
            param_counts.push(rec.param_count);
            decl_nodes.push(rec.decl_node);
            def_nodes.push(rec.def_node);
            first_tokens.push(rec.first_token_id);
            last_tokens.push(rec.last_token_id);
            raw_names.push(String::from_utf8_lossy(tca.interner.lookup_text(rec.name_id)).into_owned());
        }

        let qualified = compute_qualified_names(sta, &raw_names, &parents);
        let stable = compute_stable_ids(&kinds, &parents, &raw_names, &param_counts, &qualified);

        let mut names = StringColumn::new();
        let mut qualified_names = StringColumn::new();
        let mut stable_ids = StringColumn::new();
        for ((name, qual), stable_id) in raw_names.iter().zip(&qualified).zip(&stable) {
            names.push(name);
            qualified_names.push(qual);
            stable_ids.push(stable_id);
        }

        let mut index = Self {
            kinds,
            parents,
            modifiers,
            visibilities,
            param_counts,
            decl_nodes,
            def_nodes,
            first_tokens,
            last_tokens,
            names,
            qualified_names,
            stable_ids,
            signature_fps: vec![0; n],
            body_fps: vec![0; n],
            ..Default::default()
        };
        index.rebuild_derived();
        index
    }

    /// Installs per-symbol signature/body fingerprints (computed by the snapshot builder).
    pub fn set_fingerprints(&mut self, signature_fps: Vec<u64>, body_fps: Vec<u64>) {
        assert_eq!(signature_fps.len(), self.len(), "signature fingerprint count mismatch");
        assert_eq!(body_fps.len(), self.len(), "body fingerprint count mismatch");
        self.signature_fps = signature_fps;
        self.body_fps = body_fps;
    }

    fn rebuild_derived(&mut self) {
        let n = self.len() as u32;

        let mut stable_order: Vec<u32> = (0..n).collect();
        stable_order.sort_by(|&a, &b| {
            self.stable_ids
                .get_or_empty(a as usize)
                .cmp(self.stable_ids.get_or_empty(b as usize))
                .then(a.cmp(&b))
        });
        let mut qualified_order: Vec<u32> = (0..n).collect();
        qualified_order.sort_by(|&a, &b| {
            self.qualified_names
                .get_or_empty(a as usize)
                .cmp(self.qualified_names.get_or_empty(b as usize))
                .then(a.cmp(&b))
        });
        let mut name_order: Vec<u32> = (0..n).collect();
        name_order.sort_by(|&a, &b| {
            self.names
                .get_or_empty(a as usize)
                .cmp(self.names.get_or_empty(b as usize))
                .then(a.cmp(&b))
        });
        let mut parent_name_order: Vec<u32> = (0..n).collect();
        parent_name_order.sort_by(|&a, &b| {
            self.parents[a as usize]
                .cmp(&self.parents[b as usize])
                .then_with(|| {
                    self.names
                        .get_or_empty(a as usize)
                        .cmp(self.names.get_or_empty(b as usize))
                })
                .then(a.cmp(&b))
        });

        let pairs: Vec<(u32, u32)> = (0..n)
            .filter(|&i| self.parents[i as usize] != NO_SYMBOL)
            .map(|i| (self.parents[i as usize], i))
            .collect();
        let (child_offsets, child_adj) = csr_from_pairs(n as usize, pairs);

        self.stable_order = stable_order;
        self.qualified_order = qualified_order;
        self.name_order = name_order;
        self.parent_name_order = parent_name_order;
        self.child_offsets = child_offsets;
        self.child_adj = child_adj;
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn contains(&self, sym_id: u32) -> bool {
        (sym_id as usize) < self.len()
    }

    pub fn kind(&self, sym_id: u32) -> Option<u8> {
        self.kinds.get(sym_id as usize).copied()
    }

    pub fn parent(&self, sym_id: u32) -> Option<u32> {
        self.parents
            .get(sym_id as usize)
            .copied()
            .filter(|&p| p != NO_SYMBOL)
    }

    pub fn modifiers(&self, sym_id: u32) -> Option<u16> {
        self.modifiers.get(sym_id as usize).copied()
    }

    pub fn visibility(&self, sym_id: u32) -> Option<u8> {
        self.visibilities.get(sym_id as usize).copied()
    }

    pub fn param_count(&self, sym_id: u32) -> Option<u16> {
        self.param_counts.get(sym_id as usize).copied()
    }

    pub fn decl_node(&self, sym_id: u32) -> Option<u32> {
        self.decl_nodes
            .get(sym_id as usize)
            .copied()
            .filter(|&n| n != u32::MAX)
    }

    /// BP-AST node of the definition body (method body, class body), when recorded.
    pub fn def_node(&self, sym_id: u32) -> Option<u32> {
        self.def_nodes
            .get(sym_id as usize)
            .copied()
            .filter(|&n| n != u32::MAX)
    }

    /// `(first_token_id, last_token_id)` recorded by Phase 3, when known.
    pub fn token_span(&self, sym_id: u32) -> Option<(u32, u32)> {
        let i = sym_id as usize;
        let first = *self.first_tokens.get(i)?;
        let last = *self.last_tokens.get(i)?;
        if first == u32::MAX || last == u32::MAX || last < first {
            None
        } else {
            Some((first, last))
        }
    }

    pub fn name(&self, sym_id: u32) -> Option<&str> {
        self.names.get(sym_id as usize)
    }

    pub fn qualified_name(&self, sym_id: u32) -> Option<&str> {
        self.qualified_names.get(sym_id as usize)
    }

    pub fn stable_id(&self, sym_id: u32) -> Option<&str> {
        self.stable_ids.get(sym_id as usize)
    }

    pub fn signature_fingerprint(&self, sym_id: u32) -> Option<u64> {
        self.signature_fps.get(sym_id as usize).copied()
    }

    pub fn body_fingerprint(&self, sym_id: u32) -> Option<u64> {
        self.body_fps.get(sym_id as usize).copied()
    }

    pub fn has_modifier(&self, sym_id: u32, flag: u16) -> bool {
        self.modifiers(sym_id).map(|m| m & flag != 0).unwrap_or(false)
    }

    pub fn is_private(&self, sym_id: u32) -> bool {
        self.visibility(sym_id) == Some(2)
    }

    pub fn children(&self, parent: u32) -> &[u32] {
        row(&self.child_offsets, &self.child_adj, parent)
    }

    /// Children of `parent` named `name` in O(log n).
    pub fn children_named(&self, parent: u32, name: &str) -> &[u32] {
        equal_range(&self.parent_name_order, |i| {
            self.parents[i as usize]
                .cmp(&parent)
                .then_with(|| self.names.get_or_empty(i as usize).cmp(name))
        })
    }

    pub fn by_stable_id(&self, stable_id: &str) -> &[u32] {
        equal_range(&self.stable_order, |i| {
            self.stable_ids.get_or_empty(i as usize).cmp(stable_id)
        })
    }

    pub fn by_qualified_name(&self, qualified: &str) -> &[u32] {
        equal_range(&self.qualified_order, |i| {
            self.qualified_names.get_or_empty(i as usize).cmp(qualified)
        })
    }

    pub fn by_name(&self, name: &str) -> &[u32] {
        equal_range(&self.name_order, |i| self.names.get_or_empty(i as usize).cmp(name))
    }

    /// Resolves a user-supplied symbol reference: a numeric sym_id, a stable id, a fully
    /// qualified name, or a simple name (in that order of precedence).
    pub fn resolve(&self, spec: &str) -> Vec<u32> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Vec::new();
        }
        if let Ok(id) = spec.parse::<u32>() {
            return if self.contains(id) { vec![id] } else { Vec::new() };
        }
        if spec.starts_with(STABLE_ID_PREFIX) || spec.starts_with(LOCAL_ID_PREFIX) {
            return self.by_stable_id(spec).to_vec();
        }
        let qualified = self.by_qualified_name(spec);
        if !qualified.is_empty() {
            return qualified.to_vec();
        }
        self.by_name(spec).to_vec()
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u8s(&self.kinds);
        enc.put_u32s(&self.parents);
        enc.put_u16s(&self.modifiers);
        enc.put_u8s(&self.visibilities);
        enc.put_u16s(&self.param_counts);
        enc.put_u32s(&self.decl_nodes);
        enc.put_u32s(&self.def_nodes);
        enc.put_u32s(&self.first_tokens);
        enc.put_u32s(&self.last_tokens);
        self.names.encode(enc);
        self.qualified_names.encode(enc);
        self.stable_ids.encode(enc);
        enc.put_u64s(&self.signature_fps);
        enc.put_u64s(&self.body_fps);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let kinds = dec.u8s()?.to_vec();
        let n = kinds.len();
        let parents = dec.u32s()?;
        let modifiers = dec.u16s()?;
        let visibilities = dec.u8s()?.to_vec();
        let param_counts = dec.u16s()?;
        let decl_nodes = dec.u32s()?;
        let def_nodes = dec.u32s()?;
        let first_tokens = dec.u32s()?;
        let last_tokens = dec.u32s()?;
        let names = StringColumn::decode(dec)?;
        let qualified_names = StringColumn::decode(dec)?;
        let stable_ids = StringColumn::decode(dec)?;
        let signature_fps = dec.u64s()?;
        let body_fps = dec.u64s()?;

        dec.expect_len("parents", parents.len(), n)?;
        dec.expect_len("modifiers", modifiers.len(), n)?;
        dec.expect_len("visibilities", visibilities.len(), n)?;
        dec.expect_len("param_counts", param_counts.len(), n)?;
        dec.expect_len("decl_nodes", decl_nodes.len(), n)?;
        dec.expect_len("def_nodes", def_nodes.len(), n)?;
        dec.expect_len("first_tokens", first_tokens.len(), n)?;
        dec.expect_len("last_tokens", last_tokens.len(), n)?;
        dec.expect_len("names", names.len(), n)?;
        dec.expect_len("qualified_names", qualified_names.len(), n)?;
        dec.expect_len("stable_ids", stable_ids.len(), n)?;
        dec.expect_len("signature_fps", signature_fps.len(), n)?;
        dec.expect_len("body_fps", body_fps.len(), n)?;
        if parents.iter().any(|&p| p != NO_SYMBOL && p as usize >= n) {
            return Err(dec.malformed("symbol parent outside the symbol table"));
        }

        let mut index = Self {
            kinds,
            parents,
            modifiers,
            visibilities,
            param_counts,
            decl_nodes,
            def_nodes,
            first_tokens,
            last_tokens,
            names,
            qualified_names,
            stable_ids,
            signature_fps,
            body_fps,
            ..Default::default()
        };
        index.rebuild_derived();
        Ok(index)
    }
}

/// Walks `parents` with memoization, guarding against cycles. `f(i, parent_value)` derives
/// the value of `i` from its parent's value (`None` for roots).
fn derive_down<F>(parents: &[u32], mut f: F) -> Vec<String>
where
    F: FnMut(usize, Option<&str>) -> String,
{
    let n = parents.len();
    let mut memo: Vec<Option<String>> = vec![None; n];
    let mut path = Vec::new();
    for start in 0..n {
        if memo[start].is_some() {
            continue;
        }
        path.clear();
        let mut cur = start;
        loop {
            path.push(cur);
            let p = parents[cur];
            if p == NO_SYMBOL || memo[p as usize].is_some() || path.len() > n {
                break;
            }
            if path.contains(&(p as usize)) {
                // Parent cycle: treat the current node as a root.
                break;
            }
            cur = p as usize;
        }
        while let Some(i) = path.pop() {
            let p = parents[i];
            let parent_value = if p != NO_SYMBOL && !path.contains(&(p as usize)) {
                memo[p as usize].as_deref()
            } else {
                None
            };
            let value = f(i, parent_value);
            memo[i] = Some(value);
        }
    }
    memo.into_iter().map(|v| v.unwrap_or_default()).collect()
}

fn compute_qualified_names(sta: &SymbolTableArtifact, names: &[String], parents: &[u32]) -> Vec<String> {
    let derived = derive_down(parents, |i, parent| {
        let name = names[i].as_str();
        match parent {
            Some(p) if !p.is_empty() && !name.is_empty() => format!("{}.{}", p, name),
            Some(p) if name.is_empty() => p.to_string(),
            _ => name.to_string(),
        }
    });
    sta.symbol_records
        .iter()
        .enumerate()
        .map(|(i, rec)| {
            let qid = rec.qual_name_id;
            let from_sta = if qid == u32::MAX {
                None
            } else {
                let idx = if qid >= QualifiedNameTable::BASE_QUAL_NAME_ID {
                    (qid - QualifiedNameTable::BASE_QUAL_NAME_ID) as usize
                } else {
                    qid as usize
                };
                sta.qual_names.get(idx).filter(|s| !s.is_empty())
            };
            from_sta.cloned().unwrap_or_else(|| derived[i].clone())
        })
        .collect()
}

fn compute_stable_ids(
    kinds: &[u8],
    parents: &[u32],
    names: &[String],
    param_counts: &[u16],
    qualified: &[String],
) -> Vec<String> {
    let n = kinds.len();

    // Locals and parameters are transparent for nesting: their children hang off the nearest
    // non-variable ancestor so ordinals stay meaningful.
    let effective_parent: Vec<u32> = (0..n)
        .map(|i| {
            let mut p = parents[i];
            let mut steps = 0;
            while p != NO_SYMBOL && is_member_variable_kind(kinds[p as usize]) && steps <= n {
                p = parents[p as usize];
                steps += 1;
            }
            p
        })
        .collect();

    // Ordinals for unnamed or overloadable declarations, assigned in declaration order.
    let mut ordinal = vec![0u32; n];
    let mut counters: HashMap<(u32, u8, String, u16), u32> = HashMap::new();
    for (i, slot) in ordinal.iter_mut().enumerate() {
        let kind = kinds[i];
        let overloadable = kind == SymbolKind::SK_METHOD as u8 || kind == SymbolKind::SK_CONSTRUCTOR as u8;
        let key_name = if overloadable { names[i].clone() } else { String::new() };
        let arity = if overloadable { param_counts[i] } else { 0 };
        let counter = counters
            .entry((effective_parent[i], kind, key_name, arity))
            .or_insert(0);
        *slot = *counter;
        *counter += 1;
    }

    let descriptors = derive_down(&effective_parent, |i, parent| {
        let kind = kinds[i];
        let name = names[i].as_str();
        let own = if kind == SymbolKind::SK_PACKAGE as u8 || kind == SymbolKind::SK_MODULE as u8 {
            name.split('.')
                .filter(|s| !s.is_empty())
                .map(|s| format!("{}/", escape_identifier(s)))
                .collect::<String>()
        } else if kind == SymbolKind::SK_EXTERNAL as u8 {
            let full = if qualified[i].is_empty() { name } else { qualified[i].as_str() };
            let segments: Vec<&str> = full.split('.').filter(|s| !s.is_empty()).collect();
            match segments.split_last() {
                Some((last, init)) => {
                    let mut d: String = init.iter().map(|s| format!("{}/", escape_identifier(s))).collect();
                    d.push_str(&format!("{}#", escape_identifier(last)));
                    d
                }
                None => format!("{}#", escape_identifier(&format!("external${}", ordinal[i]))),
            }
        } else if is_type_kind(kind) {
            if name.is_empty() {
                format!("{}#", escape_identifier(&format!("anon${}", ordinal[i])))
            } else {
                format!("{}#", escape_identifier(name))
            }
        } else if kind == SymbolKind::SK_METHOD as u8 {
            format!("{}({}).", escape_identifier(name), disambiguator(param_counts[i], ordinal[i]))
        } else if kind == SymbolKind::SK_CONSTRUCTOR as u8 {
            format!("`<init>`({}).", disambiguator(param_counts[i], ordinal[i]))
        } else if kind == SymbolKind::SK_STATIC_INIT as u8 {
            format!("`<clinit>`({}).", ordinal[i])
        } else if kind == SymbolKind::SK_INSTANCE_INIT as u8 {
            format!("`<instinit>`({}).", ordinal[i])
        } else if kind == SymbolKind::SK_LAMBDA as u8 {
            format!("{}().", escape_identifier(&format!("lambda${}", ordinal[i])))
        } else if kind == SymbolKind::SK_FIELD as u8 || kind == SymbolKind::SK_ENUM_CONSTANT as u8 {
            format!("{}.", escape_identifier(name))
        } else if kind == SymbolKind::SK_PARAM as u8 {
            format!("({})", escape_identifier(name))
        } else if kind == SymbolKind::SK_TYPE_PARAM as u8 {
            format!("[{}]", escape_identifier(name))
        } else {
            // Locals: their descriptor never feeds a child (effective parents skip them).
            String::new()
        };
        match parent {
            Some(p) => format!("{}{}", p, own),
            None => own,
        }
    });

    (0..n)
        .map(|i| {
            if kinds[i] == SymbolKind::SK_LOCAL_VAR as u8 {
                format!("{}{}", LOCAL_ID_PREFIX, i)
            } else {
                format!("{}{}", STABLE_ID_PREFIX, descriptors[i])
            }
        })
        .collect()
}

fn disambiguator(param_count: u16, ordinal: u32) -> String {
    if ordinal == 0 {
        format!("{}", param_count)
    } else {
        format!("{}+{}", param_count, ordinal)
    }
}

// ── Type hierarchy ────────────────────────────────────────────────────────────

pub const REL_EXTENDS: u8 = 0;
pub const REL_IMPLEMENTS: u8 = 1;
pub const REL_USES: u8 = 2;
pub const REL_CREATES: u8 = 3;
pub const REL_ASSOCIATION: u8 = 4;

pub fn relation_name(relation: u8) -> &'static str {
    match relation {
        REL_EXTENDS => "extends",
        REL_IMPLEMENTS => "implements",
        REL_USES => "uses",
        REL_CREATES => "creates",
        _ => "association",
    }
}

/// Type-level edges: inheritance (extends/implements) and type references (uses, creates,
/// field associations), with forward and reverse CSR adjacency.
#[derive(Debug, Clone, Default)]
pub struct TypeHierarchyIndex {
    node_count: usize,
    from: Vec<u32>,
    to: Vec<u32>,
    relation: Vec<u8>,

    super_offsets: Vec<u32>,
    super_adj: Vec<u32>,
    sub_offsets: Vec<u32>,
    sub_adj: Vec<u32>,
    ref_offsets: Vec<u32>,
    ref_adj: Vec<u32>,
    dep_offsets: Vec<u32>,
    dep_adj: Vec<u32>,
}

impl TypeHierarchyIndex {
    pub fn from_artifacts(sta: &SymbolTableArtifact) -> Self {
        let n = sta.symbol_records.len();
        let mut edges: Vec<(u32, u32, u8)> = Vec::new();
        for e in &sta.th_edges {
            edges.push((e.from_sym, e.to_sym, e.relation as u8));
        }
        for a in &sta.associations {
            edges.push((a.from_symbol_id, a.to_symbol_id, REL_ASSOCIATION));
        }
        edges.retain(|&(f, t, _)| f != t && (f as usize) < n && (t as usize) < n);
        edges.sort_unstable();
        edges.dedup();
        let mut index = Self {
            node_count: n,
            from: edges.iter().map(|e| e.0).collect(),
            to: edges.iter().map(|e| e.1).collect(),
            relation: edges.iter().map(|e| e.2).collect(),
            ..Default::default()
        };
        index.rebuild_derived();
        index
    }

    fn rebuild_derived(&mut self) {
        let n = self.node_count;
        let mut supers = Vec::new();
        let mut subs = Vec::new();
        let mut refs = Vec::new();
        let mut deps = Vec::new();
        for ((&f, &t), &r) in self.from.iter().zip(&self.to).zip(&self.relation) {
            if r == REL_EXTENDS || r == REL_IMPLEMENTS {
                supers.push((f, t));
                subs.push((t, f));
            } else {
                refs.push((f, t));
                deps.push((t, f));
            }
        }
        let (o, a) = csr_from_pairs(n, supers);
        self.super_offsets = o;
        self.super_adj = a;
        let (o, a) = csr_from_pairs(n, subs);
        self.sub_offsets = o;
        self.sub_adj = a;
        let (o, a) = csr_from_pairs(n, refs);
        self.ref_offsets = o;
        self.ref_adj = a;
        let (o, a) = csr_from_pairs(n, deps);
        self.dep_offsets = o;
        self.dep_adj = a;
    }

    pub fn edges(&self) -> impl Iterator<Item = (u32, u32, u8)> + '_ {
        (0..self.from.len()).map(move |i| (self.from[i], self.to[i], self.relation[i]))
    }

    pub fn edge_count(&self) -> usize {
        self.from.len()
    }

    pub fn supertypes(&self, ty: u32) -> &[u32] {
        row(&self.super_offsets, &self.super_adj, ty)
    }

    pub fn subtypes(&self, ty: u32) -> &[u32] {
        row(&self.sub_offsets, &self.sub_adj, ty)
    }

    /// Types referenced by `ty` (uses, creates, associations).
    pub fn type_refs(&self, ty: u32) -> &[u32] {
        row(&self.ref_offsets, &self.ref_adj, ty)
    }

    /// Types that reference `ty` (reverse of `type_refs`).
    pub fn dependents(&self, ty: u32) -> &[u32] {
        row(&self.dep_offsets, &self.dep_adj, ty)
    }

    fn closure(&self, start: u32, next: impl Fn(u32) -> Vec<u32>) -> Vec<u32> {
        let mut seen = vec![false; self.node_count];
        let mut out = Vec::new();
        let mut queue = VecDeque::new();
        if (start as usize) < self.node_count {
            seen[start as usize] = true;
            queue.push_back(start);
        }
        while let Some(t) = queue.pop_front() {
            for s in next(t) {
                if (s as usize) < self.node_count && !seen[s as usize] {
                    seen[s as usize] = true;
                    out.push(s);
                    queue.push_back(s);
                }
            }
        }
        out
    }

    /// All transitive supertypes of `ty` in BFS order (nearest first), excluding `ty`.
    pub fn all_supertypes(&self, ty: u32) -> Vec<u32> {
        self.closure(ty, |t| self.supertypes(t).to_vec())
    }

    /// All transitive subtypes of `ty` in BFS order, excluding `ty`.
    pub fn all_subtypes(&self, ty: u32) -> Vec<u32> {
        self.closure(ty, |t| self.subtypes(t).to_vec())
    }

    fn can_override(symbols: &SymbolIndex, method: u32) -> bool {
        symbols.kind(method) == Some(SymbolKind::SK_METHOD as u8)
            && !symbols.has_modifier(method, SymbolModifiers::STATIC)
            && !symbols.is_private(method)
    }

    fn matching_methods(&self, symbols: &SymbolIndex, method: u32, types: Vec<u32>) -> Vec<u32> {
        if !Self::can_override(symbols, method) {
            return Vec::new();
        }
        let (Some(name), Some(arity)) = (symbols.name(method), symbols.param_count(method)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for t in types {
            for &c in symbols.children_named(t, name) {
                if c != method && Self::can_override(symbols, c) && symbols.param_count(c) == Some(arity) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Methods in supertypes that `method` overrides (same name and arity; arity is the finest
    /// signature information Phase 3 records).
    pub fn overridden_methods(&self, symbols: &SymbolIndex, method: u32) -> Vec<u32> {
        let Some(owner) = symbols.parent(method) else {
            return Vec::new();
        };
        self.matching_methods(symbols, method, self.all_supertypes(owner))
    }

    /// Methods in subtypes that override `method`.
    pub fn overriding_methods(&self, symbols: &SymbolIndex, method: u32) -> Vec<u32> {
        let Some(owner) = symbols.parent(method) else {
            return Vec::new();
        };
        self.matching_methods(symbols, method, self.all_subtypes(owner))
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u64(self.node_count as u64);
        enc.put_u32s(&self.from);
        enc.put_u32s(&self.to);
        enc.put_u8s(&self.relation);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>, symbol_count: usize) -> Result<Self, ScpgFormatError> {
        let node_count = dec.u64()? as usize;
        if node_count != symbol_count {
            return Err(dec.malformed(format!(
                "type hierarchy covers {} symbols, symbol table has {}",
                node_count, symbol_count
            )));
        }
        let from = dec.u32s()?;
        let to = dec.u32s()?;
        let relation = dec.u8s()?.to_vec();
        dec.expect_len("to", to.len(), from.len())?;
        dec.expect_len("relation", relation.len(), from.len())?;
        if from.iter().chain(to.iter()).any(|&s| s as usize >= node_count) {
            return Err(dec.malformed("type hierarchy edge references an unknown symbol"));
        }
        let mut index = Self {
            node_count,
            from,
            to,
            relation,
            ..Default::default()
        };
        index.rebuild_derived();
        Ok(index)
    }
}
