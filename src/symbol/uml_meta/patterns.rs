//! Structural design-pattern predicates over the symbol table artifact.
//!
//! These are the structural halves of the pattern definitions (in the style of PINOT, Shi &
//! Olsson, ASE 2006). They use only the EXTENDS/IMPLEMENTS hierarchy, declared types from the
//! TypeRef table and member modifiers — never identifier spelling — and exclude primitives,
//! library types and externals from every structural test. Behavioural evidence (forwarding
//! calls, loops over a component collection, hook invocation) needs the call graph and is left
//! to the Phase 9 detectors that combine these predicates with it.

use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::symbol::model::{SymbolFlags, TypeRefFlags, WildcardKind};
use crate::symbol::serializer::SymbolTableArtifact;
use std::collections::{HashMap, HashSet, VecDeque};

/// Pre-computed indices shared by all predicates of one artifact.
pub struct StructuralPatternQueries<'s> {
    sta: &'s SymbolTableArtifact,
    supertypes: HashMap<u32, Vec<u32>>,
    members: HashMap<u32, Vec<u32>>,
}

impl<'s> StructuralPatternQueries<'s> {
    pub fn new(sta: &'s SymbolTableArtifact) -> Self {
        let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
        for s in &sta.symbol_records {
            if s.parent_sym != u32::MAX {
                members.entry(s.parent_sym).or_default().push(s.symbol_id);
            }
        }
        Self {
            sta,
            supertypes: sta.supertype_index(),
            members,
        }
    }

    fn kind(&self, sym: u32) -> Option<SymbolKind> {
        self.sta.symbol(sym).map(|s| SymbolKind::from(s.kind))
    }

    fn members_of(&self, sym: u32, kind: SymbolKind) -> impl Iterator<Item = u32> + '_ {
        self.members
            .get(&sym)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(move |&m| self.kind(m) == Some(kind))
    }

    /// Direct supertypes (EXTENDS/IMPLEMENTS only; TH_USES and TH_CREATES are not supertypes).
    pub fn direct_supertypes(&self, sym: u32) -> &[u32] {
        self.supertypes
            .get(&sym)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// All project supertypes of `sym`, transitively.
    pub fn project_supertypes(&self, sym: u32) -> HashSet<u32> {
        let mut out = HashSet::new();
        let mut queue: VecDeque<u32> = self.direct_supertypes(sym).iter().copied().collect();
        while let Some(t) = queue.pop_front() {
            if !out.insert(t) {
                continue;
            }
            queue.extend(self.direct_supertypes(t).iter().copied());
        }
        out.retain(|&t| self.sta.is_project_type(t));
        out
    }

    /// Element type of a collection, map value, optional or array TypeRef, when the element is
    /// a project type.
    fn collection_element(&self, tr: u32) -> Option<u32> {
        let t = self.sta.type_ref(tr)?;
        if t.array_dims > 0 && self.sta.is_project_type(t.head) {
            return Some(t.head);
        }
        if self.sta.is_project_type(t.head) || t.args.is_empty() {
            return None;
        }
        // Library collections carry their element as the last type argument (List<E>,
        // Map<K, V>); wildcards `? extends E` contribute their bound.
        let mut arg = *t.args.last()?;
        if let Some(a) = self.sta.type_ref(arg) {
            if a.wildcard == WildcardKind::Extends as u8 {
                arg = *a.args.first()?;
            }
        }
        let e = self.sta.type_ref(arg)?;
        if e.flags & (TypeRefFlags::PRIMITIVE | TypeRefFlags::TYPE_VARIABLE) == 0
            && self.sta.is_project_type(e.head)
        {
            Some(e.head)
        } else {
            None
        }
    }

    /// Singleton: a class whose constructors are all private, with a static field of its own
    /// type and a static method returning its own type; or an enum with exactly one constant;
    /// or a Kotlin `object` declaration.
    pub fn is_singleton(&self, sym: u32) -> bool {
        let s = match self.sta.symbol(sym) {
            Some(s) => s,
            None => return false,
        };
        match SymbolKind::from(s.kind) {
            SymbolKind::SK_ENUM => self.members_of(sym, SymbolKind::SK_ENUM_CONSTANT).count() == 1,
            SymbolKind::SK_CLASS => {
                let kotlin_object = s.flags & SymbolFlags::LEXICAL != 0
                    && s.modifiers & SymbolModifiers::STATIC != 0
                    && s.modifiers & SymbolModifiers::FINAL != 0
                    && self.kind(s.parent_sym) != Some(SymbolKind::SK_CLASS);
                if kotlin_object {
                    return true;
                }
                let ctors: Vec<u32> = self.members_of(sym, SymbolKind::SK_CONSTRUCTOR).collect();
                // Without an explicit constructor the default constructor is accessible.
                if ctors.is_empty()
                    || ctors.iter().any(|&c| {
                        self.sta.symbol(c).map(|c| c.visibility)
                            != Some(SymbolVisibility::Private as u8)
                    })
                {
                    return false;
                }
                let static_self_field = self.members_of(sym, SymbolKind::SK_FIELD).any(|f| {
                    self.sta.symbol(f).is_some_and(|f| {
                        f.modifiers & SymbolModifiers::STATIC != 0 && f.type_id == sym
                    })
                });
                let static_accessor = self.members_of(sym, SymbolKind::SK_METHOD).any(|m| {
                    self.sta.symbol(m).is_some_and(|m| {
                        m.modifiers & SymbolModifiers::STATIC != 0 && m.type_id == sym
                    })
                });
                static_self_field && static_accessor
            }
            _ => false,
        }
    }

    /// Observer subject: holds a collection of an interface type `I` and declares a method
    /// with a parameter of type `I` (registration).
    pub fn observer_subject_interface(&self, sym: u32) -> Option<u32> {
        for f in self.members_of(sym, SymbolKind::SK_FIELD) {
            let tr = match self.sta.symbol_type_refs.get(f as usize) {
                Some(&t) if t != u32::MAX => t,
                _ => continue,
            };
            let elem = match self.collection_element(tr) {
                Some(e) => e,
                None => continue,
            };
            if self.kind(elem) != Some(SymbolKind::SK_INTERFACE) {
                continue;
            }
            let registers = self.members_of(sym, SymbolKind::SK_METHOD).any(|m| {
                self.members_of(m, SymbolKind::SK_PARAM)
                    .any(|p| self.sta.symbol(p).map(|p| p.type_id) == Some(elem))
            });
            if registers {
                return Some(elem);
            }
        }
        None
    }

    /// Factory Method: an abstract or interface method returning an abstract or interface
    /// project type, overridden (same name and arity) in a subtype.
    pub fn factory_methods(&self, sym: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for m in self.members_of(sym, SymbolKind::SK_METHOD) {
            let rec = match self.sta.symbol(m) {
                Some(r) => r,
                None => continue,
            };
            let ret = rec.type_id;
            let abstract_product = self.sta.is_project_type(ret)
                && self.sta.symbol(ret).is_some_and(|r| {
                    SymbolKind::from(r.kind) == SymbolKind::SK_INTERFACE
                        || r.modifiers & SymbolModifiers::ABSTRACT != 0
                });
            if !abstract_product || rec.modifiers & SymbolModifiers::ABSTRACT == 0 {
                continue;
            }
            let overridden = self.supertypes.iter().any(|(&sub, sups)| {
                sups.contains(&sym)
                    && self.members_of(sub, SymbolKind::SK_METHOD).any(|o| {
                        self.sta.symbol(o).is_some_and(|o| {
                            o.name_id == rec.name_id && o.param_count == rec.param_count
                        })
                    })
            });
            if overridden {
                out.push(m);
            }
        }
        out
    }

    /// Template Method (structural half): an abstract class with at least one concrete method
    /// and at least one abstract method (the hooks).
    pub fn is_template_method_candidate(&self, sym: u32) -> bool {
        let s = match self.sta.symbol(sym) {
            Some(s) if SymbolKind::from(s.kind) == SymbolKind::SK_CLASS => s,
            _ => return false,
        };
        if s.modifiers & SymbolModifiers::ABSTRACT == 0 {
            return false;
        }
        let (mut concrete, mut hooks) = (false, false);
        for m in self.members_of(sym, SymbolKind::SK_METHOD) {
            match self.sta.symbol(m) {
                Some(r) if r.modifiers & SymbolModifiers::ABSTRACT != 0 => hooks = true,
                Some(r) if r.flags & SymbolFlags::IMPLICIT == 0 => concrete = true,
                _ => {}
            }
        }
        concrete && hooks
    }

    /// Decorator (structural half): the type has a project supertype `S` and an instance field
    /// whose declared type is `S` (the wrapped component). Forwarding calls must be checked
    /// against the call graph.
    pub fn decorated_component(&self, sym: u32) -> Option<u32> {
        let sups = self.project_supertypes(sym);
        self.members_of(sym, SymbolKind::SK_FIELD).find_map(|f| {
            let f = self.sta.symbol(f)?;
            (f.modifiers & SymbolModifiers::STATIC == 0
                && f.flags & SymbolFlags::ARRAY == 0
                && sups.contains(&f.type_id))
            .then_some(f.type_id)
        })
    }

    /// Composite (structural half): the type has a project supertype `S` and an instance field
    /// holding a collection or array of `S`.
    pub fn composite_component(&self, sym: u32) -> Option<u32> {
        let sups = self.project_supertypes(sym);
        self.members_of(sym, SymbolKind::SK_FIELD).find_map(|f| {
            let rec = self.sta.symbol(f)?;
            if rec.modifiers & SymbolModifiers::STATIC != 0 {
                return None;
            }
            let tr = *self.sta.symbol_type_refs.get(f as usize)?;
            let elem = self.collection_element(tr)?;
            sups.contains(&elem).then_some(elem)
        })
    }
}
