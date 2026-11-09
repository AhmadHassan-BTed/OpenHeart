//! Phase 1 adapter for tree-sitter-c, and the C-family token rules shared with the C++
//! frontend.
//!
//! The tables cover every named kind and every anonymous token of tree-sitter-c 0.20.8
//! (`src/node-types.json`): leaf kinds are classified, interior rules are listed
//! explicitly as `Unknown` (they only reach the walker as leaves after error recovery),
//! and the hidden supertypes (`_declarator`, `_expression`, ...) never occur in a tree.
//!
//! Classification that needs the token text is done in `refine_token_type`:
//! * `comment` covers `//` and `/* */`; Doxygen forms (`///`, `//!`, `/**`, `/*!`) are
//!   documentation comments;
//! * `number_literal` covers integers and floating-point literals (decimal, hex with
//!   `p` exponents, digit separators, suffixes);
//! * `primitive_type` includes the `<stddef.h>` / `<stdint.h>` typedef names (`size_t`,
//!   `uint32_t`, ...), which are identifiers, not keywords.

use tree_sitter::{Language, Node};

use crate::core::types::source::{
    FileDiagnostics, FILE_FLAG_MISSING_NODES, FILE_FLAG_SYNTAX_ERRORS,
};
use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;
use crate::lang::c::Dialect;

/// Phase 1 adapter for C sources (tree-sitter-c).
pub struct CLanguageAdapter;

impl CLanguageAdapter {
    /// Creates the adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for CLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::C
    }

    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_c::language()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match classify_c_named(kind) {
            TokenType::Unknown => classify_c_anonymous(kind),
            t => t,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        classify_c_anonymous(kind) != TokenType::Unknown
    }

    fn classify_leaf(&self, kind: &str, is_named: bool) -> TokenType {
        if is_named {
            classify_c_named(kind)
        } else {
            classify_c_anonymous(kind)
        }
    }

    fn is_atomic(&self, kind: &str) -> bool {
        is_c_atomic(kind)
    }

    fn is_annotation_kind(&self, kind: &str) -> bool {
        // `[[gnu::unused]]`: `attribute` has `prefix` and `name` fields.
        kind == "attribute"
    }

    fn refine_token_type(&self, token_type: TokenType, text: &[u8]) -> TokenType {
        refine_c_family_token(token_type, text, Dialect::C)
    }
}

/// Kinds emitted as one token although the grammar gives them children: string and
/// character literals (`string_content`, `escape_sequence`, `character`, quotes and
/// encoding prefixes).
pub fn is_c_atomic(kind: &str) -> bool {
    matches!(kind, "string_literal" | "char_literal")
}

/// Token type of a named tree-sitter-c kind. Interior rules map to `Unknown`.
pub fn classify_c_named(kind: &str) -> TokenType {
    match kind {
        "identifier" | "field_identifier" | "type_identifier" | "statement_identifier" => {
            TokenType::Identifier
        }
        // Refined into integer / floating-point by `refine_token_type`.
        "number_literal" => TokenType::IntegerLiteral,
        "string_literal" | "string_content" | "system_lib_string" | "escape_sequence"
        | "concatenated_string" => TokenType::StringLiteral,
        "char_literal" | "character" => TokenType::CharLiteral,
        "true" | "false" => TokenType::BooleanLiteral,
        "null" => TokenType::NullLiteral,
        // One kind for `//` and `/* */`; refined by text.
        "comment" => TokenType::CommentBlock,
        // `int`, `double`, `size_t`, ...: typedef names are refined to identifiers.
        "primitive_type" => TokenType::Keyword,
        // `#pragma`, `#error`, `#line`, unknown `#directive`s.
        "preproc_directive" => TokenType::Keyword,
        // Specifier wrappers become leaves when the grammar inlines the keyword.
        "storage_class_specifier"
        | "type_qualifier"
        | "ms_restrict_modifier"
        | "ms_signed_ptr_modifier"
        | "ms_unsigned_ptr_modifier"
        | "ms_unaligned_ptr_modifier"
        | "ms_call_modifier"
        | "gnu_asm_qualifier" => TokenType::Keyword,
        // `...` of a variadic parameter list.
        "variadic_parameter" => TokenType::Punctuation,
        // Opaque replacement text of `#define` and argument text of other directives:
        // kept as one token so the text survives; `lang::preprocess` interprets it.
        "preproc_arg" => TokenType::Unknown,
        // Error-recovery node without children: unparsable source text.
        "ERROR" => TokenType::Unknown,
        // Interior rules of the grammar.
        "abstract_array_declarator"
        | "abstract_function_declarator"
        | "abstract_parenthesized_declarator"
        | "abstract_pointer_declarator"
        | "alignof_expression"
        | "argument_list"
        | "array_declarator"
        | "assignment_expression"
        | "attribute"
        | "attribute_declaration"
        | "attribute_specifier"
        | "attributed_declarator"
        | "attributed_statement"
        | "binary_expression"
        | "bitfield_clause"
        | "break_statement"
        | "call_expression"
        | "case_statement"
        | "cast_expression"
        | "comma_expression"
        | "compound_literal_expression"
        | "compound_statement"
        | "conditional_expression"
        | "continue_statement"
        | "declaration"
        | "declaration_list"
        | "do_statement"
        | "else_clause"
        | "enum_specifier"
        | "enumerator"
        | "enumerator_list"
        | "expression_statement"
        | "field_declaration"
        | "field_declaration_list"
        | "field_designator"
        | "field_expression"
        | "for_statement"
        | "function_declarator"
        | "function_definition"
        | "generic_expression"
        | "gnu_asm_clobber_list"
        | "gnu_asm_expression"
        | "gnu_asm_goto_list"
        | "gnu_asm_input_operand"
        | "gnu_asm_input_operand_list"
        | "gnu_asm_output_operand"
        | "gnu_asm_output_operand_list"
        | "goto_statement"
        | "if_statement"
        | "init_declarator"
        | "initializer_list"
        | "initializer_pair"
        | "labeled_statement"
        | "linkage_specification"
        | "macro_type_specifier"
        | "ms_based_modifier"
        | "ms_declspec_modifier"
        | "ms_pointer_modifier"
        | "offsetof_expression"
        | "parameter_declaration"
        | "parameter_list"
        | "parenthesized_declarator"
        | "parenthesized_expression"
        | "pointer_declarator"
        | "pointer_expression"
        | "preproc_call"
        | "preproc_def"
        | "preproc_defined"
        | "preproc_elif"
        | "preproc_elifdef"
        | "preproc_else"
        | "preproc_function_def"
        | "preproc_if"
        | "preproc_ifdef"
        | "preproc_include"
        | "preproc_params"
        | "return_statement"
        | "seh_except_clause"
        | "seh_finally_clause"
        | "seh_leave_statement"
        | "seh_try_statement"
        | "sized_type_specifier"
        | "sizeof_expression"
        | "struct_specifier"
        | "subscript_designator"
        | "subscript_expression"
        | "subscript_range_designator"
        | "switch_statement"
        | "translation_unit"
        | "type_definition"
        | "type_descriptor"
        | "unary_expression"
        | "union_specifier"
        | "update_expression"
        | "while_statement" => TokenType::Unknown,
        _ => TokenType::Unknown,
    }
}

/// Token type of an anonymous tree-sitter-c token (its kind is its spelling).
/// `Unknown` means the token is not part of the C token alphabet.
pub fn classify_c_anonymous(kind: &str) -> TokenType {
    match kind {
        // Terminator of preprocessor directives.
        "\n" => TokenType::Newline,
        // Operators (including the GNU `?:` halves and `defined`).
        "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
        | "!" | "&" | "|" | "^" | "~" | "<<" | ">>" | "++" | "--" | "=" | "+=" | "-="
        | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "?" => TokenType::Operator,
        // Punctuation, member access and attribute brackets.
        ";" | "," | "." | "->" | "(" | ")" | "[" | "]" | "{" | "}" | ":" | "::" | "..."
        | "[[" | "]]" => TokenType::Punctuation,
        // Quote delimiters and encoding prefixes (inside atomic literals).
        "\"" | "'" | "L\"" | "L'" | "u\"" | "u'" | "U\"" | "U'" | "u8\"" | "u8'" => {
            TokenType::Punctuation
        }
        "NULL" | "nullptr" => TokenType::NullLiteral,
        // Preprocessor directive keywords.
        "#include" | "#define" | "#if" | "#ifdef" | "#ifndef" | "#elif" | "#elifdef"
        | "#elifndef" | "#else" | "#endif" | "defined" => TokenType::Keyword,
        // Keywords, standard and extension spellings.
        "auto" | "break" | "case" | "const" | "constexpr" | "continue" | "default" | "do"
        | "else" | "enum" | "extern" | "for" | "goto" | "if" | "inline" | "long" | "register"
        | "restrict" | "return" | "short" | "signed" | "sizeof" | "static" | "struct"
        | "switch" | "typedef" | "union" | "unsigned" | "volatile" | "while" | "alignof"
        | "offsetof" | "noreturn" | "thread_local" | "asm" | "_Alignof" | "_Atomic"
        | "_Generic" | "_Noreturn" | "_Thread_local" | "_Alignas" | "alignas" | "_alignof"
        | "_unaligned" | "__alignof" | "__alignof__" | "__asm" | "__asm__" | "__attribute__"
        | "__attribute" | "__based" | "__cdecl" | "__clrcall" | "__declspec" | "__except"
        | "__extension__" | "__fastcall" | "__finally" | "__forceinline" | "__inline"
        | "__inline__" | "__leave" | "__restrict" | "__restrict__" | "__stdcall"
        | "__thiscall" | "__thread" | "__try" | "__unaligned" | "__vectorcall" | "__volatile__"
        | "_Nonnull" | "_Nullable" | "__ptr32" | "__ptr64" | "__sptr" | "__uptr"
        | "__w64" => TokenType::Keyword,
        _ => TokenType::Unknown,
    }
}

/// Text-based refinement shared by the C and C++ adapters.
pub fn refine_c_family_token(token_type: TokenType, text: &[u8], dialect: Dialect) -> TokenType {
    match token_type {
        TokenType::CommentBlock => classify_comment(text),
        TokenType::IntegerLiteral => classify_numeric_or_udl(text),
        TokenType::Keyword if is_library_typedef_name(text, dialect) => TokenType::Identifier,
        _ => token_type,
    }
}

/// `//` and `/* */` comments; Doxygen markers make documentation comments.
pub fn classify_comment(text: &[u8]) -> TokenType {
    if text.starts_with(b"//") {
        let doc = (text.starts_with(b"///") && !text.starts_with(b"////"))
            || text.starts_with(b"//!");
        if doc {
            TokenType::CommentDoc
        } else {
            TokenType::CommentLine
        }
    } else if (text.starts_with(b"/**") && text.len() >= 5 && !text.starts_with(b"/**/"))
        || text.starts_with(b"/*!")
    {
        TokenType::CommentDoc
    } else {
        TokenType::CommentBlock
    }
}

/// Types a numeric literal, or the literal part of a C++ user-defined literal
/// (`12_km`, `"abc"_s`, `'x'_c`), from its spelling.
pub fn classify_numeric_or_udl(text: &[u8]) -> TokenType {
    if let Some(quoted) = quoted_literal_type(text) {
        return quoted;
    }
    if is_floating_spelling(text) {
        TokenType::FloatLiteral
    } else {
        TokenType::IntegerLiteral
    }
}

/// String or character literal type of a spelling that starts with an optional encoding
/// prefix (`L`, `u`, `U`, `u8`) and raw marker (`R`) followed by a quote.
fn quoted_literal_type(text: &[u8]) -> Option<TokenType> {
    let mut i = 0usize;
    while i < text.len() && i < 3 && matches!(text[i], b'L' | b'u' | b'U' | b'8' | b'R') {
        i += 1;
    }
    match text.get(i) {
        Some(b'"') => Some(TokenType::StringLiteral),
        Some(b'\'') => Some(TokenType::CharLiteral),
        _ => None,
    }
}

/// True for floating-point spellings: a `.` or an exponent in the numeric part
/// (`e`/`E` for decimal, `p`/`P` for hexadecimal). Suffixes and digit separators are
/// skipped; `0x1e` stays an integer.
pub fn is_floating_spelling(text: &[u8]) -> bool {
    let hex = text.len() >= 2 && text[0] == b'0' && matches!(text[1], b'x' | b'X');
    let binary = text.len() >= 2 && text[0] == b'0' && matches!(text[1], b'b' | b'B');
    if binary {
        return false;
    }
    let body = if hex { &text[2..] } else { text };
    let mut i = 0usize;
    while i < body.len() {
        let c = body[i];
        let is_digit = if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        };
        if is_digit || c == b'\'' {
            i += 1;
            continue;
        }
        if c == b'.' {
            return true;
        }
        let exponent = if hex {
            matches!(c, b'p' | b'P')
        } else {
            matches!(c, b'e' | b'E')
        };
        if exponent {
            let mut j = i + 1;
            if matches!(body.get(j), Some(b'+') | Some(b'-')) {
                j += 1;
            }
            return body.get(j).is_some_and(|d| d.is_ascii_digit());
        }
        return false;
    }
    false
}

/// Library typedef names that tree-sitter-c parses as `primitive_type`. In C++ the
/// `charN_t` names are keywords.
fn is_library_typedef_name(text: &[u8], dialect: Dialect) -> bool {
    match text {
        b"size_t" | b"ssize_t" | b"ptrdiff_t" | b"intptr_t" | b"uintptr_t" | b"charptr_t"
        | b"nullptr_t" | b"max_align_t" | b"int8_t" | b"int16_t" | b"int32_t" | b"int64_t"
        | b"uint8_t" | b"uint16_t" | b"uint32_t" | b"uint64_t" => true,
        b"char8_t" | b"char16_t" | b"char32_t" => dialect == Dialect::C,
        _ => false,
    }
}

/// True for the error-recovery node kind.
#[inline]
pub fn is_error_node(node: &Node) -> bool {
    node.kind() == "ERROR"
}

/// Counts the ERROR and MISSING nodes of a parsed file and records their byte ranges.
/// Only subtrees that contain an error are visited, so clean files cost one check.
pub fn diagnose_syntax(root: Node, file_id: u16) -> FileDiagnostics {
    let mut diag = FileDiagnostics::new(file_id);
    if !root.has_error() {
        return diag;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_missing() {
            diag.missing_count = diag.missing_count.saturating_add(1);
            diag.record_error_range(node.start_byte(), node.end_byte());
        } else if is_error_node(&node) {
            diag.error_count = diag.error_count.saturating_add(1);
            diag.record_error_range(node.start_byte(), node.end_byte());
        }
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return diag;
            }
        }
    }
}

/// `SourceFileRecord.flags` bits implied by a file's syntax diagnostics.
pub fn syntax_file_flags(diag: &FileDiagnostics) -> u8 {
    let mut flags = 0u8;
    if diag.error_count > 0 {
        flags |= FILE_FLAG_SYNTAX_ERRORS;
    }
    if diag.missing_count > 0 {
        flags |= FILE_FLAG_MISSING_NODES;
    }
    flags
}
