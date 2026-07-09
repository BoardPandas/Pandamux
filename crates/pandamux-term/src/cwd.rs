//! Per-session working-directory tracking from the PTY byte stream.
//!
//! Shells report their cwd two ways in PandaMUX Everywhere: cmd embeds an OSC
//! 9;9 sequence in its prompt (`ESC ] 9 ; 9 ; <path> ST`), and the standard
//! OSC 7 (`ESC ] 7 ; file://<host><path> ST`) is emitted by many shells. Both
//! arrive inline in the terminal output, so we scan for them here as bytes are
//! fed. (bash/pwsh additionally report over the pipe via `report_pwd`, handled
//! in the app dispatcher; that path calls `PtySessionManager::set_cwd`.)
//!
//! The scanner is incremental: an OSC can be split across PTY reads, so it keeps
//! a bounded payload buffer across `feed` calls and only commits a cwd when a
//! terminator (BEL or ST) closes the sequence.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanState {
    Ground,
    Esc,
    Osc,
}

#[derive(Debug)]
pub struct CwdScanner {
    state: ScanState,
    buffer: Vec<u8>,
    cwd: Option<String>,
}

const MAX_OSC_LEN: usize = 4096;

impl Default for CwdScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl CwdScanner {
    pub fn new() -> Self {
        Self {
            state: ScanState::Ground,
            buffer: Vec::new(),
            cwd: None,
        }
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// Directly set the cwd (used by the pipe `report_pwd` path).
    pub fn set(&mut self, cwd: impl Into<String>) {
        self.cwd = Some(cwd.into());
    }

    /// Feed a chunk of PTY output, updating the cwd if a cwd OSC completes.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match self.state {
                ScanState::Ground => {
                    if byte == 0x1b {
                        self.state = ScanState::Esc;
                    }
                }
                ScanState::Esc => {
                    if byte == b']' {
                        self.state = ScanState::Osc;
                        self.buffer.clear();
                    } else {
                        // Not an OSC introducer; ESC starts a fresh scan.
                        self.state = if byte == 0x1b {
                            ScanState::Esc
                        } else {
                            ScanState::Ground
                        };
                    }
                }
                ScanState::Osc => {
                    // Terminator: BEL, or ST (`ESC \`). Treat ESC as a terminator
                    // and restart escape parsing so `ESC \` and a fresh `ESC ]`
                    // both work.
                    if byte == 0x07 {
                        self.commit();
                        self.state = ScanState::Ground;
                    } else if byte == 0x1b {
                        self.commit();
                        self.state = ScanState::Esc;
                    } else if self.buffer.len() < MAX_OSC_LEN {
                        self.buffer.push(byte);
                    } else {
                        // Overlong OSC: abandon it.
                        self.buffer.clear();
                        self.state = ScanState::Ground;
                    }
                }
            }
        }
    }

    fn commit(&mut self) {
        if let Some(cwd) = parse_cwd_osc(&self.buffer) {
            self.cwd = Some(cwd);
        }
        self.buffer.clear();
    }
}

/// Parse an OSC payload (the bytes between `ESC ]` and the terminator) into a
/// cwd, recognizing `9;9;<path>` (cmd / Windows Terminal) and `7;<uri>` (OSC 7).
fn parse_cwd_osc(payload: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(payload).ok()?;
    if let Some(rest) = text.strip_prefix("9;9;") {
        let path = normalize_path(percent_decode(rest.trim()));
        return (!path.is_empty()).then_some(path);
    }
    if let Some(rest) = text.strip_prefix("7;") {
        let rest = rest.trim();
        let path = match rest.strip_prefix("file://") {
            // Drop the host component up to the first path separator.
            Some(after) => match after.find('/') {
                Some(index) => &after[index..],
                None => "",
            },
            None => rest,
        };
        let path = normalize_path(percent_decode(path));
        return (!path.is_empty()).then_some(path);
    }
    None
}

/// Minimal percent-decoding for OSC 7 URIs.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 3 <= bytes.len()
            && let Ok(byte) = u8::from_str_radix(&input[index + 1..index + 3], 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Turn a Windows OSC 7 path (`/C:/Users/x`) into a native one (`C:/Users/x`).
fn normalize_path(path: String) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return path[1..].to_string();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc_9_9_with_bel_terminator() {
        let mut scanner = CwdScanner::new();
        scanner.feed(b"prompt\x1b]9;9;C:\\Users\\chaz\x07$ ");
        assert_eq!(scanner.cwd(), Some("C:\\Users\\chaz"));
    }

    #[test]
    fn parses_osc_7_file_uri_with_st_terminator() {
        let mut scanner = CwdScanner::new();
        scanner.feed(b"\x1b]7;file://host/C:/Users/chaz%20dev\x1b\\");
        assert_eq!(scanner.cwd(), Some("C:/Users/chaz dev"));
    }

    #[test]
    fn handles_a_sequence_split_across_feeds() {
        let mut scanner = CwdScanner::new();
        scanner.feed(b"\x1b]9;9;/home/ch");
        assert_eq!(scanner.cwd(), None);
        scanner.feed(b"az\x07");
        assert_eq!(scanner.cwd(), Some("/home/chaz"));
    }

    #[test]
    fn ignores_unrelated_osc() {
        let mut scanner = CwdScanner::new();
        scanner.feed(b"\x1b]0;window title\x07");
        assert_eq!(scanner.cwd(), None);
    }

    #[test]
    fn set_overrides_directly() {
        let mut scanner = CwdScanner::new();
        scanner.set("/reported/by/pipe");
        assert_eq!(scanner.cwd(), Some("/reported/by/pipe"));
    }
}
