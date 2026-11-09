//! InteractionOverviewExtractor — activity diagrams whose nodes reference
//! interactions (§9.2.1).
//!
//! For every scenario, the entry method's activity is the control flow of the
//! overview; each call site of the entry method that produced messages in the
//! scenario becomes an interaction use, anchored at the activity node (CFG
//! block) that contains the call. Scenarios whose entry method has no activity
//! (no CFG body) get no overview.

use std::collections::HashMap;

use crate::uma::index::UMAIndex;
use crate::uma::types::*;

pub struct InteractionOverviewExtractor;

impl InteractionOverviewExtractor {
    pub fn extract(
        index: &UMAIndex<'_>,
        activities: &[ActivityRecord],
        sequences: &[SequenceDiagramRecord],
    ) -> Vec<InteractionOverviewRecord> {
        let activity_of: HashMap<u32, usize> = activities
            .iter()
            .enumerate()
            .map(|(i, a)| (a.function_sym_id, i))
            .collect();
        let mut out = Vec::new();
        for (si, seq) in sequences.iter().enumerate() {
            let (activity_index, sequence_index) =
                match (activity_of.get(&seq.entry_method_sym), u32::try_from(si)) {
                    (Some(&ai), Ok(s)) => match u32::try_from(ai) {
                        Ok(a) => (a, s),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
            let activity = &activities[activity_index as usize];
            let mut references = Vec::new();
            for site in index.sites_of(seq.entry_method_sym) {
                let used = seq.messages.iter().any(|m| {
                    m.call_site_id == site.call_site_id && m.nesting_depth == 1
                });
                // Block 0 is the synthetic entry block; CGA reports 0 when it could
                // not place the call in a block, so it never anchors a reference.
                if !used || site.call_block == 0 {
                    continue;
                }
                if !activity.nodes.iter().any(|n| n.node_id == site.call_block) {
                    continue;
                }
                references.push(InteractionReferenceRecord {
                    activity_node: site.call_block,
                    sequence_index,
                    call_site_id: site.call_site_id,
                });
            }
            if !references.is_empty() {
                out.push(InteractionOverviewRecord {
                    entry_method_sym: seq.entry_method_sym,
                    activity_index,
                    references,
                });
            }
        }
        out
    }
}
