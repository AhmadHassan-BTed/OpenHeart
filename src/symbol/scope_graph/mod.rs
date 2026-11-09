pub mod resolver;

pub use resolver::*;

use crate::core::types::symbol::{ScopeKind, ScopeRecord};
use std::collections::{BTreeMap, HashMap};

/// The single global root scope. Package scopes are its children; every compilation unit
/// scope is a child of the scope of its package.
pub const ROOT_SCOPE: u32 = 0;

/// Bits of `ScopeRecord::flags`.
pub struct ScopeFlags;

impl ScopeFlags {
    /// The compilation unit has at least one on-demand import.
    pub const WILDCARD_IMPORT: u8 = 1 << 0;
    /// Scope of a package, shared by all compilation units of that package.
    pub const PACKAGE: u8 = 1 << 1;
    /// Scope of one compilation unit (file); it owns the file's import table.
    pub const COMPILATION_UNIT: u8 = 1 << 2;
    /// Declarations are visible only from their declaration onwards (blocks, JLS 6.3).
    pub const ORDERED: u8 = 1 << 3;
    /// The global root scope.
    pub const ROOT: u8 = 1 << 4;
}

/// Java name spaces (JLS 6.5): lookups are kind-directed, so a type context never sees a
/// variable, method or constructor of the same name.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Namespace {
    Type = 0,
    Variable = 1,
    Method = 2,
    Package = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImportKind {
    /// `import a.b.C;`
    SingleType = 0,
    /// `import a.b.*;`
    OnDemand = 1,
    /// `import static a.b.C.m;`
    SingleStatic = 2,
    /// `import static a.b.C.*;`
    StaticOnDemand = 3,
}

impl ImportKind {
    pub fn from_u8(val: u8) -> Option<Self> {
        Some(match val {
            0 => Self::SingleType,
            1 => Self::OnDemand,
            2 => Self::SingleStatic,
            3 => Self::StaticOnDemand,
            _ => return None,
        })
    }
}

/// One import declaration of a compilation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub kind: ImportKind,
    /// Dotted path without the trailing `.*`: `java.util.List`, `java.util`, `java.lang.Math.max`.
    pub path: String,
    /// Simple name the import makes visible (last segment or Kotlin alias); empty for
    /// on-demand imports.
    pub visible_name: String,
    pub first_token: u32,
    /// Token of the last path segment.
    pub name_token: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclEntry {
    pub symbol: u32,
    /// Token from which the declaration is visible in an ORDERED scope.
    pub order_token: u32,
}

#[derive(Debug, Clone)]
pub struct ScopeGraph {
    pub scopes: Vec<ScopeRecord>,
    declarations: Vec<Vec<u32>>,
    name_index: HashMap<(u32, Namespace, u32), Vec<DeclEntry>>,
    imports: BTreeMap<u32, Vec<ImportDecl>>,
}

impl Default for ScopeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeGraph {
    pub fn new() -> Self {
        let mut sg = Self {
            scopes: Vec::new(),
            declarations: Vec::new(),
            name_index: HashMap::new(),
            imports: BTreeMap::new(),
        };
        let root = sg.create_scope(u32::MAX, u32::MAX, ScopeKind::File);
        sg.add_flags(root, ScopeFlags::ROOT);
        sg
    }

    pub fn create_scope(&mut self, owner_symbol: u32, parent_scope: u32, kind: ScopeKind) -> u32 {
        let scope_id = self.scopes.len() as u32;
        let parent_scope = if (parent_scope as usize) < self.scopes.len() {
            parent_scope
        } else {
            u32::MAX
        };
        self.scopes.push(ScopeRecord {
            scope_id,
            parent_scope,
            owner_symbol,
            first_decl: u32::MAX,
            decl_count: 0,
            import_count: 0,
            scope_kind: kind as u8,
            flags: 0,
            import_table_off: u32::MAX,
            _reserved: 0,
        });
        self.declarations.push(Vec::new());
        scope_id
    }

    pub fn add_flags(&mut self, scope_id: u32, flags: u8) {
        if let Some(s) = self.scopes.get_mut(scope_id as usize) {
            s.flags |= flags;
        }
    }

    pub fn flags(&self, scope_id: u32) -> u8 {
        self.scopes
            .get(scope_id as usize)
            .map(|s| s.flags)
            .unwrap_or(0)
    }

    pub fn has_flag(&self, scope_id: u32, flag: u8) -> bool {
        self.flags(scope_id) & flag != 0
    }

    pub fn parent_of(&self, scope_id: u32) -> u32 {
        self.scopes
            .get(scope_id as usize)
            .map(|s| s.parent_scope)
            .unwrap_or(u32::MAX)
    }

    pub fn owner_of(&self, scope_id: u32) -> u32 {
        self.scopes
            .get(scope_id as usize)
            .map(|s| s.owner_symbol)
            .unwrap_or(u32::MAX)
    }

    pub fn kind_of(&self, scope_id: u32) -> Option<ScopeKind> {
        self.scopes
            .get(scope_id as usize)
            .map(|s| ScopeKind::from(s.scope_kind))
    }

    /// Records `symbol_id` as declared in `scope_id` (containment only, no name binding).
    pub fn add_declaration(&mut self, scope_id: u32, symbol_id: u32) {
        if let Some(scope) = self.scopes.get_mut(scope_id as usize) {
            if scope.first_decl == u32::MAX {
                scope.first_decl = symbol_id;
            }
            scope.decl_count = scope.decl_count.saturating_add(1);
            self.declarations[scope_id as usize].push(symbol_id);
        }
    }

    /// Makes `symbol_id` visible under `name_id` in namespace `ns` of `scope_id`.
    pub fn bind_name(
        &mut self,
        scope_id: u32,
        ns: Namespace,
        name_id: u32,
        symbol_id: u32,
        order_token: u32,
    ) {
        if name_id == u32::MAX || (scope_id as usize) >= self.scopes.len() {
            return;
        }
        self.name_index
            .entry((scope_id, ns, name_id))
            .or_default()
            .push(DeclEntry {
                symbol: symbol_id,
                order_token,
            });
    }

    /// Declarations bound under `name_id` in `ns` of `scope_id` that are visible from
    /// `at_token` (declaration order is honoured in ORDERED scopes only).
    pub fn lookup_in_scope(
        &self,
        scope_id: u32,
        ns: Namespace,
        name_id: u32,
        at_token: u32,
    ) -> impl Iterator<Item = u32> + '_ {
        let ordered = self.has_flag(scope_id, ScopeFlags::ORDERED);
        self.name_index
            .get(&(scope_id, ns, name_id))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter(move |e| !ordered || at_token == u32::MAX || e.order_token <= at_token)
            .map(|e| e.symbol)
    }

    pub fn declarations(&self, scope_id: u32) -> &[u32] {
        self.declarations
            .get(scope_id as usize)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn add_import(&mut self, scope_id: u32, import: ImportDecl) {
        if (scope_id as usize) >= self.scopes.len() {
            return;
        }
        if matches!(
            import.kind,
            ImportKind::OnDemand | ImportKind::StaticOnDemand
        ) {
            self.add_flags(scope_id, ScopeFlags::WILDCARD_IMPORT);
        }
        let scope = &mut self.scopes[scope_id as usize];
        scope.import_count = scope.import_count.saturating_add(1);
        self.imports.entry(scope_id).or_default().push(import);
    }

    pub fn imports_of(&self, scope_id: u32) -> &[ImportDecl] {
        self.imports
            .get(&scope_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// All import tables, ordered by scope id (deterministic).
    pub fn import_tables(&self) -> &BTreeMap<u32, Vec<ImportDecl>> {
        &self.imports
    }

    /// Assigns `import_table_off` of every scope: the index of its first entry in the flat
    /// import table obtained by concatenating `import_tables()` in scope order.
    pub fn assign_import_offsets(&mut self) {
        let mut offset = 0u32;
        for scope in &mut self.scopes {
            scope.import_table_off = u32::MAX;
        }
        for (&scope_id, list) in &self.imports {
            if let Some(scope) = self.scopes.get_mut(scope_id as usize) {
                scope.import_table_off = offset;
            }
            offset = offset.saturating_add(list.len() as u32);
        }
    }

    /// Nearest enclosing compilation unit scope of `scope_id` (inclusive).
    pub fn compilation_unit_of(&self, scope_id: u32) -> Option<u32> {
        self.ancestors(scope_id)
            .find(|&s| self.has_flag(s, ScopeFlags::COMPILATION_UNIT))
    }

    /// `scope_id` and its ancestors, innermost first. Stops on a corrupt parent link.
    pub fn ancestors(&self, scope_id: u32) -> impl Iterator<Item = u32> + '_ {
        let limit = self.scopes.len();
        let mut cur = scope_id;
        let mut steps = 0usize;
        std::iter::from_fn(move || {
            if cur == u32::MAX || (cur as usize) >= limit || steps > limit {
                return None;
            }
            steps += 1;
            let out = cur;
            cur = self.scopes[cur as usize].parent_scope;
            Some(out)
        })
    }

    pub fn get_scope(&self, scope_id: u32) -> Option<&ScopeRecord> {
        self.scopes.get(scope_id as usize)
    }

    pub fn scope_count(&self) -> usize {
        self.scopes.len()
    }
}
