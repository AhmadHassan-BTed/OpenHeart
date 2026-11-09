//! Sparse intraprocedural data-flow analyses over SSA (§5.2.6).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! These analyses propagate facts along the SSA def-use / operand graph of one function
//! until a fixpoint. They are **intraprocedural** and are *not* an IFDS/IDE tabulation
//! solver: there is no interprocedural supergraph and no summary edges, so facts do not
//! flow into callees or back from them. (The result type keeps its historical name
//! `IFDSResults`, which is shared with downstream phases.)
//!
//! * **Taint** — a definition is tainted by source `k` when its value contains a call to
//!   a source method of the [`TaintSpec`], or when any operand it reads is tainted.
//!   A definition whose value is a direct call to a sanitizer is clean. Facts are
//!   `(ssa_id, k + 1)`.
//! * **Nullness** (may-be-null) — a definition may be null when its value is the `null`
//!   literal, a copy/cast of a may-be-null value, a conditional expression with such a
//!   branch, or a φ with such an argument. Values produced by calls or field loads are
//!   not classified (they are neither reported nor assumed non-null).
//! * **Resource type-state** — every allocation of a closeable-looking type (by simple
//!   name) gets one state for the function, over the allocation and all its copies:
//!   `TS_CLOSED` when `close()` is invoked on it (or it is a try-with-resources
//!   resource), else `TS_ESCAPED` when it is returned, passed to a call, stored outside
//!   locals or captured, else `TS_OPEN` (not released in this function: a leak
//!   candidate). The state is flow-insensitive (it says whether *some* path closes it).

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{call_name, children, is_nested_body, subtree_end, NewParts};
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::source::SourceText;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::ssa::{IFDSResults, PhiRecord, SSARecord};
use crate::ssa::names::FunctionVars;
use crate::ssa::renaming::PHI_USE;
use std::collections::HashMap;

pub const TS_OPEN: u16 = 1;
pub const TS_CLOSED: u16 = 2;
pub const TS_ESCAPED: u16 = 3;

/// Configurable taint sources and sanitizers, matched by method simple name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaintSpec {
    sources: Vec<Vec<u8>>,
    sanitizers: Vec<Vec<u8>>,
}

impl TaintSpec {
    /// Source ids are stored as bits of a u64 mask.
    pub const MAX_SOURCES: usize = 64;

    pub fn new(sources: Vec<Vec<u8>>, sanitizers: Vec<Vec<u8>>) -> Result<Self, String> {
        if sources.len() > Self::MAX_SOURCES {
            return Err(format!(
                "taint specification has {} sources; at most {} are supported",
                sources.len(),
                Self::MAX_SOURCES
            ));
        }
        Ok(Self {
            sources,
            sanitizers,
        })
    }

    /// Servlet / I/O / environment sources and common encoders and parsers as sanitizers.
    pub fn java_default() -> Self {
        let sources: &[&[u8]] = &[
            b"getParameter",
            b"getParameterValues",
            b"getParameterMap",
            b"getHeader",
            b"getHeaders",
            b"getQueryString",
            b"getRequestURI",
            b"getRequestURL",
            b"getPathInfo",
            b"getCookies",
            b"getInputStream",
            b"getReader",
            b"readLine",
            b"nextLine",
            b"getenv",
            b"readObject",
        ];
        let sanitizers: &[&[u8]] = &[
            b"encodeForHTML",
            b"encodeForHTMLAttribute",
            b"encodeForJavaScript",
            b"encodeForSQL",
            b"encodeForURL",
            b"escapeHtml4",
            b"escapeEcmaScript",
            b"escapeXml",
            b"escapeSql",
            b"sanitize",
            b"parseInt",
            b"parseLong",
            b"parseDouble",
            b"parseBoolean",
        ];
        Self {
            sources: sources.iter().map(|s| s.to_vec()).collect(),
            sanitizers: sanitizers.iter().map(|s| s.to_vec()).collect(),
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    fn source_index(&self, name: &[u8]) -> Option<usize> {
        self.sources.iter().position(|s| s.as_slice() == name)
    }

    fn is_sanitizer(&self, name: &[u8]) -> bool {
        self.sanitizers.iter().any(|s| s.as_slice() == name)
    }
}

impl Default for TaintSpec {
    fn default() -> Self {
        Self::java_default()
    }
}

/// Everything the analyses read about one function.
pub struct DataflowInput<'d, 'a> {
    pub bpa: &'d BPASTArtifact,
    pub src: Option<&'d SourceText<'a>>,
    pub cfg: &'d FunctionCFGData,
    pub vars: &'d FunctionVars,
    pub ssa_records: &'d [SSARecord],
    pub phi_records: &'d [PhiRecord],
    pub value_nodes: &'d [u32],
    pub operand_offsets: &'d [u32],
    pub operand_adj: &'d [u32],
    pub def_offsets: &'d [u32],
    pub use_adj: &'d [u32],
}

impl<'d, 'a> DataflowInput<'d, 'a> {
    fn csr<'x>(offsets: &[u32], adj: &'x [u32], i: u32) -> &'x [u32] {
        let i = i as usize;
        if i + 1 < offsets.len() {
            let (s, e) = (offsets[i] as usize, offsets[i + 1] as usize);
            if s <= e && e <= adj.len() {
                return &adj[s..e];
            }
        }
        &[]
    }

    fn operands(&self, d: u32) -> &'d [u32] {
        Self::csr(self.operand_offsets, self.operand_adj, d)
    }

    fn uses(&self, d: u32) -> &'d [u32] {
        Self::csr(self.def_offsets, self.use_adj, d)
    }

    /// Call nodes evaluated by the expression `root` (nested bodies and lowered
    /// sub-expressions excluded).
    fn calls_in(&self, root: u32) -> Vec<u32> {
        let bpa = self.bpa;
        let end = subtree_end(bpa, root);
        let mut out = Vec::new();
        let mut k = root;
        while k < end {
            let t = bpa.node_type(k);
            if k != root && (is_nested_body(t) || self.cfg.is_lowered(k)) {
                k = subtree_end(bpa, k);
                continue;
            }
            if t == ASTNodeType::NN_CALL_EXPR {
                out.push(k);
            }
            k += 1;
        }
        out
    }

    fn call_name_text(&self, call: u32) -> Option<&'a [u8]> {
        let src = self.src?;
        let name = call_name(self.bpa, src, call)?;
        src.node_text(self.bpa, name)
    }

    fn is_null_literal(&self, node: u32) -> bool {
        self.bpa.node_type(node) == ASTNodeType::NN_LITERAL
            && self
                .src
                .is_some_and(|s| s.node_text(self.bpa, node) == Some(b"null"))
    }
}

pub struct SparseDataflowAnalyzer;

impl SparseDataflowAnalyzer {
    pub fn analyze(input: &DataflowInput<'_, '_>, spec: &TaintSpec) -> IFDSResults {
        let taint_sparse = Self::taint(input, spec);
        let nullable_sparse = Self::nullness(input);
        let type_state_sparse = Self::resource_states(input);

        if crate::core::logger::get_log_level() >= crate::core::logger::LogLevel::Trace {
            crate::core::logger::log_trace(&format!(
                "Sparse data-flow analyses complete: {} taint facts, {} nullable variables, {} resource states",
                taint_sparse.len(),
                nullable_sparse.len(),
                type_state_sparse.len()
            ));
        }

        IFDSResults {
            taint_sparse,
            nullable_sparse,
            type_state_sparse,
        }
    }

    fn taint(input: &DataflowInput<'_, '_>, spec: &TaintSpec) -> Vec<(u32, u16)> {
        let n = input.ssa_records.len();
        let mut seed = vec![0u64; n];
        let mut sanitized = vec![false; n];

        if input.src.is_some() && spec.source_count() > 0 {
            for d in 0..n {
                let value = input.value_nodes.get(d).copied().unwrap_or(u32::MAX);
                if value == u32::MAX || input.ssa_records[d].is_phi() {
                    continue;
                }
                if input.bpa.node_type(value) == ASTNodeType::NN_CALL_EXPR
                    && input
                        .call_name_text(value)
                        .is_some_and(|name| spec.is_sanitizer(name))
                {
                    sanitized[d] = true;
                    continue;
                }
                for call in input.calls_in(value) {
                    if let Some(k) = input
                        .call_name_text(call)
                        .and_then(|name| spec.source_index(name))
                    {
                        seed[d] |= 1u64 << k;
                    }
                }
            }
        }

        let mut taint = seed.clone();
        let mut changed = true;
        while changed {
            changed = false;
            for d in 0..n {
                if sanitized[d] {
                    continue;
                }
                let mut mask = taint[d];
                for &op in input.operands(d as u32) {
                    mask |= taint.get(op as usize).copied().unwrap_or(0);
                }
                if mask != taint[d] {
                    taint[d] = mask;
                    changed = true;
                }
            }
        }

        let mut out = Vec::new();
        for (d, &mask) in taint.iter().enumerate() {
            let mut m = mask;
            while m != 0 {
                let k = m.trailing_zeros();
                out.push((d as u32, (k + 1) as u16));
                m &= m - 1;
            }
        }
        out.sort_unstable();
        out
    }

    fn nullness(input: &DataflowInput<'_, '_>) -> Vec<u32> {
        let n = input.ssa_records.len();
        let bpa = input.bpa;

        // Statement node -> SSA ids it reads (branch operands of lowered conditionals are
        // statements of their own blocks).
        let mut use_at: HashMap<u32, Vec<u32>> = HashMap::new();
        for d in 0..n as u32 {
            for &stmt in input.uses(d) {
                if stmt != PHI_USE {
                    use_at.entry(stmt).or_default().push(d);
                }
            }
        }

        let mut nullable = vec![false; n];
        let mut changed = true;
        while changed {
            changed = false;
            for d in 0..n {
                if nullable[d] {
                    continue;
                }
                let rec = &input.ssa_records[d];
                let may_be_null = if rec.is_phi() {
                    input
                        .operands(d as u32)
                        .iter()
                        .any(|&a| nullable.get(a as usize).copied().unwrap_or(false))
                } else {
                    let value = input.value_nodes.get(d).copied().unwrap_or(u32::MAX);
                    value != u32::MAX
                        && Self::expr_may_be_null(input, value, d as u32, &nullable, &use_at, bpa)
                };
                if may_be_null {
                    nullable[d] = true;
                    changed = true;
                }
            }
        }

        (0..n as u32).filter(|&d| nullable[d as usize]).collect()
    }

    fn expr_may_be_null(
        input: &DataflowInput<'_, '_>,
        expr: u32,
        def: u32,
        nullable: &[bool],
        use_at: &HashMap<u32, Vec<u32>>,
        bpa: &BPASTArtifact,
    ) -> bool {
        let is_nullable = |d: u32| nullable.get(d as usize).copied().unwrap_or(false);
        match bpa.node_type(expr) {
            ASTNodeType::NN_LITERAL => input.is_null_literal(expr),
            // A copy (or cast of a copy) reads exactly the copied value.
            ASTNodeType::NN_IDENTIFIER_EXPR => input.operands(def).iter().any(|&o| is_nullable(o)),
            ASTNodeType::NN_CAST_EXPR => match children(bpa, expr).last() {
                Some(&inner) => Self::expr_may_be_null(input, inner, def, nullable, use_at, bpa),
                None => false,
            },
            ASTNodeType::NN_TERNARY_EXPR => {
                let kids = children(bpa, expr);
                kids.iter().skip(1).any(|&branch| {
                    if input.is_null_literal(branch) {
                        return true;
                    }
                    bpa.node_type(branch) == ASTNodeType::NN_IDENTIFIER_EXPR
                        && use_at
                            .get(&branch)
                            .is_some_and(|ids| ids.iter().any(|&o| is_nullable(o)))
                })
            }
            _ => false,
        }
    }

    fn is_resource_type(name: &[u8]) -> bool {
        const SUFFIXES: &[&[u8]] = &[
            b"Stream",
            b"Reader",
            b"Writer",
            b"Socket",
            b"Connection",
            b"Channel",
            b"Statement",
            b"ResultSet",
            b"Scanner",
            b"Formatter",
            b"ZipFile",
            b"JarFile",
            b"RandomAccessFile",
            b"Selector",
        ];
        SUFFIXES.iter().any(|s| name.ends_with(s))
    }

    fn resource_states(input: &DataflowInput<'_, '_>) -> Vec<(u32, u16)> {
        let src = match input.src {
            Some(s) => s,
            None => return Vec::new(),
        };
        let bpa = input.bpa;
        let n = input.ssa_records.len();

        // Copy successors: definitions that copy (or φ-merge) a value.
        let mut copies_of: Vec<Vec<u32>> = vec![Vec::new(); n];
        for d in 0..n {
            let rec = &input.ssa_records[d];
            let value = input.value_nodes.get(d).copied().unwrap_or(u32::MAX);
            let is_copy = rec.is_phi()
                || (value != u32::MAX
                    && matches!(
                        bpa.node_type(value),
                        ASTNodeType::NN_IDENTIFIER_EXPR | ASTNodeType::NN_CAST_EXPR
                    ));
            if is_copy {
                for &op in input.operands(d as u32) {
                    if (op as usize) < n {
                        copies_of[op as usize].push(d as u32);
                    }
                }
            }
        }

        let mut out = Vec::new();
        for d in 0..n {
            let rec = &input.ssa_records[d];
            let value = input.value_nodes.get(d).copied().unwrap_or(u32::MAX);
            if rec.is_phi() || value == u32::MAX || bpa.node_type(value) != ASTNodeType::NN_NEW_EXPR {
                continue;
            }
            let parts = NewParts::of(bpa, Some(src), value);
            let is_resource = parts
                .type_name
                .map(|t| Self::is_resource_type(src.text_of_id(t)))
                .unwrap_or(false);
            if !is_resource {
                continue;
            }

            // try-with-resources: the declarator is a direct child of the try statement.
            let stmt = rec.def_stmt;
            let auto_closed = stmt != u32::MAX
                && bpa.node_type(stmt) == ASTNodeType::NN_LOCAL_VAR_DECL
                && bpa.node_type(bpa.parent(stmt)) == ASTNodeType::NN_TRY_STMT;

            // Alias closure over copies and φs.
            let mut aliases = vec![d as u32];
            let mut i = 0;
            while i < aliases.len() {
                let a = aliases[i] as usize;
                for &c in &copies_of[a] {
                    if !aliases.contains(&c) {
                        aliases.push(c);
                    }
                }
                i += 1;
            }

            let mut closed = auto_closed;
            let mut escaped = false;
            for &a in &aliases {
                let orig = input.ssa_records[a as usize].orig_sym_id;
                for &use_stmt in input.uses(a) {
                    if use_stmt == PHI_USE {
                        continue;
                    }
                    let (c, e) = Self::scan_use(input, src, use_stmt, orig);
                    closed |= c;
                    escaped |= e;
                }
            }

            let state = if closed {
                TS_CLOSED
            } else if escaped {
                TS_ESCAPED
            } else {
                TS_OPEN
            };
            out.push((d as u32, state));
        }
        out.sort_unstable();
        out
    }

    /// Inspects one statement using variable `orig`: (`close()` invoked on it, value
    /// escapes the function).
    fn scan_use(
        input: &DataflowInput<'_, '_>,
        src: &SourceText<'_>,
        stmt: u32,
        orig: u32,
    ) -> (bool, bool) {
        let bpa = input.bpa;
        let vars = input.vars;
        let is_ref = |node: u32| {
            bpa.node_type(node) == ASTNodeType::NN_IDENTIFIER_EXPR
                && vars
                    .var_of_occurrence(node)
                    .is_some_and(|v| vars.orig_id(v) == orig)
        };

        let mut closed = false;
        let mut escaped = false;
        let end = subtree_end(bpa, stmt);
        let mut k = stmt;
        while k < end {
            let t = bpa.node_type(k);
            if k != stmt && t == ASTNodeType::NN_LAMBDA_EXPR {
                if vars.captures_of(k).iter().any(|&v| vars.orig_id(v) == orig) {
                    escaped = true;
                }
                k = subtree_end(bpa, k);
                continue;
            }
            if k != stmt && (is_nested_body(t) || input.cfg.is_lowered(k)) {
                k = subtree_end(bpa, k);
                continue;
            }
            match t {
                ASTNodeType::NN_CALL_EXPR => {
                    if let Some(crate::cfg::ast_query::CallShape::Method {
                        receiver,
                        name,
                        args,
                    }) = crate::cfg::ast_query::call_shape(bpa, src, k)
                    {
                        if receiver.is_some_and(is_ref) && src.node_text(bpa, name) == Some(b"close")
                        {
                            closed = true;
                        }
                        if args.iter().any(|&a| is_ref(a)) {
                            escaped = true;
                        }
                    }
                }
                ASTNodeType::NN_NEW_EXPR => {
                    if NewParts::of(bpa, Some(src), k).args.iter().any(|&a| is_ref(a)) {
                        escaped = true;
                    }
                }
                ASTNodeType::NN_RETURN_STMT | ASTNodeType::NN_YIELD_STMT => {
                    if bpa.first_child(k).is_some_and(is_ref) {
                        escaped = true;
                    }
                }
                ASTNodeType::NN_ASSIGN_EXPR => {
                    let kids = children(bpa, k);
                    if kids.len() == 2 && is_ref(kids[1]) {
                        let lhs_is_local = bpa.node_type(kids[0]) == ASTNodeType::NN_IDENTIFIER_EXPR
                            && vars.var_of_occurrence(kids[0]).is_some();
                        if !lhs_is_local {
                            escaped = true;
                        }
                    }
                }
                _ => {}
            }
            k += 1;
        }
        (closed, escaped)
    }
}
