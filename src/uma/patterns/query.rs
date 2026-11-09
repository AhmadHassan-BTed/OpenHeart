//! Shared pattern fact context (§9.2.5).
//!
//! Exposes the structural and behavioral facts every pattern rule is written
//! against: validated supertypes, typed attributes (with collection element
//! types), constructors, overriding, allocations, and calls with their receiver
//! and loop context. Primitives, stubs and unresolved externals are never
//! treated as pattern participants.

use std::collections::{BTreeSet, HashMap};

use crate::core::types::cg::CG_EDGE_CONSTRUCTOR;
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::source::{AssignedValue, BodyFacts, Receiver};
use crate::uma::types::UNKNOWN_SYM;

/// A typed attribute of a class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldFact {
    pub sym: u32,
    pub name: u32,
    /// Canonical declared type.
    pub type_id: u32,
    /// Element type for arrays and library collections (UNKNOWN_SYM when unresolved).
    pub element: u32,
    pub is_static: bool,
    pub is_final: bool,
    pub is_collection: bool,
}

impl FieldFact {
    /// The user type this attribute refers to: the element type for collections.
    pub fn referenced_type(&self) -> u32 {
        if self.is_collection {
            self.element
        } else {
            self.type_id
        }
    }
}

/// A validated call made by a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFact {
    pub callee: u32,
    pub token: u32,
    pub in_loop: bool,
    /// Receiver as written; `None` when the call token is not in the scanned body.
    pub receiver: Option<Receiver>,
}

/// An invocation written on a named receiver (`x.m(...)`), with its resolved callees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThroughCall {
    pub method_text: u32,
    pub token: u32,
    pub callees: Vec<u32>,
    pub in_loop: bool,
}

pub struct PatternContext<'c> {
    pub index: &'c UMAIndex<'c>,
    pub bodies: &'c MemberBodies,
    fields: HashMap<u32, Vec<FieldFact>>,
    /// Referenced user type -> classes holding an attribute that references it.
    referrers: HashMap<u32, Vec<u32>>,
    /// Allocated user type -> members that allocate it.
    allocators: HashMap<u32, Vec<u32>>,
}

impl<'c> PatternContext<'c> {
    pub fn new(index: &'c UMAIndex<'c>, bodies: &'c MemberBodies) -> Self {
        let mut fields = HashMap::new();
        let mut referrers: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut allocators: HashMap<u32, Vec<u32>> = HashMap::new();
        for sym_id in 0..index.symbol_count() {
            let id = sym_id as u32;
            if !index.is_user_type(id) {
                continue;
            }
            let facts: Vec<FieldFact> = index
                .fields(id)
                .into_iter()
                .map(|f| Self::field_fact(index, id, f))
                .collect();
            for f in &facts {
                let r = f.referenced_type();
                if index.is_user_type(r) {
                    // Types are visited in ascending id order, so duplicates are adjacent.
                    let list = referrers.entry(r).or_default();
                    if list.last() != Some(&id) {
                        list.push(id);
                    }
                }
            }
            if !facts.is_empty() {
                fields.insert(id, facts);
            }
            for &member in index.children(id) {
                if let Some(body) = bodies.get(member) {
                    for a in &body.allocations {
                        let t = index.resolve_type_text(a.type_text, member);
                        if index.is_user_type(t) {
                            let list = allocators.entry(t).or_default();
                            if list.last() != Some(&member) {
                                list.push(member);
                            }
                        }
                    }
                }
            }
        }
        Self {
            index,
            bodies,
            fields,
            referrers,
            allocators,
        }
    }

    /// Classes holding an attribute (or collection) that references `t`.
    pub fn referrers(&self, t: u32) -> &[u32] {
        self.referrers.get(&t).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Members that allocate `t` with `new`.
    pub fn allocators(&self, t: u32) -> &[u32] {
        self.allocators.get(&t).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `m` invokes an operation named `text` on `this` (implicitly or explicitly).
    pub fn invokes_own(&self, m: u32, text: u32) -> bool {
        match self.body(m) {
            Some(b) => b.invocations.iter().any(|inv| {
                inv.method_text == text
                    && matches!(inv.receiver, Receiver::Implicit | Receiver::This)
            }),
            None => false,
        }
    }

    /// Name of the parameter of `m` typed `t` (first match).
    pub fn param_named_by_type(&self, m: u32, t: u32) -> Option<u32> {
        self.index
            .params(m)
            .into_iter()
            .find(|&p| self.index.type_of(p) == t)
            .map(|p| self.name_id(p))
    }

    fn field_fact(index: &UMAIndex<'_>, owner: u32, f: u32) -> FieldFact {
        let type_id = index.type_of(f);
        let shape = match index.symbol(f) {
            Some(s) if s.first_token_id != u32::MAX => index.scanner.type_shape(s),
            _ => Default::default(),
        };
        let is_collection = shape.is_array || index.is_collection_type(type_id);
        let element = if is_collection {
            shape
                .type_argument_names
                .iter()
                .rev()
                .map(|&t| index.resolve_type_text(t, owner))
                .find(|&s| s != UNKNOWN_SYM)
                .unwrap_or(if shape.is_array && index.is_user_type(type_id) {
                    type_id
                } else {
                    UNKNOWN_SYM
                })
        } else {
            UNKNOWN_SYM
        };
        FieldFact {
            sym: f,
            name: index.name_id(f),
            type_id,
            element,
            is_static: index.is_static(f),
            is_final: index.is_final(f),
            is_collection,
        }
    }

    // ── Type facts ────────────────────────────────────────────────────────────

    pub fn is_user_type(&self, t: u32) -> bool {
        self.index.is_user_type(t)
    }

    pub fn is_interface(&self, t: u32) -> bool {
        self.index.is_interface(t)
    }

    pub fn is_abstract_type(&self, t: u32) -> bool {
        self.index.is_abstract_type(t)
    }

    pub fn is_concrete_class(&self, t: u32) -> bool {
        self.index.is_concrete_class(t)
    }

    /// User-declared direct supertypes.
    pub fn user_supertypes(&self, t: u32) -> Vec<u32> {
        self.index
            .direct_supertypes(t)
            .into_iter()
            .filter(|&s| self.is_user_type(s))
            .collect()
    }

    /// User-declared transitive supertypes.
    pub fn user_ancestors(&self, t: u32) -> Vec<u32> {
        self.index
            .ancestors(t)
            .into_iter()
            .filter(|&s| self.is_user_type(s))
            .collect()
    }

    /// Names (including stubs/externals) of all transitive supertypes.
    pub fn ancestor_names(&self, t: u32) -> Vec<&'c [u8]> {
        self.index
            .ancestors(t)
            .into_iter()
            .map(|a| self.index.name_bytes(a))
            .collect()
    }

    pub fn concrete_descendants(&self, t: u32) -> Vec<u32> {
        self.index.concrete_descendants(t)
    }

    pub fn descendants(&self, t: u32) -> Vec<u32> {
        self.index.descendants(t)
    }

    pub fn is_subtype_of(&self, sub: u32, sup: u32) -> bool {
        self.index.is_subtype_of(sub, sup)
    }

    pub fn related(&self, a: u32, b: u32) -> bool {
        self.index.related(a, b)
    }

    pub fn fields(&self, t: u32) -> &[FieldFact] {
        self.fields.get(&t).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Attributes whose referenced user type satisfies `pred`.
    pub fn fields_referencing(&self, t: u32, pred: impl Fn(u32) -> bool) -> Vec<&FieldFact> {
        self.fields(t)
            .iter()
            .filter(|f| {
                let r = f.referenced_type();
                self.is_user_type(r) && pred(r)
            })
            .collect()
    }

    // ── Member facts ──────────────────────────────────────────────────────────

    pub fn methods(&self, t: u32) -> Vec<u32> {
        self.index.methods(t)
    }

    pub fn constructors(&self, t: u32) -> Vec<u32> {
        self.index.constructors(t)
    }

    pub fn body(&self, m: u32) -> Option<&'c BodyFacts> {
        self.bodies.get(m)
    }

    pub fn has_body(&self, m: u32) -> bool {
        self.bodies.has_body(m)
    }

    pub fn name_id(&self, s: u32) -> u32 {
        self.index.name_id(s)
    }

    pub fn return_type(&self, m: u32) -> u32 {
        self.index.type_of(m)
    }

    pub fn param_types(&self, m: u32) -> Vec<u32> {
        self.index.param_types(m)
    }

    pub fn param_count(&self, m: u32) -> usize {
        self.index
            .symbol(m)
            .map(|s| s.param_count as usize)
            .unwrap_or(0)
    }

    pub fn is_static(&self, s: u32) -> bool {
        self.index.is_static(s)
    }

    pub fn is_final(&self, s: u32) -> bool {
        self.index.is_final(s)
    }

    pub fn is_private(&self, s: u32) -> bool {
        self.index.visibility(s) == SymbolVisibility::Private as u8
    }

    pub fn is_public(&self, s: u32) -> bool {
        self.index.visibility(s) == SymbolVisibility::Public as u8
    }

    /// Abstract operation: declared `abstract`, or a non-static interface method
    /// without a body.
    pub fn is_abstract_method(&self, m: u32) -> bool {
        let sym = match self.index.symbol(m) {
            Some(s) => s,
            None => return false,
        };
        if (sym.modifiers & SymbolModifiers::ABSTRACT) != 0 {
            return true;
        }
        let owner = self.index.enclosing_type(m);
        self.is_interface(owner)
            && (sym.modifiers & (SymbolModifiers::STATIC | SymbolModifiers::DEFAULT)) == 0
            && !self.has_body(m)
    }

    pub fn abstract_methods(&self, t: u32) -> Vec<u32> {
        self.methods(t)
            .into_iter()
            .filter(|&m| self.is_abstract_method(m))
            .collect()
    }

    /// Concrete (bodied) methods of a type.
    pub fn concrete_methods(&self, t: u32) -> Vec<u32> {
        self.methods(t)
            .into_iter()
            .filter(|&m| self.has_body(m))
            .collect()
    }

    /// Method of `base` (or of its user ancestors) that `m` overrides.
    pub fn overridden_in(&self, m: u32, base: u32) -> Option<u32> {
        if let Some(b) = self.index.same_signature_in(m, base) {
            return Some(b);
        }
        self.user_ancestors(base)
            .into_iter()
            .find_map(|a| self.index.same_signature_in(m, a))
    }

    /// Methods of `t` that override an operation declared in `base`'s hierarchy.
    pub fn overrides_of(&self, t: u32, base: u32) -> Vec<u32> {
        self.methods(t)
            .into_iter()
            .filter(|&m| self.overridden_in(m, base).is_some())
            .collect()
    }

    /// True when `root` (or one of its user ancestors) declares an operation named `text`.
    pub fn declares_operation_named(&self, root: u32, text: u32) -> bool {
        let mut types = vec![root];
        types.extend(self.user_ancestors(root));
        types
            .into_iter()
            .any(|t| self.methods(t).into_iter().any(|m| self.name_id(m) == text))
    }

    /// The invoked operation is available on `root`: a resolved callee is declared
    /// in `root`'s inheritance line (a supertype or a subtype), or — when CGA could
    /// not resolve the call — `root` declares or inherits an operation with the
    /// invoked name.
    pub fn targets_hierarchy(&self, method_text: u32, callees: &[u32], root: u32) -> bool {
        if callees.is_empty() {
            return self.declares_operation_named(root, method_text);
        }
        callees.iter().any(|&c| {
            let owner = self.index.enclosing_type(c);
            owner != UNKNOWN_SYM && self.related(owner, root)
        })
    }

    // ── Behavioral facts ──────────────────────────────────────────────────────

    /// Validated calls of a member with loop and receiver context.
    pub fn calls_of(&self, m: u32) -> Vec<CallFact> {
        let body = self.body(m);
        let receivers: HashMap<u32, Receiver> = body
            .map(|b| {
                b.invocations
                    .iter()
                    .map(|inv| (inv.name_token, inv.receiver))
                    .collect()
            })
            .unwrap_or_default();
        self.index
            .calls_of_member(m)
            .into_iter()
            .map(|c| {
                let token = self.index.site(c.site_index).call_token;
                CallFact {
                    callee: c.callee,
                    token,
                    in_loop: body.map(|b| b.in_loop(token)).unwrap_or(false),
                    receiver: receivers.get(&token).copied(),
                }
            })
            .collect()
    }

    /// Invocations on the named receiver, with the callees CGA resolved for them.
    pub fn calls_through(&self, m: u32, receiver_name: u32) -> Vec<ThroughCall> {
        let body = match self.body(m) {
            Some(b) => b,
            None => return Vec::new(),
        };
        let mut by_token: HashMap<u32, Vec<u32>> = HashMap::new();
        for c in self.index.calls_of_member(m) {
            let token = self.index.site(c.site_index).call_token;
            by_token.entry(token).or_default().push(c.callee);
        }
        body.calls_on(receiver_name)
            .map(|inv| ThroughCall {
                method_text: inv.method_text,
                token: inv.name_token,
                callees: by_token.get(&inv.name_token).cloned().unwrap_or_default(),
                in_loop: body.in_loop(inv.name_token),
            })
            .collect()
    }

    /// `m` forwards an invocation of an operation of `root`'s hierarchy through the
    /// attribute `field_name`; `same_name` additionally requires the forwarded
    /// operation to carry `m`'s own name.
    pub fn forwards_through(&self, m: u32, field_name: u32, root: u32, same_name: bool) -> bool {
        let own = self.name_id(m);
        self.calls_through(m, field_name).iter().any(|c| {
            (!same_name || c.method_text == own)
                && self.targets_hierarchy(c.method_text, &c.callees, root)
        })
    }

    /// `m` invokes an operation of `root`'s hierarchy inside a loop that iterates
    /// the attribute `collection_name`.
    pub fn loops_over_and_calls(&self, m: u32, collection_name: u32, root: u32) -> bool {
        let body = match self.body(m) {
            Some(b) => b,
            None => return false,
        };
        let mut by_token: HashMap<u32, Vec<u32>> = HashMap::new();
        for c in self.index.calls_of_member(m) {
            let token = self.index.site(c.site_index).call_token;
            by_token.entry(token).or_default().push(c.callee);
        }
        body.invocations.iter().any(|inv| {
            if !body.in_loop(inv.name_token) {
                return false;
            }
            let callees = by_token.get(&inv.name_token).cloned().unwrap_or_default();
            if !self.targets_hierarchy(inv.method_text, &callees, root) {
                return false;
            }
            body.loops.iter().any(|&(a, b)| {
                a <= inv.name_token
                    && inv.name_token <= b
                    && self.region_mentions(a, inv.name_token, collection_name)
            })
        })
    }

    /// Some token in `[a, b)` spells `text`.
    fn region_mentions(&self, a: u32, b: u32, text: u32) -> bool {
        let scanner = &self.index.scanner;
        let file = match scanner.file_of(a) {
            Some(f) => f,
            None => return false,
        };
        let mut t = a;
        while t < b {
            if scanner.text_id(t) == text {
                return true;
            }
            t = match scanner.next_code(t, file) {
                Some(n) => n,
                None => return false,
            };
        }
        false
    }

    /// User types allocated by a member (`new T`, or validated constructor calls).
    pub fn allocated_types(&self, m: u32) -> Vec<u32> {
        let mut out: BTreeSet<u32> = BTreeSet::new();
        if let Some(b) = self.body(m) {
            for a in &b.allocations {
                let t = self.index.resolve_type_text(a.type_text, m);
                if self.is_user_type(t) {
                    out.insert(t);
                }
            }
        }
        for c in self.index.calls_of_member(m) {
            if self.index.site(c.site_index).call_type != CG_EDGE_CONSTRUCTOR {
                continue;
            }
            let t = if self.index.kind(c.callee) == Some(SymbolKind::SK_CONSTRUCTOR) {
                self.index.enclosing_type(c.callee)
            } else {
                c.callee
            };
            if self.is_user_type(t) {
                out.insert(t);
            }
        }
        out.into_iter().collect()
    }

    /// Attribute `name` is assigned `new T` (T a user type) inside `m`; returns T.
    pub fn assigns_new(&self, m: u32, name: u32) -> Option<u32> {
        let body = self.body(m)?;
        body.assignments.iter().find_map(|a| {
            if a.lhs_text != name || a.via_other_receiver || a.is_declaration {
                return None;
            }
            match a.value {
                AssignedValue::New { type_text, .. } => {
                    let t = self.index.resolve_type_text(type_text, m);
                    if self.is_user_type(t) {
                        Some(t)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        })
    }

    /// Attribute `name` is assigned from a plain name (parameter or variable) inside `m`.
    pub fn assigns_from_name(&self, m: u32, name: u32) -> bool {
        match self.body(m) {
            Some(body) => body.assignments.iter().any(|a| {
                a.lhs_text == name
                    && !a.via_other_receiver
                    && !a.is_declaration
                    && matches!(a.value, AssignedValue::Name { .. })
            }),
            None => false,
        }
    }

    /// The field initializer allocates a user type (`T f = new T()`); returns T.
    pub fn initializer_allocates(&self, field: u32) -> Option<u32> {
        let scanner = &self.index.scanner;
        let sym = self.index.symbol(field)?;
        let (a, b) = scanner.initializer_range(sym)?;
        let file = scanner.file_of(a)?;
        let mut t = a;
        while t <= b {
            if scanner.is(t, b"new") {
                if let Some(n) = scanner.next_code(t, file) {
                    let ty = self.index.resolve_type_text(scanner.text_id(n), field);
                    if self.is_user_type(ty) {
                        return Some(ty);
                    }
                }
            }
            t = scanner.next_code(t, file)?;
        }
        None
    }

    /// `m` calls a method named `method_text` on the receiver `receiver_name`
    /// (library or user callee alike).
    pub fn invokes_on(&self, m: u32, receiver_name: u32, method_names: &[&[u8]]) -> bool {
        let body = match self.body(m) {
            Some(b) => b,
            None => return false,
        };
        let scanner = &self.index.scanner;
        body.calls_on(receiver_name)
            .any(|inv| method_names.contains(&scanner.text(inv.name_token)))
    }
}
