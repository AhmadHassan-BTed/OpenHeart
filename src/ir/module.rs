//! Whole-program IR: functions, fields, module variables, allocation and call sites,
//! the string table, and the explicit record of everything lowering could not derive.

use std::collections::HashMap;

use crate::ingestion::interner::StringInterner;
use crate::ir::func::FuncIr;
use crate::ir::inst::{Inst, IntrinsicKind, Region};
use crate::ir::types::{
    AllocSiteId, BlockId, CallSiteId, FieldId, FuncId, GlobalId, Span, TypeId, TypeTable,
    ValueId,
};

/// Module-level variable (a variable declared outside any type: C globals, Kotlin
/// top-level properties, script variables).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalIr {
    pub id: GlobalId,
    pub symbol: Option<u32>,
    pub name: u32,
    pub ty: TypeId,
    pub span: Span,
}

/// Field of a type. Fields the program accesses but no declaration binds are kept with
/// `symbol: None` (and `owner_symbol == u32::MAX` when the owner is unknown too), so every
/// access still names a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldIr {
    pub id: FieldId,
    /// Declaring type symbol, or `u32::MAX` when not derivable.
    pub owner_symbol: u32,
    pub name: u32,
    pub ty: TypeId,
    /// Field symbol; `None` for unresolved fields and for fields synthesized by lowering
    /// (closure captures stored in a local or anonymous class instance).
    pub symbol: Option<u32>,
    pub is_static: bool,
    /// The field was synthesized by lowering.
    pub synthetic: bool,
}

/// Allocation site: one `Inst::Alloc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocSiteIr {
    pub id: AllocSiteId,
    pub func: FuncId,
    pub ty: TypeId,
    pub span: Span,
    pub region: Region,
}

/// Call site: one `Inst::Call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteIr {
    pub id: CallSiteId,
    pub func: FuncId,
    pub block: BlockId,
    pub span: Span,
    /// Receiver of an instance call (`args[0]` of the call), for every callee kind.
    pub receiver: Option<ValueId>,
    /// Method symbols that remained applicable when overload resolution could not select
    /// one (the callee is then `Unresolved`); empty otherwise.
    pub candidates: Vec<u32>,
}

/// Category of a lowering diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticKind {
    /// The fact cannot be derived from the program text, the symbol table and the language
    /// frontend; the IR contains an explicit opaque or unresolved construct instead.
    NotDerivable,
    /// A name or call matched more than one declaration.
    Ambiguous,
    /// A name or type matched no declaration.
    Unresolved,
    /// The IR over-approximates the program (more behaviours than the source has).
    OverApproximate,
    /// The IR omits behaviours the program has (stated so consumers do not read absence of
    /// a path as a proof).
    UnderApproximate,
}

impl DiagnosticKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            DiagnosticKind::NotDerivable => "not-derivable",
            DiagnosticKind::Ambiguous => "ambiguous",
            DiagnosticKind::Unresolved => "unresolved",
            DiagnosticKind::OverApproximate => "over-approximate",
            DiagnosticKind::UnderApproximate => "under-approximate",
        }
    }
}

/// One explicit statement about a limit of the lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrDiagnostic {
    pub kind: DiagnosticKind,
    pub func: Option<FuncId>,
    pub span: Span,
    pub message: String,
}

/// The lowered program.
#[derive(Debug, Clone)]
pub struct ModuleIr {
    pub types: TypeTable,
    pub funcs: Vec<FuncIr>,
    pub globals: Vec<GlobalIr>,
    pub fields: Vec<FieldIr>,
    pub alloc_sites: Vec<AllocSiteIr>,
    pub call_sites: Vec<CallSiteIr>,
    /// String table of every interned name and string constant in the IR. It extends the
    /// token corpus interner, so a corpus text id and the IR id of the same text coincide.
    pub strings: StringInterner,
    /// Interned simple name of every symbol of the symbol table, indexed by symbol id
    /// (`u32::MAX` when the symbol has no name).
    pub symbol_names: Vec<u32>,
    pub diagnostics: Vec<IrDiagnostic>,
}

impl ModuleIr {
    pub fn new(strings: StringInterner) -> Self {
        Self {
            types: TypeTable::new(),
            funcs: Vec::new(),
            globals: Vec::new(),
            fields: Vec::new(),
            alloc_sites: Vec::new(),
            call_sites: Vec::new(),
            strings,
            symbol_names: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[inline]
    pub fn func(&self, id: FuncId) -> &FuncIr {
        &self.funcs[id.index()]
    }

    #[inline]
    pub fn func_mut(&mut self, id: FuncId) -> &mut FuncIr {
        &mut self.funcs[id.index()]
    }

    #[inline]
    pub fn field(&self, id: FieldId) -> &FieldIr {
        &self.fields[id.index()]
    }

    #[inline]
    pub fn global(&self, id: GlobalId) -> &GlobalIr {
        &self.globals[id.index()]
    }

    #[inline]
    pub fn alloc_site(&self, id: AllocSiteId) -> &AllocSiteIr {
        &self.alloc_sites[id.index()]
    }

    #[inline]
    pub fn call_site(&self, id: CallSiteId) -> &CallSiteIr {
        &self.call_sites[id.index()]
    }

    /// Text of an interned id.
    pub fn text(&self, id: u32) -> &[u8] {
        self.strings.lookup_text(id)
    }

    /// Text of an interned id as UTF-8 (lossy).
    pub fn text_lossy(&self, id: u32) -> String {
        String::from_utf8_lossy(self.strings.lookup_text(id)).into_owned()
    }

    /// Interned id of `text`, adding it when new.
    pub fn intern(&mut self, text: &[u8]) -> u32 {
        self.strings.intern(text)
    }

    /// Simple name of a symbol.
    pub fn symbol_name(&self, symbol: u32) -> Option<&[u8]> {
        match self.symbol_names.get(symbol as usize) {
            Some(&id) if id != u32::MAX => Some(self.strings.lookup_text(id)),
            _ => None,
        }
    }

    /// Intrinsic denoted by an interned intrinsic name.
    pub fn intrinsic_kind(&self, name: u32) -> Option<IntrinsicKind> {
        IntrinsicKind::from_name(self.strings.lookup_text(name))
    }

    /// Function lowered from each declaring symbol.
    pub fn function_index(&self) -> HashMap<u32, FuncId> {
        self.funcs
            .iter()
            .filter_map(|f| f.symbol.map(|s| (s, f.id)))
            .collect()
    }

    /// Field declared by each field symbol.
    pub fn field_index(&self) -> HashMap<u32, FieldId> {
        self.fields
            .iter()
            .filter_map(|f| f.symbol.map(|s| (s, f.id)))
            .collect()
    }

    /// The call instruction of a call site with its position.
    pub fn call_inst(&self, site: CallSiteId) -> Option<(&FuncIr, BlockId, usize, &Inst)> {
        let cs = self.call_sites.get(site.index())?;
        let func = self.funcs.get(cs.func.index())?;
        let block = func.blocks.get(cs.block.index())?;
        let index = block
            .insts
            .iter()
            .position(|i| matches!(i, Inst::Call { site: s, .. } if *s == site))?;
        Some((func, cs.block, index, &block.insts[index]))
    }

    /// Call sites of each function, in site order.
    pub fn call_sites_by_func(&self) -> Vec<Vec<CallSiteId>> {
        let mut out = vec![Vec::new(); self.funcs.len()];
        for cs in &self.call_sites {
            if let Some(list) = out.get_mut(cs.func.index()) {
                list.push(cs.id);
            }
        }
        out
    }

    /// Allocation sites of each function, in site order.
    pub fn alloc_sites_by_func(&self) -> Vec<Vec<AllocSiteId>> {
        let mut out = vec![Vec::new(); self.funcs.len()];
        for a in &self.alloc_sites {
            if let Some(list) = out.get_mut(a.func.index()) {
                list.push(a.id);
            }
        }
        out
    }

    /// Diagnostics attached to one function.
    pub fn diagnostics_of(&self, func: FuncId) -> impl Iterator<Item = &IrDiagnostic> {
        self.diagnostics
            .iter()
            .filter(move |d| d.func == Some(func))
    }
}
