//! C language frontend (ISO C89 through C23, plus the GNU and MSVC extensions that
//! tree-sitter-c parses).
//!
//! * `ingest`    — Phase 1 lexical adapter (token classification, syntax diagnostics);
//! * `reduce`    — Phase 2 CST reduction adapter, and the C-family reduction core that
//!                 the C++ frontend extends;
//! * `semantics` — Phase 3 declaration and type rules;
//! * `stdlib`    — the C standard library model.
//!
//! Files with the `.c` extension are C. A `.h` header is C unless `lang::profile`
//! decides it belongs to C++ (the project contains C++ sources, or the header uses
//! C++-only syntax).

pub mod ingest;
pub mod reduce;
pub mod semantics;
pub mod stdlib;

pub use ingest::CLanguageAdapter;
pub use reduce::{CASTReductionAdapter, CFamilyAttr};

/// Which member of the C family a shared C-family routine is working for. tree-sitter-cpp
/// extends tree-sitter-c, so most node kinds and tokens are common; the dialect decides
/// the few places where the two languages disagree (keywords such as `char16_t`, default
/// member access, `struct` semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// ISO C with GNU / MSVC extensions.
    C,
    /// ISO C++ with GNU / MSVC extensions.
    Cpp,
}
