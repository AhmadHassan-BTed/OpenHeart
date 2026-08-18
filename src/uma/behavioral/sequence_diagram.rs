//! SequenceDiagramExtractor — extracts SequenceDiagramRecord[] from CGA call sites (§9.2.4).

use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::actor_identification::{ActorIdentifier, EXTERNAL_ACTOR_ID};
use crate::uma::types::*;
use std::collections::{HashMap, HashSet};

pub struct SequenceDiagramExtractor;

impl SequenceDiagramExtractor {
    pub fn extract_all(
        sta: &SymbolTableArtifact,
        _cfa: &CFGArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
    ) -> Vec<SequenceDiagramRecord> {
        let entry_points = ActorIdentifier::find_entry_points(sta, cga);
        let mut sequences = Vec::new();

        for &entry_sym in &entry_points {
            if let Some(seq) = Self::extract_for_entry(entry_sym, sta, cga, tra) {
                sequences.push(seq);
            }
        }

        sequences
    }

    pub fn extract_for_entry(
        entry_sym: u32,
        sta: &SymbolTableArtifact,
        cga: &CallGraphArtifact,
        _tra: &TraceabilityArtifact,
    ) -> Option<SequenceDiagramRecord> {
        let entry_sym_rec = sta.symbol(entry_sym)?;

        let mut lifelines_map: HashMap<u32, LifelineRecord> = HashMap::new();
        let mut messages = Vec::new();
        let mut ordinal = 0u16;

        // Actor lifeline for caller
        lifelines_map.insert(
            EXTERNAL_ACTOR_ID,
            LifelineRecord {
                sym_id: EXTERNAL_ACTOR_ID,
                name_id: 0,
                type_sym_id: EXTERNAL_ACTOR_ID,
                is_actor: 1,
                _pad: [0; 3],
            },
        );

        // Target method lifeline
        let entry_class = entry_sym_rec.parent_sym;
        let entry_class_name = sta
            .symbol(entry_class)
            .map(|s| s.name_id)
            .unwrap_or(entry_class);

        lifelines_map.insert(
            entry_class,
            LifelineRecord {
                sym_id: entry_class,
                name_id: entry_class_name,
                type_sym_id: entry_class,
                is_actor: 0,
                _pad: [0; 3],
            },
        );

        messages.push(MessageRecord {
            from_lifeline: EXTERNAL_ACTOR_ID,
            to_lifeline: entry_class,
            call_site_id: 0,
            method_sym_id: entry_sym,
            message_kind: 0,
            ordinal,
            _pad: 0,
            uml_link_token: entry_sym_rec.first_token_id,
        });
        ordinal += 1;

        // Pre-index CGA call sites and site edges for O(1) lookup
        let mut site_edges: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for &(clr, callee_sym, site_id) in &cga.site_to_edge_map {
            site_edges
                .entry((clr, site_id))
                .or_default()
                .push(callee_sym);
        }

        // Trace call sites from entry_sym
        let mut visited = HashSet::new();
        let mut stack = vec![entry_sym];

        while let Some(caller_sym) = stack.pop() {
            if !visited.insert(caller_sym) {
                continue;
            }

            let caller_class = sta
                .symbol(caller_sym)
                .map(|s| s.parent_sym)
                .unwrap_or(caller_sym);

            for site in &cga.call_sites {
                if site.caller_sym == caller_sym {
                    if let Some(callees) = site_edges.get(&(caller_sym, site.call_site_id)) {
                        for &callee_sym in callees {
                            let callee_class = sta
                                .symbol(callee_sym)
                                .map(|s| s.parent_sym)
                                .unwrap_or(callee_sym);
                            let callee_name = sta
                                .symbol(callee_class)
                                .map(|s| s.name_id)
                                .unwrap_or(callee_class);

                            lifelines_map
                                .entry(callee_class)
                                .or_insert_with(|| LifelineRecord {
                                    sym_id: callee_class,
                                    name_id: callee_name,
                                    type_sym_id: callee_class,
                                    is_actor: 0,
                                    _pad: [0; 3],
                                });

                            messages.push(MessageRecord {
                                from_lifeline: caller_class,
                                to_lifeline: callee_class,
                                call_site_id: site.call_site_id,
                                method_sym_id: callee_sym,
                                message_kind: site.call_type,
                                ordinal,
                                _pad: 0,
                                uml_link_token: site.call_token,
                            });
                            ordinal += 1;

                            if (callee_sym as usize) < sta.symbol_records.len() {
                                stack.push(callee_sym);
                            }
                        }
                    }
                }
            }
        }

        let lifelines: Vec<LifelineRecord> = lifelines_map.into_values().collect();

        Some(SequenceDiagramRecord {
            scenario_name: entry_sym_rec.name_id,
            lifeline_count: lifelines.len() as u16,
            message_count: messages.len() as u16,
            fragment_count: 0,
            _reserved: 0,
            lifelines,
            messages,
            combined_fragments: Vec::new(),
        })
    }
}
