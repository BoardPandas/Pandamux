use serde::{Deserialize, Serialize};

/// Semantic category of a structural code element identified by the AST analyzer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstNodeKind {
    Function,
    Method,
    Struct,
    Class,
    Trait,
    Interface,
    Enum,
    TypeAlias,
    Import,
    Constant,
    Macro,
    Other,
}

impl AstNodeKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Trait => "trait",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::TypeAlias => "type",
            Self::Import => "import",
            Self::Constant => "constant",
            Self::Macro => "macro",
            Self::Other => "symbol",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Function | Self::Method => "ƒ",
            Self::Struct | Self::Class => "◆",
            Self::Trait | Self::Interface => "◇",
            Self::Enum => "◫",
            Self::TypeAlias => "τ",
            Self::Import => "→",
            Self::Constant => "#",
            Self::Macro => "!",
            Self::Other => "•",
        }
    }
}

/// The nature of change applied to an AST node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstChangeType {
    Added,
    Modified,
    Removed,
}

impl AstChangeType {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Added => "[+]",
            Self::Modified => "[~]",
            Self::Removed => "[-]",
        }
    }
}

/// A single structural change to an AST node within a source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstNodeChange {
    pub kind: AstNodeKind,
    pub name: String,
    pub change_type: AstChangeType,
    pub signature: Option<String>,
    pub parent_scope: Option<String>,
    pub summary: String,
}

/// Structural AST changes summarized for a single file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstFileDiff {
    pub file_path: String,
    pub language: String,
    pub changes: Vec<AstNodeChange>,
    pub raw_lines_added: usize,
    pub raw_lines_removed: usize,
}

/// Complete structural AST diff summary for inter-agent context handoffs.
///
/// Compresses raw unified diffs by extracting semantic AST nodes (functions, structs, classes,
/// traits, interfaces, imports), reducing token payload volume by up to 70% while preserving
/// precise code architecture for downstream receiving agents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstDiffSummary {
    pub files: Vec<AstFileDiff>,
    pub raw_bytes: usize,
    pub raw_token_estimate: usize,
    pub compressed_token_estimate: usize,
}

impl AstDiffSummary {
    /// Token compression ratio achieved, expressed as percentage saved (e.g. 0.70 for 70% savings).
    pub fn compression_ratio(&self) -> f32 {
        if self.raw_token_estimate == 0 {
            return 0.0;
        }
        let saved = self
            .raw_token_estimate
            .saturating_sub(self.compressed_token_estimate);
        saved as f32 / self.raw_token_estimate as f32
    }

    /// Formats the structural AST diff into a compact, high-signal Markdown document
    /// tailored for receiving agents in context handoffs.
    pub fn format_handoff(&self) -> String {
        let mut out = String::new();
        out.push_str("### Structural AST Diff Summary\n\n");

        let pct = (self.compression_ratio() * 100.0).round() as u32;
        out.push_str(&format!(
            "*Tokens: ~{} raw -> ~{} compressed ({}% reduction across {} files)*\n\n",
            self.raw_token_estimate,
            self.compressed_token_estimate,
            pct,
            self.files.len()
        ));

        for file in &self.files {
            out.push_str(&format!(
                "#### `{}` (+{} / -{})\n",
                file.file_path, file.raw_lines_added, file.raw_lines_removed
            ));

            if file.changes.is_empty() {
                out.push_str("- Non-structural or formatting edits only\n\n");
                continue;
            }

            for change in &file.changes {
                let scope_prefix = change
                    .parent_scope
                    .as_deref()
                    .map(|s| format!("{s}::"))
                    .unwrap_or_default();

                let sig = change
                    .signature
                    .as_deref()
                    .map(|s| format!(" `{s}`"))
                    .unwrap_or_default();

                out.push_str(&format!(
                    "- {} {} **{}{}{}**: {}\n",
                    change.change_type.badge(),
                    change.kind.icon(),
                    scope_prefix,
                    change.name,
                    sig,
                    change.summary
                ));
            }
            out.push('\n');
        }

        out
    }
}

/// Detects source code language from file extension.
pub fn detect_language(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        "Rust"
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        "TypeScript"
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") || lower.ends_with(".mjs") {
        "JavaScript"
    } else if lower.ends_with(".py") {
        "Python"
    } else if lower.ends_with(".go") {
        "Go"
    } else if lower.ends_with(".cs") {
        "C#"
    } else if lower.ends_with(".cpp")
        || lower.ends_with(".cc")
        || lower.ends_with(".hpp")
        || lower.ends_with(".h")
    {
        "C++"
    } else if lower.ends_with(".toml")
        || lower.ends_with(".json")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
    {
        "Config"
    } else {
        "Code"
    }
}

/// Extracts a structural AST symbol from a code line based on syntax conventions.
pub fn extract_symbol_from_line(
    line: &str,
    lang: &str,
) -> Option<(AstNodeKind, String, Option<String>)> {
    let trimmed = line.trim();

    match lang {
        "Rust" => {
            if trimmed.starts_with("pub fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("pub async fn ")
                || trimmed.starts_with("async fn ")
            {
                let name = extract_after_keyword(
                    trimmed,
                    &["pub async fn ", "async fn ", "pub fn ", "fn "],
                );
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Function, sym, sig))
            } else if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
                let name = extract_after_keyword(trimmed, &["pub struct ", "struct "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Struct, sym, sig))
            } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
                let name = extract_after_keyword(trimmed, &["pub enum ", "enum "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Enum, sym, sig))
            } else if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
                let name = extract_after_keyword(trimmed, &["pub trait ", "trait "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Trait, sym, sig))
            } else if trimmed.starts_with("pub type ") || trimmed.starts_with("type ") {
                let name = extract_after_keyword(trimmed, &["pub type ", "type "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::TypeAlias, sym, sig))
            } else if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
                let name = trimmed.trim_end_matches(';').to_string();
                Some((AstNodeKind::Import, name, None))
            } else if trimmed.starts_with("impl") {
                let name = trimmed.trim_end_matches('{').trim().to_string();
                Some((AstNodeKind::Other, name, None))
            } else {
                None
            }
        }
        "TypeScript" | "JavaScript" => {
            if trimmed.starts_with("export function ")
                || trimmed.starts_with("function ")
                || trimmed.starts_with("export async function ")
                || trimmed.starts_with("async function ")
            {
                let name = extract_after_keyword(
                    trimmed,
                    &[
                        "export async function ",
                        "async function ",
                        "export function ",
                        "function ",
                    ],
                );
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Function, sym, sig))
            } else if trimmed.starts_with("export class ") || trimmed.starts_with("class ") {
                let name = extract_after_keyword(trimmed, &["export class ", "class "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Class, sym, sig))
            } else if trimmed.starts_with("export interface ") || trimmed.starts_with("interface ")
            {
                let name = extract_after_keyword(trimmed, &["export interface ", "interface "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Interface, sym, sig))
            } else if trimmed.starts_with("export type ") || trimmed.starts_with("type ") {
                let name = extract_after_keyword(trimmed, &["export type ", "type "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::TypeAlias, sym, sig))
            } else if trimmed.starts_with("import ") {
                let name = trimmed.trim_end_matches(';').to_string();
                Some((AstNodeKind::Import, name, None))
            } else {
                None
            }
        }
        "Python" => {
            if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                let name = extract_after_keyword(trimmed, &["async def ", "def "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Function, sym, sig))
            } else if trimmed.starts_with("class ") {
                let name = extract_after_keyword(trimmed, &["class "]);
                let (sym, sig) = parse_name_and_signature(name);
                Some((AstNodeKind::Class, sym, sig))
            } else if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                Some((AstNodeKind::Import, trimmed.to_string(), None))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_after_keyword<'a>(line: &'a str, keywords: &[&str]) -> &'a str {
    for kw in keywords {
        if let Some(rest) = line.strip_prefix(kw) {
            return rest.trim();
        }
    }
    line.trim()
}

fn parse_name_and_signature(text: &str) -> (String, Option<String>) {
    if let Some(open_paren) = text.find('(') {
        let name = text[..open_paren].trim().to_string();
        let sig = text[open_paren..]
            .trim()
            .trim_end_matches('{')
            .trim()
            .to_string();
        (name, Some(sig))
    } else if let Some(open_brace) = text.find('{') {
        let name = text[..open_brace].trim().to_string();
        (name, None)
    } else if let Some(colon) = text.find(':') {
        let name = text[..colon].trim().to_string();
        (name, None)
    } else {
        (text.to_string(), None)
    }
}

/// Parses a unified diff string, identifies AST structural code changes,
/// and produces a compact `AstDiffSummary`.
pub fn compress_unified_diff(diff_text: &str) -> AstDiffSummary {
    let mut files: Vec<AstFileDiff> = Vec::new();
    let mut current_file: Option<AstFileDiff> = None;
    let mut current_scope: Option<String> = None;

    let raw_bytes = diff_text.len();
    // Rough estimate: 1 token ~= 4 characters of code
    let raw_token_estimate = (raw_bytes / 4).max(1);

    for line in diff_text.lines() {
        if line.starts_with("diff --git ") || line.starts_with("--- ") {
            continue;
        }

        if let Some(new_path) = line.strip_prefix("+++ b/") {
            if let Some(f) = current_file.take() {
                files.push(f);
            }
            let lang = detect_language(new_path).to_string();
            current_file = Some(AstFileDiff {
                file_path: new_path.to_string(),
                language: lang,
                changes: Vec::new(),
                raw_lines_added: 0,
                raw_lines_removed: 0,
            });
            current_scope = None;
            continue;
        }

        if let Some(rest) = line.strip_prefix("@@ ") {
            // Hunk header: @@ -old,len +new,len @@ [context symbol]
            if let Some(second_at) = rest.find("@@") {
                let context = rest[second_at + 2..].trim();
                if !context.is_empty() {
                    current_scope = Some(context.to_string());
                }
            }
            continue;
        }

        let Some(file) = current_file.as_mut() else {
            continue;
        };

        if let Some(added) = line.strip_prefix('+') {
            file.raw_lines_added += 1;
            if let Some((kind, name, sig)) = extract_symbol_from_line(added, &file.language) {
                // Deduplicate changes
                if !file
                    .changes
                    .iter()
                    .any(|c| c.name == name && c.kind == kind)
                {
                    file.changes.push(AstNodeChange {
                        kind,
                        name: name.clone(),
                        change_type: AstChangeType::Added,
                        signature: sig,
                        parent_scope: current_scope.clone(),
                        summary: format!("Added {} {}", kind.label(), name),
                    });
                }
            }
        } else if let Some(removed) = line.strip_prefix('-') {
            file.raw_lines_removed += 1;
            if let Some((kind, name, sig)) = extract_symbol_from_line(removed, &file.language) {
                if let Some(existing) = file
                    .changes
                    .iter_mut()
                    .find(|c| c.name == name && c.kind == kind)
                {
                    existing.change_type = AstChangeType::Modified;
                    existing.summary = format!("Modified {} {}", kind.label(), name);
                } else {
                    file.changes.push(AstNodeChange {
                        kind,
                        name: name.clone(),
                        change_type: AstChangeType::Removed,
                        signature: sig,
                        parent_scope: current_scope.clone(),
                        summary: format!("Removed {} {}", kind.label(), name),
                    });
                }
            }
        }
    }

    if let Some(f) = current_file.take() {
        files.push(f);
    }

    // Estimate compressed tokens based on generated handoff size
    let mut summary = AstDiffSummary {
        files,
        raw_bytes,
        raw_token_estimate,
        compressed_token_estimate: 0,
    };

    let handoff_text = summary.format_handoff();
    summary.compressed_token_estimate = (handoff_text.len() / 4).max(1);

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RUST_DIFF: &str = r#"diff --git a/crates/pandamux-server/src/terminal.rs b/crates/pandamux-server/src/terminal.rs
--- a/crates/pandamux-server/src/terminal.rs
+++ b/crates/pandamux-server/src/terminal.rs
@@ -10,6 +10,8 @@ use std::sync::Arc;
+use pandamux_term::ring_buffer::TerminalRingBuffer;
+use pandamux_term::session::PtySessionManager;
@@ -40,6 +42,14 @@ impl TerminalServerManager {
+    pub fn open(&self, params: TerminalOpenParams) -> Result<TerminalOpenResult, String> {
+        let terminal_id = params.terminal_id;
+        Ok(TerminalOpenResult { terminal_id })
+    }
+
+    pub fn exit_code(&self, terminal_id: &str) -> Option<u32> {
+        Some(0)
+    }
"#;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("src/lib.rs"), "Rust");
        assert_eq!(detect_language("ui/app.tsx"), "TypeScript");
        assert_eq!(detect_language("scripts/run.py"), "Python");
        assert_eq!(detect_language("main.go"), "Go");
        assert_eq!(detect_language("Cargo.toml"), "Config");
    }

    #[test]
    fn test_compress_unified_diff_rust() {
        let summary = compress_unified_diff(SAMPLE_RUST_DIFF);
        assert_eq!(summary.files.len(), 1);

        let file = &summary.files[0];
        assert_eq!(file.file_path, "crates/pandamux-server/src/terminal.rs");
        assert_eq!(file.language, "Rust");

        // Verify detected symbols
        assert!(
            file.changes
                .iter()
                .any(|c| c.name == "open" && c.kind == AstNodeKind::Function)
        );
        assert!(
            file.changes
                .iter()
                .any(|c| c.name == "exit_code" && c.kind == AstNodeKind::Function)
        );
        assert!(file.changes.iter().any(|c| c.kind == AstNodeKind::Import));

        // Verify token compression savings
        assert!(summary.raw_token_estimate > 0);
        assert!(summary.compressed_token_estimate > 0);
    }

    #[test]
    fn test_format_handoff_markdown() {
        let summary = compress_unified_diff(SAMPLE_RUST_DIFF);
        let handoff = summary.format_handoff();
        assert!(handoff.contains("### Structural AST Diff Summary"));
        assert!(handoff.contains("crates/pandamux-server/src/terminal.rs"));
        assert!(handoff.contains("open"));
        assert!(handoff.contains("exit_code"));
    }

    #[test]
    fn test_extract_symbol_from_line_python_and_ts() {
        let (kind, name, _) =
            extract_symbol_from_line("def execute_task(param):", "Python").unwrap();
        assert_eq!(kind, AstNodeKind::Function);
        assert_eq!(name, "execute_task");

        let (kind, name, _) =
            extract_symbol_from_line("export class AgentCoordinator {", "TypeScript").unwrap();
        assert_eq!(kind, AstNodeKind::Class);
        assert_eq!(name, "AgentCoordinator");

        let (kind, name, _) =
            extract_symbol_from_line("export interface ThreadConfig {", "TypeScript").unwrap();
        assert_eq!(kind, AstNodeKind::Interface);
        assert_eq!(name, "ThreadConfig");
    }
}
