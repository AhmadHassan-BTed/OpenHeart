//! C++ language frontend (C++98 through C++20 as parsed by tree-sitter-cpp, plus the
//! GNU and MSVC extensions it inherits from tree-sitter-c).
//!
//! * `ingest`    — Phase 1 lexical adapter (C++ keywords, alternative operator
//!                 spellings, raw strings, user-defined literals);
//! * `reduce`    — Phase 2 CST reduction adapter for the C++-only kinds; every kind
//!                 shared with C is reduced by `lang::c::reduce`;
//! * `semantics` — Phase 3 declaration, scope and type rules;
//! * `stdlib`    — the C++ standard library model.
//!
//! Extensions: `.cpp .cc .cxx .c++ .hpp .hh .hxx .h++ .ipp .tpp .inl .ixx .cppm`. `.h`
//! headers are assigned by `lang::profile`.

pub mod ingest;
pub mod reduce;
pub mod semantics;
pub mod stdlib;

pub use ingest::CppLanguageAdapter;
pub use reduce::CppASTReductionAdapter;
