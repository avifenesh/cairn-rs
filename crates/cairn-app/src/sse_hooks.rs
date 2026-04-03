//! SSE publisher hooks wiring cairn-memory proposal events to cairn-api SSE frames.

use cairn_api::memory_api::MemoryItem;
use cairn_api::sse_payloads::build_memory_proposed_frame;
use cairn_memory::api_impl::MemoryProposalHook;

/// SSE publisher hook that emits `memory_proposed` frames when
/// cairn-memory creates a Proposed memory.
///
/// Wire into MemoryApiImpl:
/// ```ignore
/// let hook = SseMemoryProposalHook::new(sse_sender);
/// let api = MemoryApiImpl::new(retrieval).with_proposal_hook(Box::new(hook));
/// ```
pub struct SseMemoryProposalHook {
    /// In a real server, this would hold an SSE broadcast channel sender.
    /// For now, we collect frames for testing.
    frames: std::sync::Mutex<Vec<cairn_api::sse::SseFrame>>,
}

impl SseMemoryProposalHook {
    pub fn new() -> Self {
        Self {
            frames: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn collected_frames(&self) -> Vec<cairn_api::sse::SseFrame> {
        self.frames.lock().unwrap().clone()
    }
}

impl MemoryProposalHook for SseMemoryProposalHook {
    fn on_proposed(&self, item: &MemoryItem) {
        let frame = build_memory_proposed_frame(item.clone(), None);
        self.frames.lock().unwrap().push(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_api::memory_api::{MemoryItem, MemoryStatus};

    #[test]
    fn hook_captures_memory_proposed_frame() {
        let hook = SseMemoryProposalHook::new();

        let item = MemoryItem {
            id: "mem_1".to_owned(),
            content: "Important fact".to_owned(),
            category: Some("facts".to_owned()),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: "2026-04-03T10:00:00Z".to_owned(),
        };

        hook.on_proposed(&item);

        let frames = hook.collected_frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].event,
            cairn_api::sse::SseEventName::MemoryProposed
        );
        assert_eq!(frames[0].data["memory"]["content"], "Important fact");
        assert_eq!(frames[0].data["memory"]["status"], "proposed");
    }
}
