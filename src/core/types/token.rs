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
}

impl From<u8> for LangId {
    fn from(val: u8) -> Self {
        match val {
            0x01 => LangId::Java,
            0x02 => LangId::Kotlin,
            0x03 => LangId::Swift,
            0x04 => LangId::Python,
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

/// Constructs a packed 64-bit sort key.
/// Layout:
/// Bits 63..48 : file_id (u16)
/// Bits 47..24 : line    (u24)
/// Bits 23..8  : col     (u16)
/// Bits  7..0  : flags   (u8, reserved = 0)
#[inline(always)]
pub fn build_sort_key(file_id: u16, line: u32, col: u16) -> u64 {
    debug_assert!(line <= 0x00FF_FFFF, "line number exceeds 24-bit range");
    ((file_id as u64) << 48) | (((line & 0x00FF_FFFF) as u64) << 24) | ((col as u64) << 8)
}

/// Extracts file_id, line, col from a packed sort_key.
#[inline(always)]
pub fn unpack_sort_key(sort_key: u64) -> (u16, u32, u16) {
    let file_id = (sort_key >> 48) as u16;
    let line = ((sort_key >> 24) & 0x00FF_FFFF) as u32;
    let col = ((sort_key >> 8) & 0x0000_FFFF) as u16;
    (file_id, line, col)
}

/// Forward index entry (16 bytes). Sorted by `sort_key` ascending.
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
