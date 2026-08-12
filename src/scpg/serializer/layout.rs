//! SectionLayoutPlanner — orders 11 sections hot -> warm -> cold (§10.2).

use crate::scpg::types::SCPGSectionType;

pub struct SectionLayoutPlanner;

impl SectionLayoutPlanner {
    /// Returns the exact 11 sections in hot -> warm -> cold disk layout order.
    pub fn ordered_sections() -> &'static [SCPGSectionType] {
        &[
            SCPGSectionType::TokenTable,       // 0x01 (HOT)
            SCPGSectionType::StringTable,      // 0x02 (HOT)
            SCPGSectionType::Traceability,     // 0x09 (HOT)
            SCPGSectionType::SymbolTable,      // 0x04 (HOT)
            SCPGSectionType::TypeHierarchy,    // 0x08 (HOT)
            SCPGSectionType::SemanticMetadata, // 0x0A (WARM)
            SCPGSectionType::CallGraph,        // 0x07 (WARM)
            SCPGSectionType::BpAst,            // 0x03 (COLD)
            SCPGSectionType::Cfg,              // 0x05 (COLD)
            SCPGSectionType::SsaDfg,           // 0x06 (COLD)
            SCPGSectionType::PathSummaries,    // 0x0B (COLD)
        ]
    }
}
