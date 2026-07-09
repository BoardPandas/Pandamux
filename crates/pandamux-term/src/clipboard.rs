//! OSC 52 clipboard policy and bracketed-paste helpers (plan F1).
//!
//! Copy-over-SSH works because a remote program's OSC 52 "store" escape is
//! surfaced by the terminal engine identically whether the byte source is a
//! local PTY or an SSH channel: alacritty decodes the base64 payload and hands
//! us a [`ClipboardStore`], which the app layer forwards to the OS clipboard
//! (arboard). Load (a remote reading the local clipboard) is denied by default;
//! see [`ClipboardPolicy`].

/// Which selection buffer an OSC 52 write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardKind {
    /// The system clipboard (`c`).
    Clipboard,
    /// The primary selection (`p` / `s`).
    Selection,
}

/// A captured OSC 52 clipboard-store request (already base64-decoded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardStore {
    pub kind: ClipboardKind,
    pub text: String,
}

/// Policy for handling OSC 52 traffic. Secure by default: writes (copy) are
/// allowed up to a size cap; reads (a remote reading the local clipboard) are
/// denied unless explicitly opted in per host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardPolicy {
    /// Maximum decoded bytes accepted from a single store; larger writes are
    /// dropped so a remote cannot flood the local clipboard.
    pub max_store_bytes: usize,
    /// Whether a remote may read the local clipboard (OSC 52 load). Off by
    /// default; the plan gates this as a per-host opt-in.
    pub allow_load: bool,
}

impl Default for ClipboardPolicy {
    fn default() -> Self {
        Self {
            // 1 MiB: generous for text, small enough to bound abuse.
            max_store_bytes: 1024 * 1024,
            allow_load: false,
        }
    }
}

impl ClipboardPolicy {
    /// Whether a store of `len` decoded bytes is within the size cap.
    pub fn accepts_store(&self, len: usize) -> bool {
        len <= self.max_store_bytes
    }
}

/// Bracketed-paste start marker (`ESC [ 200 ~`).
pub const PASTE_START: &[u8] = b"\x1b[200~";
/// Bracketed-paste end marker (`ESC [ 201 ~`).
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// Wrap paste `bytes` in bracketed-paste markers when the terminal has requested
/// bracketed-paste mode (DECSET 2004). When it has not, the bytes are returned
/// unchanged. Callers pass [`crate::TerminalGrid::bracketed_paste_active`] as
/// `bracketed`.
pub fn wrap_paste(bytes: &[u8], bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return bytes.to_vec();
    }
    let mut wrapped = Vec::with_capacity(bytes.len() + PASTE_START.len() + PASTE_END.len());
    wrapped.extend_from_slice(PASTE_START);
    wrapped.extend_from_slice(bytes);
    wrapped.extend_from_slice(PASTE_END);
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_denies_load_and_caps_store_by_default() {
        let policy = ClipboardPolicy::default();
        assert!(!policy.allow_load);
        assert!(policy.accepts_store(10));
        assert!(policy.accepts_store(policy.max_store_bytes));
        assert!(!policy.accepts_store(policy.max_store_bytes + 1));
    }

    #[test]
    fn wrap_paste_only_brackets_when_active() {
        assert_eq!(wrap_paste(b"hi", false), b"hi".to_vec());
        let wrapped = wrap_paste(b"hi", true);
        assert!(wrapped.starts_with(PASTE_START));
        assert!(wrapped.ends_with(PASTE_END));
        assert!(wrapped.windows(2).any(|w| w == b"hi"));
    }
}
