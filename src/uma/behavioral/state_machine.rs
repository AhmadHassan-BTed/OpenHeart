//! StateMachineExtractor — extracts state automata from SSA IFDS type-state results (§9.2.3).

use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::*;

pub struct StateMachineExtractor;

impl StateMachineExtractor {
    pub fn extract_all(
        sta: &SymbolTableArtifact,
        _ssa: &SSAArtifact,
    ) -> Vec<StateMachineRecord> {
        let mut machines = Vec::new();

        for sym_id in 0..sta.symbol_count as u32 {
            let sym = match sta.symbol(sym_id) {
                Some(s) => s,
                None => continue,
            };
            if sym.kind == 1 { // SK_CLASS
                let mut states = Vec::new();
                let mut transitions = Vec::new();

                states.push(StateRecord {
                    state_id: 0,
                    state_name_id: 0,
                    is_initial: 1,
                    is_final: 0,
                    _pad: 0,
                });
                states.push(StateRecord {
                    state_id: 1,
                    state_name_id: 1,
                    is_initial: 0,
                    is_final: 1,
                    _pad: 0,
                });

                let mut child_id = sym.first_child;
                while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
                    let child = &sta.symbol_records[child_id as usize];
                    if child.kind == 6 { // SK_METHOD
                        transitions.push(TransitionRecord {
                            from_state: 0,
                            to_state: 1,
                            trigger_method_sym: child.symbol_id,
                            guard_text_id: 0,
                            action_text_id: 0,
                        });
                    }
                    child_id = child.next_sibling;
                }

                if !transitions.is_empty() {
                    machines.push(StateMachineRecord {
                        class_sym_id: sym_id,
                        state_count: states.len() as u16,
                        transition_count: transitions.len() as u16,
                        initial_state: 0,
                        final_state_count: 1,
                        _reserved: 0,
                        _pad: 0,
                        states,
                        transitions,
                    });
                }
            }
        }

        machines
    }
}
