//! Thin best-effort wrapper over the OS clipboard (arboard) for plan F1. OSC 52
//! copies captured from a terminal grid (local or SSH) are forwarded here, and
//! `clipboard.copy` / `clipboard.get` pipe methods read/write it. Every call is
//! best-effort: on a headless CI box without a clipboard the calls fail softly
//! rather than panicking.

/// Write `text` to the OS clipboard. Returns an error string on failure.
pub fn set_text(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("open clipboard: {error}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| format!("set clipboard: {error}"))
}

/// Read text from the OS clipboard. Returns an error string on failure (e.g. the
/// clipboard holds a non-text value).
pub fn get_text() -> Result<String, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("open clipboard: {error}"))?;
    clipboard
        .get_text()
        .map_err(|error| format!("get clipboard: {error}"))
}
