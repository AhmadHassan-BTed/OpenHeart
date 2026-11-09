use std::fmt;

/// Language identifier enum (u8 repr).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum LangId {
    Unknown = 0x00,
    Java = 0x01,
    Kotlin = 0x02,
    Swift = 0x03,
    Python = 0x04,
    JavaScript = 0x05,
    TypeScript = 0x06,
    Rust = 0x07,
    Cpp = 0x08,
    Go = 0x09,
    Generic = 0x0A,
}

impl From<u8> for LangId {
    fn from(val: u8) -> Self {
        match val {
            0x01 => LangId::Java,
            0x02 => LangId::Kotlin,
            0x03 => LangId::Swift,
            0x04 => LangId::Python,
            0x05 => LangId::JavaScript,
            0x06 => LangId::TypeScript,
            0x07 => LangId::Rust,
            0x08 => LangId::Cpp,
            0x09 => LangId::Go,
            0x0A => LangId::Generic,
            _ => LangId::Unknown,
        }
    }
}

/// Token Type Alphabet (Σ_T).
/// 0x00–0x7F: Language-agnostic core types
/// 0x80–0xFF: Language-specific extension types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum TokenType {
    Unknown = 0x00,
    Identifier = 0x01,
    Keyword = 0x02,
    Operator = 0x03,
    Punctuation = 0x04,
    IntegerLiteral = 0x05,
    FloatLiteral = 0x06,
    StringLiteral = 0x07,
    CharLiteral = 0x08,
    BooleanLiteral = 0x09,
    NullLiteral = 0x0A,
    CommentLine = 0x0B,
    CommentBlock = 0x0C,
    CommentDoc = 0x0D,
    Whitespace = 0x0E,
    Newline = 0x0F,
    Annotation = 0x10,
    TypeParameter = 0x11,
    LabeledStmt = 0x12,
    /// Contextual keyword that is also a valid identifier (Kotlin `data`, `value`, `field`, ...).
    /// Consumers decide from context whether it acts as a keyword.
    SoftKeyword = 0x13,

    // Java-specific extensions (0x80..0xFF)
    JavaAnnotationMarker = 0x80,
    JavaGenericDiamond = 0x81,
    JavaVarKeyword = 0x82,
    JavaSealedKeyword = 0x83,
}

impl TokenType {
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            0x00 => TokenType::Unknown,
            0x01 => TokenType::Identifier,
            0x02 => TokenType::Keyword,
            0x03 => TokenType::Operator,
            0x04 => TokenType::Punctuation,
            0x05 => TokenType::IntegerLiteral,
            0x06 => TokenType::FloatLiteral,
            0x07 => TokenType::StringLiteral,
            0x08 => TokenType::CharLiteral,
            0x09 => TokenType::BooleanLiteral,
            0x0A => TokenType::NullLiteral,
            0x0B => TokenType::CommentLine,
            0x0C => TokenType::CommentBlock,
            0x0D => TokenType::CommentDoc,
            0x0E => TokenType::Whitespace,
            0x0F => TokenType::Newline,
            0x10 => TokenType::Annotation,
            0x11 => TokenType::TypeParameter,
            0x12 => TokenType::LabeledStmt,
            0x13 => TokenType::SoftKeyword,

            0x80 => TokenType::JavaAnnotationMarker,
            0x81 => TokenType::JavaGenericDiamond,
            0x82 => TokenType::JavaVarKeyword,
            0x83 => TokenType::JavaSealedKeyword,

            _ => TokenType::Unknown,
        }
    }
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Largest line number representable in a sort key (24 bits, 1-based lines).
pub const SORT_KEY_MAX_LINE: u32 = 0x00FF_FFFF;
/// Largest byte column representable in a sort key (16 bits, 0-based columns).
pub const SORT_KEY_MAX_COL: u32 = 0xFFFF;
/// Largest file id representable in a sort key (16 bits).
pub const SORT_KEY_MAX_FILE_ID: u32 = 0xFFFF;
/// Mask that removes the low "flags / duplicate" byte from a sort key, leaving the position.
pub const SORT_KEY_POSITION_MASK: u64 = !0xFF;

/// A source position that cannot be encoded in the 64-bit sort-key layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKeyError {
    FileIdOutOfRange(u32),
    LineOutOfRange(u32),
    ColumnOutOfRange(u32),
}

impl fmt::Display for SortKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SortKeyError::FileIdOutOfRange(v) => write!(
                f,
                "file id {} exceeds the sort-key limit of {}",
                v, SORT_KEY_MAX_FILE_ID
            ),
            SortKeyError::LineOutOfRange(v) => write!(
                f,
                "line {} exceeds the sort-key limit of {}",
                v, SORT_KEY_MAX_LINE
            ),
            SortKeyError::ColumnOutOfRange(v) => write!(
                f,
                "byte column {} exceeds the sort-key limit of {}",
                v, SORT_KEY_MAX_COL
            ),
        }
    }
}

impl std::error::Error for SortKeyError {}

/// Constructs a packed 64-bit sort key.
/// Layout:
/// Bits 63..48 : file_id (u16)
/// Bits 47..24 : line    (u24)
/// Bits 23..8  : col     (u16)
/// Bits  7..0  : flags   (u8, reserved = 0)
///
/// Callers that cannot guarantee the ranges must use `try_build_sort_key`, which
/// reports out-of-range positions instead of wrapping them.
#[inline(always)]
pub fn build_sort_key(file_id: u16, line: u32, col: u16) -> u64 {
    debug_assert!(line <= SORT_KEY_MAX_LINE, "line number exceeds 24-bit range");
    ((file_id as u64) << 48) | (((line & 0x00FF_FFFF) as u64) << 24) | ((col as u64) << 8)
}

/// Range-checked sort-key construction: an unrepresentable position is an error,
/// never a silently wrapped key.
#[inline]
pub fn try_build_sort_key(file_id: u32, line: u32, col: u32) -> Result<u64, SortKeyError> {
    if file_id > SORT_KEY_MAX_FILE_ID {
        return Err(SortKeyError::FileIdOutOfRange(file_id));
    }
    if line > SORT_KEY_MAX_LINE {
        return Err(SortKeyError::LineOutOfRange(line));
    }
    if col > SORT_KEY_MAX_COL {
        return Err(SortKeyError::ColumnOutOfRange(col));
    }
    Ok(build_sort_key(file_id as u16, line, col as u16))
}

/// Sort key of a position, saturating the line and column at their maxima. Used for the
/// exclusive upper bound of a source range: no token can start beyond a saturated
/// coordinate, because Phase 1 rejects files with unrepresentable token positions.
#[inline]
pub fn saturating_sort_key(file_id: u16, line: u32, col: u32) -> u64 {
    if line > SORT_KEY_MAX_LINE {
        return build_sort_key(file_id, SORT_KEY_MAX_LINE, SORT_KEY_MAX_COL as u16) | 0xFF;
    }
    if col > SORT_KEY_MAX_COL {
        return build_sort_key(file_id, line, SORT_KEY_MAX_COL as u16) | 0xFF;
    }
    build_sort_key(file_id, line, col as u16)
}

/// Extracts file_id, line, col from a packed sort_key.
#[inline(always)]
pub fn unpack_sort_key(sort_key: u64) -> (u16, u32, u16) {
    let file_id = (sort_key >> 48) as u16;
    let line = ((sort_key >> 24) & 0x00FF_FFFF) as u32;
    let col = ((sort_key >> 8) & 0x0000_FFFF) as u16;
    (file_id, line, col)
}

/// `TokenRecord::_padding` / `TokenEntry::_padding` flag: the token is longer than
/// `u16::MAX` bytes, so `len` is saturated; the full text is available through `text_id`.
pub const TOKEN_FLAG_LEN_SATURATED: u8 = 1 << 0;

/// Forward index entry (16 bytes). Sorted by `sort_key` ascending.
///
/// `len` is the byte length saturated at `u16::MAX`; `_padding` carries `TOKEN_FLAG_*` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct TokenRecord {
    pub sort_key: u64,
    pub text_id: u32,
    pub len: u16,
    pub token_type: u8,
    pub _padding: u8,
}

/// Backward index entry (16 bytes). Dense array indexed directly by `token_id`.
/// Field semantics match `TokenRecord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct TokenEntry {
    pub sort_key: u64,
    pub text_id: u32,
    pub len: u16,
    pub token_type: u8,
    pub _padding: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_record_layout() {
        assert_eq!(std::mem::size_of::<TokenRecord>(), 16);
        assert_eq!(std::mem::size_of::<TokenEntry>(), 16);
    }

    #[test]
    fn test_sort_key_packing() {
        let key = build_sort_key(0x1234, 0x0056789A, 0xABCD);
        let (file_id, line, col) = unpack_sort_key(key);
        assert_eq!(file_id, 0x1234);
        assert_eq!(line, 0x0056789A);
        assert_eq!(col, 0xABCD);
    }
}
