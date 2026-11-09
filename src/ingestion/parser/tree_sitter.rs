use tree_sitter::{Language, Parser, Tree};

use crate::ingestion::parser::{CSTParser, ParseError};

pub struct TreeSitterParser {
    parser: Parser,
    timeout_micros: u64,
}

impl TreeSitterParser {
    pub fn new() -> Result<Self, String> {
        let parser = Parser::new();
        Ok(Self {
            parser,
            timeout_micros: 0,
        })
    }

    /// Parser that gives up on any single file after `timeout_micros` (0 = no limit).
    pub fn with_timeout(timeout_micros: u64) -> Result<Self, String> {
        let mut parser = Self::new()?;
        parser.set_timeout_micros(timeout_micros);
        Ok(parser)
    }

    pub fn set_timeout_micros(&mut self, timeout_micros: u64) {
        self.timeout_micros = timeout_micros;
        self.parser.set_timeout_micros(timeout_micros);
    }

    /// Parses `source`, reusing `old_tree` for incremental reparsing. The caller must have
    /// applied every edit since `old_tree` was produced through `Tree::edit`. Timeouts
    /// and other parser failures are reported as typed errors.
    pub fn parse_incremental(
        &mut self,
        source: &[u8],
        language: Language,
        old_tree: Option<&Tree>,
    ) -> Result<Tree, ParseError> {
        self.parser
            .set_language(&language)
            .map_err(|e| ParseError::Language(format!("{:?}", e)))?;
        match self.parser.parse(source, old_tree) {
            Some(tree) => Ok(tree),
            None => {
                // The parser keeps partial state after an aborted parse; clear it so the
                // next file starts fresh.
                self.parser.reset();
                if self.timeout_micros > 0 {
                    Err(ParseError::TimedOut {
                        timeout_micros: self.timeout_micros,
                    })
                } else {
                    Err(ParseError::Aborted)
                }
            }
        }
    }
}

impl CSTParser for TreeSitterParser {
    fn parse(&mut self, source: &[u8], language: Language) -> Result<Tree, String> {
        self.parse_incremental(source, language, None)
            .map_err(|e| e.to_string())
    }
}
