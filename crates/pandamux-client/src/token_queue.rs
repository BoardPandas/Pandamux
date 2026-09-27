use std::collections::VecDeque;

/// A frame-aligned token velocity smoothing queue.
/// Smooths rapid token bursts into display pulses targeting 120 FPS (8.33ms / 16.6ms intervals)
/// so UI scrolling remains fluid without hitching or jitter.
#[derive(Clone, Debug)]
pub struct TokenSmoothingQueue {
    buffer: VecDeque<(String, String)>,
    pending_chars: usize,
    target_fps: u32,
}

impl Default for TokenSmoothingQueue {
    fn default() -> Self {
        Self::new(120)
    }
}

impl TokenSmoothingQueue {
    pub fn new(target_fps: u32) -> Self {
        Self {
            buffer: VecDeque::new(),
            pending_chars: 0,
            target_fps: target_fps.max(30),
        }
    }

    /// Enqueue a newly arrived text chunk for an item.
    pub fn push(&mut self, item_id: impl Into<String>, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.pending_chars += text.len();
        self.buffer.push_back((item_id.into(), text));
    }

    /// Number of buffered characters awaiting release.
    pub fn pending_chars(&self) -> usize {
        self.pending_chars
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Drain up to `max_chars` from the queue for the current frame render tick.
    pub fn drain_frame(&mut self, max_chars: usize) -> Vec<(String, String)> {
        let mut result = Vec::new();
        let mut budget = max_chars;

        while budget > 0 && !self.buffer.is_empty() {
            let (item_id, text) = self.buffer.pop_front().unwrap();
            if text.len() <= budget {
                budget -= text.len();
                self.pending_chars -= text.len();
                result.push((item_id, text));
            } else {
                // Split text across frame boundary
                let take = budget;
                let head = text[..take].to_string();
                let tail = text[take..].to_string();
                self.pending_chars -= take;
                self.buffer.push_front((item_id.clone(), tail));
                result.push((item_id, head));
                break;
            }
        }

        result
    }

    /// Flush all remaining tokens immediately (e.g. on turn completed or interrupt).
    pub fn flush(&mut self) -> Vec<(String, String)> {
        self.pending_chars = 0;
        self.buffer.drain(..).collect()
    }

    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}
