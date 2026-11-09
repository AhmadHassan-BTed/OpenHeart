//! Application binary interface models: how a resolved, typed program occupies memory, how
//! its objects are born and die, and how values cross call boundaries.
//!
//! Everything here is computed, never assumed. A frontend in `src/lang` describes each
//! aggregate in the neutral vocabulary of [`layout`] (fields, bases, virtual methods, special
//! members, layout attributes) and names the object model that governs it. The models then
//! derive the facts a compiler back end would:
//! - [`target`] — pointer width, endianness, scalar sizes and alignments, bitfield and
//!   tail-padding rules of concrete target triples.
//! - [`layout`] — the language-neutral layout result and the engine that dispatches every
//!   aggregate to its object model, caching instantiations.
//! - [`cxx`] — C and C++ record layout per the Itanium C++ ABI and the Microsoft ABI,
//!   including bitfields, empty bases, virtual inheritance, vtables, vbtables and member
//!   pointers.
//! - [`jvm`] — the JVM object model of the reference VM (HotSpot field layout).
//! - [`clr`] — the .NET CLR object model (CoreCLR method tables, value types, boxing,
//!   StructLayout, strings and arrays).
//! - [`lifetime`] — construction and destruction order, special-member semantics, ownership,
//!   finalization, disposal and allocation-site lifetimes over the IR.
//! - [`calling`] — receiver passing, by-value versus by-reference transfer, hidden return
//!   buffers and per-architecture register classification.
//!
//! When a fact cannot be derived (an incomplete type, an unresolved symbol, an unsupported
//! construct) the result says so explicitly through its derivation tag instead of guessing.

pub mod calling;
pub mod clr;
pub mod cxx;
pub mod jvm;
pub mod layout;
pub mod lifetime;
pub mod target;

// The IR vocabulary every ABI model codes against. Importing it in one place keeps the
// models independent of how `src/ir` arranges its submodules.
pub(crate) use crate::ir::{
    AllocSiteId, BlockId, FieldId, FuncId, Inst, IrType, ModuleIr, Place, RefKind, Region, Span,
    TypeId, ValueId,
};

pub use calling::{CallSignature, CallingModel, ParamDecl, ParamMode, SignatureDecl};
pub use layout::{
    AbiProgram, AbiScalar, AbiType, AggregateDecl, AggregateKind, BaseDecl, FieldDecl,
    IrTypeFacts, LayoutDerivation, LayoutEngine, LayoutTable, ObjectModel, RecordLayout,
    Storage, VirtualMethodDecl,
};
pub use lifetime::{LifetimeFacts, LifetimeModel, OwnershipKind};
pub use target::{Arch, CScalar, CxxAbi, DataModel, Endianness, TargetDesc};
