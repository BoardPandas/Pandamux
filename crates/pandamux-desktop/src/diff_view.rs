use std::collections::HashSet;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::theme::{Radii, Theme, ThemeMode, Typography};

/// View mode for the diff viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiffViewMode {
    #[default]
    Unified,
    Split,
}

/// Type of line in a diff hunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Header,
}

/// Classification of a syntax token for syntax-highlighted code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxTokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Operator,
    Punctuation,
    PlainText,
}

impl SyntaxTokenKind {
    pub fn style_name(&self) -> Option<&'static str> {
        match self {
            Self::Keyword => Some("keyword"),
            Self::String => Some("string"),
            Self::Comment => Some("comment"),
            Self::Number => Some("number"),
            Self::Type => Some("type"),
            Self::Function => Some("function"),
            Self::Operator => Some("operator"),
            Self::Punctuation => Some("punctuation.delimiter"),
            Self::PlainText => None,
        }
    }
}

/// A word or token span inside a diff line, with optional intra-line emphasis and syntax highlighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffWord {
    pub text: String,
    pub emphasized: bool,
    pub token_kind: SyntaxTokenKind,
}

/// A parsed line in a diff hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub content: String,
    pub words: Vec<DiffWord>,
}

/// A parsed hunk within a file diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub section_heading: Option<String>,
    pub lines: Vec<DiffLine>,
    pub additions: usize,
    pub deletions: usize,
}

/// A parsed file diff containing hunks and metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffFile {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub display_path: String,
    pub language: String,
    pub hunks: Vec<DiffHunk>,
    pub total_additions: usize,
    pub total_deletions: usize,
    pub is_binary: bool,
}

/// Synchronized row for split (side-by-side) view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitDiffRow {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

/// Interactive state for the diff viewer.
#[derive(Clone, Debug)]
pub struct DiffViewerState {
    pub mode: DiffViewMode,
    pub collapsed_hunks: HashSet<usize>,
    pub word_emphasis: bool,
    pub selected_file_idx: usize,
}

impl Default for DiffViewerState {
    fn default() -> Self {
        Self {
            mode: DiffViewMode::Unified,
            collapsed_hunks: HashSet::new(),
            word_emphasis: true,
            selected_file_idx: 0,
        }
    }
}

impl DiffViewerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mode(mut self, mode: DiffViewMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DiffViewMode::Unified => DiffViewMode::Split,
            DiffViewMode::Split => DiffViewMode::Unified,
        };
    }

    pub fn toggle_hunk(&mut self, hunk_idx: usize) {
        if self.collapsed_hunks.contains(&hunk_idx) {
            self.collapsed_hunks.remove(&hunk_idx);
        } else {
            self.collapsed_hunks.insert(hunk_idx);
        }
    }

    pub fn is_hunk_collapsed(&self, hunk_idx: usize) -> bool {
        self.collapsed_hunks.contains(&hunk_idx)
    }

    pub fn collapse_all(&mut self, total_hunks: usize) {
        for i in 0..total_hunks {
            self.collapsed_hunks.insert(i);
        }
    }

    pub fn expand_all(&mut self) {
        self.collapsed_hunks.clear();
    }

    pub fn toggle_word_emphasis(&mut self) {
        self.word_emphasis = !self.word_emphasis;
    }
}

/// Detects the programming language name from a file path.
pub fn detect_language(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".rs") {
        "rust"
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        "typescript"
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") {
        "javascript"
    } else if lower.ends_with(".py") {
        "python"
    } else if lower.ends_with(".go") {
        "go"
    } else if lower.ends_with(".json") {
        "json"
    } else if lower.ends_with(".toml") {
        "toml"
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "yaml"
    } else if lower.ends_with(".md") {
        "markdown"
    } else if lower.ends_with(".css") {
        "css"
    } else if lower.ends_with(".html") {
        "html"
    } else if lower.ends_with(".sh") || lower.ends_with(".bash") {
        "bash"
    } else if lower.ends_with(".c") || lower.ends_with(".h") {
        "c"
    } else if lower.ends_with(".cpp") || lower.ends_with(".hpp") {
        "cpp"
    } else {
        "text"
    }
}

/// Tokenizes text into word-diff chunks (words, whitespace, and symbols).
fn tokenize_for_word_diff(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = s.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_alphanumeric() || ch == '_' {
            let mut end = start + ch.len_utf8();
            chars.next();
            while let Some(&(idx, next_ch)) = chars.peek() {
                if next_ch.is_alphanumeric() || next_ch == '_' {
                    end = idx + next_ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(&s[start..end]);
        } else if ch.is_whitespace() {
            let mut end = start + ch.len_utf8();
            chars.next();
            while let Some(&(idx, next_ch)) = chars.peek() {
                if next_ch.is_whitespace() {
                    end = idx + next_ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(&s[start..end]);
        } else {
            let end = start + ch.len_utf8();
            chars.next();
            tokens.push(&s[start..end]);
        }
    }

    tokens
}

/// Computes the Longest Common Subsequence between two token sequences.
fn compute_lcs<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<&'a str> {
    let n = a.len();
    let m = b.len();
    if n == 0 || m == 0 {
        return Vec::new();
    }

    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in 0..n {
        for j in 0..m {
            if a[i] == b[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut lcs = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            lcs.push(a[i - 1]);
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    lcs.reverse();
    lcs
}

/// Tokenizes line content into words with intra-line emphasis relative to counter-tokens.
fn diff_words_with_emphasis(
    line: &str,
    paired_counter_tokens: Option<&[&str]>,
    is_addition: bool,
    language: &str,
) -> Vec<DiffWord> {
    let tokens = tokenize_for_word_diff(line);

    let emphasized_indices: HashSet<usize> = if let Some(counter) = paired_counter_tokens {
        let lcs = if is_addition {
            compute_lcs(counter, &tokens)
        } else {
            compute_lcs(&tokens, counter)
        };

        let mut matched = HashSet::new();
        let mut lcs_idx = 0;

        for (idx, &token) in tokens.iter().enumerate() {
            if lcs_idx < lcs.len() && token == lcs[lcs_idx] {
                lcs_idx += 1;
            } else if !token.trim().is_empty() {
                matched.insert(idx);
            }
        }
        matched
    } else {
        HashSet::new()
    };

    let mut result = Vec::with_capacity(tokens.len());
    for (idx, &token) in tokens.iter().enumerate() {
        let emphasized = emphasized_indices.contains(&idx);
        let token_kind = classify_syntax_token(token, language);
        result.push(DiffWord {
            text: token.to_string(),
            emphasized,
            token_kind,
        });
    }

    result
}

/// Classifies a token for language-specific syntax highlighting.
fn classify_syntax_token(token: &str, language: &str) -> SyntaxTokenKind {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return SyntaxTokenKind::PlainText;
    }

    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
        return SyntaxTokenKind::Comment;
    }

    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('`') && trimmed.ends_with('`'))
    {
        return SyntaxTokenKind::String;
    }

    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return SyntaxTokenKind::Number;
    }

    match language {
        "rust" => match trimmed {
            "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn"
            | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in"
            | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return"
            | "self" | "Self" | "static" | "struct" | "super" | "trait" | "true" | "type"
            | "unsafe" | "use" | "where" | "while" => SyntaxTokenKind::Keyword,
            _ if trimmed.chars().next().is_some_and(|c| c.is_uppercase()) => SyntaxTokenKind::Type,
            "+" | "-" | "*" | "/" | "%" | "=" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&"
            | "||" | "!" | "&" | "|" | "^" | "<<" | ">>" | "=>" | "->" => SyntaxTokenKind::Operator,
            "," | ";" | ":" | "::" | "." | "{" | "}" | "(" | ")" | "[" | "]" => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::PlainText,
        },
        "typescript" | "javascript" => match trimmed {
            "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger"
            | "default" | "delete" | "do" | "else" | "export" | "extends" | "finally" | "for"
            | "function" | "if" | "import" | "in" | "instanceof" | "new" | "return" | "super"
            | "switch" | "this" | "throw" | "try" | "typeof" | "var" | "void" | "while"
            | "with" | "yield" | "let" | "static" | "enum" | "await" | "async" | "from" | "as"
            | "true" | "false" | "null" | "undefined" => SyntaxTokenKind::Keyword,
            _ if trimmed.chars().next().is_some_and(|c| c.is_uppercase()) => SyntaxTokenKind::Type,
            _ => SyntaxTokenKind::PlainText,
        },
        "python" => match trimmed {
            "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue"
            | "def" | "del" | "elif" | "else" | "except" | "finally" | "for" | "from"
            | "global" | "if" | "import" | "in" | "is" | "lambda" | "nonlocal" | "not" | "or"
            | "pass" | "raise" | "return" | "try" | "while" | "with" | "yield" | "True"
            | "False" | "None" => SyntaxTokenKind::Keyword,
            _ if trimmed.chars().next().is_some_and(|c| c.is_uppercase()) => SyntaxTokenKind::Type,
            _ => SyntaxTokenKind::PlainText,
        },
        _ => SyntaxTokenKind::PlainText,
    }
}

/// Parses a unified diff string into a list of DiffFiles.
pub fn parse_unified_diff(raw_diff: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let mut current_old_line = 0usize;
    let mut current_new_line = 0usize;

    // Buffer to track consecutive removed/added lines for intra-line word emphasis
    let mut pending_removed: Vec<usize> = Vec::new();
    let mut pending_added: Vec<usize> = Vec::new();

    let flush_pending_emphasis = |hunk: &mut DiffHunk,
                                  pending_removed: &mut Vec<usize>,
                                  pending_added: &mut Vec<usize>,
                                  language: &str| {
        let pair_count = pending_removed.len().min(pending_added.len());
        for i in 0..pair_count {
            let rem_idx = pending_removed[i];
            let add_idx = pending_added[i];

            let rem_content = hunk.lines[rem_idx].content.clone();
            let add_content = hunk.lines[add_idx].content.clone();

            let rem_tokens = tokenize_for_word_diff(&rem_content);
            let add_tokens = tokenize_for_word_diff(&add_content);

            hunk.lines[rem_idx].words =
                diff_words_with_emphasis(&rem_content, Some(&add_tokens), false, language);
            hunk.lines[add_idx].words =
                diff_words_with_emphasis(&add_content, Some(&rem_tokens), true, language);
        }
        pending_removed.clear();
        pending_added.clear();
    };

    for line in raw_diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(mut hunk) = current_hunk.take()
                && let Some(file) = current_file.as_mut()
            {
                flush_pending_emphasis(
                    &mut hunk,
                    &mut pending_removed,
                    &mut pending_added,
                    &file.language,
                );
                file.total_additions += hunk.additions;
                file.total_deletions += hunk.deletions;
                file.hunks.push(hunk);
            }
            if let Some(file) = current_file.take() {
                files.push(file);
            }

            // Extract file names: "diff --git a/src/lib.rs b/src/lib.rs"
            let parts: Vec<&str> = line.split_whitespace().collect();
            let old_path = parts.get(2).map(|p| p.trim_start_matches("a/").to_string());
            let new_path = parts.get(3).map(|p| p.trim_start_matches("b/").to_string());
            let display_path = new_path
                .clone()
                .or_else(|| old_path.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let language = detect_language(&display_path).to_string();

            current_file = Some(DiffFile {
                old_path,
                new_path,
                display_path,
                language,
                hunks: Vec::new(),
                total_additions: 0,
                total_deletions: 0,
                is_binary: false,
            });
        } else if line.starts_with("--- ") {
            if current_file.is_none() {
                let path = line
                    .trim_start_matches("--- ")
                    .trim()
                    .trim_start_matches("a/");
                let display = path.to_string();
                let language = detect_language(&display).to_string();
                current_file = Some(DiffFile {
                    old_path: Some(display.clone()),
                    new_path: None,
                    display_path: display,
                    language,
                    hunks: Vec::new(),
                    total_additions: 0,
                    total_deletions: 0,
                    is_binary: false,
                });
            }
        } else if line.starts_with("+++ ") {
            let path = line
                .trim_start_matches("+++ ")
                .trim()
                .trim_start_matches("b/");
            if let Some(file) = current_file.as_mut() {
                file.new_path = Some(path.to_string());
                if file.display_path == "unknown" || file.display_path.is_empty() {
                    file.display_path = path.to_string();
                    file.language = detect_language(path).to_string();
                }
            } else {
                let display = path.to_string();
                let language = detect_language(&display).to_string();
                current_file = Some(DiffFile {
                    old_path: None,
                    new_path: Some(display.clone()),
                    display_path: display,
                    language,
                    hunks: Vec::new(),
                    total_additions: 0,
                    total_deletions: 0,
                    is_binary: false,
                });
            }
        } else if line.starts_with("Binary files ") {
            if let Some(file) = current_file.as_mut() {
                file.is_binary = true;
            }
        } else if line.starts_with("@@ ") {
            // New hunk header: @@ -old_start,old_count +new_start,new_count @@ heading
            if let Some(mut hunk) = current_hunk.take()
                && let Some(file) = current_file.as_mut()
            {
                flush_pending_emphasis(
                    &mut hunk,
                    &mut pending_removed,
                    &mut pending_added,
                    &file.language,
                );
                file.total_additions += hunk.additions;
                file.total_deletions += hunk.deletions;
                file.hunks.push(hunk);
            }

            if current_file.is_none() {
                current_file = Some(DiffFile {
                    old_path: None,
                    new_path: None,
                    display_path: "diff_snippet.txt".to_string(),
                    language: "text".to_string(),
                    hunks: Vec::new(),
                    total_additions: 0,
                    total_deletions: 0,
                    is_binary: false,
                });
            }

            let (old_start, old_count, new_start, new_count, heading) = parse_hunk_header(line);
            current_old_line = old_start;
            current_new_line = new_start;

            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                old_start,
                old_count,
                new_start,
                new_count,
                section_heading: heading,
                lines: Vec::new(),
                additions: 0,
                deletions: 0,
            });
        } else if let Some(hunk) = current_hunk.as_mut() {
            let lang = current_file
                .as_ref()
                .map(|f| f.language.as_str())
                .unwrap_or("text");

            if line.starts_with('+') && !line.starts_with("+++") {
                let content = line[1..].to_string();
                let words = diff_words_with_emphasis(&content, None, true, lang);
                let line_idx = hunk.lines.len();
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_lineno: None,
                    new_lineno: Some(current_new_line),
                    content,
                    words,
                });
                hunk.additions += 1;
                current_new_line += 1;
                pending_added.push(line_idx);
            } else if line.starts_with('-') && !line.starts_with("---") {
                let content = line[1..].to_string();
                let words = diff_words_with_emphasis(&content, None, false, lang);
                let line_idx = hunk.lines.len();
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_lineno: Some(current_old_line),
                    new_lineno: None,
                    content,
                    words,
                });
                hunk.deletions += 1;
                current_old_line += 1;
                pending_removed.push(line_idx);
            } else if line.starts_with(' ') || line.is_empty() {
                flush_pending_emphasis(hunk, &mut pending_removed, &mut pending_added, lang);

                let content = if let Some(stripped) = line.strip_prefix(' ') {
                    stripped.to_string()
                } else {
                    line.to_string()
                };
                let words = diff_words_with_emphasis(&content, None, false, lang);
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    old_lineno: Some(current_old_line),
                    new_lineno: Some(current_new_line),
                    content,
                    words,
                });
                current_old_line += 1;
                current_new_line += 1;
            }
        } else if (line.starts_with('+') || line.starts_with('-')) && current_hunk.is_none() {
            // Implicit raw hunk when no @@ header was supplied
            if current_file.is_none() {
                current_file = Some(DiffFile {
                    old_path: None,
                    new_path: None,
                    display_path: "snippet".to_string(),
                    language: "text".to_string(),
                    hunks: Vec::new(),
                    total_additions: 0,
                    total_deletions: 0,
                    is_binary: false,
                });
            }

            let lang = current_file
                .as_ref()
                .map(|f| f.language.clone())
                .unwrap_or_else(|| "text".to_string());

            current_old_line = 1;
            current_new_line = 1;

            let mut hunk = DiffHunk {
                header: "@@ -1,1 +1,1 @@".to_string(),
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 1,
                section_heading: None,
                lines: Vec::new(),
                additions: 0,
                deletions: 0,
            };

            let is_add = line.starts_with('+');
            let content = line[1..].to_string();
            let words = diff_words_with_emphasis(&content, None, is_add, &lang);

            if is_add {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_lineno: None,
                    new_lineno: Some(1),
                    content,
                    words,
                });
                hunk.additions += 1;
                current_new_line += 1;
            } else {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_lineno: Some(1),
                    new_lineno: None,
                    content,
                    words,
                });
                hunk.deletions += 1;
                current_old_line += 1;
            }
            current_hunk = Some(hunk);
        }
    }

    if let Some(mut hunk) = current_hunk.take()
        && let Some(file) = current_file.as_mut()
    {
        flush_pending_emphasis(
            &mut hunk,
            &mut pending_removed,
            &mut pending_added,
            &file.language,
        );
        file.total_additions += hunk.additions;
        file.total_deletions += hunk.deletions;
        file.hunks.push(hunk);
    }
    if let Some(file) = current_file.take() {
        files.push(file);
    }

    files
}

/// Helper to parse hunk header: `@@ -10,4 +10,6 @@ section title`
fn parse_hunk_header(line: &str) -> (usize, usize, usize, usize, Option<String>) {
    let mut old_start = 1;
    let mut old_count = 1;
    let mut new_start = 1;
    let mut new_count = 1;

    let parts: Vec<&str> = line.split("@@").collect();
    if parts.len() >= 2 {
        let coords = parts[1].trim();
        let coord_parts: Vec<&str> = coords.split_whitespace().collect();

        if let Some(old_str) = coord_parts.first() {
            let old_str = old_str.trim_start_matches('-');
            let sub: Vec<&str> = old_str.split(',').collect();
            old_start = sub[0].parse().unwrap_or(1);
            if sub.len() > 1 {
                old_count = sub[1].parse().unwrap_or(1);
            }
        }

        if let Some(new_str) = coord_parts.get(1) {
            let new_str = new_str.trim_start_matches('+');
            let sub: Vec<&str> = new_str.split(',').collect();
            new_start = sub[0].parse().unwrap_or(1);
            if sub.len() > 1 {
                new_count = sub[1].parse().unwrap_or(1);
            }
        }
    }

    let heading = if parts.len() >= 3 {
        let trimmed = parts[2].trim();
        if !trimmed.is_empty() {
            Some(trimmed.to_string())
        } else {
            None
        }
    } else {
        None
    };

    (old_start, old_count, new_start, new_count, heading)
}

/// Generates synchronized rows for Split (side-by-side) view from a hunk.
pub fn generate_split_rows(hunk: &DiffHunk) -> Vec<SplitDiffRow> {
    let mut rows = Vec::new();
    let mut idx = 0;
    let lines = &hunk.lines;

    while idx < lines.len() {
        let line = &lines[idx];
        match line.kind {
            DiffLineKind::Context => {
                rows.push(SplitDiffRow {
                    left: Some(line.clone()),
                    right: Some(line.clone()),
                });
                idx += 1;
            }
            DiffLineKind::Removed => {
                // Collect run of removals
                let mut rems = Vec::new();
                while idx < lines.len() && lines[idx].kind == DiffLineKind::Removed {
                    rems.push(lines[idx].clone());
                    idx += 1;
                }
                // Collect immediately following additions
                let mut adds = Vec::new();
                while idx < lines.len() && lines[idx].kind == DiffLineKind::Added {
                    adds.push(lines[idx].clone());
                    idx += 1;
                }

                let max_len = rems.len().max(adds.len());
                for i in 0..max_len {
                    rows.push(SplitDiffRow {
                        left: rems.get(i).cloned(),
                        right: adds.get(i).cloned(),
                    });
                }
            }
            DiffLineKind::Added => {
                // Standalone addition run without preceding removals
                rows.push(SplitDiffRow {
                    left: None,
                    right: Some(line.clone()),
                });
                idx += 1;
            }
            DiffLineKind::Header => {
                idx += 1;
            }
        }
    }

    rows
}

/// Resolves syntax highlighting color from gpui-kit's HighlightTheme.
fn get_syntax_color(
    kind: SyntaxTokenKind,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
) -> Option<Hsla> {
    kind.style_name()
        .and_then(|name| highlight_theme.style(name))
        .and_then(|style| style.color)
        .or_else(|| match kind {
            SyntaxTokenKind::Keyword => Some(theme.accent.color().into()),
            SyntaxTokenKind::String => Some(theme.terminal.success.into()),
            SyntaxTokenKind::Comment => Some(theme.terminal.dim.into()),
            SyntaxTokenKind::Number => Some(theme.terminal.warn.into()),
            SyntaxTokenKind::Type => Some(rgb(0x60a5fa).into()),
            SyntaxTokenKind::Function => Some(rgb(0x93c5fd).into()),
            SyntaxTokenKind::Operator => Some(theme.chrome.text_t3.into()),
            SyntaxTokenKind::Punctuation => Some(theme.chrome.text_t4.into()),
            SyntaxTokenKind::PlainText => None,
        })
}

/// Renders a single diff line content with word-level emphasis and syntax highlighting.
fn render_words_span(
    line: &DiffLine,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
    word_emphasis_active: bool,
) -> impl IntoElement {
    div()
        .h_flex()
        .flex_wrap()
        .items_center()
        .children(line.words.iter().map(|word| {
            let (bg_color, text_color, font_weight) = match line.kind {
                DiffLineKind::Added => {
                    let base_text: Hsla = get_syntax_color(word.token_kind, theme, highlight_theme)
                        .unwrap_or_else(|| theme.terminal.success.into());
                    if word.emphasized && word_emphasis_active {
                        (
                            rgba(0x7fd88f40),
                            theme.terminal.success.into(),
                            FontWeight::BOLD,
                        )
                    } else {
                        (rgba(0x00000000), base_text, FontWeight::NORMAL)
                    }
                }
                DiffLineKind::Removed => {
                    let base_text: Hsla = get_syntax_color(word.token_kind, theme, highlight_theme)
                        .unwrap_or_else(|| rgb(0xf87171).into());
                    if word.emphasized && word_emphasis_active {
                        (rgba(0xf8717140), rgb(0xf87171).into(), FontWeight::BOLD)
                    } else {
                        (rgba(0x00000000), base_text, FontWeight::NORMAL)
                    }
                }
                _ => {
                    let base_text: Hsla = get_syntax_color(word.token_kind, theme, highlight_theme)
                        .unwrap_or_else(|| theme.chrome.text_t1.into());
                    (rgba(0x00000000), base_text, FontWeight::NORMAL)
                }
            };

            div()
                .bg(bg_color)
                .rounded(Radii::CHIP)
                .px_0p5()
                .text_color(text_color)
                .font_weight(font_weight)
                .child(if word.text.is_empty() {
                    " ".to_string()
                } else {
                    word.text.clone()
                })
        }))
}

/// Renders a diff hunk in Unified mode.
pub fn render_diff_hunk_unified(
    hunk_idx: usize,
    hunk: &DiffHunk,
    is_collapsed: bool,
    word_emphasis: bool,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
    on_toggle_hunk: impl Fn(usize) + 'static + Clone,
) -> AnyElement {
    let hunk_toggle = on_toggle_hunk.clone();
    let old_range = format!("-{},{}", hunk.old_start, hunk.old_count);
    let new_range = format!("+{},{}", hunk.new_start, hunk.new_count);

    let arrow = if is_collapsed { "▶" } else { "▼" };

    div()
        .w_full()
        .v_flex()
        .border_b_1()
        .border_color(rgba(0xffffff0d))
        // Hunk Header Bar
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .bg(rgba(0x43d9c90a))
                .border_t_1()
                .border_b_1()
                .border_color(rgba(0x43d9c91f))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_ev, _win, _cx| {
                    hunk_toggle(hunk_idx);
                })
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(arrow),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0f))
                                .text_size(Typography::META_SIZE)
                                .font_family(Typography::MONO_FAMILY)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(format!("@@ {old_range} {new_range} @@")),
                        )
                        .when_some(hunk.section_heading.clone(), |this, heading| {
                            this.child(
                                div()
                                    .text_size(Typography::META_SIZE)
                                    .font_family(Typography::MONO_FAMILY)
                                    .text_color(theme.chrome.text_t3)
                                    .child(heading),
                            )
                        }),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .child(
                            div()
                                .text_color(theme.terminal.success)
                                .child(format!("+{}", hunk.additions)),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xf87171))
                                .child(format!("-{}", hunk.deletions)),
                        ),
                ),
        )
        // Hunk Lines
        .when(!is_collapsed, |this| {
            this.children(hunk.lines.iter().map(|line| {
                let (line_bg, border_color, prefix_char, prefix_color) = match line.kind {
                    DiffLineKind::Added => (
                        rgba(0x7fd88f14),
                        rgba(0x7fd88f33),
                        "+",
                        theme.terminal.success,
                    ),
                    DiffLineKind::Removed => {
                        (rgba(0xf8717114), rgba(0xf8717133), "-", rgb(0xf87171))
                    }
                    DiffLineKind::Context => (
                        rgba(0x00000000),
                        rgba(0x00000000),
                        " ",
                        theme.chrome.text_t4,
                    ),
                    DiffLineKind::Header => (
                        rgba(0x43d9c90a),
                        rgba(0x43d9c91a),
                        "@",
                        theme.accent.color(),
                    ),
                };

                let old_num_str = line.old_lineno.map(|n| n.to_string()).unwrap_or_default();
                let new_num_str = line.new_lineno.map(|n| n.to_string()).unwrap_or_default();

                div()
                    .w_full()
                    .h_flex()
                    .items_center()
                    .bg(line_bg)
                    .border_l_2()
                    .border_color(border_color)
                    .font_family(Typography::MONO_FAMILY)
                    .text_size(Typography::META_SIZE)
                    // Line number gutter (Old line)
                    .child(
                        div()
                            .w(px(36.0))
                            .px_1()
                            .text_align(TextAlign::Right)
                            .text_color(theme.chrome.text_t4)
                            .child(old_num_str),
                    )
                    // Line number gutter (New line)
                    .child(
                        div()
                            .w(px(36.0))
                            .px_1()
                            .text_align(TextAlign::Right)
                            .text_color(theme.chrome.text_t4)
                            .border_r_1()
                            .border_color(rgba(0xffffff0d))
                            .child(new_num_str),
                    )
                    // Prefix symbol (+ / - / space)
                    .child(
                        div()
                            .w(px(20.0))
                            .text_align(TextAlign::Center)
                            .font_weight(FontWeight::BOLD)
                            .text_color(prefix_color)
                            .child(prefix_char),
                    )
                    // Line Content with word emphasis
                    .child(
                        div()
                            .flex_1()
                            .px_1()
                            .overflow_hidden()
                            .child(render_words_span(
                                line,
                                theme,
                                highlight_theme,
                                word_emphasis,
                            )),
                    )
            }))
        })
        .into_any_element()
}

/// Renders a diff hunk in Split (side-by-side) mode.
pub fn render_diff_hunk_split(
    hunk_idx: usize,
    hunk: &DiffHunk,
    is_collapsed: bool,
    word_emphasis: bool,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
    on_toggle_hunk: impl Fn(usize) + 'static + Clone,
) -> AnyElement {
    let hunk_toggle = on_toggle_hunk.clone();
    let old_range = format!("-{},{}", hunk.old_start, hunk.old_count);
    let new_range = format!("+{},{}", hunk.new_start, hunk.new_count);
    let arrow = if is_collapsed { "▶" } else { "▼" };

    let split_rows = generate_split_rows(hunk);

    div()
        .w_full()
        .v_flex()
        .border_b_1()
        .border_color(rgba(0xffffff0d))
        // Hunk Header Bar
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .bg(rgba(0x43d9c90a))
                .border_t_1()
                .border_b_1()
                .border_color(rgba(0x43d9c91f))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_ev, _win, _cx| {
                    hunk_toggle(hunk_idx);
                })
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(arrow),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0f))
                                .text_size(Typography::META_SIZE)
                                .font_family(Typography::MONO_FAMILY)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(format!("@@ {old_range} {new_range} @@")),
                        )
                        .when_some(hunk.section_heading.clone(), |this, heading| {
                            this.child(
                                div()
                                    .text_size(Typography::META_SIZE)
                                    .font_family(Typography::MONO_FAMILY)
                                    .text_color(theme.chrome.text_t3)
                                    .child(heading),
                            )
                        }),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .child(
                            div()
                                .text_color(theme.terminal.success)
                                .child(format!("+{}", hunk.additions)),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xf87171))
                                .child(format!("-{}", hunk.deletions)),
                        ),
                ),
        )
        // Split Rows (Left: Old / Right: New)
        .when(!is_collapsed, |this| {
            this.children(split_rows.into_iter().map(|row| {
                div()
                    .w_full()
                    .h_flex()
                    .font_family(Typography::MONO_FAMILY)
                    .text_size(Typography::META_SIZE)
                    // Left Column (Deletions / Context)
                    .child(render_split_cell(
                        row.left.as_ref(),
                        true,
                        word_emphasis,
                        theme,
                        highlight_theme,
                    ))
                    // Divider
                    .child(div().w(px(1.0)).h_full().bg(rgba(0xffffff14)))
                    // Right Column (Additions / Context)
                    .child(render_split_cell(
                        row.right.as_ref(),
                        false,
                        word_emphasis,
                        theme,
                        highlight_theme,
                    ))
            }))
        })
        .into_any_element()
}

/// Helper to render one half of a split diff row.
fn render_split_cell(
    line_opt: Option<&DiffLine>,
    is_left: bool,
    word_emphasis: bool,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
) -> impl IntoElement {
    if let Some(line) = line_opt {
        let (bg_color, border_col, prefix_char, prefix_color, lineno_str) = match line.kind {
            DiffLineKind::Removed => (
                rgba(0xf8717114),
                rgba(0xf8717133),
                "-",
                rgb(0xf87171),
                line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
            ),
            DiffLineKind::Added => (
                rgba(0x7fd88f14),
                rgba(0x7fd88f33),
                "+",
                theme.terminal.success,
                line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
            ),
            _ => (
                rgba(0x00000000),
                rgba(0x00000000),
                " ",
                theme.chrome.text_t4,
                if is_left {
                    line.old_lineno.map(|n| n.to_string()).unwrap_or_default()
                } else {
                    line.new_lineno.map(|n| n.to_string()).unwrap_or_default()
                },
            ),
        };

        div()
            .flex_1()
            .h_flex()
            .items_center()
            .bg(bg_color)
            .border_l_2()
            .border_color(border_col)
            .child(
                div()
                    .w(px(36.0))
                    .px_1()
                    .text_align(TextAlign::Right)
                    .text_color(theme.chrome.text_t4)
                    .border_r_1()
                    .border_color(rgba(0xffffff0d))
                    .child(lineno_str),
            )
            .child(
                div()
                    .w(px(18.0))
                    .text_align(TextAlign::Center)
                    .font_weight(FontWeight::BOLD)
                    .text_color(prefix_color)
                    .child(prefix_char),
            )
            .child(
                div()
                    .flex_1()
                    .px_1()
                    .overflow_hidden()
                    .child(render_words_span(
                        line,
                        theme,
                        highlight_theme,
                        word_emphasis,
                    )),
            )
    } else {
        // Blank placeholder spacer row
        div()
            .flex_1()
            .h_flex()
            .bg(rgba(0x0000001f))
            .child(
                div()
                    .w(px(36.0))
                    .border_r_1()
                    .border_color(rgba(0xffffff0d))
                    .child(" "),
            )
            .child(div().w(px(18.0)).child(" "))
            .child(div().flex_1().child(" "))
    }
}

/// Renders a full file diff component with header, controls, and hunks.
pub fn render_diff_file(
    file: &DiffFile,
    state: &DiffViewerState,
    theme: &Theme,
    highlight_theme: &HighlightTheme,
    on_toggle_mode: impl Fn() + 'static + Clone,
    on_toggle_hunk: impl Fn(usize) + 'static + Clone,
    on_toggle_emphasis: impl Fn() + 'static + Clone,
) -> AnyElement {
    let mode_toggle = on_toggle_mode.clone();
    let emp_toggle = on_toggle_emphasis.clone();

    div()
        .w_full()
        .rounded(Radii::ROW)
        .bg(theme.terminal.surface)
        .border_1()
        .border_color(rgba(0xffffff14))
        .overflow_hidden()
        .v_flex()
        // Top Toolbar & File Header
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_2p5()
                .bg(theme.chrome.panel2)
                .border_b_1()
                .border_color(rgba(0xffffff0d))
                // File info
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_size(Typography::BODY_SIZE).child("📄"))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child(file.display_path.clone()),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0a))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(file.language.clone()),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x43d9c914))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child("Read-Only"),
                        ),
                )
                // Controls (Mode Switcher, Word Diff, Stats)
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        // Additions / Deletions pills
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(rgba(0x7fd88f1f))
                                        .text_size(Typography::META_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.terminal.success)
                                        .child(format!("+{}", file.total_additions)),
                                )
                                .child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(rgba(0xf871711f))
                                        .text_size(Typography::META_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0xf87171))
                                        .child(format!("-{}", file.total_deletions)),
                                ),
                        )
                        // Word Emphasis Toggle Pill
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded(Radii::CHIP)
                                .bg(if state.word_emphasis {
                                    rgba(0x43d9c926)
                                } else {
                                    rgba(0xffffff0d)
                                })
                                .border_1()
                                .border_color(if state.word_emphasis {
                                    theme.accent.color()
                                } else {
                                    rgba(0xffffff14)
                                })
                                .cursor_pointer()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(if state.word_emphasis {
                                    theme.accent.color()
                                } else {
                                    theme.chrome.text_t3
                                })
                                .on_mouse_down(MouseButton::Left, move |_ev, _win, _cx| {
                                    emp_toggle();
                                })
                                .child(if state.word_emphasis {
                                    "Word Diff: ON"
                                } else {
                                    "Word Diff: OFF"
                                }),
                        )
                        // Unified vs Split Mode Switcher Pill
                        .child(
                            div()
                                .h_flex()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x00000040))
                                .p_0p5()
                                .border_1()
                                .border_color(rgba(0xffffff14))
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .cursor_pointer()
                                        .bg(if state.mode == DiffViewMode::Unified {
                                            theme.accent.color()
                                        } else {
                                            rgba(0x00000000)
                                        })
                                        .text_color(if state.mode == DiffViewMode::Unified {
                                            rgb(0x0c1114)
                                        } else {
                                            theme.chrome.text_t3
                                        })
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(Typography::META_SIZE)
                                        .on_mouse_down(MouseButton::Left, {
                                            let toggle = mode_toggle.clone();
                                            move |_ev, _win, _cx| {
                                                toggle();
                                            }
                                        })
                                        .child("Unified"),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .cursor_pointer()
                                        .bg(if state.mode == DiffViewMode::Split {
                                            theme.accent.color()
                                        } else {
                                            rgba(0x00000000)
                                        })
                                        .text_color(if state.mode == DiffViewMode::Split {
                                            rgb(0x0c1114)
                                        } else {
                                            theme.chrome.text_t3
                                        })
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(Typography::META_SIZE)
                                        .on_mouse_down(MouseButton::Left, move |_ev, _win, _cx| {
                                            mode_toggle();
                                        })
                                        .child("Split"),
                                ),
                        ),
                ),
        )
        // Hunks List
        .child(
            div()
                .w_full()
                .v_flex()
                .children(file.hunks.iter().enumerate().map(|(hunk_idx, hunk)| {
                    let is_collapsed = state.is_hunk_collapsed(hunk_idx);
                    if state.mode == DiffViewMode::Split {
                        render_diff_hunk_split(
                            hunk_idx,
                            hunk,
                            is_collapsed,
                            state.word_emphasis,
                            theme,
                            highlight_theme,
                            on_toggle_hunk.clone(),
                        )
                    } else {
                        render_diff_hunk_unified(
                            hunk_idx,
                            hunk,
                            is_collapsed,
                            state.word_emphasis,
                            theme,
                            highlight_theme,
                            on_toggle_hunk.clone(),
                        )
                    }
                })),
        )
        .into_any_element()
}

/// Renders a full diff viewer with multi-file support and collapsible hunks.
pub fn render_diff_viewer(
    raw_diff: &str,
    state: &DiffViewerState,
    theme: &Theme,
    on_toggle_mode: impl Fn() + 'static + Clone,
    on_toggle_hunk: impl Fn(usize) + 'static + Clone,
    on_toggle_emphasis: impl Fn() + 'static + Clone,
) -> AnyElement {
    let files = parse_unified_diff(raw_diff);

    let highlight_theme = if theme.mode == ThemeMode::Dark {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
    };

    if files.is_empty() {
        return div()
            .p_4()
            .rounded(Radii::ROW)
            .bg(theme.terminal.surface)
            .border_1()
            .border_color(rgba(0xffffff0d))
            .text_size(Typography::BODY_SIZE)
            .text_color(theme.chrome.text_t3)
            .child("No diff changes to display.")
            .into_any_element();
    }

    div()
        .w_full()
        .v_flex()
        .gap_3()
        .children(files.iter().map(|file| {
            render_diff_file(
                file,
                state,
                theme,
                &highlight_theme,
                on_toggle_mode.clone(),
                on_toggle_hunk.clone(),
                on_toggle_emphasis.clone(),
            )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_parse_unified_diff_basic() {
        let raw = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc..def 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,5 @@ fn test() {
-let old = 1;
+let new = 2;
+let extra = 3;
 println!("done");
"#;

        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 1);

        let file = &files[0];
        assert_eq!(file.display_path, "src/lib.rs");
        assert_eq!(file.language, "rust");
        assert_eq!(file.total_additions, 2);
        assert_eq!(file.total_deletions, 1);
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 10);
        assert_eq!(hunk.new_start, 10);
        assert_eq!(hunk.section_heading.as_deref(), Some("fn test() {"));
        assert_eq!(hunk.lines.len(), 4);

        // Line 0: removed
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Removed);
        assert_eq!(hunk.lines[0].old_lineno, Some(10));
        assert_eq!(hunk.lines[0].new_lineno, None);

        // Line 1: added
        assert_eq!(hunk.lines[1].kind, DiffLineKind::Added);
        assert_eq!(hunk.lines[1].old_lineno, None);
        assert_eq!(hunk.lines[1].new_lineno, Some(10));

        // Line 2: added
        assert_eq!(hunk.lines[2].kind, DiffLineKind::Added);
        assert_eq!(hunk.lines[2].old_lineno, None);
        assert_eq!(hunk.lines[2].new_lineno, Some(11));

        // Line 3: context
        assert_eq!(hunk.lines[3].kind, DiffLineKind::Context);
        assert_eq!(hunk.lines[3].old_lineno, Some(11));
        assert_eq!(hunk.lines[3].new_lineno, Some(12));
    }

    #[test]
    fn test_intra_line_word_emphasis() {
        let raw = r#"@@ -1,1 +1,1 @@
-let result = calculate_slow(x);
+let result = calculate_fast(x);
"#;

        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 1);
        let hunk = &files[0].hunks[0];

        let rem_line = &hunk.lines[0];
        let add_line = &hunk.lines[1];

        // "calculate_slow" should be emphasized in removed line
        let rem_emp: Vec<&str> = rem_line
            .words
            .iter()
            .filter(|w| w.emphasized)
            .map(|w| w.text.as_str())
            .collect();
        assert!(rem_emp.contains(&"calculate_slow"));

        // "calculate_fast" should be emphasized in added line
        let add_emp: Vec<&str> = add_line
            .words
            .iter()
            .filter(|w| w.emphasized)
            .map(|w| w.text.as_str())
            .collect();
        assert!(add_emp.contains(&"calculate_fast"));
    }

    #[test]
    fn test_split_rows_alignment() {
        let hunk = DiffHunk {
            header: "@@ -1,2 +1,2 @@".to_string(),
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 2,
            section_heading: None,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Removed,
                    old_lineno: Some(1),
                    new_lineno: None,
                    content: "old line".to_string(),
                    words: vec![],
                },
                DiffLine {
                    kind: DiffLineKind::Added,
                    old_lineno: None,
                    new_lineno: Some(1),
                    content: "new line".to_string(),
                    words: vec![],
                },
                DiffLine {
                    kind: DiffLineKind::Context,
                    old_lineno: Some(2),
                    new_lineno: Some(2),
                    content: "ctx".to_string(),
                    words: vec![],
                },
            ],
            additions: 1,
            deletions: 1,
        };

        let rows = generate_split_rows(&hunk);
        assert_eq!(rows.len(), 2);

        // Row 0: paired modification
        assert!(rows[0].left.is_some());
        assert!(rows[0].right.is_some());
        assert_eq!(rows[0].left.as_ref().unwrap().content, "old line");
        assert_eq!(rows[0].right.as_ref().unwrap().content, "new line");

        // Row 1: context line on both sides
        assert!(rows[1].left.is_some());
        assert!(rows[1].right.is_some());
        assert_eq!(rows[1].left.as_ref().unwrap().content, "ctx");
        assert_eq!(rows[1].right.as_ref().unwrap().content, "ctx");
    }

    #[test]
    fn test_diff_viewer_state_toggles() {
        let mut state = DiffViewerState::new();
        assert_eq!(state.mode, DiffViewMode::Unified);
        assert!(state.word_emphasis);

        state.toggle_mode();
        assert_eq!(state.mode, DiffViewMode::Split);

        state.toggle_hunk(3);
        assert!(state.is_hunk_collapsed(3));
        assert!(!state.is_hunk_collapsed(2));

        state.toggle_hunk(3);
        assert!(!state.is_hunk_collapsed(3));

        state.collapse_all(5);
        for i in 0..5 {
            assert!(state.is_hunk_collapsed(i));
        }

        state.expand_all();
        assert!(!state.is_hunk_collapsed(0));

        state.toggle_word_emphasis();
        assert!(!state.word_emphasis);
    }
}
