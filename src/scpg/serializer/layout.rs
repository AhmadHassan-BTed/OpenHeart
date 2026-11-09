//! SectionLayoutPlanner — orders 11 sections hot -> warm -> cold and aligns them (§10.2).

use crate::scpg::types::{SCPGSectionType, SCPG_PAGE_ALIGN, SCPG_SECTION_ALIGN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionTemperature {
    Hot,
    Warm,
    Cold,
}

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

    pub fn temperature(section: SCPGSectionType) -> SectionTemperature {
        match section {
            SCPGSectionType::TokenTable
            | SCPGSectionType::StringTable
            | SCPGSectionType::Traceability
            | SCPGSectionType::SymbolTable
            | SCPGSectionType::TypeHierarchy => SectionTemperature::Hot,
            SCPGSectionType::SemanticMetadata | SCPGSectionType::CallGraph => {
                SectionTemperature::Warm
            }
            SCPGSectionType::BpAst
            | SCPGSectionType::Cfg
            | SCPGSectionType::SsaDfg
            | SCPGSectionType::PathSummaries => SectionTemperature::Cold,
        }
    }

    /// Sections that span at least one page start on a page boundary so the OS can page hot
    /// and cold regions independently; smaller sections only need cache-line alignment.
    pub fn alignment_for(payload_len: u64) -> u64 {
        if payload_len >= SCPG_PAGE_ALIGN {
            SCPG_PAGE_ALIGN
        } else {
            SCPG_SECTION_ALIGN
        }
    }
}

/// Rounds `offset` up to a multiple of `align` (a power of two); `None` on overflow.
pub fn align_up(offset: u64, align: u64) -> Option<u64> {
    debug_assert!(align.is_power_of_two());
    offset.checked_add(align - 1).map(|v| v & !(align - 1))
}
