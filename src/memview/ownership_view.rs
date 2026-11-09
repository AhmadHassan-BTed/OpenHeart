//! Ownership-transfer graphs: who creates, owns, borrows and releases each object.
//!
//! Ownership is read off two sources and the edge says which one it came from:
//!
//! * the declared reference kind of the slot an object is stored into (`RefKind::Owning`,
//!   `Shared`, `Weak`, `Borrowed`) — `Exact`, because the type system states it;
//! * the observed behaviour of the code (which frame releases the object, which callee
//!   keeps it beyond the call) — taken from callee summaries, `Exact` when the summary
//!   holds on every path of a single target and `OverApproximate` otherwise.
//!
//! A raw pointer or a reference without a declared kind gives a `Holds` edge: the slot
//! references the object but nothing states that it owns it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::ir::{Inst, IrType, ModuleIr, RefKind, TypeId};
use crate::memview::heap_shape::{HeapResult, MemEvent, MemEventKind};
use crate::memview::{AbsObj, Derivation, FieldKey, ProgramPoint, ReleaseKind, SrcSpan};

/// Kind of an ownership-graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "node")]
pub enum OwnerNodeKind {
    /// An activation of a function.
    Frame { func: u32, name: u32 },
    /// An abstract object (as an owned object, a container, or both).
    Object { obj: AbsObj },
    /// Code or memory outside the analyzed IR.
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipNode {
    pub key: String,
    pub kind: OwnerNodeKind,
    pub spans: Vec<SrcSpan>,
}

/// Relation an edge states about `object`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipEdgeKind {
    /// `from` allocated the object.
    Creates,
    /// `from` holds the object through an owning slot (or is its creating frame).
    Owns,
    /// `from` holds a shared (reference-counted) owning reference.
    SharesOwnership,
    /// `from` holds a weak reference.
    WeakRef,
    /// `from` holds a borrowed reference.
    Borrows,
    /// `from` references the object; the slot type states no ownership.
    Holds,
    /// Ownership moves from `from` to `to` (a callee that releases or keeps it).
    TransfersTo,
    /// The object leaves `from` by return into `to`.
    ReturnsTo,
    /// `from` releases the object.
    Releases,
    /// The object reaches code or memory outside the analyzed IR.
    EscapesTo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipEdge {
    pub from: String,
    pub to: String,
    pub kind: OwnershipEdgeKind,
    /// Key of the object the relation is about.
    pub object: String,
    pub points: Vec<ProgramPoint>,
    pub spans: Vec<SrcSpan>,
    pub derivation: Derivation,
}

/// Ownership summary of one allocation site's objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectOwnership {
    pub object: String,
    pub creator: Option<String>,
    /// Holders of owning, shared or transferred ownership.
    pub owners: Vec<String>,
    pub borrowers: Vec<String>,
    pub releasers: Vec<String>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipView {
    pub nodes: Vec<OwnershipNode>,
    pub edges: Vec<OwnershipEdge>,
    pub objects: Vec<ObjectOwnership>,
    pub derivation: Derivation,
}

const EXTERNAL: &str = "external";

/// Reference kind a slot's declared type states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaredRef {
    Owning,
    Shared,
    Weak,
    Borrowed,
}

fn frame_key(func: u32) -> String {
    format!("frame.f{func}")
}

struct Builder<'a> {
    module: &'a ModuleIr,
    result: &'a HeapResult,
    nodes: BTreeMap<String, OwnershipNode>,
    edges: BTreeMap<(String, String, OwnershipEdgeKind, String), OwnershipEdge>,
}

impl<'a> Builder<'a> {
    fn frame(&mut self, func: u32) -> String {
        let key = frame_key(func);
        if !self.nodes.contains_key(&key) {
            let (name, spans) = match self.result.index.func(self.module, func) {
                Some((f, _)) => (f.name, vec![SrcSpan::from_ir(&f.span)]),
                None => (crate::memview::NO_ID, Vec::new()),
            };
            self.nodes.insert(
                key.clone(),
                OwnershipNode {
                    key: key.clone(),
                    kind: OwnerNodeKind::Frame { func, name },
                    spans,
                },
            );
        }
        key
    }

    fn object(&mut self, obj: AbsObj) -> String {
        let group = obj.as_recent();
        let key = group.key();
        if !self.nodes.contains_key(&key) {
            let spans = match group {
                AbsObj::Site { site, .. } => vec![self.result.index.site_span(self.module, site)],
                _ => Vec::new(),
            };
            self.nodes.insert(
                key.clone(),
                OwnershipNode {
                    key: key.clone(),
                    kind: OwnerNodeKind::Object { obj: group },
                    spans,
                },
            );
        }
        key
    }

    fn external(&mut self) -> String {
        if !self.nodes.contains_key(EXTERNAL) {
            self.nodes.insert(
                EXTERNAL.to_string(),
                OwnershipNode {
                    key: EXTERNAL.to_string(),
                    kind: OwnerNodeKind::External,
                    spans: Vec::new(),
                },
            );
        }
        EXTERNAL.to_string()
    }

    fn edge(
        &mut self,
        from: String,
        to: String,
        kind: OwnershipEdgeKind,
        object: &str,
        e: &MemEvent,
        derivation: Derivation,
    ) {
        let entry = self
            .edges
            .entry((from.clone(), to.clone(), kind, object.to_string()))
            .or_insert_with(|| OwnershipEdge {
                from,
                to,
                kind,
                object: object.to_string(),
                points: Vec::new(),
                spans: Vec::new(),
                derivation: derivation.clone(),
            });
        if !entry.points.contains(&e.point) {
            entry.points.push(e.point);
            entry.spans.push(e.span);
        }
        let merged = std::mem::take(&mut entry.derivation).weakest(derivation);
        entry.derivation = merged;
    }

    /// Declared reference kind of a slot, when its type states one.
    fn slot_ref_kind(&self, container: AbsObj, slot: FieldKey) -> Option<DeclaredRef> {
        let ty: TypeId = match slot {
            FieldKey::Field(f) => {
                let p = *self.result.index.field_pos.get(&f)?;
                self.module.fields[p].ty
            }
            FieldKey::Whole => self
                .result
                .index
                .site_type(self.module, container.site()?)?,
            FieldKey::Elem => {
                let t = self
                    .result
                    .index
                    .site_type(self.module, container.site()?)?;
                match self.module.types.get(t)? {
                    IrType::Array { elem, .. } => *elem,
                    _ => return None,
                }
            }
            FieldKey::Capture(_) => return None,
        };
        match self.module.types.get(ty)? {
            IrType::Ref { kind, .. } => Some(match kind {
                RefKind::Owning => DeclaredRef::Owning,
                RefKind::Shared => DeclaredRef::Shared,
                RefKind::Weak => DeclaredRef::Weak,
                RefKind::Borrowed => DeclaredRef::Borrowed,
            }),
            _ => None,
        }
    }

    /// Callee targets of the call at `point`, with the parameter index of `arg`.
    fn call_params(&self, point: ProgramPoint, arg: Option<u32>) -> Vec<(u32, Option<u32>)> {
        let Some((f, fi)) = self.result.index.func(self.module, point.func) else {
            return Vec::new();
        };
        let Some(&bp) = fi.block_pos.get(&point.block) else {
            return Vec::new();
        };
        let Some(Inst::Call { site, args, .. }) = f.blocks[bp].insts.get(point.index as usize)
        else {
            return Vec::new();
        };
        let targets = self
            .result
            .index
            .callees
            .get(&site.0)
            .cloned()
            .unwrap_or_default();
        targets
            .into_iter()
            .map(|t| {
                let n = self
                    .result
                    .index
                    .func(self.module, t)
                    .map(|(cf, _)| cf.params.len())
                    .unwrap_or(args.len());
                let receiver_param = n == args.len() + 1;
                let param = match arg {
                    None if receiver_param => Some(0),
                    None => None,
                    Some(i) if receiver_param => Some(i + 1),
                    Some(i) => Some(i),
                };
                (t, param)
            })
            .collect()
    }

    fn on_event(&mut self, e: &MemEvent, callers_of: &HashMap<u32, BTreeSet<u32>>) {
        let obj_key = self.object(e.obj);
        let frame = self.frame(e.point.func);
        let old = if e.obj.is_old() {
            Derivation::OverApproximate
        } else {
            Derivation::Exact
        };
        match &e.kind {
            MemEventKind::Allocated { .. } => {
                self.edge(frame.clone(), obj_key.clone(), OwnershipEdgeKind::Creates, &obj_key, e, Derivation::Exact);
                self.edge(frame, obj_key.clone(), OwnershipEdgeKind::Owns, &obj_key, e, Derivation::Exact);
            }
            MemEventKind::StoredInto { container, slot } => {
                let holder = self.object(*container);
                let (kind, d) = match self.slot_ref_kind(*container, *slot) {
                    Some(DeclaredRef::Owning) => (OwnershipEdgeKind::Owns, old),
                    Some(DeclaredRef::Shared) => (OwnershipEdgeKind::SharesOwnership, old),
                    Some(DeclaredRef::Weak) => (OwnershipEdgeKind::WeakRef, old),
                    Some(DeclaredRef::Borrowed) => (OwnershipEdgeKind::Borrows, old),
                    None => (OwnershipEdgeKind::Holds, Derivation::OverApproximate),
                };
                self.edge(holder, obj_key.clone(), kind, &obj_key, e, d);
            }
            MemEventKind::Captured { closure, .. } => {
                let holder = self.object(*closure);
                self.edge(holder, obj_key.clone(), OwnershipEdgeKind::Holds, &obj_key, e, Derivation::OverApproximate);
            }
            MemEventKind::Passed { arg, callees, .. } => {
                if callees.is_empty() {
                    return;
                }
                let params = self.call_params(e.point, *arg);
                let single = params.len() == 1;
                for (t, param) in params {
                    let callee = self.frame(t);
                    let Some(sum) = self.result.summaries.get(&t) else {
                        continue;
                    };
                    let Some(p) = param else {
                        continue;
                    };
                    let precision = if single {
                        old.clone()
                    } else {
                        Derivation::OverApproximate
                    };
                    let released = sum.released.get(&p);
                    if released.is_some() || sum.escapes.contains(&p) {
                        let d = match released {
                            Some(r) if r.must => precision,
                            _ => Derivation::OverApproximate,
                        };
                        self.edge(frame.clone(), callee, OwnershipEdgeKind::TransfersTo, &obj_key, e, d);
                    } else {
                        self.edge(callee, obj_key.clone(), OwnershipEdgeKind::Borrows, &obj_key, e, precision);
                    }
                }
            }
            MemEventKind::Returned => {
                let callers = callers_of.get(&e.point.func).cloned().unwrap_or_default();
                let d = if callers.len() <= 1 {
                    old
                } else {
                    Derivation::OverApproximate
                };
                for c in callers {
                    let to = self.frame(c);
                    self.edge(frame.clone(), to, OwnershipEdgeKind::ReturnsTo, &obj_key, e, d.clone());
                }
            }
            MemEventKind::Released { kind, must } => {
                if *kind == Some(ReleaseKind::Unlock) {
                    return;
                }
                // A release at a call point is performed by the callee(s).
                let params = self.call_params(e.point, None);
                let releasers: Vec<String> = if params.is_empty() {
                    vec![frame]
                } else {
                    params.iter().map(|(t, _)| frame_key(*t)).collect()
                };
                for (t, _) in &params {
                    self.frame(*t);
                }
                let mut d = if *must {
                    old.clone()
                } else {
                    Derivation::OverApproximate
                };
                if kind.is_none() {
                    d = d.weakest(Derivation::partial(
                        "release semantics not supplied by the language layer",
                    ));
                }
                for r in releasers {
                    self.edge(r, obj_key.clone(), OwnershipEdgeKind::Releases, &obj_key, e, d.clone());
                }
            }
            MemEventKind::Escaped | MemEventKind::Thrown => {
                let ext = self.external();
                self.edge(frame, ext, OwnershipEdgeKind::EscapesTo, &obj_key, e, Derivation::partial(
                    "the object reaches code or memory outside the analyzed IR",
                ));
            }
            MemEventKind::Accessed { .. } | MemEventKind::Unreachable => {}
        }
    }
}

/// Builds the ownership-transfer graph of the module.
pub fn build(result: &HeapResult, module: &ModuleIr) -> OwnershipView {
    let mut callers_of: HashMap<u32, BTreeSet<u32>> = HashMap::new();
    for (callee, sites) in &result.index.callers {
        for s in sites {
            if let Some(&(fp, _, _)) = result.index.call_inst.get(s) {
                callers_of
                    .entry(*callee)
                    .or_default()
                    .insert(module.funcs[fp].id.0);
            }
        }
    }
    let mut b = Builder {
        module,
        result,
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
    };
    for e in result.all_events() {
        if e.obj.is_allocation() || matches!(e.obj, AbsObj::Closure { .. }) {
            b.on_event(e, &callers_of);
        }
    }

    let edges: Vec<OwnershipEdge> = b.edges.into_values().collect();
    let mut per_object: BTreeMap<String, ObjectOwnership> = BTreeMap::new();
    for e in &edges {
        let o = per_object
            .entry(e.object.clone())
            .or_insert_with(|| ObjectOwnership {
                object: e.object.clone(),
                creator: None,
                owners: Vec::new(),
                borrowers: Vec::new(),
                releasers: Vec::new(),
                derivation: Derivation::Exact,
            });
        let push = |v: &mut Vec<String>, k: &str| {
            if !v.iter().any(|x| x == k) {
                v.push(k.to_string());
            }
        };
        match e.kind {
            OwnershipEdgeKind::Creates => o.creator = Some(e.from.clone()),
            OwnershipEdgeKind::Owns | OwnershipEdgeKind::SharesOwnership => push(&mut o.owners, &e.from),
            OwnershipEdgeKind::TransfersTo | OwnershipEdgeKind::ReturnsTo => push(&mut o.owners, &e.to),
            OwnershipEdgeKind::Borrows | OwnershipEdgeKind::WeakRef => push(&mut o.borrowers, &e.from),
            OwnershipEdgeKind::Releases => push(&mut o.releasers, &e.from),
            OwnershipEdgeKind::EscapesTo => push(&mut o.owners, &e.to),
            OwnershipEdgeKind::Holds => {}
        }
        o.derivation = std::mem::take(&mut o.derivation).weakest(e.derivation.clone());
    }
    let mut derivation = result.derivation.clone();
    for o in per_object.values() {
        derivation = derivation.weakest(o.derivation.clone());
    }
    OwnershipView {
        nodes: b.nodes.into_values().collect(),
        edges,
        objects: per_object.into_values().collect(),
        derivation,
    }
}
