//! Hand-built URL detection over grid lines.
//!
//! Replaces the xterm web-links addon. Dependency-free and deterministic: it
//! scans each line for a small set of URL schemes (plus bare `www.`), reports
//! character-offset spans, and trims trailing punctuation so a URL at the end of
//! a sentence does not swallow the period. Offsets are character (grid-column)
//! offsets, matching the search module.

/// A detected link: the character span `[start, end)` on `line`, and the
/// normalized URL to open (bare `www.` hosts get an `http://` scheme).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedLink {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub url: String,
}

const SCHEMES: [&str; 4] = ["https://", "http://", "file://", "ftp://"];

/// Detect links across `lines`.
pub fn detect_links(lines: &[String]) -> Vec<DetectedLink> {
    let mut links = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        detect_in_line(line, line_index, &mut links);
    }
    links
}

fn detect_in_line(line: &str, line_index: usize, out: &mut Vec<DetectedLink>) {
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if !at_boundary(&chars, index) {
            index += 1;
            continue;
        }
        if let Some((end, needs_scheme)) = match_start(&chars, index) {
            // Extend to the end of the URL body.
            let mut stop = end;
            while stop < chars.len() && is_url_char(chars[stop]) {
                stop += 1;
            }
            // Trim trailing punctuation that is almost never part of the URL.
            while stop > end && is_trailing_punct(chars[stop - 1]) {
                stop -= 1;
            }
            if stop > end {
                let raw: String = chars[index..stop].iter().collect();
                let url = if needs_scheme {
                    format!("http://{raw}")
                } else {
                    raw.clone()
                };
                out.push(DetectedLink {
                    line: line_index,
                    start: index,
                    end: stop,
                    url,
                });
                index = stop;
                continue;
            }
        }
        index += 1;
    }
}

/// A URL may only start at the beginning of the line or after a non-URL char.
fn at_boundary(chars: &[char], index: usize) -> bool {
    index == 0 || !is_url_char(chars[index - 1])
}

/// If a scheme (or bare `www.`) starts at `index`, return the index just past
/// the scheme prefix and whether a synthetic `http://` scheme is needed.
fn match_start(chars: &[char], index: usize) -> Option<(usize, bool)> {
    for scheme in SCHEMES {
        if starts_with(chars, index, scheme) {
            return Some((index + scheme.chars().count(), false));
        }
    }
    if starts_with(chars, index, "www.") {
        return Some((index + 4, true));
    }
    None
}

fn starts_with(chars: &[char], index: usize, prefix: &str) -> bool {
    let prefix: Vec<char> = prefix.chars().collect();
    if index + prefix.len() > chars.len() {
        return false;
    }
    prefix
        .iter()
        .enumerate()
        .all(|(offset, expected)| chars[index + offset].eq_ignore_ascii_case(expected))
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | '`' | '|' | '\'' | '(' | ')')
}

fn is_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | ']' | '}' | '>' | '\u{201d}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn detects_https_url() {
        let hits = detect_links(&lines(&["see https://example.com/path for docs"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/path");
        assert_eq!(hits[0].start, 4);
    }

    #[test]
    fn trims_trailing_sentence_punctuation() {
        let hits = detect_links(&lines(&["go to https://example.com."]));
        assert_eq!(hits[0].url, "https://example.com");
        assert!(!hits[0].url.ends_with('.'));
    }

    #[test]
    fn bare_www_gets_http_scheme() {
        let hits = detect_links(&lines(&["visit www.example.org today"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "http://www.example.org");
    }

    #[test]
    fn multiple_links_on_a_line() {
        let hits = detect_links(&lines(&["http://a.com and http://b.com"]));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "http://a.com");
        assert_eq!(hits[1].url, "http://b.com");
    }

    #[test]
    fn offsets_are_character_based() {
        let hits = detect_links(&lines(&["\u{732b} https://x.io"]));
        // Cat glyph is one char at column 0, space at 1, url starts at 2.
        assert_eq!(hits[0].start, 2);
    }

    #[test]
    fn ignores_plain_text() {
        assert!(detect_links(&lines(&["no links here, just words."])).is_empty());
    }
}
