//! PatternDetector — coordinates all 6 design pattern structural queries (§9.2.5, §9.3).

pub mod builder;
pub mod factory;
pub mod observer;
pub mod singleton;
pub mod state;
pub mod template_method;

pub use builder::is_builder;
pub use factory::is_factory;
pub use observer::is_observer_subject;
pub use singleton::is_singleton;
pub use state::is_state;
pub use template_method::is_template_method;

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::*;

pub struct PatternDetector;

impl PatternDetector {
    pub fn detect_all(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
        classes: &mut [ClassRecord],
    ) -> Vec<DesignPatternRecord> {
        let mut records = Vec::new();

        for class_rec in classes.iter_mut() {
            let sym_id = class_rec.sym_id;

            // Singleton
            if let (true, conf) = is_singleton(sym_id, sta) {
                class_rec.design_pattern = PATTERN_SINGLETON;
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_SINGLETON as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }

            // Observer
            if let (true, conf) = is_observer_subject(sym_id, sta, tca, cga) {
                if class_rec.design_pattern == PATTERN_NONE {
                    class_rec.design_pattern = PATTERN_OBSERVER;
                }
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_OBSERVER as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }

            // Factory
            if let (true, conf) = is_factory(sym_id, sta, tca) {
                if class_rec.design_pattern == PATTERN_NONE {
                    class_rec.design_pattern = PATTERN_FACTORY;
                }
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_FACTORY as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }

            // Builder
            if let (true, conf) = is_builder(sym_id, sta, tca) {
                if class_rec.design_pattern == PATTERN_NONE {
                    class_rec.design_pattern = PATTERN_BUILDER;
                }
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_BUILDER as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }

            // State
            if let (true, conf) = is_state(sym_id, sta, tca) {
                if class_rec.design_pattern == PATTERN_NONE {
                    class_rec.design_pattern = PATTERN_STATE;
                }
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_STATE as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }

            // Template Method
            if let (true, conf) = is_template_method(sym_id, sta) {
                if class_rec.design_pattern == PATTERN_NONE {
                    class_rec.design_pattern = PATTERN_TEMPLATE_METHOD;
                }
                records.push(DesignPatternRecord {
                    class_sym: sym_id,
                    pattern_kind: PATTERN_TEMPLATE_METHOD as u16,
                    confidence: conf,
                    _reserved: 0,
                });
            }
        }

        records
    }
}
