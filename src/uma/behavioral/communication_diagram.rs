//! CommunicationDiagramExtractor — collaboration graphs derived from sequence
//! scenarios (§9.2.1; Rountev & Connell, ICSE 2005).
//!
//! A communication diagram shows the same interaction as a sequence diagram,
//! organized by links between lifelines: every link carries the ordinals of the
//! call, create and asynchronous messages sent along it, in send order. Reply
//! and found messages are not drawn on links.

use crate::uma::types::*;

pub struct CommunicationDiagramExtractor;

impl CommunicationDiagramExtractor {
    pub fn extract(sequences: &[SequenceDiagramRecord]) -> Vec<CommunicationDiagramRecord> {
        let mut out = Vec::new();
        for (i, seq) in sequences.iter().enumerate() {
            let sequence_index = match u32::try_from(i) {
                Ok(v) => v,
                Err(_) => break,
            };
            let mut links: Vec<CommunicationLinkRecord> = Vec::new();
            for msg in &seq.messages {
                if !matches!(
                    msg.message_kind,
                    MSG_KIND_SYNC | MSG_KIND_ASYNC | MSG_KIND_CREATE
                ) {
                    continue;
                }
                match links
                    .iter_mut()
                    .find(|l| l.from_lifeline == msg.from_lifeline && l.to_lifeline == msg.to_lifeline)
                {
                    Some(link) => link.message_ordinals.push(msg.ordinal),
                    None => links.push(CommunicationLinkRecord {
                        from_lifeline: msg.from_lifeline,
                        to_lifeline: msg.to_lifeline,
                        message_ordinals: vec![msg.ordinal],
                    }),
                }
            }
            if !links.is_empty() {
                out.push(CommunicationDiagramRecord {
                    sequence_index,
                    links,
                });
            }
        }
        out
    }
}
