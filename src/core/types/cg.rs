//! Core Types for Phase 6 Inter-procedural Call Graph & Points-To Analysis.
//! Authored solely by Ahmad Hassan (B-Ted).

use std::fmt;

/// Call Edge Types (Σ_CG) (§6.2.1)
pub const CG_EDGE_DIRECT: u8 = 0x00;
pub const CG_EDGE_SPECIAL: u8 = 0x01;
pub const CG_EDGE_VIRTUAL: u8 = 0x02;
pub const CG_EDGE_INTERFACE: u8 = 0x03;
pub const CG_EDGE_CONSTRUCTOR: u8 = 0x04;
pub const CG_EDGE_DYNAMIC: u8 = 0x05;
pub const CG_EDGE_REFLECTION: u8 = 0x06;

/// Call Site Flags (§6.4)
pub const CALL_SITE_FLAG_IS_POLYMORPHIC: u8 = 1 << 0;
pub const CALL_SITE_FLAG_HAS_NULL_RECEIVER: u8 = 1 << 1;
pub const CALL_SITE_FLAG_IS_TAIL_CALL: u8 = 1 << 2;

/// Dispatch kind of a call edge (protocol field `dispatch`, see
/// `docs/engine-query-protocol.md` section 2.2). Statically bound calls (static methods,
/// private methods, `super.m()`) are `static`; lambda creation and method references are
/// `lambda`.
pub const CG_DISPATCH_STATIC: u8 = 0;
pub const CG_DISPATCH_VIRTUAL: u8 = 1;
pub const CG_DISPATCH_INTERFACE: u8 = 2;
pub const CG_DISPATCH_LAMBDA: u8 = 3;
pub const CG_DISPATCH_CONSTRUCTOR: u8 = 4;

/// How the target of a call edge was established (protocol field `resolution`):
/// * `exact`: compile-time selection with no run-time choice (static, special, constructor,
///   lambda creation, method reference);
/// * `cha`: run-time dispatch expanded over the class hierarchy of the static receiver type
///   (CHA filtered by RTA), including the fallback to the declared method when the project
///   has no concrete override;
/// * `points_to`: a virtual call narrowed to the allocation types its receiver points to;
/// * `symbol_table`: Phase 6 found no target and the Phase 3 symbol table selected one.
pub const CG_RESOLUTION_EXACT: u8 = 0;
pub const CG_RESOLUTION_CHA: u8 = 1;
pub const CG_RESOLUTION_POINTS_TO: u8 = 2;
pub const CG_RESOLUTION_SYMBOL_TABLE: u8 = 3;

/// Why a call site has no target (`UnresolvedCallSite::reason`).
pub const CG_UNRESOLVED_NO_SOURCE_TEXT: u8 = 1;
pub const CG_UNRESOLVED_CALL_SHAPE: u8 = 2;
pub const CG_UNRESOLVED_RECEIVER_TYPE_UNKNOWN: u8 = 3;
pub const CG_UNRESOLVED_LIBRARY_MEMBER: u8 = 4;
pub const CG_UNRESOLVED_MEMBER_NOT_FOUND: u8 = 5;
pub const CG_UNRESOLVED_TYPE_UNKNOWN: u8 = 6;
pub const CG_UNRESOLVED_IMPLICIT_CONSTRUCTOR: u8 = 7;
pub const CG_UNRESOLVED_NO_MATCHING_CONSTRUCTOR: u8 = 8;
pub const CG_UNRESOLVED_ANONYMOUS_CLASS: u8 = 9;
pub const CG_UNRESOLVED_LAMBDA_WITHOUT_SYMBOL: u8 = 10;
pub const CG_UNRESOLVED_AMBIGUOUS: u8 = 11;

/// Protocol name of a `CG_DISPATCH_*` value.
pub fn dispatch_name(dispatch: u8) -> Option<&'static str> {
    Some(match dispatch {
        CG_DISPATCH_STATIC => "static",
        CG_DISPATCH_VIRTUAL => "virtual",
        CG_DISPATCH_INTERFACE => "interface",
        CG_DISPATCH_LAMBDA => "lambda",
        CG_DISPATCH_CONSTRUCTOR => "constructor",
        _ => return None,
    })
}

/// Protocol name of a `CG_RESOLUTION_*` value.
pub fn resolution_name(resolution: u8) -> Option<&'static str> {
    Some(match resolution {
        CG_RESOLUTION_EXACT => "exact",
        CG_RESOLUTION_CHA => "cha",
        CG_RESOLUTION_POINTS_TO => "points_to",
        CG_RESOLUTION_SYMBOL_TABLE => "symbol_table",
        _ => return None,
    })
}

/// Dispatch kind of a `CG_EDGE_*` call type. `None` for `CG_EDGE_REFLECTION` (never
/// produced: reflective calls are not modelled) and for unknown codes.
pub fn dispatch_of_edge_type(edge_type: u8) -> Option<u8> {
    Some(match edge_type {
        CG_EDGE_DIRECT | CG_EDGE_SPECIAL => CG_DISPATCH_STATIC,
        CG_EDGE_VIRTUAL => CG_DISPATCH_VIRTUAL,
        CG_EDGE_INTERFACE => CG_DISPATCH_INTERFACE,
        CG_EDGE_CONSTRUCTOR => CG_DISPATCH_CONSTRUCTOR,
        CG_EDGE_DYNAMIC => CG_DISPATCH_LAMBDA,
        _ => return None,
    })
}

/// Short description of a `CG_UNRESOLVED_*` reason.
pub fn unresolved_reason_text(reason: u8) -> &'static str {
    match reason {
        CG_UNRESOLVED_NO_SOURCE_TEXT => "call roles cannot be recovered without the token corpus",
        CG_UNRESOLVED_CALL_SHAPE => "call syntax not recognised",
        CG_UNRESOLVED_RECEIVER_TYPE_UNKNOWN => "receiver type unknown",
        CG_UNRESOLVED_LIBRARY_MEMBER => "library member (library members are not modelled)",
        CG_UNRESOLVED_MEMBER_NOT_FOUND => "no member with this name and number of arguments",
        CG_UNRESOLVED_TYPE_UNKNOWN => "type not found",
        CG_UNRESOLVED_IMPLICIT_CONSTRUCTOR => "implicit default constructor (no symbol)",
        CG_UNRESOLVED_NO_MATCHING_CONSTRUCTOR => "no constructor with this number of arguments",
        CG_UNRESOLVED_ANONYMOUS_CLASS => "anonymous class creation (no constructor symbol)",
        CG_UNRESOLVED_LAMBDA_WITHOUT_SYMBOL => "lambda without a symbol",
        CG_UNRESOLVED_AMBIGUOUS => "several candidates; the arguments do not select one",
        _ => "unknown reason",
    }
}

/// Dispatch and resolution kind of one `site_to_edge_map` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiteEdgeKind {
    /// `CG_DISPATCH_*`.
    pub dispatch: u8,
    /// `CG_RESOLUTION_*`.
    pub resolution: u8,
}

/// A call site for which no target was derived, with the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnresolvedCallSite {
    pub call_site_id: u32,
    /// `CG_UNRESOLVED_*`.
    pub reason: u8,
    /// Interned text id (token corpus) of the name written at the site: the method name,
    /// or the simple name of the instantiated type; `u32::MAX` when not recoverable.
    pub name_id: u32,
    /// Type symbol the reason refers to (receiver, owner or instantiated type), or
    /// `u32::MAX`.
    pub type_sym: u32,
}

/// SCC Classification Class (§6.4)
pub const SCC_CLASS_NON_RECURSIVE: u8 = 0;
pub const SCC_CLASS_SELF_RECURSIVE: u8 = 1;
pub const SCC_CLASS_MUTUAL_RECURSIVE: u8 = 2;

/// Fixed-size 28-byte CallSite (§6.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSite {
    pub call_site_id: u32,
    pub caller_sym: u32,
    pub call_node: u32,
    pub receiver_ssa: u32,
    pub call_block: u32,
    pub call_token: u32,
    pub call_type: u8,
    pub flags: u8,
    pub arg_count: u16,
}

impl CallSite {
    pub fn new(
        call_site_id: u32,
        caller_sym: u32,
        call_node: u32,
        receiver_ssa: u32,
        call_block: u32,
        call_token: u32,
        call_type: u8,
        flags: u8,
        arg_count: u16,
    ) -> Self {
        Self {
            call_site_id,
            caller_sym,
            call_node,
            receiver_ssa,
            call_block,
            call_token,
            call_type,
            flags,
            arg_count,
        }
    }

    pub fn is_polymorphic(&self) -> bool {
        (self.flags & CALL_SITE_FLAG_IS_POLYMORPHIC) != 0
    }
}

/// Fixed-size 16-byte CallEdge (§6.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallEdge {
    pub callee_sym: u32,
    pub call_site_id: u32,
    pub edge_type: u8,
    pub _padding: [u8; 7],
}

impl CallEdge {
    pub fn new(callee_sym: u32, call_site_id: u32, edge_type: u8) -> Self {
        Self {
            callee_sym,
            call_site_id,
            edge_type,
            _padding: [0; 7],
        }
    }
}

/// Fixed-size 16-byte SCCRecord (§6.4).
///
/// `member_count` is a full u32: an SCC may contain every method of the program, so a
/// 16-bit count silently wrapped on large codebases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SCCRecord {
    pub scc_id: u32,
    pub member_offset: u32,
    pub member_count: u32,
    pub scc_class: u8,
    pub _padding: [u8; 3],
}

impl SCCRecord {
    /// `member_count` accepts any unsigned width up to u32, so legacy u16 readers still
    /// construct records; producers should pass the exact `u32` count.
    pub fn new(
        scc_id: u32,
        member_offset: u32,
        member_count: impl Into<u32>,
        scc_class: u8,
    ) -> Self {
        Self {
            scc_id,
            member_offset,
            member_count: member_count.into(),
            scc_class,
            _padding: [0; 3],
        }
    }
}

/// Points-To Entry from Anderson analysis (§6.2.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsToEntry {
    pub ssa_id: u32,
    pub alloc_type_sym_id: u32,
}

/// CSR representation for Call Graph edges
#[derive(Debug, Clone, Default)]
pub struct CGCSR {
    pub offsets: Vec<u32>,
    pub adj: Vec<u32>,
    pub edge_types: Vec<u8>,
}

impl CGCSR {
    pub fn edges_of(&self, method_id: u32) -> (&[u32], &[u8]) {
        let idx = method_id as usize;
        if idx + 1 < self.offsets.len() {
            let start = self.offsets[idx] as usize;
            let end = self.offsets[idx + 1] as usize;
            if start <= end && end <= self.adj.len() {
                return (&self.adj[start..end], &self.edge_types[start..end]);
            }
        }
        (&[], &[])
    }
}

/// In-memory CallGraphArtifact (.cga) Persisted Representation (§6.6)
#[derive(Debug, Clone)]
pub struct CallGraphArtifact {
    pub format_version: u32,
    pub method_count: u32,
    pub call_site_count: u32,
    pub call_edge_count: u32,
    pub ssa_hash: u64,
    pub sta_hash: u64,

    pub call_sites: Vec<CallSite>,
    pub callee_csr: CGCSR,
    pub caller_csr: CGCSR,
    pub site_to_edge_map: Vec<(u32, u32, u32)>, // (caller_sym, callee_sym, call_site_id)
    pub points_to_table: Vec<PointsToEntry>,
    pub sccs: Vec<SCCRecord>,
    pub scc_members: Vec<u32>,
    /// Dispatch and resolution kind of every `site_to_edge_map` entry (same order and
    /// length). Format version 4.
    pub edge_kinds: Vec<SiteEdgeKind>,
    /// Call sites without any target, sorted by `call_site_id`. Format version 4.
    pub unresolved_sites: Vec<UnresolvedCallSite>,
}

impl CallGraphArtifact {
    /// Unresolved record of a call site, if the site has no target.
    pub fn unresolved_site(&self, call_site_id: u32) -> Option<&UnresolvedCallSite> {
        self.unresolved_sites
            .binary_search_by_key(&call_site_id, |u| u.call_site_id)
            .ok()
            .map(|i| &self.unresolved_sites[i])
    }

    /// Call site by id (`call_sites` is indexed by id).
    pub fn site(&self, call_site_id: u32) -> Option<&CallSite> {
        self.call_sites
            .get(call_site_id as usize)
            .filter(|s| s.call_site_id == call_site_id)
    }
}

impl fmt::Display for CallSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CallSite #{} [caller={} node={} block={} type={:#04x} args={}]",
            self.call_site_id,
            self.caller_sym,
            self.call_node,
            self.call_block,
            self.call_type,
            self.arg_count
        )
    }
}
