//! Shared language layer for the C family: C, C++ and C# / .NET.
//!
//! `profile` is the language registry (extensions, `.h` disambiguation, per-phase
//! capabilities, source discovery); `build` reads compilation databases, CMake and MSBuild
//! files; `preprocess` resolves `#include`s and evaluates C/C++ conditional directives;
//! `project_model` merges the build readers into one language-neutral project graph used
//! by the component, package and deployment views. The per-language frontends (`c`,
//! `cpp`, `csharp`) plug into Phases 1–3 through the existing adapter traits.

pub mod profile;
pub mod build;
pub mod preprocess;
pub mod project_model;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod uml_semantics;
pub mod idioms;
