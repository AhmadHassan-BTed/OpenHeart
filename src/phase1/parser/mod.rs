pub mod tree_sitter;

use ::tree_sitter::{Language, Tree};

pub trait CSTParser {
    fn parse(&mut self, source: &[u8], language: Language) -> Result<Tree, String>;
}
