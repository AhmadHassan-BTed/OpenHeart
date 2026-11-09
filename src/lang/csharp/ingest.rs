use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::{refine_doc_comment, LanguageAdapter};

/// Phase 1 adapter for tree-sitter-c-sharp (0.23; C# 1 through C# 12).
///
/// The tables cover every named leaf kind and every anonymous token of the grammar's
/// `node-types.json`: the reserved keywords, the contextual keywords (accessors, LINQ query
/// clauses, patterns, attribute targets, `record`, `required`, `file`, `scoped`, ...), the
/// preprocessor directive introducers (`#if`, `#region`, `#pragma`, ...) and their option
/// words, the function-pointer calling conventions, all operators (including `>>>`, `>>>=`,
/// `??=`, `..`, `->`) and all separators.
///
/// `identifier` is atomic. The grammar wraps a contextual keyword used as a name (`var`,
/// `from`, `global`, `when`, ...) in an `identifier` node, so emitting that node as one token
/// types every such name `Identifier`, and every contextual keyword the walker reaches as an
/// anonymous leaf is acting as a keyword and is typed `Keyword`. `implicit_parameter` (the
/// `x` of `x => ...`) is an aliased identifier and is atomic for the same reason.
///
/// Regular, UTF-8 (`"..."u8`), character and raw (`"""..."""`) literals are atomic: their
/// content tokens are fragments of one literal. Interpolated strings are not atomic, so the
/// expressions inside `{...}` keep their own tokens; the delimiters (`interpolation_start`,
/// `interpolation_quote`, `"`), text segments (`string_content`) and escapes are typed
/// `StringLiteral`, the braces `Punctuation`. An interpolation's format clause (`:N2`) is
/// atomic because the grammar hides its text.
///
/// Preprocessor directives are typed by their introducer token (`#if`, `#define`, ...,
/// `Keyword`), their option words (`enable`, `warning`, `disable`, `checksum`, ...,
/// `Keyword`) and their free-text argument (`preproc_arg`: the symbol of `#define`, the text
/// of `#region` / `#error` / `#warning`, `StringLiteral`). `#if` / `#elif` conditions are
/// ordinary identifier and operator tokens.
///
/// `comment` covers `//` and `/* */`; `refine_token_type` types `///` XML documentation
/// comments and `/** */` comments `CommentDoc`. A `#!` script line of a `.csx` file is a
/// `CommentLine`. Attribute names (`[Obsolete]`, `[System.Serializable]`) are typed
/// `Annotation` through the walker's `name`-field rule.
pub struct CSharpLanguageAdapter;

impl CSharpLanguageAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CSharpLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for CSharpLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::CSharp
    }

    fn file_extensions(&self) -> &[&str] {
        &["cs", "csx"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            // ── Named leaves ──
            "identifier" | "implicit_parameter" | "discard" => TokenType::Identifier,
            "integer_literal" => TokenType::IntegerLiteral,
            "real_literal" => TokenType::FloatLiteral,
            "string_literal" | "verbatim_string_literal" | "raw_string_literal" => {
                TokenType::StringLiteral
            }
            "character_literal" => TokenType::CharLiteral,
            "boolean_literal" => TokenType::BooleanLiteral,
            "null_literal" => TokenType::NullLiteral,
            "comment" => TokenType::CommentLine,
            "shebang_directive" => TokenType::CommentLine,
            // `predefined_type` is one token (`int`, `string`, `void`, ...); `implicit_type`
            // is the `var` of a declaration; a `modifier` reaches the walker as a leaf only
            // when the grammar reduces it to its keyword.
            "predefined_type" | "implicit_type" | "modifier" => TokenType::Keyword,
            "preproc_arg" => TokenType::StringLiteral,
            // Pieces of interpolated strings (outside interpolations).
            "interpolation_start" | "interpolation_quote" | "string_content" | "escape_sequence"
            | "interpolation_format_clause" => TokenType::StringLiteral,
            "interpolation_brace" => TokenType::Punctuation,

            // ── Literal pieces (only reachable inside the atomic literals above) ──
            "string_literal_content" | "string_literal_encoding" | "raw_string_start"
            | "raw_string_content" | "raw_string_end" | "\"" => TokenType::StringLiteral,
            "character_literal_content" | "'" => TokenType::CharLiteral,
            "true" | "false" => TokenType::BooleanLiteral,

            // ── Operators ──
            "+" | "-" | "*" | "/" | "%" | "^" | "&" | "|" | "~" | "!" | "=" | "<" | ">" | "?"
            | "??" | "++" | "--" | "&&" | "||" | "->" | "==" | "!=" | "<=" | ">=" | "+="
            | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<" | "<<=" | ">>" | ">>="
            | ">>>" | ">>>=" | "??=" | "=>" | ".." => TokenType::Operator,

            // ── Punctuation ──
            ";" | "," | "." | "(" | ")" | "[" | "]" | "{" | "}" | ":" | "::" => {
                TokenType::Punctuation
            }

            // ── Preprocessor directive introducers ──
            "#if" | "#elif" | "#else" | "#endif" | "#define" | "#undef" | "#region"
            | "#endregion" | "#error" | "#warning" | "#line" | "#nullable" | "#pragma" => {
                TokenType::Keyword
            }

            // ── Keywords (reserved) ──
            "abstract" | "as" | "base" | "break" | "case" | "catch" | "checked" | "class"
            | "const" | "continue" | "default" | "delegate" | "do" | "else" | "enum"
            | "event" | "explicit" | "extern" | "finally" | "fixed" | "for" | "foreach"
            | "goto" | "if" | "implicit" | "in" | "interface" | "internal" | "is" | "lock"
            | "namespace" | "new" | "operator" | "out" | "override" | "params" | "private"
            | "protected" | "public" | "readonly" | "ref" | "return" | "sealed" | "sizeof"
            | "stackalloc" | "static" | "struct" | "switch" | "this" | "throw" | "try"
            | "typeof" | "unchecked" | "unsafe" | "using" | "virtual" | "volatile" | "while"
            | "__makeref" | "__reftype" | "__refvalue" => TokenType::Keyword,

            // ── Keywords (contextual; as names they are inside an atomic identifier) ──
            "add" | "alias" | "and" | "ascending" | "async" | "await" | "by" | "descending"
            | "equals" | "file" | "from" | "get" | "global" | "group" | "init" | "into"
            | "join" | "let" | "managed" | "not" | "notnull" | "on" | "or" | "orderby"
            | "partial" | "record" | "remove" | "required" | "scoped" | "select" | "set"
            | "unmanaged" | "var" | "when" | "where" | "with" | "yield" => TokenType::Keyword,
            // Attribute targets (`[assembly: ...]`, `[return: ...]`).
            "assembly" | "module" | "field" | "method" | "param" | "property" | "type" => {
                TokenType::Keyword
            }
            // Function-pointer calling conventions (`delegate* unmanaged[Cdecl]<...>`).
            "Cdecl" | "Fastcall" | "Stdcall" | "Thiscall" => TokenType::Keyword,
            // Preprocessor option words (`#nullable enable warnings`, `#pragma warning
            // restore`, `#pragma checksum`, `#line hidden`).
            "enable" | "disable" | "restore" | "warning" | "warnings" | "annotations"
            | "checksum" | "hidden" => TokenType::Keyword,

            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        self.map_node_type(kind) != TokenType::Unknown
    }

    fn is_atomic(&self, kind: &str) -> bool {
        matches!(
            kind,
            "identifier"
                | "implicit_parameter"
                | "string_literal"
                | "character_literal"
                | "raw_string_literal"
                | "interpolation_format_clause"
        )
    }

    fn is_annotation_kind(&self, kind: &str) -> bool {
        kind == "attribute"
    }

    /// tree-sitter-c-sharp uses one `comment` kind for `//` and `/* */` comments; the form is
    /// recovered from the text. `///` (but not `////`) starts an XML documentation comment, as
    /// does `/**` (but not the empty `/**/`).
    fn refine_token_type(&self, token_type: TokenType, text: &[u8]) -> TokenType {
        if token_type != TokenType::CommentLine {
            return token_type;
        }
        if text.starts_with(b"/*") {
            return refine_doc_comment(TokenType::CommentBlock, text);
        }
        if text.starts_with(b"///") && !text.starts_with(b"////") {
            return TokenType::CommentDoc;
        }
        TokenType::CommentLine
    }
}
