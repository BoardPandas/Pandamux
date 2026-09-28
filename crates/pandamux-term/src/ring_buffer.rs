use std::collections::VecDeque;

/// Default ring buffer capacity (4 MiB) matching Section 6.3 specifications.
pub const DEFAULT_RING_BUFFER_CAPACITY: usize = 4 * 1024 * 1024;

/// A bounded circular byte buffer addressed by monotonic byte offsets.
///
/// Every byte written advances `head_offset`. When total stored bytes exceed
/// `capacity`, the oldest bytes are evicted and `tail_offset` advances.
/// Clients can request bytes `since_offset`; if the requested offset has been
/// evicted, the ring buffer returns `truncated: true` along with all currently
/// retained bytes so the client can reset its grid and re-render.
#[derive(Clone, Debug)]
pub struct TerminalRingBuffer {
    buffer: VecDeque<u8>,
    capacity: usize,
    head_offset: u64,
    tail_offset: u64,
}

impl Default for TerminalRingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_RING_BUFFER_CAPACITY)
    }
}

impl TerminalRingBuffer {
    /// Creates a new ring buffer with the specified byte capacity.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            buffer: VecDeque::with_capacity(cap.min(64 * 1024)),
            capacity: cap,
            head_offset: 0,
            tail_offset: 0,
        }
    }

    /// Total bytes currently stored in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer is currently empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Maximum capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Monotonic offset of the next byte to be written.
    pub fn head_offset(&self) -> u64 {
        self.head_offset
    }

    /// Monotonic offset of the oldest byte currently retained.
    pub fn tail_offset(&self) -> u64 {
        self.tail_offset
    }

    /// Appends bytes to the ring buffer, evicting oldest bytes if capacity is exceeded.
    pub fn write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        if data.len() >= self.capacity {
            // Data is larger than capacity, retain only the trailing slice
            self.buffer.clear();
            let slice = &data[data.len() - self.capacity..];
            self.buffer.extend(slice);
            self.head_offset = self.head_offset.saturating_add(data.len() as u64);
            self.tail_offset = self.head_offset - self.capacity as u64;
            return;
        }

        self.buffer.extend(data);
        self.head_offset = self.head_offset.saturating_add(data.len() as u64);

        if self.buffer.len() > self.capacity {
            let excess = self.buffer.len() - self.capacity;
            self.buffer.drain(..excess);
            self.tail_offset = self.head_offset - self.buffer.len() as u64;
        }
    }

    /// Reads output accumulated since `since_offset`.
    ///
    /// Returns `(head_offset, bytes, truncated)`:
    /// - If `since_offset < tail_offset`: older bytes were evicted, returns all
    ///   currently retained bytes with `truncated: true`.
    /// - If `since_offset >= tail_offset && since_offset <= head_offset`:
    ///   returns only the new bytes with `truncated: false`.
    /// - If `since_offset > head_offset`: client is ahead, returns empty with
    ///   `truncated: false`.
    pub fn read_since(&self, since_offset: u64) -> (u64, Vec<u8>, bool) {
        if since_offset < self.tail_offset {
            // Buffer was evicted past since_offset
            let mut all = Vec::with_capacity(self.buffer.len());
            let (s1, s2) = self.buffer.as_slices();
            all.extend_from_slice(s1);
            all.extend_from_slice(s2);
            (self.head_offset, all, true)
        } else if since_offset <= self.head_offset {
            let skip = (since_offset - self.tail_offset) as usize;
            let available = self.buffer.len().saturating_sub(skip);
            let mut out = Vec::with_capacity(available);

            let (s1, s2) = self.buffer.as_slices();
            if skip < s1.len() {
                out.extend_from_slice(&s1[skip..]);
                out.extend_from_slice(s2);
            } else {
                let s2_skip = skip - s1.len();
                if s2_skip < s2.len() {
                    out.extend_from_slice(&s2[s2_skip..]);
                }
            }
            (self.head_offset, out, false)
        } else {
            (self.head_offset, Vec::new(), false)
        }
    }

    /// Clears all stored buffer content and resets offsets.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.head_offset = 0;
        self.tail_offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_basic_write_and_read() {
        let mut rb = TerminalRingBuffer::new(1024);
        assert_eq!(rb.head_offset(), 0);
        assert_eq!(rb.tail_offset(), 0);
        assert!(rb.is_empty());

        rb.write(b"hello ");
        assert_eq!(rb.head_offset(), 6);
        assert_eq!(rb.tail_offset(), 0);

        let (head, data, truncated) = rb.read_since(0);
        assert_eq!(head, 6);
        assert_eq!(data, b"hello ");
        assert!(!truncated);

        rb.write(b"world");
        assert_eq!(rb.head_offset(), 11);

        let (head, data, truncated) = rb.read_since(6);
        assert_eq!(head, 11);
        assert_eq!(data, b"world");
        assert!(!truncated);
    }

    #[test]
    fn test_ring_buffer_eviction_and_truncation() {
        // Small 16-byte capacity
        let mut rb = TerminalRingBuffer::new(16);
        rb.write(b"0123456789"); // 10 bytes: offset 0..10
        assert_eq!(rb.head_offset(), 10);
        assert_eq!(rb.tail_offset(), 0);

        rb.write(b"abcdefghij"); // 10 bytes: total 20 bytes, capacity 16 -> evicts 4 bytes (0..4)
        assert_eq!(rb.head_offset(), 20);
        assert_eq!(rb.tail_offset(), 4);
        assert_eq!(rb.len(), 16);

        // Reading from offset 0 should signal truncated because offset 0 was evicted
        let (head, data, truncated) = rb.read_since(0);
        assert_eq!(head, 20);
        assert!(truncated);
        assert_eq!(data.len(), 16);
        assert_eq!(data, b"456789abcdefghij");

        // Reading from offset 10 (still present) should not signal truncated
        let (head, data, truncated) = rb.read_since(10);
        assert_eq!(head, 20);
        assert!(!truncated);
        assert_eq!(data, b"abcdefghij");
    }

    #[test]
    fn test_ring_buffer_large_write_exceeding_capacity() {
        let mut rb = TerminalRingBuffer::new(1024);
        let large_payload = vec![b'x'; 2048];
        rb.write(&large_payload);

        assert_eq!(rb.head_offset(), 2048);
        assert_eq!(rb.tail_offset(), 1024);
        assert_eq!(rb.len(), 1024);

        let (head, data, truncated) = rb.read_since(0);
        assert_eq!(head, 2048);
        assert!(truncated);
        assert_eq!(data.len(), 1024);
    }

    #[test]
    fn test_ring_buffer_performance_20mb_stream() {
        // Simulate cat of a 20 MB file into a 4 MiB buffer
        let mut rb = TerminalRingBuffer::new(4 * 1024 * 1024);
        let chunk = vec![b'A'; 64 * 1024]; // 64 KiB chunks
        let total_chunks = (20 * 1024 * 1024) / chunk.len();

        let start = std::time::Instant::now();
        for _ in 0..total_chunks {
            rb.write(&chunk);
        }
        let elapsed = start.elapsed();

        assert_eq!(rb.head_offset(), 20 * 1024 * 1024);
        assert_eq!(rb.len(), 4 * 1024 * 1024);
        assert_eq!(rb.tail_offset(), 16 * 1024 * 1024);
        // Writing 20 MB should take well under 100ms
        assert!(
            elapsed.as_millis() < 100,
            "20 MB write took too long: {:?}",
            elapsed
        );
    }
}
