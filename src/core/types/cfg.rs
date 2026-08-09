//! Core Control Flow Graph (CFG) Types and Alphabets for Phase 4.

/// CFG Edge Type Alphabet (Σ_CFG)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum CFGEdgeType {
    Uncond = 0x00,
    True = 0x01,
    False = 0x02,
    Except = 0x03,
    Return = 0x04,
    Switch = 0x05,
    LoopBack = 0x06,
}

impl From<u8> for CFGEdgeType {
    fn from(val: u8) -> Self {
        match val {
            0x01 => CFGEdgeType::True,
            0x02 => CFGEdgeType::False,
            0x03 => CFGEdgeType::Except,
            0x04 => CFGEdgeType::Return,
            0x05 => CFGEdgeType::Switch,
            0x06 => CFGEdgeType::LoopBack,
            _ => CFGEdgeType::Uncond,
        }
    }
}

/// In-memory Basic Block representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: u32,
    pub stmts: Vec<u32>, // BP AST pre-order indices
    pub first_token: u32,
    pub last_token: u32,
    pub is_entry: bool,
    pub is_exit: bool,
}

impl BasicBlock {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            stmts: Vec::new(),
            first_token: u32::MAX,
            last_token: 0,
            is_entry: false,
            is_exit: false,
        }
    }
}

/// Pending Edge waiting to connect to a target block
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingEdge {
    pub from: u32,
    pub edge_type: CFGEdgeType,
}

/// Break target frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakFrame {
    pub target: u32,
    pub label: Option<u32>,
}

/// Continue target frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinueFrame {
    pub target: u32,
    pub label: Option<u32>,
}

/// Exception handler frame for try/catch/finally
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionFrame {
    pub handlers: Vec<(u32, u32)>, // (exception_type_sym_id, catch_block_id)
    pub finally_block: Option<u32>,
}
