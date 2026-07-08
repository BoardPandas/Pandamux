//! Hand-built terminal search over serialized grid lines.
//!
//! This replaces the xterm search addon. It matches on character offsets (grid
//! columns), not byte offsets, so the results line up with the rendered grid.
//! Matches are non-overlapping, left to right, top to bottom.

/// One search hit: a character span `[start, end)` on line `line` of the
/// searched line vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Search behavior toggles (mirrors the xterm find-addon options we expose).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

impl SearchOptions {
    pub fn case_sensitive() -> Self {
        Self {
            case_sensitive: true,
            whole_word: false,
        }
    }
}

/// Find every non-overlapping occurrence of `query` across `lines`.
pub fn search_lines(lines: &[String], query: &str, options: SearchOptions) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = query.chars().collect();
    let mut matches = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let haystack: Vec<char> = line.chars().collect();
        find_in_line(&haystack, &needle, options, line_index, &mut matches);
    }
    matches
}

fn find_in_line(
    haystack: &[char],
    needle: &[char],
    options: SearchOptions,
    line: usize,
    out: &mut Vec<SearchMatch>,
) {
    if needle.is_empty() || needle.len() > haystack.len() {
        return;
    }
    let normalize = |c: char| {
        if options.case_sensitive {
            c
        } else {
            c.to_ascii_lowercase()
        }
    };
    let mut index = 0;
    while index + needle.len() <= haystack.len() {
        let window_matches = needle
            .iter()
            .enumerate()
            .all(|(offset, expected)| normalize(haystack[index + offset]) == normalize(*expected));
        if window_matches {
            let start = index;
            let end = index + needle.len();
            if !options.whole_word || is_word_boundary(haystack, start, end) {
                out.push(SearchMatch { line, start, end });
                index = end; // non-overlapping
                continue;
            }
        }
        index += 1;
    }
}

fn is_word_boundary(haystack: &[char], start: usize, end: usize) -> bool {
    let before_ok = start == 0 || !is_word_char(haystack[start - 1]);
    let after_ok = end >= haystack.len() || !is_word_char(haystack[end]);
    before_ok && after_ok
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn finds_case_insensitive_by_default() {
        let source = lines(&["Error: build FAILED", "error again"]);
        let hits = search_lines(&source, "error", SearchOptions::default());
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0],
            SearchMatch {
                line: 0,
                start: 0,
                end: 5
            }
        );
        assert_eq!(
            hits[1],
            SearchMatch {
                line: 1,
                start: 0,
                end: 5
            }
        );
    }

    #[test]
    fn case_sensitive_narrows_results() {
        let source = lines(&["Error error ERROR"]);
        let hits = search_lines(&source, "error", SearchOptions::case_sensitive());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start, 6);
    }

    #[test]
    fn matches_are_non_overlapping() {
        let source = lines(&["aaaa"]);
        let hits = search_lines(&source, "aa", SearchOptions::default());
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0],
            SearchMatch {
                line: 0,
                start: 0,
                end: 2
            }
        );
        assert_eq!(
            hits[1],
            SearchMatch {
                line: 0,
                start: 2,
                end: 4
            }
        );
    }

    #[test]
    fn whole_word_rejects_substrings() {
        let source = lines(&["cargo cargonaut cargo-watch"]);
        let hits = search_lines(
            &source,
            "cargo",
            SearchOptions {
                case_sensitive: false,
                whole_word: true,
            },
        );
        // "cargo" at 0 matches; "cargonaut" does not (a letter follows); "cargo"
        // in "cargo-watch" matches (a hyphen is a word boundary).
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].start, 0);
        assert_eq!(hits[1].start, 16);
    }

    #[test]
    fn column_offsets_track_characters_not_bytes() {
        // Wide/multibyte chars must not shift the reported column.
        let source = lines(&["\u{732b}\u{732b}cat"]);
        let hits = search_lines(&source, "cat", SearchOptions::default());
        assert_eq!(
            hits[0],
            SearchMatch {
                line: 0,
                start: 2,
                end: 5
            }
        );
    }

    #[test]
    fn empty_query_finds_nothing() {
        let source = lines(&["anything"]);
        assert!(search_lines(&source, "", SearchOptions::default()).is_empty());
    }
}
