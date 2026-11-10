//! Structural queries over the BP AST shared by Phases 4-6.
//!
//! Phase 2 discards tree-sitter field names and punctuation, so roles such as "call
//! name", "loop body" or "case label" are recovered here once, from node kinds, sibling
//! order and (when available) the token text, instead of being re-guessed by every
//! consumer.

use crate::ast::BPASTArtifact;
use crate::cfg::source::SourceText;
use crate::core::types::ast::{ASTNodeType, OperatorId};

use ASTNodeType::*;

/// Direct children of `node`, in source order.
pub fn children(bpa: &BPASTArtifact, node: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cur = bpa.first_child(node);
    while let Some(c) = cur {
        out.push(c);
        cur = bpa.next_sibling(c);
    }
    out
}

/// One past the last preorder index of `node`'s subtree. Subtrees are contiguous
/// preorder ranges in the BP encoding, so `[node, subtree_end)` is exactly the subtree.
#[inline]
pub fn subtree_end(bpa: &BPASTArtifact, node: u32) -> u32 {
    node.saturating_add(bpa.subtree_size(node).max(1))
}

/// True when `node` lies in the subtree rooted at `ancestor` (inclusive).
#[inline]
pub fn in_subtree(bpa: &BPASTArtifact, ancestor: u32, node: u32) -> bool {
    node >= ancestor && node < subtree_end(bpa, ancestor)
}

/// Operator id stored in bits 12..19 of the NodeAttr word.
#[inline]
pub fn operator_id(bpa: &BPASTArtifact, node: u32) -> u8 {
    ((bpa.node_attr(node) >> 12) & 0xFF) as u8
}

#[inline]
pub fn is_logical_and(bpa: &BPASTArtifact, node: u32) -> bool {
    bpa.node_type(node) == NN_BINARY_EXPR && operator_id(bpa, node) == OperatorId::LogicalAnd as u8
}

#[inline]
pub fn is_logical_or(bpa: &BPASTArtifact, node: u32) -> bool {
    bpa.node_type(node) == NN_BINARY_EXPR && operator_id(bpa, node) == OperatorId::LogicalOr as u8
}

/// True for `x = e` (Phase 2 encodes only the plain `=` operator; every compound
/// assignment operator is stored as `OperatorId::None`).
#[inline]
pub fn is_simple_assignment(bpa: &BPASTArtifact, node: u32) -> bool {
    operator_id(bpa, node) == OperatorId::Assign as u8
}

/// Type-declaration kinds.
#[inline]
pub fn is_type_decl(t: ASTNodeType) -> bool {
    matches!(
        t,
        NN_CLASS_DECL | NN_INTERFACE_DECL | NN_ENUM_DECL | NN_RECORD_DECL | NN_ANNOTATION_DECL
    )
}

/// Nodes whose subtrees are separate functions or types and therefore never part of
/// the statements of the function that lexically contains them.
#[inline]
pub fn is_nested_body(t: ASTNodeType) -> bool {
    is_type_decl(t)
        || matches!(
            t,
            NN_LAMBDA_EXPR
                | NN_METHOD_DECL
                | NN_CONSTRUCTOR_DECL
                | NN_FIELD_DECL
                | NN_JAVA_STATIC_INIT
                | NN_JAVA_INSTANCE_INIT
        )
}

/// Function-like declaration kinds that own a CFG.
#[inline]
pub fn is_function_node(t: ASTNodeType) -> bool {
    matches!(
        t,
        NN_METHOD_DECL
            | NN_CONSTRUCTOR_DECL
            | NN_LAMBDA_EXPR
            | NN_JAVA_STATIC_INIT
            | NN_JAVA_INSTANCE_INIT
    )
}

/// Statement kinds, as opposed to expressions. Used to split switch-case labels (always
/// expressions or patterns) from the statements of the case body.
#[inline]
pub fn is_statement_kind(t: ASTNodeType) -> bool {
    is_type_decl(t)
        || matches!(
            t,
            NN_BLOCK
                | NN_IF_STMT
                | NN_FOR_STMT
                | NN_ENHANCED_FOR
                | NN_WHILE_STMT
                | NN_DO_WHILE_STMT
                | NN_SWITCH_STMT
                | NN_SWITCH_EXPR
                | NN_TRY_STMT
                | NN_RETURN_STMT
                | NN_THROW_STMT
                | NN_BREAK_STMT
                | NN_CONTINUE_STMT
                | NN_EXPR_STMT
                | NN_LOCAL_VAR_DECL
                | NN_YIELD_STMT
                | NN_JAVA_ASSERT_STMT
                | NN_JAVA_LABELED_STMT
                | NN_JAVA_SYNCHRONIZED
        )
}

/// First direct child of the given kind.
pub fn first_child_of_type(bpa: &BPASTArtifact, node: u32, t: ASTNodeType) -> Option<u32> {
    let mut cur = bpa.first_child(node);
    while let Some(c) = cur {
        if bpa.node_type(c) == t {
            return Some(c);
        }
        cur = bpa.next_sibling(c);
    }
    None
}

/// Name identifier of a variable declarator (`variable_declarator`, which Phase 2 maps to
/// `NN_LOCAL_VAR_DECL`, or a try-with-resources `resource`). The enclosing
/// `local_variable_declaration` shares the kind but never has a direct identifier child.
pub fn declarator_name(bpa: &BPASTArtifact, node: u32) -> Option<u32> {
    if bpa.node_type(node) != NN_LOCAL_VAR_DECL {
        return None;
    }
    first_child_of_type(bpa, node, NN_IDENTIFIER_EXPR)
}

/// Name identifier of a formal parameter (the last direct identifier child; the type is a
/// `NN_TYPE_REF`).
pub fn param_name(bpa: &BPASTArtifact, node: u32) -> Option<u32> {
    if bpa.node_type(node) != NN_PARAM_DECL {
        return None;
    }
    children(bpa, node)
        .into_iter()
        .rev()
        .find(|&c| bpa.node_type(c) == NN_IDENTIFIER_EXPR)
}

/// Roles of the children of an enhanced `for` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForEachParts {
    /// Identifier declared by the loop header, when present in the AST.
    pub var: Option<u32>,
    /// Children that make up the variable declaration (modifiers, type, name).
    pub decl_parts: Vec<u32>,
    pub iterable: Option<u32>,
    pub body: Option<u32>,
    /// False when the roles were inferred from child positions only.
    pub exact: bool,
}

impl ForEachParts {
    pub fn of(bpa: &BPASTArtifact, src: Option<&SourceText<'_>>, node: u32) -> Self {
        let kids = children(bpa, node);
        let header = src.and_then(|s| {
            let (ft, _) = bpa.token_range(node);
            if ft == u32::MAX {
                None
            } else {
                s.paren_header(ft, None)
            }
        });

        let (header_kids, body, exact) = match header {
            Some(h) => {
                let mut head = Vec::new();
                let mut body = None;
                for &c in &kids {
                    let (cft, _) = bpa.token_range(c);
                    if cft != u32::MAX && cft > h.close {
                        body = Some(c);
                    } else {
                        head.push(c);
                    }
                }
                (head, body, true)
            }
            None => {
                if kids.len() >= 3 {
                    let (head, body) = kids.split_at(kids.len() - 1);
                    (head.to_vec(), Some(body[0]), false)
                } else {
                    (kids.clone(), None, false)
                }
            }
        };

        let iterable = header_kids.last().copied();
        let decl_parts: Vec<u32> = if header_kids.is_empty() {
            Vec::new()
        } else {
            header_kids[..header_kids.len() - 1].to_vec()
        };
        let var = decl_parts
            .iter()
            .rev()
            .copied()
            .find(|&c| bpa.node_type(c) == NN_IDENTIFIER_EXPR);

        Self {
            var,
            decl_parts,
            iterable,
            body,
            exact,
        }
    }
}

/// Name identifier of a method invocation: the first identifier child immediately
/// followed by `(`. The receiver of `obj.m()` is followed by `.` and arguments are
/// preceded by `(` or `,`, so this is exact. Needs token text.
pub fn call_name(bpa: &BPASTArtifact, src: &SourceText<'_>, call: u32) -> Option<u32> {
    let mut cur = bpa.first_child(call);
    while let Some(c) = cur {
        if bpa.node_type(c) == NN_IDENTIFIER_EXPR && src.text_after(bpa, c) == Some(&b"("[..]) {
            return Some(c);
        }
        cur = bpa.next_sibling(c);
    }
    None
}

/// Shape of a call-like node after role recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallShape {
    /// `name(args)` or `receiver.name(args)`.
    Method {
        receiver: Option<u32>,
        name: u32,
        args: Vec<u32>,
    },
    /// `this(args)` / `super(args)` explicit constructor invocation.
    ConstructorChain { is_super: bool, args: Vec<u32> },
}

/// Recovers receiver, name and arguments of an `NN_CALL_EXPR`. Returns `None` when the
/// roles cannot be determined (no token text, or an AST shape Phase 2 did not produce).
pub fn call_shape(bpa: &BPASTArtifact, src: &SourceText<'_>, call: u32) -> Option<CallShape> {
    let kids = children(bpa, call);
    if let Some(&first) = kids.first() {
        let t = bpa.node_type(first);
        if (t == NN_THIS_EXPR || t == NN_SUPER_EXPR)
            && src.text_after(bpa, first) == Some(&b"("[..])
        {
            return Some(CallShape::ConstructorChain {
                is_super: t == NN_SUPER_EXPR,
                args: kids[1..].to_vec(),
            });
        }
    }
    let name = call_name(bpa, src, call)?;
    let pos = kids.iter().position(|&c| c == name)?;
    // Children before the name are the receiver and explicit type arguments.
    let receiver = kids[..pos]
        .iter()
        .copied()
        .find(|&c| bpa.node_type(c) != NN_TYPE_REF);
    Some(CallShape::Method {
        receiver,
        name,
        args: kids[pos + 1..].to_vec(),
    })
}

/// Parts of an object-creation expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewParts {
    /// Text id of the simple name of the instantiated type (`HashMap` in
    /// `new java.util.HashMap<>()`), recovered from the tokens after `new`.
    pub type_name: Option<u32>,
    /// `NN_TYPE_REF` child naming the type, when Phase 2 kept one.
    pub type_ref: Option<u32>,
    pub args: Vec<u32>,
    /// Member declarations of an anonymous class body.
    pub anonymous_members: Vec<u32>,
}

impl NewParts {
    pub fn of(bpa: &BPASTArtifact, src: Option<&SourceText<'_>>, node: u32) -> Self {
        let kids = children(bpa, node);
        let (ft, _) = bpa.token_range(node);

        let mut type_name = None;
        let mut arg_open = None;
        let mut arg_close = None;
        if let (Some(s), true) = (src, ft != u32::MAX) {
            // Tokens: `new` [Ident ('.' Ident)*] [type arguments] '(' args ')'
            let mut t = ft;
            if s.token_is(t, b"new") {
                t += 1;
            }
            let mut last_ident = None;
            while (t as usize) < s.token_count() {
                match s.text(t) {
                    Some(b".") => {}
                    Some(b"<") | Some(b"(") | Some(b"[") | Some(b"{") => break,
                    Some(text)
                        if !text.is_empty()
                            && (text[0].is_ascii_alphabetic()
                                || text[0] == b'_'
                                || text[0] == b'$') =>
                    {
                        last_ident = s.text_id(t);
                    }
                    _ => break,
                }
                t += 1;
            }
            type_name = last_ident;
            if let Some(h) = s.paren_header(ft, None) {
                arg_open = Some(h.open);
                arg_close = Some(h.close);
            }
        }

        let type_ref = kids
            .iter()
            .copied()
            .find(|&c| bpa.node_type(c) == NN_TYPE_REF);
        if type_name.is_none() {
            if let (Some(s), Some(tr)) = (src, type_ref) {
                type_name = s.node_text_id(bpa, tr);
            }
        }

        let mut args = Vec::new();
        let mut anonymous_members = Vec::new();
        for &c in &kids {
            let t = bpa.node_type(c);
            if is_nested_body(t) || t == NN_BLOCK {
                anonymous_members.push(c);
                continue;
            }
            match (arg_open, arg_close) {
                (Some(open), Some(close)) => {
                    let (cft, _) = bpa.token_range(c);
                    if cft != u32::MAX && cft > open && cft < close {
                        args.push(c);
                    }
                }
                _ => {
                    if t != NN_TYPE_REF && t != NN_ANNOTATION_USE {
                        args.push(c);
                    }
                }
            }
        }

        Self {
            type_name,
            type_ref,
            args,
            anonymous_members,
        }
    }
}

/// Returns true when a unary/update expression is `++x`, `x++`, `--x` or `x--`.
/// Phase 2 does not encode unary operators, so this needs token text.
pub fn is_update_expression(bpa: &BPASTArtifact, src: &SourceText<'_>, node: u32) -> bool {
    if bpa.node_type(node) != NN_UNARY_EXPR {
        return false;
    }
    let (ft, _) = bpa.token_range(node);
    if ft == u32::MAX {
        return false;
    }
    // Prefix form: the node starts with the operator token.
    if src.token_is(ft, b"++") || src.token_is(ft, b"--") {
        return true;
    }
    // Postfix form: the operator token is dropped by Phase 2, so the node starts with its
    // operand and its recorded range ends right before the operator.
    match bpa.first_child(node) {
        Some(operand) if bpa.token_range(operand).0 == ft => {
            matches!(src.text_after(bpa, node), Some(b"++") | Some(b"--"))
        }
        _ => false,
    }
}

/// Returns true when a unary expression is a logical negation `!x`.
pub fn is_logical_not(bpa: &BPASTArtifact, src: &SourceText<'_>, node: u32) -> bool {
    if bpa.node_type(node) != NN_UNARY_EXPR {
        return false;
    }
    src.node_text(bpa, node) == Some(&b"!"[..])
}

/// Literal text, when the node is a literal and token text is available.
pub fn literal_text<'a>(bpa: &BPASTArtifact, src: &SourceText<'a>, node: u32) -> Option<&'a [u8]> {
    if bpa.node_type(node) != NN_LITERAL {
        return None;
    }
    src.node_text(bpa, node)
}
