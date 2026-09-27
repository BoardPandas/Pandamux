use crate::ids::EnvironmentId;
use serde::{Deserialize, Serialize};

/// Terminal session metadata for local or remote shell surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionMeta {
    pub id: String,
    pub environment_id: EnvironmentId,
    pub cwd: String,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    #[serde(default = "default_ring_buffer_bytes")]
    pub ring_buffer_bytes: usize,
}

fn default_ring_buffer_bytes() -> usize {
    1024 * 1024 // 1 MiB default ring buffer
}

/// Ring buffer configuration for terminal output buffering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RingBufferConfig {
    pub capacity_bytes: usize,
    pub max_scrollback_lines: usize,
}

impl Default for RingBufferConfig {
    fn default() -> Self {
        Self {
            capacity_bytes: default_ring_buffer_bytes(),
            max_scrollback_lines: 10_000,
        }
    }
}

/// Requested dimensions for resizing a terminal surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResize {
    pub cols: u16,
    pub rows: u16,
}
