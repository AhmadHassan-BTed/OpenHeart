//! Language facts that lowering needs and the neutral AST does not encode.
//!
//! The lowering algorithms are language-agnostic. Wherever the meaning of a construct
//! depends on the source language (how a literal is spelled, which keyword names a
//! primitive type, which methods implement the iteration protocol, in which order field
//! initialisers run, whether nested functions capture variables by reference) the answer
//! comes from the `LoweringSemantics` of the file's language, which the language frontend
//! (`src/lang`) provides.
//!
//! Every method has a default that answers "not known". Lowering never substitutes a guess
//! for a missing answer: it emits the explicit not-derivable form (an opaque intrinsic, an
//! unresolved callee, an unknown type) and records an `IrDiagnostic`.

use crate::core::types::ast::{ASTNodeType, OperatorArity, OperatorId};
use crate::core::types::symbol::SymbolRecord;
use crate::core::types::token::LangId;
use crate::ir::lower::syntax::NodeClass;
use crate::ir::types::IrType;

/// Punctuation spellings used to recover syntactic roles the AST encoding drops (the
/// separators of a `for` header, the parentheses of an argument list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Punctuation {
    pub open_paren: &'static [u8],
    pub close_paren: &'static [u8],
    /// Separator between the clauses of a counted-loop header.
    pub header_separator: &'static [u8],
    /// Tokens that open a nesting level (parentheses, brackets, braces).
    pub nesting_open: &'static [&'static [u8]],
    /// Tokens that close a nesting level, pairwise with `nesting_open`.
    pub nesting_close: &'static [&'static [u8]],
}

/// Lexical category of a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiteralClass {
    Integer,
    Float,
    String,
    Char,
    Boolean,
    Null,
    TextBlock,
    /// Type literal (`T.class`, `typeof(T)`).
    Type,
}

/// Value of a literal as the language defines it.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Bool(bool),
    Int(i128),
    Float(f64),
    /// Code point of a character literal.
    Char(u32),
    /// Decoded contents of a string literal (escapes applied, delimiters removed).
    Str(Vec<u8>),
    Null,
}

/// A literal with its type.
#[derive(Debug, Clone, PartialEq)]
pub struct Literal {
    pub value: LiteralValue,
    pub ty: IrType,
}

/// Step of the iteration protocol an iterator loop desugars to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IterationStep {
    /// Method of the iterable that returns an iterator.
    Begin,
    /// Method of the iterator that tests for a further element.
    HasNext,
    /// Method of the iterator that returns the next element.
    Next,
}

/// How a resource region (try-with-resources, `using`, `with`) releases its resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceProtocol {
    /// Method called on the resource at region exit.
    pub release_method: &'static [u8],
    /// The release is skipped when the resource is null.
    pub null_checked: bool,
}

/// Position of instance field initialisers relative to the superclass constructor call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldInitOrder {
    AfterSuperConstructor,
    BeforeSuperConstructor,
}

/// Object construction rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructionRules {
    /// A constructor without an explicit superclass constructor call invokes the
    /// superclass's no-argument constructor.
    pub implicit_super_call: bool,
    pub field_init: FieldInitOrder,
    /// A constructor that delegates to another constructor of the same type does not run
    /// the instance initialisers itself.
    pub delegating_skips_init: bool,
}

/// Dispatch of an instance method call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchRule {
    /// Selected at run time from the receiver's dynamic type.
    Virtual,
    /// Bound statically to the selected method.
    Direct,
}

/// How a nested function observes the variables of its enclosing function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// The value at closure creation is copied (the language forbids later mutation, or
    /// copies explicitly).
    ByValue,
    /// The closure shares the variable with the enclosing function.
    ByReference,
}

/// Language semantics consumed by lowering. Implemented by every language frontend.
pub trait LoweringSemantics: Send + Sync {
    /// Language described.
    fn language(&self) -> LangId;

    /// Neutral class of an AST node, overriding the default mapping of `ASTNodeType`
    /// (frontends use it for constructs the shared node alphabet lacks: `goto`, `defer`,
    /// `using`, ...).
    fn classify(&self, _node_type: ASTNodeType, _attr: u32) -> Option<NodeClass> {
        None
    }

    fn punctuation(&self) -> Option<Punctuation> {
        None
    }

    /// Operator of an operator token in the given position, for nodes whose attribute word
    /// carries no operator.
    fn operator(&self, _text: &[u8], _arity: OperatorArity) -> Option<OperatorId> {
        None
    }

    /// Built-in type named by a type keyword (`int`, `bool`, `string`, ...).
    fn primitive_type(&self, _text: &[u8]) -> Option<IrType> {
        None
    }

    /// True when the type keyword requests inference from the initializer (`var`, `auto`).
    fn infers_type(&self, _text: &[u8]) -> bool {
        false
    }

    /// Value and type of a literal token.
    fn literal(&self, _class: LiteralClass, _text: &[u8]) -> Option<Literal> {
        None
    }

    /// Method name of an iteration protocol step.
    fn iteration_method(&self, _step: IterationStep) -> Option<&'static [u8]> {
        None
    }

    /// Name of the built-in length member of arrays (`length`, `Length`).
    fn array_length_member(&self) -> Option<&'static [u8]> {
        None
    }

    fn resource_protocol(&self) -> Option<ResourceProtocol> {
        None
    }

    fn construction(&self) -> Option<ConstructionRules> {
        None
    }

    /// Dispatch of a call to the instance method `method`. `None` lowers to virtual
    /// dispatch, which over-approximates a direct binding and is therefore sound.
    fn instance_dispatch(&self, _method: &SymbolRecord) -> Option<DispatchRule> {
        None
    }

    /// Capture discipline of lambdas and local classes. `None` lowers captured variables
    /// to shared cells, which is exact under either discipline.
    fn capture_mode(&self) -> Option<CaptureMode> {
        None
    }

    /// True when a local of a class type (not a pointer or reference) owns a stack object
    /// that is destroyed when its scope exits (C++ RAII).
    fn scope_exit_destruction(&self) -> bool {
        false
    }

    /// True when non-static member types hold a reference to an enclosing instance.
    fn inner_types_capture_outer_instance(&self) -> Option<bool> {
        None
    }
}

/// Semantics of a file whose language has no frontend: every question is unanswered.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSemantics;

impl LoweringSemantics for NoSemantics {
    fn language(&self) -> LangId {
        LangId::Unknown
    }
}

static NO_SEMANTICS: NoSemantics = NoSemantics;

/// Semantics of each language present in the corpus.
#[derive(Default)]
pub struct SemanticsTable<'a> {
    entries: Vec<(LangId, &'a dyn LoweringSemantics)>,
}

impl<'a> SemanticsTable<'a> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Registers (or replaces) the semantics of `lang`.
    pub fn register(&mut self, lang: LangId, semantics: &'a dyn LoweringSemantics) {
        if let Some(entry) = self.entries.iter_mut().find(|(l, _)| *l == lang) {
            entry.1 = semantics;
        } else {
            self.entries.push((lang, semantics));
        }
    }

    pub fn has(&self, lang: LangId) -> bool {
        self.entries.iter().any(|(l, _)| *l == lang)
    }

    /// Semantics of `lang`, or `NoSemantics` when no frontend registered one.
    pub fn get(&self, lang: LangId) -> &'a dyn LoweringSemantics {
        self.entries
            .iter()
            .find(|(l, _)| *l == lang)
            .map(|(_, s)| *s)
            .unwrap_or(&NO_SEMANTICS)
    }
}
