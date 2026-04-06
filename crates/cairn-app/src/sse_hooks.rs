//! SSE publisher hooks wiring cairn-memory proposal events to cairn-api SSE frames.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use cairn_api::memory_api::MemoryItem;
use cairn_api::sse::SseFrame;
use cairn_api::sse_payloads::build_memory_proposed_frame;
use cairn_memory::api_impl::MemoryProposalHook;
use tokio::sync::broadcast;

const SSE_BUFFER_CAPACITY: usize = 10_000;

/// SSE publisher hook that emits `memory_proposed` frames when
/// cairn-memory creates a Proposed memory.
///
/// Wire into MemoryApiImpl:
/// ```ignore
/// let hook = SseMemoryProposalHook::with_sse_channel(sse_tx, buffer, seq);
/// let api = MemoryApiImpl::new(retrieval, store).with_proposal_hook(Box::new(hook));
/// ```
pub struct SseMemoryProposalHook {
    /// Collected frames for test assertions.
    frames: std::sync::Mutex<Vec<SseFrame>>,
    /// Live SSE broadcast channel sender.
    sse_tx: Option<broadcast::Sender<SseFrame>>,
    /// Replay buffer shared with AppState for Last-Event-ID reconnect.
    sse_buffer: Option<Arc<RwLock<VecDeque<(u64, SseFrame)>>>>,
    /// Monotonic sequence counter shared with AppState.
    sse_seq: Option<Arc<AtomicU64>>,
}

impl Default for SseMemoryProposalHook {
    fn default() -> Self {
        Self::new()
    }
}

impl SseMemoryProposalHook {
    /// Test-only constructor: collects frames in a Vec without broadcasting.
    pub fn new() -> Self {
        Self {
            frames: std::sync::Mutex::new(Vec::new()),
            sse_tx: None,
            sse_buffer: None,
            sse_seq: None,
        }
    }

    /// Production constructor: broadcasts frames to the SSE channel and replay buffer.
    pub fn with_sse_channel(
        sse_tx: broadcast::Sender<SseFrame>,
        sse_buffer: Arc<RwLock<VecDeque<(u64, SseFrame)>>>,
        sse_seq: Arc<AtomicU64>,
    ) -> Self {
        Self {
            frames: std::sync::Mutex::new(Vec::new()),
            sse_tx: Some(sse_tx),
            sse_buffer: Some(sse_buffer),
            sse_seq: Some(sse_seq),
        }
    }

    pub fn collected_frames(&self) -> Vec<SseFrame> {
        self.frames.lock().unwrap().clone()
    }
}

impl MemoryProposalHook for SseMemoryProposalHook {
    fn on_proposed(&self, item: &MemoryItem) {
        let mut frame = build_memory_proposed_frame(item.clone(), None);

        // Assign sequence ID and broadcast if wired to the SSE channel.
        if let (Some(tx), Some(buffer), Some(seq)) =
            (&self.sse_tx, &self.sse_buffer, &self.sse_seq)
        {
            let id = seq.fetch_add(1, Ordering::SeqCst);
            frame.id = Some(id.to_string());

            // Push to replay buffer.
            if let Ok(mut buf) = buffer.write() {
                if buf.len() >= SSE_BUFFER_CAPACITY {
                    buf.pop_front();
                }
                buf.push_back((id, frame.clone()));
            }

            let _ = tx.send(frame.clone());
        }

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

    #[test]
    fn hook_broadcasts_to_sse_channel() {
        let (tx, mut rx) = broadcast::channel(16);
        let buffer = Arc::new(RwLock::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(100));

        let hook = SseMemoryProposalHook::with_sse_channel(tx, buffer.clone(), seq.clone());

        let item = MemoryItem {
            id: "mem_2".to_owned(),
            content: "Broadcast test".to_owned(),
            category: Some("facts".to_owned()),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: "2026-04-03T10:00:00Z".to_owned(),
        };

        hook.on_proposed(&item);

        // Frame was broadcast.
        let received = rx.try_recv().unwrap();
        assert_eq!(received.event, cairn_api::sse::SseEventName::MemoryProposed);
        assert_eq!(received.id, Some("100".to_owned()));

        // Frame was buffered for replay.
        let buf = buffer.read().unwrap();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf[0].0, 100);

        // Sequence counter advanced.
        assert_eq!(seq.load(Ordering::SeqCst), 101);

        // Frame also collected locally.
        assert_eq!(hook.collected_frames().len(), 1);
    }
}
