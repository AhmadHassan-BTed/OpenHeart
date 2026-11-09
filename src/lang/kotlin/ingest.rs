use tree_sitter::{Language, Node};

use crate::core::types::source::{FileDiagnostics, MAX_RECORDED_ERROR_RANGES};
use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

/// Phase 1 adapter for tree-sitter-kotlin (fwcd grammar, 0.3.x).
///
/// The tables cover every named leaf kind and every anonymous token of the grammar's
/// `node-types.json`. Keyword typing follows the Kotlin specification and the Kotlin lexical
/// frontend: hard keywords and the soft / modifier keywords the grammar only ever produces in
/// keyword position (`override`, `by`, `where`, `field:`, ...) are `Keyword`; the soft keywords
/// the grammar also accepts as identifiers (`data`, `value`, `inner`, `expect`, `actual`, `get`,
/// `set` inside `simple_identifier`) are `SoftKeyword`, for consumers to resolve from context.
///
/// String and character literals are atomic: the grammar hides string fragments and quote
/// tokens (external scanner), so a literal is emitted as one token that keeps its full text,
/// `${...}` templates included. `long_literal` / `unsigned_literal` (`10L`, `7u`) are atomic
/// for the same reason, and so is `shebang_line`.
///
/// The grammar declares no fields, so annotation names cannot be singled out through the
/// walker's `name`-field rule; they are emitted as `Identifier` inside the annotation's
/// `user_type`, and Phase 2 marks the annotation node itself.
pub struct KotlinLanguageAdapter;

impl KotlinLanguageAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KotlinLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for KotlinLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::Kotlin
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_kotlin::language()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            // ── Named leaves ──
            "simple_identifier" | "type_identifier" => TokenType::Identifier,
            // `name@` (statement / lambda label) and the label of `return@name`, `break@name`.
            "label" => TokenType::LabeledStmt,
            "integer_literal" | "hex_literal" | "bin_literal" | "long_literal"
            | "unsigned_literal" => TokenType::IntegerLiteral,
            "real_literal" => TokenType::FloatLiteral,
            "string_literal" | "interpolated_identifier" | "interpolated_expression" => {
                TokenType::StringLiteral
            }
            "character_literal" | "character_escape_seq" => TokenType::CharLiteral,
            "boolean_literal" => TokenType::BooleanLiteral,
            "line_comment" | "shebang_line" => TokenType::CommentLine,
            "multiline_comment" => TokenType::CommentBlock,
            // Single-keyword modifier rules the grammar reduces to one token (`const`,
            // `reified`); the other `*_modifier` rules wrap an anonymous keyword token.
            "property_modifier" | "reification_modifier" | "class_modifier"
            | "member_modifier" | "visibility_modifier" | "function_modifier"
            | "inheritance_modifier" | "parameter_modifier" | "platform_modifier"
            | "variance_modifier" => TokenType::Keyword,

            // ── Literal pieces (only reachable inside the atomic literals above) ──
            "$" | "${" => TokenType::StringLiteral,
            "'" | "\\u" => TokenType::CharLiteral,
            "L" => TokenType::IntegerLiteral,
            "#!" => TokenType::CommentLine,
            "true" | "false" => TokenType::BooleanLiteral,
            "null" => TokenType::NullLiteral,

            // ── Operators ──
            "+" | "-" | "*" | "/" | "%" | "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "++" | "--"
            | "&&" | "||" | "!" | "!!" | "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">="
            | "?:" | "?." | "::" | ".." | "..<" | "->" | "&" | "!in" | "!is" => {
                TokenType::Operator
            }

            // ── Punctuation ──
            "(" | ")" | "[" | "]" | "{" | "}" | "," | "." | ":" | ";" | "@" | ".*" => {
                TokenType::Punctuation
            }

            // ── Hard keywords (Kotlin specification, "Keywords and operators") ──
            "as" | "as?" | "break" | "break@" | "class" | "continue" | "continue@" | "do"
            | "else" | "for" | "fun" | "if" | "in" | "interface" | "is" | "object" | "package"
            | "return" | "return@" | "super" | "super@" | "this" | "this@" | "throw" | "try"
            | "typealias" | "typeof" | "val" | "var" | "when" | "while" => TokenType::Keyword,

            // ── Soft and modifier keywords the grammar only produces in keyword position ──
            "abstract" | "annotation" | "by" | "catch" | "companion" | "constructor"
            | "crossinline" | "delegate" | "dynamic" | "enum" | "external" | "field" | "file"
            | "final" | "finally" | "import" | "infix" | "init" | "inline" | "internal"
            | "lateinit" | "noinline" | "open" | "operator" | "out" | "override" | "param"
            | "private" | "property" | "protected" | "public" | "receiver" | "sealed"
            | "setparam" | "suspend" | "tailrec" | "vararg" | "where" | "const" | "reified" => {
                TokenType::Keyword
            }

            // ── Soft keywords the grammar also accepts as `simple_identifier` ──
            "actual" | "data" | "expect" | "get" | "inner" | "set" | "value" => {
                TokenType::SoftKeyword
            }

            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        self.map_node_type(kind) != TokenType::Unknown
    }

    fn is_atomic(&self, kind: &str) -> bool {
        matches!(
            kind,
            "string_literal"
                | "character_literal"
                | "long_literal"
                | "unsigned_literal"
                | "boolean_literal"
                | "shebang_line"
        )
    }
}

/// Syntax-error accounting of one parsed Kotlin file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KotlinSyntaxErrorScan {
    /// Number of ERROR nodes (regions the parser could not fit to the grammar).
    pub error_count: u32,
    /// Number of MISSING nodes (zero-width tokens the parser inserted to recover).
    pub missing_count: u32,
    /// Byte ranges `[start, end)` of the first `MAX_RECORDED_ERROR_RANGES` ERROR / MISSING
    /// nodes, in document order.
    pub ranges: Vec<(u32, u32)>,
}

impl KotlinSyntaxErrorScan {
    /// True when the tree contains neither ERROR nor MISSING nodes.
    pub fn is_clean(&self) -> bool {
        self.error_count == 0 && self.missing_count == 0
    }

    /// Overwrites the ERROR / MISSING accounting of `diagnostics` with this scan, so a file
    /// re-parsed outside the Phase 1 walker reports exactly what its tree contains.
    pub fn store_into(&self, diagnostics: &mut FileDiagnostics) {
        diagnostics.error_count = self.error_count;
        diagnostics.missing_count = self.missing_count;
        diagnostics.error_ranges.clear();
        for &(start, end) in &self.ranges {
            diagnostics.record_error_range(start as usize, end as usize);
        }
    }
}

/// Counts the ERROR and MISSING nodes of a Kotlin syntax tree.
///
/// The Phase 1 walker records the same counts while it emits tokens; this scan serves callers
/// that parse Kotlin without walking for tokens (Phase 2 rebuilds, incremental re-parses in the
/// server) and need the identical accounting. The traversal uses one `TreeCursor`, so its
/// native stack depth is constant. Subtrees whose root reports `has_error() == false` contain
/// neither kind of node and are skipped.
pub fn scan_syntax_errors(root: Node) -> KotlinSyntaxErrorScan {
    let mut scan = KotlinSyntaxErrorScan::default();
    if !root.has_error() {
        return scan;
    }

    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let mut descend = node.has_error();

        if node.is_missing() {
            scan.missing_count = scan.missing_count.saturating_add(1);
            record_range(&mut scan, node);
            descend = false;
        } else if node.kind() == "ERROR" {
            scan.error_count = scan.error_count.saturating_add(1);
            record_range(&mut scan, node);
        }

        if descend && cursor.goto_first_child() {
            continue;
        }

        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return scan;
            }
        }
    }
}

fn record_range(scan: &mut KotlinSyntaxErrorScan, node: Node) {
    if scan.ranges.len() < MAX_RECORDED_ERROR_RANGES {
        let start = u32::try_from(node.start_byte()).unwrap_or(u32::MAX);
        let end = u32::try_from(node.end_byte()).unwrap_or(u32::MAX);
        scan.ranges.push((start, end));
    }
}
