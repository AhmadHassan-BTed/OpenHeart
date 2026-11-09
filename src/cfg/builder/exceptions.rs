//! Exception-type reasoning for handler selection (JLS 11.2, 14.20.1).
//!
//! Types are identified by their simple name. Project classes are walked up through the
//! symbol table's `TH_EXTENDS` edges; JDK exception classes that commonly appear in
//! `catch` clauses are walked through a built-in table of `java.lang` / `java.io`
//! superclasses. A chain that leaves both (an unknown library type) is *incomplete*, and
//! an incomplete chain can never prove that a handler does not match.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{children, first_child_of_type, NewParts};
use crate::cfg::source::SourceText;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{SymbolKind, THRelation};
use crate::symbol::SymbolTableArtifact;
use std::collections::{HashMap, HashSet};

/// Statically known information about a thrown exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThrownType {
    /// Any exception may be raised (implicit exceptions of calls, casts, array accesses,
    /// `throw` of a variable, close() of a resource, ...).
    Unknown,
    /// `throw new T(...)`: the runtime class is exactly the named type (text id).
    Exact(u32),
}

/// Type named by a catch clause (one alternative of a multi-catch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchType {
    /// Simple-name text id of the caught type.
    Named(u32),
    /// The type is not present in the AST or no token text is available.
    Unknown,
}

/// Outcome of matching a thrown exception against a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchMatch {
    /// The handler catches every exception of this kind; propagation stops.
    Definite,
    /// The handler may catch it; propagation continues to outer handlers as well.
    Possible,
    /// The handler provably cannot catch it.
    Never,
}

/// Superclass of well-known JDK throwables, by simple name.
fn jdk_superclass(name: &[u8]) -> Option<&'static [u8]> {
    let parent: &'static [u8] = match name {
        b"Exception" | b"Error" => b"Throwable",
        b"RuntimeException"
        | b"IOException"
        | b"InterruptedException"
        | b"ReflectiveOperationException"
        | b"CloneNotSupportedException"
        | b"TimeoutException"
        | b"ExecutionException"
        | b"GeneralSecurityException"
        | b"SQLException"
        | b"URISyntaxException"
        | b"ParseException" => b"Exception",
        b"ClassNotFoundException"
        | b"NoSuchMethodException"
        | b"NoSuchFieldException"
        | b"InstantiationException"
        | b"IllegalAccessException"
        | b"InvocationTargetException" => b"ReflectiveOperationException",
        b"FileNotFoundException"
        | b"EOFException"
        | b"UnsupportedEncodingException"
        | b"MalformedURLException"
        | b"SocketException"
        | b"UnknownHostException"
        | b"InterruptedIOException"
        | b"CharacterCodingException" => b"IOException",
        b"IllegalArgumentException"
        | b"IllegalStateException"
        | b"NullPointerException"
        | b"ArithmeticException"
        | b"ClassCastException"
        | b"IndexOutOfBoundsException"
        | b"UnsupportedOperationException"
        | b"ArrayStoreException"
        | b"NegativeArraySizeException"
        | b"ConcurrentModificationException"
        | b"NoSuchElementException"
        | b"SecurityException"
        | b"UncheckedIOException"
        | b"DateTimeException"
        | b"EmptyStackException"
        | b"MissingResourceException" => b"RuntimeException",
        b"NumberFormatException" | b"PatternSyntaxException" => b"IllegalArgumentException",
        b"InputMismatchException" => b"NoSuchElementException",
        b"ArrayIndexOutOfBoundsException" | b"StringIndexOutOfBoundsException" => {
            b"IndexOutOfBoundsException"
        }
        b"AssertionError"
        | b"LinkageError"
        | b"VirtualMachineError"
        | b"ThreadDeath" => b"Error",
        b"OutOfMemoryError" | b"StackOverflowError" | b"InternalError" => {
            b"VirtualMachineError"
        }
        b"NoClassDefFoundError" | b"ExceptionInInitializerError" => b"LinkageError",
        _ => return None,
    };
    Some(parent)
}

/// Hierarchy oracle shared (read-only) by every function built in one Phase 4 run.
pub struct ExceptionTypes<'a> {
    src: Option<SourceText<'a>>,
    /// Simple-name text id -> text ids of the direct superclasses of project types with
    /// that name (several entries when the name is declared more than once).
    project_supers: HashMap<u32, Vec<u32>>,
    /// Names of project (non-external) types.
    project_types: HashSet<u32>,
}

impl<'a> ExceptionTypes<'a> {
    pub fn new(sta: &SymbolTableArtifact, src: Option<SourceText<'a>>) -> Self {
        let mut project_supers: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut project_types: HashSet<u32> = HashSet::new();

        let is_class_like = |k: u8| {
            k == SymbolKind::SK_CLASS as u8
                || k == SymbolKind::SK_INTERFACE as u8
                || k == SymbolKind::SK_ENUM as u8
                || k == SymbolKind::SK_RECORD as u8
        };

        for sym in &sta.symbol_records {
            if is_class_like(sym.kind) && sym.name_id != u32::MAX {
                project_types.insert(sym.name_id);
            }
        }
        for edge in &sta.th_edges {
            if edge.relation != THRelation::TH_EXTENDS {
                continue;
            }
            let (from, to) = match (sta.symbol(edge.from_sym), sta.symbol(edge.to_sym)) {
                (Some(f), Some(t)) => (f, t),
                _ => continue,
            };
            if !is_class_like(from.kind) || from.name_id == u32::MAX || to.name_id == u32::MAX {
                continue;
            }
            let list = project_supers.entry(from.name_id).or_default();
            if !list.contains(&to.name_id) {
                list.push(to.name_id);
            }
        }

        Self {
            src,
            project_supers,
            project_types,
        }
    }

    /// Types caught by a catch clause. A clause whose types are absent from the AST
    /// yields a single `CatchType::Unknown`.
    pub fn catch_types(&self, bpa: &BPASTArtifact, catch_node: u32) -> Vec<CatchType> {
        let mut out = Vec::new();
        let scan = |container: u32, out: &mut Vec<CatchType>| {
            for c in children(bpa, container) {
                if bpa.node_type(c) == ASTNodeType::NN_TYPE_REF {
                    let ty = self
                        .src
                        .as_ref()
                        .and_then(|s| s.node_text_id(bpa, c))
                        .map(CatchType::Named)
                        .unwrap_or(CatchType::Unknown);
                    out.push(ty);
                }
            }
        };
        scan(catch_node, &mut out);
        if let Some(param) = first_child_of_type(bpa, catch_node, ASTNodeType::NN_PARAM_DECL) {
            scan(param, &mut out);
        }
        if out.is_empty() {
            out.push(CatchType::Unknown);
        }
        out
    }

    /// Statically known type of the exception raised by a `throw` statement.
    pub fn thrown_type(&self, bpa: &BPASTArtifact, throw_node: u32) -> ThrownType {
        let expr = match bpa.first_child(throw_node) {
            Some(e) => e,
            None => return ThrownType::Unknown,
        };
        if bpa.node_type(expr) != ASTNodeType::NN_NEW_EXPR {
            return ThrownType::Unknown;
        }
        let parts = NewParts::of(bpa, self.src.as_ref(), expr);
        if !parts.anonymous_members.is_empty() {
            // `throw new E() { ... }` creates an anonymous subclass of E.
            return ThrownType::Unknown;
        }
        match parts.type_name {
            Some(t) => ThrownType::Exact(t),
            None => ThrownType::Unknown,
        }
    }

    /// Type named by text, if token text is available.
    pub fn named(&self, text: &[u8]) -> ThrownType {
        match self.src.as_ref().and_then(|s| s.find_text_id(text)) {
            Some(id) => ThrownType::Exact(id),
            None => ThrownType::Unknown,
        }
    }

    fn name_bytes(&self, id: u32) -> Option<&'a [u8]> {
        self.src.as_ref().map(|s| s.text_of_id(id))
    }

    /// Ancestor names of `start` (inclusive) and whether the chain is complete, i.e.
    /// every branch ended at `Throwable` or `Object`.
    fn ancestors(&self, start: &[u8]) -> (Vec<Vec<u8>>, bool) {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut complete = true;
        let mut work: Vec<Vec<u8>> = vec![start.to_vec()];
        while let Some(name) = work.pop() {
            if out.contains(&name) {
                continue;
            }
            out.push(name.clone());
            if name.as_slice() == b"Throwable" || name.as_slice() == b"Object" {
                continue;
            }
            let src = match self.src.as_ref() {
                Some(s) => s,
                None => {
                    complete = false;
                    continue;
                }
            };
            let id = src.find_text_id(&name);
            let project_parents = id.and_then(|i| self.project_supers.get(&i));
            match project_parents {
                Some(parents) if !parents.is_empty() => {
                    for &p in parents {
                        work.push(src.text_of_id(p).to_vec());
                    }
                }
                _ => {
                    let is_project = id.is_some_and(|i| self.project_types.contains(&i));
                    if let Some(parent) = jdk_superclass(&name) {
                        work.push(parent.to_vec());
                    } else if is_project {
                        // A project class without `extends` is a direct Object subclass.
                        work.push(b"Object".to_vec());
                    } else {
                        complete = false;
                    }
                }
            }
        }
        (out, complete)
    }

    /// Matches a thrown exception against one handler type.
    pub fn relation(&self, thrown: ThrownType, catch: CatchType) -> CatchMatch {
        let catch_name = match catch {
            CatchType::Unknown => return CatchMatch::Possible,
            CatchType::Named(id) => match self.name_bytes(id) {
                Some(n) => n,
                None => return CatchMatch::Possible,
            },
        };
        if catch_name == b"Throwable" {
            return CatchMatch::Definite;
        }
        let thrown_name = match thrown {
            ThrownType::Unknown => return CatchMatch::Possible,
            ThrownType::Exact(id) => match self.name_bytes(id) {
                Some(n) => n,
                None => return CatchMatch::Possible,
            },
        };
        let (chain, complete) = self.ancestors(thrown_name);
        if chain.iter().any(|a| a.as_slice() == catch_name) {
            CatchMatch::Definite
        } else if complete {
            CatchMatch::Never
        } else {
            CatchMatch::Possible
        }
    }
}
