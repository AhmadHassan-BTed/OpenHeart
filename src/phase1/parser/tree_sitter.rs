use tree_sitter::{Language, Parser, Tree};

use crate::phase1::parser::CSTParser;

pub struct TreeSitterParser {
    parser: Parser,
}

impl TreeSitterParser {
    pub fn new() -> Result<Self, String> {
        let parser = Parser::new();
        Ok(Self { parser })
    }
}

impl CSTParser for TreeSitterParser {
    fn parse(&mut self, source: &[u8], language: Language) -> Result<Tree, String> {
        self.parser
            .set_language(language)
            .map_err(|e| format!("Failed to set tree-sitter language: {:?}", e))?;
        self.parser
            .parse(source, None)
            .ok_or_else(|| "Tree-sitter parse returned None".to_string())
    }
}
