use std::collections::VecDeque;

/// A frame-aligned token velocity smoothing queue targeting 120 FPS.
///
/// Smooths rapid token bursts from high-speed AI providers into display pulses
/// targeting 120 FPS (8.33ms intervals), eliminating UI micro-stutters and hitching
/// while maintaining responsive interactive latency under massive bursts.
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
    /// Creates a new token smoothing queue targeting the specified refresh rate (defaults to 120 FPS).
    pub fn new(target_fps: u32) -> Self {
        Self {
            buffer: VecDeque::new(),
            pending_chars: 0,
            target_fps: target_fps.max(30),
        }
    }

    /// Frame duration budget in milliseconds for the target FPS (e.g. ~8.33ms at 120 FPS).
    pub fn frame_interval_ms(&self) -> f32 {
        1000.0 / self.target_fps as f32
    }

    /// Enqueue a newly arrived text chunk for a thread item.
    pub fn push(&mut self, item_id: impl Into<String>, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.pending_chars += text.len();
        self.buffer.push_back((item_id.into(), text));
    }

    /// Total number of buffered characters awaiting release.
    pub fn pending_chars(&self) -> usize {
        self.pending_chars
    }

    /// Whether the smoothing queue is currently empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Calculates an adaptive character release budget for the upcoming 120 FPS frame render tick.
    ///
    /// At low backlogs, releases steady, human-readable typing streams (~6 to 12 chars per frame).
    /// Under large token bursts, dynamically scales up throughput to clear the queue within
    /// ~150 to 200ms (18 to 24 frames at 120 FPS), completely preventing frame drops and UI lag.
    pub fn adaptive_frame_budget(&self) -> usize {
        if self.pending_chars == 0 {
            return 0;
        }

        // Target smooth convergence in ~18 to 20 frames at 120 FPS
        let target_frames = 20;
        let base_pace = (self.pending_chars / target_frames).max(6);

        if self.pending_chars > 2000 {
            base_pace.max(120)
        } else if self.pending_chars > 500 {
            base_pace.max(36)
        } else {
            base_pace.min(self.pending_chars)
        }
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

    /// Advances the smoothing queue by one 120 FPS frame tick using the adaptive frame budget.
    pub fn tick_frame(&mut self) -> Vec<(String, String)> {
        let budget = self.adaptive_frame_budget();
        self.drain_frame(budget)
    }

    /// Flush all remaining tokens immediately (e.g. on turn completed or interrupt).
    pub fn flush(&mut self) -> Vec<(String, String)> {
        self.pending_chars = 0;
        self.buffer.drain(..).collect()
    }

    /// Configured target frame rate (e.g. 120).
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_queue_120fps_defaults() {
        let queue = TokenSmoothingQueue::default();
        assert_eq!(queue.target_fps(), 120);
        let interval = queue.frame_interval_ms();
        assert!((interval - 8.333).abs() < 0.01);
        assert!(queue.is_empty());
        assert_eq!(queue.pending_chars(), 0);
    }

    #[test]
    fn test_token_queue_push_and_drain() {
        let mut queue = TokenSmoothingQueue::new(120);
        queue.push("msg-1", "Hello World! This is smooth streaming.");
        assert_eq!(queue.pending_chars(), 38);

        // Drain first frame with budget of 10 chars
        let chunk1 = queue.drain_frame(10);
        assert_eq!(chunk1.len(), 1);
        assert_eq!(chunk1[0].1, "Hello Worl");
        assert_eq!(queue.pending_chars(), 28);

        // Drain next frame
        let chunk2 = queue.drain_frame(10);
        assert_eq!(chunk2[0].1, "d! This is");
        assert_eq!(queue.pending_chars(), 18);
    }

    #[test]
    fn test_token_queue_adaptive_frame_budget() {
        let mut queue = TokenSmoothingQueue::new(120);

        // Small text
        queue.push("msg-1", "Short prompt");
        let budget_small = queue.adaptive_frame_budget();
        assert!(budget_small >= 6);

        // Massive token burst
        let large_burst = "A".repeat(3000);
        queue.push("msg-2", large_burst);
        let budget_large = queue.adaptive_frame_budget();
        assert!(budget_large >= 120);
    }

    #[test]
    fn test_token_queue_tick_frame_and_flush() {
        let mut queue = TokenSmoothingQueue::new(120);
        queue.push("msg-1", "Token sequence to be flushed");

        let ticked = queue.tick_frame();
        assert!(!ticked.is_empty());
        assert!(queue.pending_chars() > 0);

        // Turn completion flush
        let flushed = queue.flush();
        assert!(!flushed.is_empty());
        assert_eq!(queue.pending_chars(), 0);
        assert!(queue.is_empty());
    }
}
