//! C# / .NET frontend: tree-sitter-c-sharp (0.23) adapters for Phases 1–3.
//!
//! `ingest` classifies every CST leaf into the token alphabet (Phase 1), `reduce` maps the
//! concrete syntax tree onto the language-neutral BP AST node set and packs the C# modifier,
//! accessibility and construct flags into the `NodeAttr` word (Phase 2), `semantics`
//! implements the C# declaration, scoping and member-lookup rules (Phase 3), and `stdlib`
//! is the model of the .NET base class library that Phase 3 binds library references
//! against.
//!
//! Files with the `.cs` (C# source) and `.csx` (C# script) extensions are C#.

pub mod ingest;
pub mod reduce;
pub mod semantics;
pub mod stdlib;

pub use ingest::CSharpLanguageAdapter;
pub use reduce::{CSharpASTReductionAdapter, CSharpAttr, CSharpModifiers, CSharpReductionStats};
