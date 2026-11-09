//! Neutral view of the BP AST for lowering: a child CSR over the preorder parent map,
//! token access, the neutral node classes, and the recovery of syntactic roles (condition,
//! body, receiver, initializer, ...) from node classes, attribute flags, child order and,
//! where the encoding drops a role, token positions.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorId};
use crate::core::types::token::{unpack_sort_key, TokenType};
use crate::ingestion::serializer::TokenCorpusArtifact;
use crate::ir::lower::semantics::{LiteralClass, LoweringSemantics, Punctuation};
use crate::ir::types::Span;

/// Language-neutral class of an AST node: what lowering does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeClass {
    // Declarations
    Module,
    TypeDecl,
    Function,
    Constructor,
    Initializer { is_static: bool },
    Field,
    Param,
    /// Local variable declaration: a wrapper of declarators, or a single declarator.
    LocalDecl,
    Declarator,
    EnumConstant,
    AnonBody,
    /// Package, import and module directives: no executable content.
    Directive,
    TypeParams,
    Annotation,
    /// Throws and permits clauses.
    TypeList,
    Resources,
    // Statements
    Block,
    If,
    Else,
    While,
    DoWhile,
    For,
    ForEach,
    Switch,
    Case,
    Try,
    Catch,
    Finally,
    Return,
    Throw,
    Break,
    Continue,
    ExprStmt,
    Assert,
    Labeled,
    /// Region guarded by a lock (`synchronized`, `lock`).
    Lock,
    Yield,
    Goto,
    /// Deferred statement run at scope exit (`defer`).
    Defer,
    /// Resource region without handlers (`using`, `with`).
    Using,
    Empty,
    // Expressions
    Assign,
    Binary,
    Unary,
    Ternary,
    Call,
    New,
    FieldAccess,
    Index,
    Cast,
    TypeTest,
    Lambda,
    MethodRef,
    ArrayNew,
    ArrayLit,
    Name,
    Literal,
    This,
    Super,
    SwitchExpr,
    Pattern,
    TypeRef,
    Unknown,
}

impl NodeClass {
    pub fn is_type_decl(self) -> bool {
        matches!(self, NodeClass::TypeDecl)
    }

    /// Declarations that own a function body.
    pub fn is_function_like(self) -> bool {
        matches!(
            self,
            NodeClass::Function | NodeClass::Constructor | NodeClass::Initializer { .. }
        )
    }

    pub fn is_statement(self) -> bool {
        matches!(
            self,
            NodeClass::Block
                | NodeClass::If
                | NodeClass::While
                | NodeClass::DoWhile
                | NodeClass::For
                | NodeClass::ForEach
                | NodeClass::Switch
                | NodeClass::Try
                | NodeClass::Return
                | NodeClass::Throw
                | NodeClass::Break
                | NodeClass::Continue
                | NodeClass::ExprStmt
                | NodeClass::Assert
                | NodeClass::Labeled
                | NodeClass::Lock
                | NodeClass::Yield
                | NodeClass::Goto
                | NodeClass::Defer
                | NodeClass::Using
                | NodeClass::Empty
                | NodeClass::LocalDecl
                | NodeClass::TypeDecl
                | NodeClass::Function
        )
    }

    pub fn is_expression(self) -> bool {
        matches!(
            self,
            NodeClass::Assign
                | NodeClass::Binary
                | NodeClass::Unary
                | NodeClass::Ternary
                | NodeClass::Call
                | NodeClass::New
                | NodeClass::FieldAccess
                | NodeClass::Index
                | NodeClass::Cast
                | NodeClass::TypeTest
                | NodeClass::Lambda
                | NodeClass::MethodRef
                | NodeClass::ArrayNew
                | NodeClass::ArrayLit
                | NodeClass::Name
                | NodeClass::Literal
                | NodeClass::This
                | NodeClass::Super
                | NodeClass::SwitchExpr
        )
    }

    /// Nodes that carry no executable content of the enclosing function.
    pub fn is_inert(self) -> bool {
        matches!(
            self,
            NodeClass::Directive
                | NodeClass::TypeParams
                | NodeClass::Annotation
                | NodeClass::TypeList
                | NodeClass::TypeRef
                | NodeClass::Empty
        )
    }
}

/// Default class of each node type of the shared AST alphabet.
pub fn default_class(t: ASTNodeType) -> NodeClass {
    use ASTNodeType::*;
    match t {
        NN_MODULE => NodeClass::Module,
        NN_CLASS_DECL | NN_INTERFACE_DECL | NN_ENUM_DECL | NN_RECORD_DECL | NN_ANNOTATION_DECL => {
            NodeClass::TypeDecl
        }
        NN_METHOD_DECL => NodeClass::Function,
        NN_CONSTRUCTOR_DECL => NodeClass::Constructor,
        NN_JAVA_STATIC_INIT => NodeClass::Initializer { is_static: true },
        NN_JAVA_INSTANCE_INIT => NodeClass::Initializer { is_static: false },
        NN_FIELD_DECL => NodeClass::Field,
        NN_PARAM_DECL => NodeClass::Param,
        NN_LOCAL_VAR_DECL => NodeClass::LocalDecl,
        NN_VAR_DECLARATOR => NodeClass::Declarator,
        NN_ENUM_CONSTANT => NodeClass::EnumConstant,
        NN_ANON_CLASS_BODY => NodeClass::AnonBody,
        NN_PACKAGE_DECL | NN_IMPORT_DECL | NN_JAVA_MODULE_DECL | NN_JAVA_MODULE_DIRECTIVE => {
            NodeClass::Directive
        }
        NN_TYPE_PARAM => NodeClass::TypeParams,
        NN_ANNOTATION_USE => NodeClass::Annotation,
        NN_THROWS_CLAUSE | NN_PERMITS_CLAUSE => NodeClass::TypeList,
        NN_RESOURCE_SPEC => NodeClass::Resources,
        NN_BLOCK => NodeClass::Block,
        NN_IF_STMT => NodeClass::If,
        NN_ELSE_CLAUSE => NodeClass::Else,
        NN_WHILE_STMT => NodeClass::While,
        NN_DO_WHILE_STMT => NodeClass::DoWhile,
        NN_FOR_STMT => NodeClass::For,
        NN_ENHANCED_FOR => NodeClass::ForEach,
        NN_SWITCH_STMT => NodeClass::Switch,
        NN_SWITCH_CASE => NodeClass::Case,
        NN_TRY_STMT => NodeClass::Try,
        NN_CATCH_CLAUSE => NodeClass::Catch,
        NN_FINALLY_CLAUSE => NodeClass::Finally,
        NN_RETURN_STMT => NodeClass::Return,
        NN_THROW_STMT => NodeClass::Throw,
        NN_BREAK_STMT => NodeClass::Break,
        NN_CONTINUE_STMT => NodeClass::Continue,
        NN_EXPR_STMT => NodeClass::ExprStmt,
        NN_JAVA_ASSERT_STMT => NodeClass::Assert,
        NN_JAVA_LABELED_STMT => NodeClass::Labeled,
        NN_JAVA_SYNCHRONIZED => NodeClass::Lock,
        NN_YIELD_STMT => NodeClass::Yield,
        NN_ASSIGN_EXPR => NodeClass::Assign,
        NN_BINARY_EXPR => NodeClass::Binary,
        NN_UNARY_EXPR => NodeClass::Unary,
        NN_TERNARY_EXPR => NodeClass::Ternary,
        NN_CALL_EXPR => NodeClass::Call,
        NN_NEW_EXPR => NodeClass::New,
        NN_FIELD_ACCESS => NodeClass::FieldAccess,
        NN_ARRAY_ACCESS => NodeClass::Index,
        NN_CAST_EXPR => NodeClass::Cast,
        NN_INSTANCEOF_EXPR => NodeClass::TypeTest,
        NN_LAMBDA_EXPR => NodeClass::Lambda,
        NN_METHOD_REF => NodeClass::MethodRef,
        NN_ARRAY_CREATE => NodeClass::ArrayNew,
        NN_ARRAY_INIT => NodeClass::ArrayLit,
        NN_TYPE_REF => NodeClass::TypeRef,
        NN_IDENTIFIER_EXPR => NodeClass::Name,
        NN_LITERAL => NodeClass::Literal,
        NN_SUPER_EXPR => NodeClass::Super,
        NN_THIS_EXPR => NodeClass::This,
        NN_SWITCH_EXPR => NodeClass::SwitchExpr,
        NN_PATTERN_MATCH => NodeClass::Pattern,
        NN_SYNTHETIC | NN_UNKNOWN => NodeClass::Unknown,
    }
}

/// Structural view of the BP AST and the token corpus.
pub struct Tree<'a> {
    pub bpa: &'a BPASTArtifact,
    pub tca: &'a TokenCorpusArtifact,
    parents: Vec<u32>,
    child_offsets: Vec<u32>,
    children: Vec<u32>,
    subtree_end: Vec<u32>,
}

impl<'a> Tree<'a> {
    pub fn build(bpa: &'a BPASTArtifact, tca: &'a TokenCorpusArtifact) -> Self {
        let n = bpa.node_count as usize;
        // A parent always precedes its children in preorder; any other link is corrupt and
        // the node is treated as a root.
        let mut parents = Vec::with_capacity(n);
        let mut counts = vec![0u32; n];
        for i in 0..n {
            let p = bpa.parent(i as u32);
            let p = if (p as usize) < i { p } else { u32::MAX };
            if p != u32::MAX {
                counts[p as usize] += 1;
            }
            parents.push(p);
        }
        let mut child_offsets = vec![0u32; n + 1];
        for i in 0..n {
            child_offsets[i + 1] = child_offsets[i] + counts[i];
        }
        let mut fill: Vec<u32> = child_offsets[..n].to_vec();
        let mut children = vec![0u32; child_offsets[n] as usize];
        for (i, &p) in parents.iter().enumerate() {
            if p != u32::MAX {
                let slot = &mut fill[p as usize];
                children[*slot as usize] = i as u32;
                *slot += 1;
            }
        }
        let mut subtree_end = vec![0u32; n];
        for i in (0..n).rev() {
            let (lo, hi) = (child_offsets[i] as usize, child_offsets[i + 1] as usize);
            subtree_end[i] = if hi > lo {
                subtree_end[children[hi - 1] as usize]
            } else {
                i as u32 + 1
            };
        }
        Self {
            bpa,
            tca,
            parents,
            child_offsets,
            children,
            subtree_end,
        }
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.parents.len() as u32
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }

    #[inline]
    pub fn node_type(&self, n: u32) -> ASTNodeType {
        self.bpa.node_type(n)
    }

    #[inline]
    pub fn attr(&self, n: u32) -> u32 {
        self.bpa.node_attr(n)
    }

    #[inline]
    pub fn has_aux(&self, n: u32, flag: u8) -> bool {
        NodeAttr::unpack_aux_flags(self.attr(n)) & flag != 0
    }

    #[inline]
    pub fn lang_byte(&self, n: u32) -> u8 {
        NodeAttr::unpack_lang_flags(self.attr(n))
    }

    #[inline]
    pub fn modifiers(&self, n: u32) -> u8 {
        NodeAttr::unpack_modifiers(self.attr(n))
    }

    /// Operator id recorded in the attribute word (`None` when absent or invalid).
    #[inline]
    pub fn operator(&self, n: u32) -> OperatorId {
        NodeAttr::unpack_operator_id(self.attr(n))
    }

    pub fn parent(&self, n: u32) -> Option<u32> {
        match self.parents.get(n as usize) {
            Some(&p) if p != u32::MAX => Some(p),
            _ => None,
        }
    }

    pub fn children(&self, n: u32) -> &[u32] {
        let i = n as usize;
        if i + 1 >= self.child_offsets.len() {
            return &[];
        }
        &self.children[self.child_offsets[i] as usize..self.child_offsets[i + 1] as usize]
    }

    /// Root nodes (compilation units).
    pub fn roots(&self) -> Vec<u32> {
        (0..self.len()).filter(|&i| self.parents[i as usize] == u32::MAX).collect()
    }

    /// True when `n` lies in the subtree of `ancestor` (inclusive).
    pub fn is_within(&self, n: u32, ancestor: u32) -> bool {
        n >= ancestor
            && self
                .subtree_end
                .get(ancestor as usize)
                .is_some_and(|&end| n < end)
    }

    /// Inclusive token range of a node.
    pub fn token_range(&self, n: u32) -> Option<(u32, u32)> {
        let (ft, lt) = self.bpa.token_range(n);
        if ft == u32::MAX || (ft as usize) >= self.tca.token_records.len() {
            None
        } else {
            Some((ft, lt.max(ft)))
        }
    }

    #[inline]
    pub fn first_token(&self, n: u32) -> Option<u32> {
        self.token_range(n).map(|r| r.0)
    }

    #[inline]
    pub fn last_token(&self, n: u32) -> Option<u32> {
        self.token_range(n).map(|r| r.1)
    }

    #[inline]
    pub fn token_count(&self) -> u32 {
        self.tca.token_records.len() as u32
    }

    pub fn token_file(&self, t: u32) -> Option<u16> {
        self.tca
            .token_records
            .get(t as usize)
            .map(|r| unpack_sort_key(r.sort_key).0)
    }

    /// Source line and column of a token.
    pub fn token_position(&self, t: u32) -> Option<(u32, u16)> {
        self.tca.token_records.get(t as usize).map(|r| {
            let (_, line, col) = unpack_sort_key(r.sort_key);
            (line, col)
        })
    }

    pub fn token_text_id(&self, t: u32) -> Option<u32> {
        self.tca.token_records.get(t as usize).map(|r| r.text_id)
    }

    pub fn token_text(&self, t: u32) -> &'a [u8] {
        let tca: &'a TokenCorpusArtifact = self.tca;
        match tca.token_records.get(t as usize) {
            Some(r) => tca.interner.lookup_text(r.text_id),
            None => b"",
        }
    }

    pub fn token_type(&self, t: u32) -> TokenType {
        self.tca
            .token_records
            .get(t as usize)
            .map(|r| TokenType::from_u8(r.token_type))
            .unwrap_or(TokenType::Unknown)
    }

    pub fn node_file(&self, n: u32) -> Option<u16> {
        self.first_token(n).and_then(|t| self.token_file(t))
    }

    /// Provenance of a node.
    pub fn span(&self, n: u32) -> Span {
        match self.token_range(n) {
            Some((ft, lt)) => Span::new(
                self.token_file(ft).map(u32::from).unwrap_or(u32::MAX),
                ft,
                lt,
            ),
            None => Span::NONE,
        }
    }

    pub fn token_span(&self, t: u32) -> Span {
        match self.token_file(t) {
            Some(f) => Span::new(f as u32, t, t),
            None => Span::NONE,
        }
    }

    /// Text id of a node's first token (the name of an identifier leaf).
    pub fn text_id(&self, n: u32) -> Option<u32> {
        self.first_token(n).and_then(|t| self.token_text_id(t))
    }

    pub fn text(&self, n: u32) -> &'a [u8] {
        match self.first_token(n) {
            Some(t) => self.token_text(t),
            None => b"",
        }
    }

    /// Token texts of an inclusive range, concatenated without separators.
    pub fn joined_text(&self, first: u32, last: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut t = first;
        while t <= last && t < self.token_count() {
            out.extend_from_slice(self.token_text(t));
            t += 1;
        }
        out
    }

    /// Identifier tokens of a node's range, in order (the segments of a qualified name).
    pub fn identifier_tokens(&self, n: u32) -> Vec<u32> {
        let Some((ft, lt)) = self.token_range(n) else {
            return Vec::new();
        };
        (ft..=lt)
            .filter(|&t| {
                matches!(
                    self.token_type(t),
                    TokenType::Identifier | TokenType::SoftKeyword
                )
            })
            .collect()
    }

    /// True when the token after `t` exists in the same file.
    fn next_in_file(&self, t: u32) -> Option<u32> {
        let next = t.checked_add(1)?;
        if next < self.token_count() && self.token_file(next) == self.token_file(t) {
            Some(next)
        } else {
            None
        }
    }

    fn prev_in_file(&self, t: u32) -> Option<u32> {
        let prev = t.checked_sub(1)?;
        if self.token_file(prev) == self.token_file(t) {
            Some(prev)
        } else {
            None
        }
    }

    /// Token immediately before a node.
    pub fn token_before(&self, n: u32) -> Option<u32> {
        self.first_token(n).and_then(|t| self.prev_in_file(t))
    }

    /// Token immediately after a node.
    pub fn token_after(&self, n: u32) -> Option<u32> {
        self.last_token(n).and_then(|t| self.next_in_file(t))
    }
}

/// Positions of a parenthesised header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParenHeader {
    pub open: u32,
    pub close: u32,
    /// Separator tokens at nesting depth 1, in order.
    pub separators: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionParts {
    pub params: Vec<u32>,
    pub body: Option<u32>,
    pub ret_type: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalDeclParts {
    pub type_ref: Option<u32>,
    pub declarators: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaratorParts {
    pub name_token: Option<u32>,
    pub init: Option<u32>,
    /// Type written on the declarator itself (single-declarator encodings).
    pub type_ref: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamParts {
    pub name_token: Option<u32>,
    pub type_refs: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IfParts {
    pub cond: Option<u32>,
    pub then_s: Option<u32>,
    pub else_s: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForParts {
    pub init: Vec<u32>,
    pub cond: Option<u32>,
    pub update: Vec<u32>,
    pub body: Option<u32>,
    /// False when the header clauses could not be told apart; `unassigned` then holds the
    /// header expressions whose role is not derivable.
    pub exact: bool,
    pub unassigned: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForEachParts {
    pub var_token: Option<u32>,
    pub type_ref: Option<u32>,
    pub iterable: Option<u32>,
    pub body: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaseParts {
    pub labels: Vec<u32>,
    pub body: Vec<u32>,
    pub is_default: bool,
    pub is_arrow: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TryParts {
    pub resources: Option<u32>,
    pub body: Option<u32>,
    pub catches: Vec<u32>,
    pub finally: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatchParts {
    pub name_token: Option<u32>,
    pub types: Vec<u32>,
    pub body: Option<u32>,
}

/// Shape of a call node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallForm {
    /// `[receiver.]name(args)`.
    Method {
        receiver: Option<u32>,
        name_token: u32,
        args: Vec<u32>,
    },
    /// Call of a computed function value: `f(args)` where `f` is not a name.
    Value { callee: u32, args: Vec<u32> },
    /// Explicit constructor chaining (`this(..)`, `super(..)`).
    ConstructorChain {
        to_super: bool,
        qualifier: Option<u32>,
        args: Vec<u32>,
    },
    /// The roles cannot be recovered.
    Opaque,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewParts {
    pub qualifier: Option<u32>,
    pub type_ref: Option<u32>,
    pub args: Vec<u32>,
    /// `NN_ANON_CLASS_BODY` node, or the new-expression itself when the encoding inlines the
    /// anonymous members (see `anon_members`).
    pub anon_body: Option<u32>,
    pub anon_members: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LambdaParts {
    /// Parameter declaration nodes or bare parameter names, in order.
    pub params: Vec<u32>,
    pub body: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeRefParts {
    /// Name segments (identifier tokens) of the head type, outermost first.
    pub segments: Vec<u32>,
    /// First token of the head (for keyword types the keyword itself).
    pub head_token: Option<u32>,
    pub args: Vec<u32>,
    pub dims: u32,
    pub is_wildcard: bool,
}

/// Lowering view of the tree for one source language.
#[derive(Clone, Copy)]
pub struct Syntax<'t, 'a> {
    pub tree: &'t Tree<'a>,
    pub sem: &'t dyn LoweringSemantics,
}

impl<'t, 'a> Syntax<'t, 'a> {
    pub fn new(tree: &'t Tree<'a>, sem: &'t dyn LoweringSemantics) -> Self {
        Self { tree, sem }
    }

    pub fn class(&self, n: u32) -> NodeClass {
        let t = self.tree.node_type(n);
        self.sem
            .classify(t, self.tree.attr(n))
            .unwrap_or_else(|| default_class(t))
    }

    #[inline]
    pub fn kids(&self, n: u32) -> &'t [u32] {
        self.tree.children(n)
    }

    pub fn kids_of(&self, n: u32, class: NodeClass) -> Vec<u32> {
        self.kids(n)
            .iter()
            .copied()
            .filter(|&c| self.class(c) == class)
            .collect()
    }

    pub fn first_kid_of(&self, n: u32, class: NodeClass) -> Option<u32> {
        self.kids(n).iter().copied().find(|&c| self.class(c) == class)
    }

    /// Children that are statements or expressions (inert children dropped).
    pub fn active_kids(&self, n: u32) -> Vec<u32> {
        self.kids(n)
            .iter()
            .copied()
            .filter(|&c| !self.class(c).is_inert())
            .collect()
    }

    fn token_is(&self, t: u32, text: &[u8]) -> bool {
        !text.is_empty() && self.tree.token_text(t) == text
    }

    /// The parenthesised header that starts at or after token `from`, with the depth-1
    /// tokens spelled `separator`. Needs the language's punctuation.
    pub fn paren_header(&self, from: u32, separator: Option<&[u8]>) -> Option<ParenHeader> {
        let p: Punctuation = self.sem.punctuation()?;
        let file = self.tree.token_file(from)?;
        let count = self.tree.token_count();
        let mut t = from;
        while t < count && self.tree.token_file(t) == Some(file) {
            if self.token_is(t, p.open_paren) {
                break;
            }
            t += 1;
        }
        if t >= count || !self.token_is(t, p.open_paren) {
            return None;
        }
        let open = t;
        let mut depth = 0usize;
        let mut separators = Vec::new();
        while t < count && self.tree.token_file(t) == Some(file) {
            let text = self.tree.token_text(t);
            if text == p.open_paren || p.nesting_open.iter().any(|o| *o == text) {
                depth += 1;
            } else if text == p.close_paren || p.nesting_close.iter().any(|c| *c == text) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(ParenHeader {
                        open,
                        close: t,
                        separators,
                    });
                }
            } else if depth == 1 && separator == Some(text) {
                separators.push(t);
            }
            t += 1;
        }
        None
    }

    /// Body, parameters and declared result type of a function-like declaration.
    pub fn function_parts(&self, n: u32) -> FunctionParts {
        let kids = self.kids(n);
        let mut parts = FunctionParts::default();
        for &c in kids {
            match self.class(c) {
                NodeClass::Param => parts.params.push(c),
                NodeClass::Block => parts.body = Some(c),
                NodeClass::TypeRef
                    if parts.ret_type.is_none() && parts.params.is_empty() && parts.body.is_none() =>
                {
                    // The declared result type precedes the parameters.
                    parts.ret_type = Some(c)
                }
                _ => {}
            }
        }
        // An expression-bodied function keeps its body as the last executable child.
        if parts.body.is_none() {
            if let Some(&last) = kids.last() {
                if self.class(last).is_expression() {
                    parts.body = Some(last);
                }
            }
        }
        parts
    }

    /// Type and declarators of a local variable or field declaration. A declaration
    /// without declarator children is its own single declarator.
    pub fn local_decl_parts(&self, n: u32) -> LocalDeclParts {
        let mut parts = LocalDeclParts::default();
        for &c in self.kids(n) {
            match self.class(c) {
                NodeClass::TypeRef if parts.type_ref.is_none() && parts.declarators.is_empty() => {
                    parts.type_ref = Some(c)
                }
                NodeClass::Declarator | NodeClass::LocalDecl => parts.declarators.push(c),
                _ => {}
            }
        }
        if parts.declarators.is_empty() {
            parts.declarators.push(n);
        }
        parts
    }

    /// Name and initializer of a declarator. The name is the identifier at the
    /// declarator's first token (a declarator starts with its name); the initializer is
    /// the last executable child that is not the name.
    pub fn declarator_parts(&self, n: u32) -> DeclaratorParts {
        let mut parts = DeclaratorParts::default();
        let first = self.tree.first_token(n);
        let kids = self.kids(n);
        let mut name_node = None;
        for &c in kids {
            match self.class(c) {
                NodeClass::TypeRef => {
                    if parts.type_ref.is_none() {
                        parts.type_ref = Some(c);
                    }
                }
                NodeClass::Name if name_node.is_none() => {
                    // In a single-declarator encoding the type precedes the name, so the
                    // first identifier after any type is the name.
                    if self.tree.first_token(c) == first || parts.type_ref.is_some() {
                        name_node = Some(c);
                    }
                }
                _ => {}
            }
        }
        parts.name_token = match name_node {
            Some(c) => self.tree.first_token(c),
            None => {
                // The declarator's own first token is its name when no identifier child
                // was kept (the name is held by the declarator node).
                match first {
                    Some(t) if self.tree.token_type(t) == TokenType::Identifier => Some(t),
                    _ => None,
                }
            }
        };
        parts.init = kids
            .iter()
            .rev()
            .copied()
            .find(|&c| Some(c) != name_node && self.class(c).is_expression());
        parts
    }

    /// Name and type of a formal parameter (the name is the last identifier child).
    pub fn param_parts(&self, n: u32) -> ParamParts {
        let mut parts = ParamParts::default();
        for &c in self.kids(n) {
            match self.class(c) {
                NodeClass::TypeRef => parts.type_refs.push(c),
                NodeClass::Name => parts.name_token = self.tree.first_token(c),
                _ => {}
            }
        }
        if parts.name_token.is_none() {
            // A parameter whose name is held by the node itself ends with the name.
            if let Some(t) = self.tree.last_token(n) {
                if self.tree.token_type(t) == TokenType::Identifier {
                    parts.name_token = Some(t);
                }
            }
        }
        parts
    }

    pub fn if_parts(&self, n: u32) -> IfParts {
        let kids = self.active_kids(n);
        let mut parts = IfParts {
            cond: kids.first().copied(),
            then_s: kids.get(1).copied(),
            else_s: kids.get(2).copied(),
        };
        if let Some(e) = parts.else_s {
            if self.class(e) == NodeClass::Else {
                parts.else_s = self.active_kids(e).first().copied();
            }
        }
        if let Some(t) = parts.then_s {
            if self.class(t) == NodeClass::Else {
                // `if (c) ; else s`: the empty consequence was dropped by the encoding.
                parts.else_s = self.active_kids(t).first().copied();
                parts.then_s = None;
            }
        }
        parts
    }

    /// Roles of a counted loop. The body is the last child when it is a statement. Header
    /// clauses are told apart by the header separators (with the language punctuation), or
    /// structurally when the clause pattern is unambiguous.
    pub fn for_parts(&self, n: u32) -> ForParts {
        let kids = self.active_kids(n);
        let mut parts = ForParts::default();
        let mut header: Vec<u32> = kids.clone();
        if let Some(&last) = kids.last() {
            if self.class(last).is_statement() && self.class(last) != NodeClass::LocalDecl {
                parts.body = Some(last);
                header.pop();
            }
        }

        let sep = self.sem.punctuation().map(|p| p.header_separator);
        let hdr = match (sep, self.tree.first_token(n)) {
            (Some(s), Some(ft)) => self.paren_header(ft, Some(s)),
            _ => None,
        };
        if let Some(h) = hdr.filter(|h| h.separators.len() >= 2) {
            let (s1, s2) = (h.separators[0], h.separators[1]);
            for &c in &header {
                let ct = self.tree.first_token(c).unwrap_or(u32::MAX);
                if ct < s1 {
                    parts.init.push(c);
                } else if ct < s2 {
                    parts.cond = Some(c);
                } else {
                    parts.update.push(c);
                }
            }
            parts.exact = true;
            return parts;
        }

        // Structural recovery: leading declarations initialise; the rest is decidable only
        // when exactly two (condition, update) or three (init, condition, update)
        // expressions remain, or none.
        let mut rest: Vec<u32> = Vec::new();
        for &c in &header {
            if self.class(c) == NodeClass::LocalDecl && rest.is_empty() {
                parts.init.push(c);
            } else {
                rest.push(c);
            }
        }
        match (parts.init.is_empty(), rest.len()) {
            (_, 0) => parts.exact = true,
            (_, 1) => {
                parts.exact = false;
                parts.unassigned = rest;
            }
            (false, 2) => {
                parts.cond = Some(rest[0]);
                parts.update.push(rest[1]);
                parts.exact = true;
            }
            (true, 2) => {
                // `for (a; b;)`, `for (a; ; b)` and `for (; a; b)` are indistinguishable
                // without the separators.
                parts.exact = false;
                parts.unassigned = rest;
            }
            (true, 3) => {
                parts.init.push(rest[0]);
                parts.cond = Some(rest[1]);
                parts.update.push(rest[2]);
                parts.exact = true;
            }
            _ => {
                parts.exact = false;
                parts.unassigned = rest;
            }
        }
        parts
    }

    /// Roles of an iterator loop: the body is the last statement child, the iterable the
    /// child before it, and the loop variable the last name among the remaining children.
    pub fn foreach_parts(&self, n: u32) -> ForEachParts {
        let kids = self.active_kids(n);
        let mut parts = ForEachParts::default();
        let mut idx = kids.len();
        if idx > 0 && self.class(kids[idx - 1]).is_statement() {
            parts.body = Some(kids[idx - 1]);
            idx -= 1;
        }
        if idx > 0 {
            parts.iterable = Some(kids[idx - 1]);
            idx -= 1;
        }
        for &c in kids[..idx].iter() {
            match self.class(c) {
                NodeClass::TypeRef => parts.type_ref = Some(c),
                NodeClass::Name => parts.var_token = self.tree.first_token(c),
                NodeClass::LocalDecl | NodeClass::Param | NodeClass::Declarator => {
                    let d = self.declarator_parts(c);
                    parts.var_token = d.name_token.or(parts.var_token);
                    if let Some(t) = d.type_ref.or_else(|| self.first_kid_of(c, NodeClass::TypeRef)) {
                        parts.type_ref = Some(t);
                    }
                }
                _ => {}
            }
        }
        // The kept type reference child may precede the name outside the inert filter.
        if parts.type_ref.is_none() {
            parts.type_ref = self
                .kids(n)
                .iter()
                .copied()
                .find(|&c| self.class(c) == NodeClass::TypeRef);
        }
        parts
    }

    /// Scrutinee and case groups of a switch.
    pub fn switch_parts(&self, n: u32) -> (Option<u32>, Vec<u32>) {
        let mut scrutinee = None;
        let mut cases = Vec::new();
        for &c in self.kids(n) {
            match self.class(c) {
                NodeClass::Case => cases.push(c),
                cl if scrutinee.is_none() && cases.is_empty() && cl.is_expression() => {
                    scrutinee = Some(c)
                }
                _ => {}
            }
        }
        (scrutinee, cases)
    }

    /// Labels and body of a case group: labels are the expression or pattern children
    /// before the first statement. An arrow case has exactly one body child, the last.
    pub fn case_parts(&self, n: u32) -> CaseParts {
        let kids = self.active_kids(n);
        let mut parts = CaseParts {
            is_arrow: self.tree.has_aux(n, NodeAttr::AUX_CASE_ARROW),
            is_default: self.tree.has_aux(n, NodeAttr::AUX_CASE_DEFAULT),
            ..CaseParts::default()
        };
        if parts.is_arrow {
            if let Some((&last, labels)) = kids.split_last() {
                parts.body.push(last);
                parts.labels = labels.to_vec();
            }
        } else {
            let mut in_body = false;
            for &c in &kids {
                let cl = self.class(c);
                if !in_body && (cl.is_expression() || cl == NodeClass::Pattern) {
                    parts.labels.push(c);
                } else {
                    in_body = true;
                    parts.body.push(c);
                }
            }
        }
        if parts.labels.is_empty() {
            // A case group with no label expression is the default group.
            parts.is_default = true;
        }
        parts
    }

    pub fn try_parts(&self, n: u32) -> TryParts {
        let mut parts = TryParts::default();
        for &c in self.kids(n) {
            match self.class(c) {
                NodeClass::Resources => parts.resources = Some(c),
                NodeClass::Block if parts.body.is_none() => parts.body = Some(c),
                NodeClass::Catch => parts.catches.push(c),
                NodeClass::Finally => parts.finally = Some(c),
                _ => {}
            }
        }
        parts
    }

    /// Exception types, parameter name and body of a catch clause.
    pub fn catch_parts(&self, n: u32) -> CatchParts {
        let mut parts = CatchParts::default();
        for &c in self.kids(n) {
            match self.class(c) {
                NodeClass::Param => {
                    let p = self.param_parts(c);
                    parts.name_token = p.name_token;
                    parts.types.extend(p.type_refs);
                }
                NodeClass::TypeRef => parts.types.push(c),
                NodeClass::Name => parts.name_token = self.tree.first_token(c),
                NodeClass::Block => parts.body = Some(c),
                _ => {}
            }
        }
        parts
    }

    /// Body block of a finally clause (or the clause itself when it is the block).
    pub fn finally_body(&self, n: u32) -> u32 {
        self.first_kid_of(n, NodeClass::Block).unwrap_or(n)
    }

    /// Shape of a call. Explicit constructor chaining is read from the attribute flags or
    /// a leading `this`/`super` child applied to arguments; the method name is the name
    /// child followed by the argument list (punctuation), or, under the attribute contract,
    /// the first name child after the receiver.
    pub fn call_parts(&self, n: u32) -> CallForm {
        let kids: Vec<u32> = self
            .kids(n)
            .iter()
            .copied()
            .filter(|&c| self.class(c) != NodeClass::Annotation)
            .collect();
        let explicit_ctor = self.tree.has_aux(n, NodeAttr::AUX_CALL_EXPLICIT_CTOR);
        if explicit_ctor {
            let to_super = self.tree.has_aux(n, NodeAttr::AUX_CALL_SUPER_TARGET);
            let mut args = Vec::new();
            let mut qualifier = None;
            for (i, &c) in kids.iter().enumerate() {
                match self.class(c) {
                    NodeClass::This | NodeClass::Super if i == 0 || qualifier.is_some() => {}
                    NodeClass::TypeRef => {}
                    _ if i == 0 && self.tree.has_aux(n, NodeAttr::AUX_CALL_QUALIFIED) => {
                        qualifier = Some(c)
                    }
                    _ => args.push(c),
                }
            }
            return CallForm::ConstructorChain {
                to_super,
                qualifier,
                args,
            };
        }

        if let Some(&first) = kids.first() {
            let cl = self.class(first);
            if (cl == NodeClass::This || cl == NodeClass::Super) && self.is_applied(first) {
                return CallForm::ConstructorChain {
                    to_super: cl == NodeClass::Super,
                    qualifier: None,
                    args: kids[1..].to_vec(),
                };
            }
        }

        // Name child followed by the argument list.
        if self.sem.punctuation().is_some() {
            for (pos, &c) in kids.iter().enumerate() {
                if self.class(c) == NodeClass::Name && self.is_applied(c) {
                    let receiver = kids[..pos]
                        .iter()
                        .copied()
                        .find(|&r| self.class(r) != NodeClass::TypeRef);
                    let Some(name_token) = self.tree.first_token(c) else {
                        return CallForm::Opaque;
                    };
                    return CallForm::Method {
                        receiver,
                        name_token,
                        args: kids[pos + 1..].to_vec(),
                    };
                }
            }
            // No name is applied: the callee is a computed value.
            if let Some((&callee, args)) = kids.split_first() {
                if self.is_applied(callee) {
                    return CallForm::Value {
                        callee,
                        args: args.to_vec(),
                    };
                }
            }
            return CallForm::Opaque;
        }

        // Attribute contract: `[receiver] [type args] name args..`.
        let qualified = self.tree.has_aux(n, NodeAttr::AUX_CALL_QUALIFIED);
        let mut pos = 0usize;
        let mut receiver = None;
        if qualified {
            if let Some(&r) = kids.first() {
                receiver = Some(r);
                pos = 1;
            }
        }
        while pos < kids.len() && self.class(kids[pos]) == NodeClass::TypeRef {
            pos += 1;
        }
        match kids.get(pos) {
            Some(&c) if self.class(c) == NodeClass::Name => match self.tree.first_token(c) {
                Some(name_token) => CallForm::Method {
                    receiver,
                    name_token,
                    args: kids[pos + 1..].to_vec(),
                },
                None => CallForm::Opaque,
            },
            Some(&c) if !qualified && self.class(c).is_expression() => CallForm::Value {
                callee: c,
                args: kids[pos + 1..].to_vec(),
            },
            _ => CallForm::Opaque,
        }
    }

    /// True when the token after node `c` opens an argument list.
    fn is_applied(&self, c: u32) -> bool {
        match (self.sem.punctuation(), self.tree.token_after(c)) {
            (Some(p), Some(t)) => self.token_is(t, p.open_paren),
            _ => false,
        }
    }

    /// Parts of an object creation.
    pub fn new_parts(&self, n: u32) -> NewParts {
        let mut parts = NewParts::default();
        let qualified = self.tree.has_aux(n, NodeAttr::AUX_NEW_QUALIFIED);
        for (i, &c) in self.kids(n).iter().enumerate() {
            let cl = self.class(c);
            match cl {
                _ if i == 0 && qualified && cl.is_expression() => parts.qualifier = Some(c),
                NodeClass::TypeRef if parts.type_ref.is_none() => parts.type_ref = Some(c),
                NodeClass::TypeRef | NodeClass::Annotation => {}
                NodeClass::AnonBody => parts.anon_body = Some(c),
                NodeClass::Function
                | NodeClass::Constructor
                | NodeClass::Field
                | NodeClass::Initializer { .. }
                | NodeClass::TypeDecl => {
                    // Members of an anonymous body inlined into the creation node.
                    parts.anon_body = Some(n);
                    parts.anon_members.push(c);
                }
                _ if cl.is_expression() => parts.args.push(c),
                _ => {}
            }
        }
        if let Some(body) = parts.anon_body {
            if body != n {
                parts.anon_members = self.kids(body).to_vec();
            }
        }
        parts
    }

    pub fn lambda_parts(&self, n: u32) -> LambdaParts {
        let kids = self.kids(n);
        let mut parts = LambdaParts::default();
        let Some((&last, rest)) = kids.split_last() else {
            return parts;
        };
        parts.body = Some(last);
        for &c in rest {
            if matches!(self.class(c), NodeClass::Param | NodeClass::Name) {
                parts.params.push(c);
            }
        }
        parts
    }

    /// Object and member name of a member access (the name is the last child).
    pub fn member_parts(&self, n: u32) -> (Option<u32>, Option<u32>) {
        let kids = self.kids(n);
        match kids.split_last() {
            Some((&name, rest)) if self.class(name) == NodeClass::Name => {
                (rest.first().copied(), self.tree.first_token(name))
            }
            Some((_, _)) => (kids.first().copied(), self.tree.last_token(n)),
            None => (None, None),
        }
    }

    /// Label text id of a break, continue, goto or labeled statement.
    pub fn label_of(&self, n: u32) -> Option<u32> {
        self.first_kid_of(n, NodeClass::Name)
            .and_then(|c| self.tree.text_id(c))
    }

    /// Lexical category of a literal: the attribute payload, else the token type.
    pub fn literal_class(&self, n: u32) -> Option<LiteralClass> {
        let by_attr = match self.tree.lang_byte(n) {
            NodeAttr::LITERAL_INTEGER => Some(LiteralClass::Integer),
            NodeAttr::LITERAL_FLOAT => Some(LiteralClass::Float),
            NodeAttr::LITERAL_STRING => Some(LiteralClass::String),
            NodeAttr::LITERAL_CHAR => Some(LiteralClass::Char),
            NodeAttr::LITERAL_BOOLEAN => Some(LiteralClass::Boolean),
            NodeAttr::LITERAL_NULL => Some(LiteralClass::Null),
            NodeAttr::LITERAL_TEXT_BLOCK => Some(LiteralClass::TextBlock),
            NodeAttr::LITERAL_CLASS => Some(LiteralClass::Type),
            _ => None,
        };
        if by_attr.is_some() {
            return by_attr;
        }
        let t = self.tree.first_token(n)?;
        match self.tree.token_type(t) {
            TokenType::IntegerLiteral => Some(LiteralClass::Integer),
            TokenType::FloatLiteral => Some(LiteralClass::Float),
            TokenType::StringLiteral => Some(LiteralClass::String),
            TokenType::CharLiteral => Some(LiteralClass::Char),
            TokenType::BooleanLiteral => Some(LiteralClass::Boolean),
            TokenType::NullLiteral => Some(LiteralClass::Null),
            _ => None,
        }
    }

    /// Head name, type arguments and array dimensions of a type reference.
    pub fn type_ref_parts(&self, n: u32) -> TypeRefParts {
        let mut parts = TypeRefParts {
            is_wildcard: self.tree.has_aux(n, NodeAttr::AUX_TYPE_WILDCARD),
            ..TypeRefParts::default()
        };
        let type_kids: Vec<u32> = self
            .kids(n)
            .iter()
            .copied()
            .filter(|&c| self.class(c) == NodeClass::TypeRef)
            .collect();
        if self.tree.has_aux(n, NodeAttr::AUX_TYPE_ARRAY) {
            parts.dims = self.tree.lang_byte(n).max(1) as u32;
            if let Some(&elem) = type_kids.first() {
                let inner = self.type_ref_parts(elem);
                parts.segments = inner.segments;
                parts.head_token = inner.head_token;
                parts.args = inner.args;
                parts.dims += inner.dims;
                return parts;
            }
        }
        if self.tree.has_aux(n, NodeAttr::AUX_TYPE_GENERIC) {
            if let Some((&head, args)) = type_kids.split_first() {
                let inner = self.type_ref_parts(head);
                parts.segments = inner.segments;
                parts.head_token = inner.head_token;
                parts.args = args.to_vec();
                parts.dims += inner.dims;
                return parts;
            }
        }
        if parts.is_wildcard {
            // The bound of a wildcard is its single type child.
            parts.args = type_kids;
            return parts;
        }
        parts.head_token = self.tree.first_token(n);
        // The name segments end at the first nested type (type arguments are children).
        let limit = type_kids
            .first()
            .and_then(|&c| self.tree.first_token(c))
            .unwrap_or(u32::MAX);
        parts.segments = self
            .tree
            .identifier_tokens(n)
            .into_iter()
            .filter(|&t| t < limit)
            .collect();
        parts.args = type_kids;
        parts
    }
}
