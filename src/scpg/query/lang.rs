//! OpenHeart Datalog — a declarative query language over the SCPG relations (§10.5).
//!
//! Every SCPG layer is exposed as a set of relations (`RELATIONS`). Programs are Datalog
//! rules plus exactly one query:
//!
//! ```text
//! % transitive callers of every method named "save"
//! reaches(X, Y) :- calls(X, Y).
//! reaches(X, Z) :- calls(X, Y), reaches(Y, Z).
//! ?- symbol(T, "method", "save"), reaches(C, T), qualified_name(C, Caller).
//! ```
//!
//! - Terms: variables (`Upper` or `_name`), the wildcard `_`, integers, `"strings"`.
//! - Literals: atoms, negated atoms `!rel(..)`, comparisons `=`, `!=`, `<`, `<=`, `>`, `>=`.
//! - Recursion is evaluated bottom-up with semi-naive iteration; negation must be stratified.
//! - Rules must be range-restricted: every head, comparison and negated variable is bound by
//!   a positive atom. Built-in relations cannot be redefined.
//! - Evaluation is bounded by `EvalLimits` so a runaway program fails instead of exhausting
//!   memory. Output rows are sorted, so results are deterministic.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::{Serialize, Serializer};

use crate::scpg::query::graph::call_type_name;
use crate::scpg::query::sdg::{dependence_name, node_kind_name};
use crate::scpg::query::semantic::{pattern_name, stereotype_name};
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::query::symbols::*;

// ── Values ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    Int(u64),
    Str(Arc<str>),
}

impl Value {
    pub fn int(v: impl Into<u64>) -> Self {
        Value::Int(v.into())
    }

    pub fn str(s: &str) -> Self {
        Value::Str(Arc::from(s))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Str(s) => write!(f, "{:?}", s),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Int(i) => serializer.serialize_u64(*i),
            Value::Str(s) => serializer.serialize_str(s),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryOutput {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Set when more than `EvalLimits::max_rows` rows matched; `rows` holds the first ones.
    pub truncated: bool,
    pub total_rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalLimits {
    pub max_tuples: usize,
    pub max_rows: usize,
}

impl Default for EvalLimits {
    fn default() -> Self {
        Self {
            max_tuples: 10_000_000,
            max_rows: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryLangError {
    #[error("parse error at {line}:{col}: {message}")]
    Parse {
        line: usize,
        col: usize,
        message: String,
    },
    #[error("unknown relation '{0}'")]
    UnknownRelation(String),
    #[error("relation '{name}' takes {expected} arguments, got {found}")]
    Arity {
        name: String,
        expected: usize,
        found: usize,
    },
    #[error("unsafe rule for '{relation}': variable {variable} is not bound by a positive atom")]
    Unsafe { relation: String, variable: String },
    #[error("'{0}' is a built-in relation and cannot be redefined")]
    Shadowing(String),
    #[error("program is not stratifiable: '{0}' depends negatively on itself")]
    Unstratifiable(String),
    #[error("program has no query (`?- ...`)")]
    NoQuery,
    #[error("program has more than one query")]
    MultipleQueries,
    #[error("evaluation exceeded the limit of {0} derived tuples")]
    TupleLimit(usize),
}

// ── Relation catalog ──────────────────────────────────────────────────────────

pub struct RelationSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub description: &'static str,
}

pub const RELATIONS: &[RelationSpec] = &[
    RelationSpec { name: "symbol", columns: &["id", "kind", "name"], description: "every symbol with its kind name and simple name" },
    RelationSpec { name: "qualified_name", columns: &["id", "qualified_name"], description: "fully qualified name of a symbol" },
    RelationSpec { name: "stable_id", columns: &["id", "stable_id"], description: "SCIP-style stable symbol id" },
    RelationSpec { name: "parent", columns: &["child", "parent"], description: "lexical nesting of symbols" },
    RelationSpec { name: "modifier", columns: &["id", "modifier"], description: "declared modifiers (static, final, ...)" },
    RelationSpec { name: "visibility", columns: &["id", "visibility"], description: "public / private / protected / package" },
    RelationSpec { name: "param_count", columns: &["id", "count"], description: "declared parameter count of callables" },
    RelationSpec { name: "calls", columns: &["caller", "callee"], description: "resolved call-graph edges" },
    RelationSpec { name: "call_site", columns: &["site", "caller", "callee", "kind"], description: "resolved call sites with dispatch kind" },
    RelationSpec { name: "call_site_location", columns: &["site", "file", "line", "col"], description: "source position of each call site" },
    RelationSpec { name: "extends", columns: &["sub", "super"], description: "class / interface extension" },
    RelationSpec { name: "implements", columns: &["sub", "super"], description: "interface implementation" },
    RelationSpec { name: "uses_type", columns: &["from", "to"], description: "type references" },
    RelationSpec { name: "creates", columns: &["from", "to"], description: "instantiations" },
    RelationSpec { name: "association", columns: &["from", "to"], description: "field-based UML associations" },
    RelationSpec { name: "overrides", columns: &["method", "overridden"], description: "method overriding (name + arity)" },
    RelationSpec { name: "scc", columns: &["method", "component"], description: "call-graph strongly connected component" },
    RelationSpec { name: "recursive", columns: &["method"], description: "methods on a call-graph cycle" },
    RelationSpec { name: "located", columns: &["id", "file", "line_start", "line_end"], description: "source span of a symbol" },
    RelationSpec { name: "file", columns: &["file", "path"], description: "source files" },
    RelationSpec { name: "path_count", columns: &["method", "paths"], description: "#SAT feasible paths (ROBDD)" },
    RelationSpec { name: "cyclomatic", columns: &["method", "complexity"], description: "cyclomatic complexity" },
    RelationSpec { name: "uml_class", columns: &["class", "stereotype"], description: "UML classifiers" },
    RelationSpec { name: "design_pattern", columns: &["class", "pattern", "confidence"], description: "detected design pattern participants" },
    RelationSpec { name: "pattern_role", columns: &["instance", "class", "role"], description: "role bindings of design pattern instances" },
    RelationSpec { name: "sdg_node", columns: &["node", "method", "kind", "ast"], description: "system dependence graph nodes" },
    RelationSpec { name: "dep", columns: &["to", "from", "kind"], description: "SDG dependence edges (to depends on from)" },
    RelationSpec { name: "stmt_location", columns: &["method", "ast", "file", "line"], description: "source line of SDG statements" },
];

/// A curated starter library: `(name, description, program)`.
pub const STARTER_QUERIES: &[(&str, &str, &str)] = &[
    (
        "recursive_methods",
        "Methods that take part in direct or mutual recursion",
        "?- recursive(M), qualified_name(M, Method).",
    ),
    (
        "uncalled_methods",
        "Methods without any resolved caller",
        "called(M) :- calls(_, M).\n?- symbol(M, \"method\", _), !called(M), qualified_name(M, Method).",
    ),
    (
        "call_chains",
        "Transitive caller -> callee pairs",
        "reaches(X, Y) :- calls(X, Y).\nreaches(X, Z) :- reaches(X, Y), calls(Y, Z).\n?- reaches(A, B), qualified_name(A, From), qualified_name(B, To).",
    ),
    (
        "inheritance_closure",
        "Every (subtype, transitive supertype) pair",
        "ancestor(T, S) :- extends(T, S).\nancestor(T, S) :- implements(T, S).\nancestor(T, S) :- ancestor(T, U), ancestor(U, S).\n?- ancestor(T, S), qualified_name(T, Sub), qualified_name(S, Super).",
    ),
    (
        "overrides",
        "Overriding methods and the methods they override",
        "?- overrides(M, O), qualified_name(M, Method), qualified_name(O, Overridden).",
    ),
    (
        "pattern_instances",
        "Detected design pattern instances",
        "?- design_pattern(C, Pattern, Confidence), qualified_name(C, Class).",
    ),
    (
        "complex_methods",
        "Methods with cyclomatic complexity of at least 10",
        "?- cyclomatic(M, V), V >= 10, qualified_name(M, Method).",
    ),
    (
        "unreferenced_classes",
        "Classes no other type extends, implements, references or instantiates",
        "referenced(T) :- uses_type(_, T).\nreferenced(T) :- creates(_, T).\nreferenced(T) :- extends(_, T).\nreferenced(T) :- implements(_, T).\nreferenced(T) :- association(_, T).\n?- symbol(T, \"class\", _), !referenced(T), qualified_name(T, Class).",
    ),
];

fn relation_spec(name: &str) -> Option<&'static RelationSpec> {
    RELATIONS.iter().find(|r| r.name == name)
}

fn materialize(name: &str, s: &ScpgSnapshot) -> Vec<Vec<Value>> {
    let syms = &s.symbols;
    let n = syms.len() as u32;
    let mut out: Vec<Vec<Value>> = Vec::new();
    match name {
        "symbol" => {
            for i in 0..n {
                out.push(vec![
                    Value::int(i),
                    Value::str(kind_name(syms.kind(i).unwrap_or(0))),
                    Value::str(syms.name(i).unwrap_or("")),
                ]);
            }
        }
        "qualified_name" => {
            for i in 0..n {
                out.push(vec![Value::int(i), Value::str(syms.qualified_name(i).unwrap_or(""))]);
            }
        }
        "stable_id" => {
            for i in 0..n {
                out.push(vec![Value::int(i), Value::str(syms.stable_id(i).unwrap_or(""))]);
            }
        }
        "parent" => {
            for i in 0..n {
                if let Some(p) = syms.parent(i) {
                    out.push(vec![Value::int(i), Value::int(p)]);
                }
            }
        }
        "modifier" => {
            for i in 0..n {
                let m = syms.modifiers(i).unwrap_or(0);
                for (flag, label) in MODIFIER_NAMES {
                    if m & flag != 0 {
                        out.push(vec![Value::int(i), Value::str(label)]);
                    }
                }
            }
        }
        "visibility" => {
            for i in 0..n {
                out.push(vec![Value::int(i), Value::str(visibility_name(syms.visibility(i).unwrap_or(0)))]);
            }
        }
        "param_count" => {
            for i in 0..n {
                if syms.kind(i).map(is_callable_kind).unwrap_or(false) {
                    out.push(vec![Value::int(i), Value::int(syms.param_count(i).unwrap_or(0))]);
                }
            }
        }
        "calls" => {
            for (u, v, _) in s.call_graph.edges() {
                out.push(vec![Value::int(u), Value::int(v)]);
            }
        }
        "call_site" => {
            for (row, site) in s.call_graph.call_sites().iter().enumerate() {
                for &callee in s.call_graph.site_targets(row) {
                    out.push(vec![
                        Value::int(site.site_id),
                        Value::int(site.caller),
                        Value::int(callee),
                        Value::str(call_type_name(site.call_type)),
                    ]);
                }
            }
        }
        "call_site_location" => {
            for site in s.call_graph.call_sites() {
                if let Some(p) = s.tokens.start(site.call_token) {
                    out.push(vec![
                        Value::int(site.site_id),
                        Value::int(p.file_id),
                        Value::int(p.line),
                        Value::int(p.col),
                    ]);
                }
            }
        }
        "extends" | "implements" | "uses_type" | "creates" | "association" => {
            let wanted = match name {
                "extends" => REL_EXTENDS,
                "implements" => REL_IMPLEMENTS,
                "uses_type" => REL_USES,
                "creates" => REL_CREATES,
                _ => REL_ASSOCIATION,
            };
            for (f, t, r) in s.hierarchy.edges() {
                if r == wanted {
                    out.push(vec![Value::int(f), Value::int(t)]);
                }
            }
        }
        "overrides" => {
            for i in 0..n {
                for o in s.hierarchy.overridden_methods(syms, i) {
                    out.push(vec![Value::int(i), Value::int(o)]);
                }
            }
        }
        "scc" => {
            for m in 0..s.call_graph.node_count() as u32 {
                if let Some(c) = s.call_graph.component_of(m) {
                    out.push(vec![Value::int(m), Value::int(c)]);
                }
            }
        }
        "recursive" => {
            for m in 0..s.call_graph.node_count() as u32 {
                if s.call_graph.is_recursive(m) {
                    out.push(vec![Value::int(m)]);
                }
            }
        }
        "located" => {
            for span in s.navigation.spans() {
                out.push(vec![
                    Value::int(span.sym_id),
                    Value::int(span.file_id),
                    Value::int(span.line_start),
                    Value::int(span.line_end),
                ]);
            }
        }
        "file" => {
            for &f in s.files.file_ids() {
                if let Some(p) = s.files.path(f) {
                    out.push(vec![Value::int(f), Value::str(p)]);
                }
            }
        }
        "path_count" => {
            for f in s.paths.functions() {
                out.push(vec![Value::int(f.sym_id), Value::int(f.sat_count)]);
            }
        }
        "cyclomatic" => {
            for f in s.paths.functions() {
                out.push(vec![Value::int(f.sym_id), Value::int(f.cyclomatic)]);
            }
        }
        "uml_class" => {
            for c in &s.semantic.classes {
                out.push(vec![Value::int(c.sym_id), Value::str(stereotype_name(c.stereotype))]);
            }
        }
        "design_pattern" => {
            for p in &s.semantic.patterns {
                out.push(vec![
                    Value::int(p.class_sym),
                    Value::str(pattern_name(p.pattern_kind)),
                    Value::int(p.confidence),
                ]);
            }
        }
        "pattern_role" => {
            for p in &s.semantic.patterns {
                out.push(vec![Value::int(p.instance_id), Value::int(p.class_sym), Value::int(p.role)]);
            }
        }
        "sdg_node" => {
            for id in 0..s.sdg.node_count() as u32 {
                if let Some(node) = s.sdg.node(id) {
                    out.push(vec![
                        Value::int(id),
                        Value::int(node.method),
                        Value::str(node_kind_name(node.kind)),
                        Value::int(node.ast_node),
                    ]);
                }
            }
        }
        "dep" => {
            for to in 0..s.sdg.node_count() as u32 {
                let (froms, kinds) = s.sdg.preds(to);
                for (&from, &k) in froms.iter().zip(kinds) {
                    out.push(vec![Value::int(to), Value::int(from), Value::str(dependence_name(k))]);
                }
            }
        }
        "stmt_location" => {
            for id in 0..s.sdg.node_count() as u32 {
                let Some(node) = s.sdg.node(id) else { continue };
                if node.kind != crate::scpg::query::sdg::NODE_STATEMENT {
                    continue;
                }
                let pos = s
                    .ast
                    .token_range(node.ast_node)
                    .and_then(|(first, _)| s.tokens.start(first));
                if let Some(p) = pos {
                    out.push(vec![
                        Value::int(node.method),
                        Value::int(node.ast_node),
                        Value::int(p.file_id),
                        Value::int(p.line),
                    ]);
                }
            }
        }
        _ => {}
    }
    out
}

// ── Lexer ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Var(String),
    Wild,
    Int(u64),
    Str(String),
    LParen,
    RParen,
    Comma,
    Dot,
    Implies,
    Query,
    Bang,
    Op(CmpOp),
}

#[derive(Debug, Clone)]
struct Spanned {
    tok: Tok,
    line: usize,
    col: usize,
}

fn parse_error(line: usize, col: usize, message: impl Into<String>) -> QueryLangError {
    QueryLangError::Parse {
        line,
        col,
        message: message.into(),
    }
}

fn tokenize(src: &str) -> Result<Vec<Spanned>, QueryLangError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);
    let advance = |i: &mut usize, line: &mut usize, col: &mut usize, c: char| {
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let (tl, tc) = (line, col);
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            advance(&mut i, &mut line, &mut col, c);
            continue;
        }
        if c == '%' || (c == '/' && next == Some('/')) {
            while i < chars.len() && chars[i] != '\n' {
                let ch = chars[i];
                advance(&mut i, &mut line, &mut col, ch);
            }
            continue;
        }
        let simple = match (c, next) {
            ('(', _) => Some((Tok::LParen, 1)),
            (')', _) => Some((Tok::RParen, 1)),
            (',', _) => Some((Tok::Comma, 1)),
            ('.', _) => Some((Tok::Dot, 1)),
            (':', Some('-')) => Some((Tok::Implies, 2)),
            ('?', Some('-')) => Some((Tok::Query, 2)),
            ('!', Some('=')) => Some((Tok::Op(CmpOp::Ne), 2)),
            ('!', _) => Some((Tok::Bang, 1)),
            ('=', _) => Some((Tok::Op(CmpOp::Eq), 1)),
            ('<', Some('=')) => Some((Tok::Op(CmpOp::Le), 2)),
            ('<', _) => Some((Tok::Op(CmpOp::Lt), 1)),
            ('>', Some('=')) => Some((Tok::Op(CmpOp::Ge), 2)),
            ('>', _) => Some((Tok::Op(CmpOp::Gt), 1)),
            _ => None,
        };
        if let Some((tok, width)) = simple {
            for _ in 0..width {
                let ch = chars[i];
                advance(&mut i, &mut line, &mut col, ch);
            }
            toks.push(Spanned { tok, line: tl, col: tc });
            continue;
        }
        if c == '"' {
            advance(&mut i, &mut line, &mut col, c);
            let mut s = String::new();
            loop {
                let Some(&ch) = chars.get(i) else {
                    return Err(parse_error(tl, tc, "unterminated string literal"));
                };
                advance(&mut i, &mut line, &mut col, ch);
                match ch {
                    '"' => break,
                    '\\' => {
                        let Some(&esc) = chars.get(i) else {
                            return Err(parse_error(tl, tc, "unterminated string literal"));
                        };
                        advance(&mut i, &mut line, &mut col, esc);
                        s.push(match esc {
                            'n' => '\n',
                            't' => '\t',
                            '\\' => '\\',
                            '"' => '"',
                            other => return Err(parse_error(line, col, format!("unknown escape '\\{}'", other))),
                        });
                    }
                    other => s.push(other),
                }
            }
            toks.push(Spanned { tok: Tok::Str(s), line: tl, col: tc });
            continue;
        }
        if c.is_ascii_digit() {
            let mut digits = String::new();
            while let Some(&d) = chars.get(i).filter(|d| d.is_ascii_digit()) {
                digits.push(d);
                advance(&mut i, &mut line, &mut col, d);
            }
            let v = digits
                .parse::<u64>()
                .map_err(|_| parse_error(tl, tc, format!("integer '{}' is out of range", digits)))?;
            toks.push(Spanned { tok: Tok::Int(v), line: tl, col: tc });
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut word = String::new();
            while let Some(&w) = chars.get(i).filter(|w| w.is_alphanumeric() || **w == '_') {
                word.push(w);
                advance(&mut i, &mut line, &mut col, w);
            }
            let tok = if word == "_" {
                Tok::Wild
            } else if word.starts_with('_') || word.chars().next().map(|f| f.is_uppercase()).unwrap_or(false) {
                Tok::Var(word)
            } else {
                Tok::Ident(word)
            };
            toks.push(Spanned { tok, line: tl, col: tc });
            continue;
        }
        return Err(parse_error(tl, tc, format!("unexpected character '{}'", c)));
    }
    Ok(toks)
}

// ── Parser ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Var(String),
    Wildcard,
    Const(Value),
}

#[derive(Debug, Clone)]
struct Atom {
    name: String,
    args: Vec<Term>,
}

#[derive(Debug, Clone)]
enum Literal {
    Pos(Atom),
    Neg(Atom),
    Cmp(Term, CmpOp, Term),
}

#[derive(Debug, Clone)]
struct Rule {
    head: Atom,
    body: Vec<Literal>,
}

struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
    end: (usize, usize),
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|s| &s.tok)
    }

    fn here(&self) -> (usize, usize) {
        self.toks.get(self.pos).map(|s| (s.line, s.col)).unwrap_or(self.end)
    }

    fn error(&self, message: impl Into<String>) -> QueryLangError {
        let (line, col) = self.here();
        parse_error(line, col, message)
    }

    fn expect(&mut self, want: &Tok, what: &str) -> Result<(), QueryLangError> {
        if self.peek() == Some(want) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected {}", what)))
        }
    }

    fn program(&mut self) -> Result<(Vec<Rule>, Vec<Vec<Literal>>), QueryLangError> {
        let mut rules = Vec::new();
        let mut queries = Vec::new();
        while self.peek().is_some() {
            if self.peek() == Some(&Tok::Query) {
                self.pos += 1;
                let body = self.body()?;
                self.expect(&Tok::Dot, "'.' after the query")?;
                queries.push(body);
            } else {
                let head = self.atom()?;
                let body = if self.peek() == Some(&Tok::Implies) {
                    self.pos += 1;
                    self.body()?
                } else {
                    Vec::new()
                };
                self.expect(&Tok::Dot, "'.' after the rule")?;
                rules.push(Rule { head, body });
            }
        }
        Ok((rules, queries))
    }

    fn body(&mut self) -> Result<Vec<Literal>, QueryLangError> {
        let mut lits = vec![self.literal()?];
        while self.peek() == Some(&Tok::Comma) {
            self.pos += 1;
            lits.push(self.literal()?);
        }
        Ok(lits)
    }

    fn literal(&mut self) -> Result<Literal, QueryLangError> {
        if self.peek() == Some(&Tok::Bang) {
            self.pos += 1;
            return Ok(Literal::Neg(self.atom()?));
        }
        let is_atom = matches!(self.peek(), Some(Tok::Ident(_)))
            && self.toks.get(self.pos + 1).map(|s| s.tok == Tok::LParen).unwrap_or(false);
        if is_atom {
            return Ok(Literal::Pos(self.atom()?));
        }
        let lhs = self.term()?;
        let op = match self.peek() {
            Some(Tok::Op(op)) => *op,
            _ => return Err(self.error("expected an atom or a comparison")),
        };
        self.pos += 1;
        let rhs = self.term()?;
        Ok(Literal::Cmp(lhs, op, rhs))
    }

    fn atom(&mut self) -> Result<Atom, QueryLangError> {
        let name = match self.peek() {
            Some(Tok::Ident(name)) => name.clone(),
            _ => return Err(self.error("expected a relation name")),
        };
        self.pos += 1;
        self.expect(&Tok::LParen, "'('")?;
        let mut args = Vec::new();
        if self.peek() != Some(&Tok::RParen) {
            args.push(self.term()?);
            while self.peek() == Some(&Tok::Comma) {
                self.pos += 1;
                args.push(self.term()?);
            }
        }
        self.expect(&Tok::RParen, "')'")?;
        Ok(Atom { name, args })
    }

    fn term(&mut self) -> Result<Term, QueryLangError> {
        let term = match self.peek() {
            Some(Tok::Var(v)) => Term::Var(v.clone()),
            Some(Tok::Wild) => Term::Wildcard,
            Some(Tok::Int(i)) => Term::Const(Value::Int(*i)),
            Some(Tok::Str(s)) => Term::Const(Value::str(s)),
            _ => return Err(self.error("expected a variable, '_', an integer or a string")),
        };
        self.pos += 1;
        Ok(term)
    }
}

// ── Compilation ───────────────────────────────────────────────────────────────

const QUERY_RELATION: &str = "?query";

#[derive(Debug, Clone)]
enum Slot {
    Var(usize),
    Const(Value),
}

#[derive(Debug, Clone)]
enum ArgOp {
    Bind(usize),
    Check(usize),
    Const(Value),
    Ignore,
}

#[derive(Debug, Clone)]
enum Step {
    Scan {
        rel: String,
        args: Vec<ArgOp>,
        index_col: Option<usize>,
    },
    Neg {
        rel: String,
        args: Vec<ArgOp>,
        index_col: Option<usize>,
    },
    Cmp {
        lhs: Slot,
        op: CmpOp,
        rhs: Slot,
    },
}

#[derive(Debug, Clone)]
struct CompiledRule {
    head_rel: String,
    head: Vec<Slot>,
    steps: Vec<Step>,
    var_count: usize,
}

fn compile_rule(head_rel: &str, head_args: &[Term], body: &[Literal]) -> Result<CompiledRule, QueryLangError> {
    let mut var_ids: HashMap<String, usize> = HashMap::new();
    let mut bound: Vec<bool> = Vec::new();
    let mut steps = Vec::new();
    let mut pending: Vec<&Literal> = Vec::new();
    let unsafe_var = |v: &str| QueryLangError::Unsafe {
        relation: head_rel.to_string(),
        variable: v.to_string(),
    };

    fn var_id(name: &str, var_ids: &mut HashMap<String, usize>, bound: &mut Vec<bool>) -> usize {
        if let Some(&id) = var_ids.get(name) {
            return id;
        }
        let id = var_ids.len();
        var_ids.insert(name.to_string(), id);
        bound.push(false);
        id
    }

    fn literal_ready(lit: &Literal, var_ids: &HashMap<String, usize>, bound: &[bool]) -> bool {
        let is_bound = |t: &Term| match t {
            Term::Var(v) => var_ids.get(v).map(|&id| bound[id]).unwrap_or(false),
            _ => true,
        };
        match lit {
            Literal::Neg(a) => a.args.iter().all(is_bound),
            Literal::Cmp(l, _, r) => is_bound(l) && is_bound(r),
            Literal::Pos(_) => true,
        }
    }

    let flush = |pending: &mut Vec<&Literal>,
                 steps: &mut Vec<Step>,
                 var_ids: &HashMap<String, usize>,
                 bound: &[bool]|
     -> Result<(), QueryLangError> {
        let mut keep = Vec::new();
        for lit in pending.drain(..) {
            if !literal_ready(lit, var_ids, bound) {
                keep.push(lit);
                continue;
            }
            let slot = |t: &Term| -> Result<Slot, QueryLangError> {
                match t {
                    Term::Var(v) => Ok(Slot::Var(var_ids[v])),
                    Term::Const(c) => Ok(Slot::Const(c.clone())),
                    Term::Wildcard => Err(QueryLangError::Unsafe {
                        relation: head_rel.to_string(),
                        variable: "_".to_string(),
                    }),
                }
            };
            match lit {
                Literal::Cmp(l, op, r) => steps.push(Step::Cmp {
                    lhs: slot(l)?,
                    op: *op,
                    rhs: slot(r)?,
                }),
                Literal::Neg(a) => {
                    let args: Vec<ArgOp> = a
                        .args
                        .iter()
                        .map(|t| match t {
                            Term::Var(v) => ArgOp::Check(var_ids[v]),
                            Term::Const(c) => ArgOp::Const(c.clone()),
                            Term::Wildcard => ArgOp::Ignore,
                        })
                        .collect();
                    let index_col = args.iter().position(|a| !matches!(a, ArgOp::Ignore));
                    steps.push(Step::Neg {
                        rel: a.name.clone(),
                        args,
                        index_col,
                    });
                }
                Literal::Pos(_) => {}
            }
        }
        *pending = keep;
        Ok(())
    };

    for lit in body {
        match lit {
            Literal::Pos(atom) => {
                let mut args = Vec::with_capacity(atom.args.len());
                let mut index_col = None;
                let mut newly_bound = Vec::new();
                for (j, t) in atom.args.iter().enumerate() {
                    let op = match t {
                        Term::Var(v) => {
                            let id = var_id(v, &mut var_ids, &mut bound);
                            if bound[id] {
                                if index_col.is_none() {
                                    index_col = Some(j);
                                }
                                ArgOp::Check(id)
                            } else if newly_bound.contains(&id) {
                                ArgOp::Check(id)
                            } else {
                                newly_bound.push(id);
                                ArgOp::Bind(id)
                            }
                        }
                        Term::Const(c) => {
                            if index_col.is_none() {
                                index_col = Some(j);
                            }
                            ArgOp::Const(c.clone())
                        }
                        Term::Wildcard => ArgOp::Ignore,
                    };
                    args.push(op);
                }
                for id in newly_bound {
                    bound[id] = true;
                }
                steps.push(Step::Scan {
                    rel: atom.name.clone(),
                    args,
                    index_col,
                });
            }
            other => {
                if let Literal::Neg(a) = other {
                    for t in &a.args {
                        if let Term::Var(v) = t {
                            var_id(v, &mut var_ids, &mut bound);
                        }
                    }
                }
                if let Literal::Cmp(l, _, r) = other {
                    for t in [l, r] {
                        if let Term::Var(v) = t {
                            var_id(v, &mut var_ids, &mut bound);
                        }
                    }
                }
                pending.push(other);
            }
        }
        flush(&mut pending, &mut steps, &var_ids, bound.as_slice())?;
    }
    flush(&mut pending, &mut steps, &var_ids, bound.as_slice())?;
    if let Some(lit) = pending.first() {
        let unbound = match lit {
            Literal::Neg(a) => a.args.iter().find_map(|t| match t {
                Term::Var(v) if !var_ids.get(v).map(|&id| bound[id]).unwrap_or(false) => Some(v.clone()),
                _ => None,
            }),
            Literal::Cmp(l, _, r) => [l, r].into_iter().find_map(|t| match t {
                Term::Var(v) if !var_ids.get(v).map(|&id| bound[id]).unwrap_or(false) => Some(v.clone()),
                _ => None,
            }),
            Literal::Pos(_) => None,
        };
        return Err(unsafe_var(&unbound.unwrap_or_else(|| "?".to_string())));
    }

    let mut head = Vec::with_capacity(head_args.len());
    for t in head_args {
        head.push(match t {
            Term::Var(v) => match var_ids.get(v) {
                Some(&id) if bound[id] => Slot::Var(id),
                _ => return Err(unsafe_var(v)),
            },
            Term::Const(c) => Slot::Const(c.clone()),
            Term::Wildcard => return Err(unsafe_var("_")),
        });
    }
    Ok(CompiledRule {
        head_rel: head_rel.to_string(),
        head,
        steps,
        var_count: var_ids.len(),
    })
}

// ── Evaluation ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Relation {
    tuples: Vec<Vec<Value>>,
    set: HashSet<Vec<Value>>,
    indexes: HashMap<usize, (usize, HashMap<Value, Vec<u32>>)>,
}

impl Relation {
    fn from_tuples(tuples: Vec<Vec<Value>>) -> Self {
        let mut rel = Relation::default();
        for t in tuples {
            rel.insert(t);
        }
        rel
    }

    fn insert(&mut self, t: Vec<Value>) -> bool {
        if self.set.contains(&t) {
            return false;
        }
        self.set.insert(t.clone());
        self.tuples.push(t);
        true
    }

    fn ensure_index(&mut self, col: usize) {
        let entry = self.indexes.entry(col).or_insert_with(|| (0, HashMap::new()));
        for i in entry.0..self.tuples.len() {
            if let Some(v) = self.tuples[i].get(col) {
                entry.1.entry(v.clone()).or_default().push(i as u32);
            }
        }
        entry.0 = self.tuples.len();
    }

    fn postings(&self, col: usize, value: &Value) -> Option<&[u32]> {
        self.indexes
            .get(&col)
            .map(|(_, m)| m.get(value).map(|v| v.as_slice()).unwrap_or(&[]))
    }
}

fn compare(a: &Value, op: CmpOp, b: &Value) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        _ => match (a, b) {
            (Value::Int(x), Value::Int(y)) => match op {
                CmpOp::Lt => x < y,
                CmpOp::Le => x <= y,
                CmpOp::Gt => x > y,
                _ => x >= y,
            },
            (Value::Str(x), Value::Str(y)) => match op {
                CmpOp::Lt => x < y,
                CmpOp::Le => x <= y,
                CmpOp::Gt => x > y,
                _ => x >= y,
            },
            _ => false,
        },
    }
}

struct Evaluator {
    rels: HashMap<String, Relation>,
    limits: EvalLimits,
    derived_total: usize,
}

impl Evaluator {
    fn prepare_indexes(&mut self, rules: &[&CompiledRule]) {
        for rule in rules {
            for step in &rule.steps {
                if let Step::Scan { rel, index_col: Some(c), .. } | Step::Neg { rel, index_col: Some(c), .. } = step {
                    if let Some(r) = self.rels.get_mut(rel) {
                        r.ensure_index(*c);
                    }
                }
            }
        }
    }

    fn resolve<'b>(slot: &'b Slot, bindings: &'b [Option<Value>]) -> Option<&'b Value> {
        match slot {
            Slot::Var(id) => bindings[*id].as_ref(),
            Slot::Const(c) => Some(c),
        }
    }

    fn arg_value<'b>(op: &'b ArgOp, bindings: &'b [Option<Value>]) -> Option<&'b Value> {
        match op {
            ArgOp::Check(id) => bindings[*id].as_ref(),
            ArgOp::Const(c) => Some(c),
            _ => None,
        }
    }

    fn join(
        &self,
        rule: &CompiledRule,
        step_idx: usize,
        bindings: &mut [Option<Value>],
        ranges: &[(usize, usize)],
        out: &mut Vec<Vec<Value>>,
    ) -> Result<(), QueryLangError> {
        let Some(step) = rule.steps.get(step_idx) else {
            // Range restriction guarantees every head variable is bound here.
            let tuple: Option<Vec<Value>> = rule
                .head
                .iter()
                .map(|s| Self::resolve(s, bindings).cloned())
                .collect();
            if let Some(tuple) = tuple {
                out.push(tuple);
            }
            if self.derived_total + out.len() > self.limits.max_tuples {
                return Err(QueryLangError::TupleLimit(self.limits.max_tuples));
            }
            return Ok(());
        };
        match step {
            Step::Scan { rel, args, index_col } => {
                let Some(r) = self.rels.get(rel) else {
                    return Ok(());
                };
                let (lo, hi) = ranges[step_idx];
                let hi = hi.min(r.tuples.len());
                let indexed = index_col.and_then(|c| {
                    let v = Self::arg_value(&args[c], bindings)?;
                    r.postings(c, v)
                });
                let candidates: Box<dyn Iterator<Item = usize> + '_> = match indexed {
                    Some(postings) => {
                        let start = postings.partition_point(|&t| (t as usize) < lo);
                        let end = postings.partition_point(|&t| (t as usize) < hi);
                        Box::new(postings[start..end.max(start)].iter().map(|&t| t as usize))
                    }
                    None => Box::new(lo..hi.max(lo)),
                };
                // Candidates borrow only the relation, so bindings stay mutable below.
                for t in candidates {
                    let tuple = &r.tuples[t];
                    let mut ok = true;
                    let mut bound_here = Vec::new();
                    for (j, op) in args.iter().enumerate() {
                        match op {
                            ArgOp::Bind(id) => {
                                bindings[*id] = Some(tuple[j].clone());
                                bound_here.push(*id);
                            }
                            ArgOp::Check(id) => {
                                if bindings[*id].as_ref() != Some(&tuple[j]) {
                                    ok = false;
                                    break;
                                }
                            }
                            ArgOp::Const(c) => {
                                if &tuple[j] != c {
                                    ok = false;
                                    break;
                                }
                            }
                            ArgOp::Ignore => {}
                        }
                    }
                    if ok {
                        self.join(rule, step_idx + 1, bindings, ranges, out)?;
                    }
                    for id in bound_here {
                        bindings[id] = None;
                    }
                }
                Ok(())
            }
            Step::Neg { rel, args, index_col } => {
                let exists = match self.rels.get(rel) {
                    None => false,
                    Some(r) => {
                        let matches = |tuple: &Vec<Value>| {
                            args.iter().enumerate().all(|(j, op)| match Self::arg_value(op, bindings) {
                                Some(v) => &tuple[j] == v,
                                None => true,
                            })
                        };
                        let indexed = index_col.and_then(|c| {
                            let v = Self::arg_value(&args[c], bindings)?;
                            r.postings(c, v)
                        });
                        match indexed {
                            Some(postings) => postings.iter().any(|&t| matches(&r.tuples[t as usize])),
                            None => r.tuples.iter().any(matches),
                        }
                    }
                };
                if !exists {
                    self.join(rule, step_idx + 1, bindings, ranges, out)?;
                }
                Ok(())
            }
            Step::Cmp { lhs, op, rhs } => {
                let holds = match (Self::resolve(lhs, bindings), Self::resolve(rhs, bindings)) {
                    (Some(a), Some(b)) => compare(a, *op, b),
                    _ => false,
                };
                if holds {
                    self.join(rule, step_idx + 1, bindings, ranges, out)?;
                }
                Ok(())
            }
        }
    }

    fn eval_rule(&self, rule: &CompiledRule, ranges: &[(usize, usize)]) -> Result<Vec<Vec<Value>>, QueryLangError> {
        let mut bindings = vec![None; rule.var_count];
        let mut out = Vec::new();
        self.join(rule, 0, &mut bindings, ranges, &mut out)?;
        Ok(out)
    }

    fn full_ranges(&self, rule: &CompiledRule) -> Vec<(usize, usize)> {
        rule.steps
            .iter()
            .map(|s| match s {
                Step::Scan { rel, .. } => (0, self.rels.get(rel).map(|r| r.tuples.len()).unwrap_or(0)),
                _ => (0, 0),
            })
            .collect()
    }

    fn eval_stratum(&mut self, rules: &[&CompiledRule], stratum_rels: &HashSet<String>) -> Result<(), QueryLangError> {
        let mut deltas: HashMap<String, (usize, usize)> = HashMap::new();
        let mut first = true;
        loop {
            self.prepare_indexes(rules);
            let mut derived: Vec<(String, Vec<Vec<Value>>)> = Vec::new();
            for rule in rules {
                let full = self.full_ranges(rule);
                if first {
                    derived.push((rule.head_rel.clone(), self.eval_rule(rule, &full)?));
                    continue;
                }
                for (p, step) in rule.steps.iter().enumerate() {
                    let Step::Scan { rel, .. } = step else { continue };
                    if !stratum_rels.contains(rel) {
                        continue;
                    }
                    let Some(&(dlo, dhi)) = deltas.get(rel) else { continue };
                    if dlo >= dhi {
                        continue;
                    }
                    let mut ranges = full.clone();
                    ranges[p] = (dlo, dhi);
                    derived.push((rule.head_rel.clone(), self.eval_rule(rule, &ranges)?));
                }
            }
            first = false;

            let before: HashMap<String, usize> = stratum_rels
                .iter()
                .map(|r| (r.clone(), self.rels.get(r).map(|x| x.tuples.len()).unwrap_or(0)))
                .collect();
            for (rel, tuples) in derived {
                let target = self.rels.entry(rel).or_default();
                for t in tuples {
                    if target.insert(t) {
                        self.derived_total += 1;
                    }
                }
                if self.derived_total > self.limits.max_tuples {
                    return Err(QueryLangError::TupleLimit(self.limits.max_tuples));
                }
            }
            let mut changed = false;
            deltas.clear();
            for r in stratum_rels {
                let after = self.rels.get(r).map(|x| x.tuples.len()).unwrap_or(0);
                let b = before.get(r).copied().unwrap_or(0);
                if after > b {
                    changed = true;
                }
                deltas.insert(r.clone(), (b, after));
            }
            if !changed {
                return Ok(());
            }
        }
    }
}

fn atoms_of(body: &[Literal]) -> impl Iterator<Item = (&Atom, bool)> {
    body.iter().filter_map(|l| match l {
        Literal::Pos(a) => Some((a, false)),
        Literal::Neg(a) => Some((a, true)),
        Literal::Cmp(..) => None,
    })
}

/// Parses, checks and evaluates a program against a snapshot.
pub fn run_program(snapshot: &ScpgSnapshot, source: &str, limits: EvalLimits) -> Result<QueryOutput, QueryLangError> {
    let toks = tokenize(source)?;
    let end = toks.last().map(|t| (t.line, t.col + 1)).unwrap_or((1, 1));
    let mut parser = Parser { toks, pos: 0, end };
    let (rules, mut queries) = parser.program()?;
    let query = match queries.len() {
        0 => return Err(QueryLangError::NoQuery),
        1 => queries.remove(0),
        _ => return Err(QueryLangError::MultipleQueries),
    };

    // Arity table: built-ins first, then rule heads.
    let mut arity: HashMap<String, usize> = RELATIONS.iter().map(|r| (r.name.to_string(), r.columns.len())).collect();
    let mut idb: Vec<String> = Vec::new();
    for rule in &rules {
        let name = &rule.head.name;
        if relation_spec(name).is_some() {
            return Err(QueryLangError::Shadowing(name.clone()));
        }
        match arity.get(name) {
            Some(&a) if a != rule.head.args.len() => {
                return Err(QueryLangError::Arity {
                    name: name.clone(),
                    expected: a,
                    found: rule.head.args.len(),
                })
            }
            Some(_) => {}
            None => {
                arity.insert(name.clone(), rule.head.args.len());
                idb.push(name.clone());
            }
        }
    }
    let bodies = rules.iter().map(|r| r.body.as_slice()).chain(std::iter::once(query.as_slice()));
    for body in bodies.clone() {
        for (atom, _) in atoms_of(body) {
            match arity.get(&atom.name) {
                None => return Err(QueryLangError::UnknownRelation(atom.name.clone())),
                Some(&a) if a != atom.args.len() => {
                    return Err(QueryLangError::Arity {
                        name: atom.name.clone(),
                        expected: a,
                        found: atom.args.len(),
                    })
                }
                _ => {}
            }
        }
    }

    // Stratification: stratum(head) ≥ stratum(pos body), > stratum(neg body).
    let idb_set: HashSet<String> = idb.iter().cloned().collect();
    let mut stratum: HashMap<String, usize> = idb.iter().map(|n| (n.clone(), 0)).collect();
    let max_stratum = idb.len();
    loop {
        let mut changed = false;
        for rule in &rules {
            let mut need = stratum[&rule.head.name];
            for (atom, negated) in atoms_of(&rule.body) {
                if let Some(&s) = stratum.get(&atom.name) {
                    need = need.max(if negated { s + 1 } else { s });
                }
            }
            if need > stratum[&rule.head.name] {
                if need > max_stratum {
                    return Err(QueryLangError::Unstratifiable(rule.head.name.clone()));
                }
                stratum.insert(rule.head.name.clone(), need);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Compile rules and the query.
    let mut compiled: Vec<(usize, CompiledRule)> = Vec::new();
    for rule in &rules {
        compiled.push((stratum[&rule.head.name], compile_rule(&rule.head.name, &rule.head.args, &rule.body)?));
    }
    let mut columns: Vec<String> = Vec::new();
    for (atom, negated) in atoms_of(&query) {
        if negated {
            continue;
        }
        for t in &atom.args {
            if let Term::Var(v) = t {
                if !v.starts_with('_') && !columns.contains(v) {
                    columns.push(v.clone());
                }
            }
        }
    }
    let head_terms: Vec<Term> = columns.iter().map(|c| Term::Var(c.clone())).collect();
    let query_rule = compile_rule(QUERY_RELATION, &head_terms, &query)?;
    let query_stratum = stratum.values().copied().max().map(|m| m + 1).unwrap_or(0);
    compiled.push((query_stratum, query_rule));

    // Materialize only the built-in relations the program references.
    let mut rels: HashMap<String, Relation> = HashMap::new();
    for body in bodies {
        for (atom, _) in atoms_of(body) {
            if !idb_set.contains(&atom.name) {
                rels.entry(atom.name.clone())
                    .or_insert_with(|| Relation::from_tuples(materialize(&atom.name, snapshot)));
            }
        }
    }
    for name in &idb {
        rels.entry(name.clone()).or_default();
    }
    rels.entry(QUERY_RELATION.to_string()).or_default();

    let mut evaluator = Evaluator {
        rels,
        limits,
        derived_total: 0,
    };
    for level in 0..=query_stratum {
        let rules_here: Vec<&CompiledRule> = compiled.iter().filter(|(s, _)| *s == level).map(|(_, r)| r).collect();
        if rules_here.is_empty() {
            continue;
        }
        let stratum_rels: HashSet<String> = rules_here.iter().map(|r| r.head_rel.clone()).collect();
        evaluator.eval_stratum(&rules_here, &stratum_rels)?;
    }

    let mut rows = evaluator
        .rels
        .remove(QUERY_RELATION)
        .map(|r| r.tuples)
        .unwrap_or_default();
    rows.sort();
    let total_rows = rows.len();
    let truncated = total_rows > limits.max_rows;
    rows.truncate(limits.max_rows);
    Ok(QueryOutput {
        columns,
        rows,
        truncated,
        total_rows,
    })
}
