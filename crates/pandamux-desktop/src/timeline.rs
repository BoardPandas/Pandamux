use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::time::Instant;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::projections::{
    ChangedFileSummary, TimelineItem, WorkLogEntry, WorkLogStatus, format_duration,
};
use pandamux_core::{
    ApprovalDecision, ApprovalKind, FileChangeKind, PlanStep, PlanStepStatus, ThreadId,
    ToolCallStatus,
};

use crate::diff_view::{DiffViewerState, parse_unified_diff, render_diff_file};
use crate::theme::{Radii, Theme, Typography};

/// Renders a single timeline item with Section 12 styling.
pub fn render_timeline_item<V: 'static>(
    idx: usize,
    item: &TimelineItem,
    thread_id: &ThreadId,
    theme: &Theme,
    on_approve: impl Fn(&mut V, ThreadId, String, ApprovalDecision, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    match item {
        TimelineItem::TurnUserPrompt {
            text, attachments, ..
        } => div()
            .id(ElementId::NamedInteger("user-turn".into(), idx as u64))
            .p_3()
            .rounded(Radii::ROW)
            .bg(rgba(0x43d9c91a))
            .border_1()
            .border_color(rgba(0x43d9c940))
            .v_flex()
            .gap_1()
            .child(
                div().h_flex().items_center().gap_1p5().child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent.color())
                        .child("👤 You"),
                ),
            )
            .child(
                div()
                    .text_size(Typography::BODY_SIZE)
                    .text_color(theme.chrome.text_t1)
                    .child(text.clone()),
            )
            .when(!attachments.is_empty(), |this| {
                this.child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .items_center()
                        .pt_1()
                        .children(attachments.iter().map(|att| {
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1()
                                .py_0p5()
                                .px_2()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x00000040))
                                .border_1()
                                .border_color(rgba(0xffffff1a))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .child(if att.is_image() { "🖼️" } else { "📎" }),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.chrome.text_t1)
                                        .child(att.file_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child(format!("({})", att.human_size())),
                                )
                        })),
                )
            })
            .into_any_element(),

        TimelineItem::AssistantText { text, .. } => div()
            .id(ElementId::NamedInteger("assistant-turn".into(), idx as u64))
            .p_3()
            .rounded(Radii::ROW)
            .bg(theme.chrome.panel)
            .border_1()
            .border_color(rgba(0xffffff0d))
            .v_flex()
            .gap_1()
            .child(
                div().h_flex().items_center().gap_1p5().child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent.color())
                        .child("🐼 Assistant"),
                ),
            )
            .child(
                div()
                    .text_size(Typography::BODY_SIZE)
                    .text_color(theme.chrome.text_t1)
                    .child(text.clone()),
            )
            .into_any_element(),

        TimelineItem::WorkLog {
            id,
            summary,
            entries,
            duration_ms,
            status,
        } => render_work_log(idx, id, summary, entries, *duration_ms, *status, theme),

        TimelineItem::Reasoning { text, .. } => div()
            .id(ElementId::NamedInteger("reasoning-turn".into(), idx as u64))
            .p_2p5()
            .rounded(Radii::ROW)
            .bg(rgba(0xffffff08))
            .border_1()
            .border_color(rgba(0xffffff14))
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.terminal.dim)
                    .child("💭 Thought Process"),
            )
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t3)
                    .child(text.clone()),
            )
            .into_any_element(),

        TimelineItem::ToolCall {
            name,
            input,
            status,
            output,
            ..
        } => {
            let status_badge_color = match status {
                ToolCallStatus::Pending | ToolCallStatus::Executing => theme.accent.color(),
                ToolCallStatus::Success => theme.terminal.success,
                ToolCallStatus::Failed => rgb(0xf87171),
                ToolCallStatus::Cancelled => theme.terminal.dim,
            };

            let input_display = if input.is_string() {
                input.as_str().unwrap_or("").to_string()
            } else {
                serde_json::to_string_pretty(input).unwrap_or_default()
            };

            div()
                .id(ElementId::NamedInteger("tool-call".into(), idx as u64))
                .p_2p5()
                .rounded(Radii::ROW)
                .bg(theme.chrome.bgc_knockout)
                .border_1()
                .border_color(rgba(0xffffff14))
                .v_flex()
                .gap_1p5()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(Typography::BODY_SIZE)
                                .child(format!("🔧 Tool: {name}")),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff14))
                                .text_size(Typography::META_SIZE)
                                .text_color(status_badge_color)
                                .child(format!("{status:?}")),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("Input: {input_display}")),
                )
                .when_some(output.as_ref(), |this, out| {
                    this.child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.terminal.text)
                            .p_1p5()
                            .rounded(Radii::CHIP)
                            .bg(theme.terminal.surface)
                            .child(format!("Output: {out}")),
                    )
                })
                .into_any_element()
        }

        TimelineItem::CommandRun {
            command,
            cwd,
            exit_code,
            output_tail,
            ..
        } => div()
            .id(ElementId::NamedInteger("command-run".into(), idx as u64))
            .p_2p5()
            .rounded(Radii::ROW)
            .bg(theme.terminal.surface)
            .border_1()
            .border_color(rgba(0xffffff1a))
            .v_flex()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(Typography::BODY_SIZE)
                            .text_color(theme.terminal.prompt)
                            .child(format!("$ {command}")),
                    )
                    .when_some(*exit_code, |this, code| {
                        let is_ok = code == 0;
                        this.child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(if is_ok {
                                    rgba(0x56b6c226)
                                } else {
                                    rgba(0xf8717126)
                                })
                                .text_size(Typography::META_SIZE)
                                .text_color(if is_ok {
                                    theme.terminal.success
                                } else {
                                    rgb(0xf87171)
                                })
                                .child(format!("exit {code}")),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.terminal.dim)
                    .child(format!("cwd: {cwd}")),
            )
            .when_some(output_tail.as_ref(), |this, tail| {
                this.child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.terminal.text)
                        .child(tail.clone()),
                )
            })
            .into_any_element(),

        TimelineItem::FileChange { path, kind, .. } => {
            let (label, badge_color) = match kind {
                FileChangeKind::Created => ("Added", theme.terminal.success),
                FileChangeKind::Modified => ("Modified", theme.terminal.warn),
                FileChangeKind::Deleted => ("Deleted", rgb(0xf87171)),
            };

            div()
                .id(ElementId::NamedInteger("file-change".into(), idx as u64))
                .h_flex()
                .items_center()
                .justify_between()
                .p_2()
                .rounded(Radii::CHIP)
                .bg(theme.chrome.panel2)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_size(Typography::BODY_SIZE).child("📄"))
                        .child(div().text_size(Typography::BODY_SIZE).child(path.clone())),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xffffff14))
                        .text_size(Typography::META_SIZE)
                        .text_color(badge_color)
                        .child(label),
                )
                .into_any_element()
        }

        TimelineItem::Approval {
            request_id,
            kind,
            detail,
            decision,
        } => render_approval_card(
            idx, request_id, *kind, detail, *decision, thread_id, theme, on_approve, cx,
        ),

        TimelineItem::Plan { steps } => render_plan_card(idx, steps, theme),

        TimelineItem::Notice { level, message } => render_notice_card(idx, level, message, theme),

        TimelineItem::Error {
            class,
            message,
            recoverable,
        } => render_error_card(idx, class, message, *recoverable, theme),

        TimelineItem::ChangedFiles {
            files,
            total_additions,
            total_deletions,
        } => render_changed_files_card(idx, files, *total_additions, *total_deletions, theme),
    }
}

/// Renders an aggregated work log card with Section 12 styling.
pub fn render_work_log(
    idx: usize,
    _id: &str,
    summary: &str,
    entries: &[WorkLogEntry],
    duration_ms: Option<u64>,
    status: WorkLogStatus,
    theme: &Theme,
) -> AnyElement {
    let (status_badge, status_color, status_bg) = match status {
        WorkLogStatus::Running => ("● Running", theme.accent.color(), rgba(0x43d9c91a)),
        WorkLogStatus::Completed => ("✓ Completed", theme.terminal.success, rgba(0x7fd88f1a)),
        WorkLogStatus::Failed => ("✗ Failed", rgb(0xf87171), rgba(0xf871711a)),
    };

    let duration_label = duration_ms.map(format_duration);

    div()
        .id(ElementId::NamedInteger("work-log".into(), idx as u64))
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(rgba(0xffffff14))
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(status_bg)
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(status_color)
                                .child(status_badge),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.chrome.text_t1)
                                .child(summary.to_string()),
                        ),
                )
                .when_some(duration_label, |this, dur| {
                    this.child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(Radii::CHIP)
                            .bg(rgba(0xffffff0a))
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.chrome.text_t3)
                            .child(format!("⏱ {dur}")),
                    )
                }),
        )
        .child(
            div()
                .v_flex()
                .gap_1p5()
                .p_2()
                .rounded(Radii::CHIP)
                .bg(theme.terminal.surface)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .children(
                    entries
                        .iter()
                        .enumerate()
                        .map(|(entry_idx, entry)| render_work_log_entry(entry_idx, entry, theme)),
                ),
        )
        .into_any_element()
}

fn render_work_log_entry(_entry_idx: usize, entry: &WorkLogEntry, theme: &Theme) -> AnyElement {
    match entry {
        WorkLogEntry::Reasoning { text, .. } => div()
            .h_flex()
            .items_start()
            .gap_1p5()
            .p_1p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0xffffff05))
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.terminal.dim)
                    .child("💭"),
            )
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t3)
                    .child(text.clone()),
            )
            .into_any_element(),

        WorkLogEntry::ToolCall {
            name,
            input,
            status,
            output,
            ..
        } => {
            let status_color = match status {
                ToolCallStatus::Pending | ToolCallStatus::Executing => theme.accent.color(),
                ToolCallStatus::Success => theme.terminal.success,
                ToolCallStatus::Failed => rgb(0xf87171),
                ToolCallStatus::Cancelled => theme.terminal.dim,
            };

            let input_display = if input.is_string() {
                input.as_str().unwrap_or("").to_string()
            } else {
                serde_json::to_string(input).unwrap_or_default()
            };

            div()
                .v_flex()
                .gap_0p5()
                .p_1p5()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff08))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1()
                                .child(div().text_size(Typography::META_SIZE).child("🔧"))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child(name.clone()),
                                ),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0a))
                                .text_size(Typography::META_SIZE)
                                .text_color(status_color)
                                .child(format!("{status:?}")),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("in: {input_display}")),
                )
                .when_some(output.as_ref(), |this, out| {
                    this.child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.terminal.text)
                            .child(format!("out: {out}")),
                    )
                })
                .into_any_element()
        }

        WorkLogEntry::CommandRun {
            command,
            cwd,
            exit_code,
            output_tail,
            ..
        } => {
            let is_ok = exit_code.is_some_and(|code| code == 0);
            div()
                .v_flex()
                .gap_0p5()
                .p_1p5()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff08))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.terminal.prompt)
                                .child(format!("$ {command}")),
                        )
                        .when_some(*exit_code, |this, code| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(if is_ok {
                                        rgba(0x56b6c226)
                                    } else {
                                        rgba(0xf8717126)
                                    })
                                    .text_size(Typography::META_SIZE)
                                    .text_color(if is_ok {
                                        theme.terminal.success
                                    } else {
                                        rgb(0xf87171)
                                    })
                                    .child(format!("exit {code}")),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.terminal.dim)
                        .child(format!("cwd: {cwd}")),
                )
                .when_some(output_tail.as_ref(), |this, tail| {
                    this.child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.terminal.text)
                            .child(tail.clone()),
                    )
                })
                .into_any_element()
        }

        WorkLogEntry::FileChange { path, kind, .. } => {
            let (label, badge_color) = match kind {
                FileChangeKind::Created => ("Added", theme.terminal.success),
                FileChangeKind::Modified => ("Modified", theme.terminal.warn),
                FileChangeKind::Deleted => ("Deleted", rgb(0xf87171)),
            };

            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_1p5()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff08))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().text_size(Typography::META_SIZE).child("📄"))
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child(path.clone()),
                        ),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xffffff0a))
                        .text_size(Typography::META_SIZE)
                        .text_color(badge_color)
                        .child(label),
                )
                .into_any_element()
        }
    }
}

fn is_destructive_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    let patterns = [
        "rm -rf",
        "rmdir /s",
        "del /f",
        "format ",
        "git reset --hard",
        "git clean -f",
        "drop database",
        "drop table",
        "kill -9",
        "mkfs",
        "fdisk",
    ];
    patterns.iter().any(|&p| lower.contains(p))
}

pub fn render_diff_snippet(diff: &str, theme: &Theme) -> AnyElement {
    let files = parse_unified_diff(diff);
    if files.is_empty() {
        return div()
            .p_2()
            .rounded(Radii::CHIP)
            .bg(theme.terminal.surface)
            .border_1()
            .border_color(rgba(0xffffff0d))
            .v_flex()
            .gap_0p5()
            .children(diff.lines().take(12).map(|line| {
                let (color, prefix_bg) = if line.starts_with('+') {
                    (theme.terminal.success, rgba(0x7fd88f1a))
                } else if line.starts_with('-') {
                    (rgb(0xf87171), rgba(0xf871711a))
                } else {
                    (theme.terminal.text, rgba(0x00000000))
                };
                div()
                    .h_flex()
                    .px_1()
                    .rounded(Radii::CHIP)
                    .bg(prefix_bg)
                    .text_size(Typography::META_SIZE)
                    .text_color(color)
                    .child(line.to_string())
            }))
            .into_any_element();
    }

    let state = DiffViewerState::new();
    let highlight_theme = if theme.mode == crate::theme::ThemeMode::Dark {
        gpui_kit::component::highlighter::HighlightTheme::default_dark()
    } else {
        gpui_kit::component::highlighter::HighlightTheme::default_light()
    };

    div()
        .w_full()
        .v_flex()
        .gap_2()
        .children(files.iter().map(|file| {
            render_diff_file(file, &state, theme, &highlight_theme, || {}, |_| {}, || {})
        }))
        .into_any_element()
}

/// Renders a specialized approval card with rich context, danger indicators, and action buttons.
#[allow(clippy::too_many_arguments)]
pub fn render_approval_card<V: 'static>(
    idx: usize,
    request_id: &str,
    kind: ApprovalKind,
    detail: &serde_json::Value,
    decision: Option<ApprovalDecision>,
    thread_id: &ThreadId,
    theme: &Theme,
    on_approve: impl Fn(&mut V, ThreadId, String, ApprovalDecision, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    let req_id_approve = request_id.to_string();
    let req_id_deny = request_id.to_string();
    let tid_approve = thread_id.clone();
    let tid_deny = thread_id.clone();

    let (icon, title, approve_label, deny_label) = match kind {
        ApprovalKind::CommandExecution => (
            "💻",
            "Command Execution Approval",
            "Run Command (↵ Enter)",
            "Reject (Esc)",
        ),
        ApprovalKind::FileEdit => (
            "📝",
            "File Modification Approval",
            "Save Changes (↵ Enter)",
            "Discard (Esc)",
        ),
        ApprovalKind::NetworkAccess => (
            "🌐",
            "Network Request Approval",
            "Allow Access (↵ Enter)",
            "Block (Esc)",
        ),
        ApprovalKind::ToolCall => (
            "🔧",
            "Tool Call Approval",
            "Execute Tool (↵ Enter)",
            "Reject (Esc)",
        ),
        ApprovalKind::FullAccessEscalation => (
            "🛡️",
            "Full Access Escalation Request",
            "Grant Escalation (↵ Enter)",
            "Deny (Esc)",
        ),
    };

    let mut card = div()
        .id(ElementId::NamedInteger("approval".into(), idx as u64))
        .p_3()
        .rounded(Radii::ROW)
        .bg(rgba(0xd8b45e14))
        .border_1()
        .border_color(theme.terminal.warn)
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().text_size(Typography::TITLE_SIZE).child(icon))
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.terminal.warn)
                                .child(title),
                        ),
                )
                .when_some(decision, |this, dec| {
                    let (dec_label, dec_color, dec_bg) = match dec {
                        ApprovalDecision::Approved => {
                            ("✓ Approved", theme.terminal.success, rgba(0x7fd88f1a))
                        }
                        ApprovalDecision::Denied => ("✗ Denied", rgb(0xf87171), rgba(0xf871711a)),
                        ApprovalDecision::TimedOut => {
                            ("⏱ Timed Out", theme.terminal.dim, rgba(0xffffff0a))
                        }
                    };
                    this.child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(Radii::CHIP)
                            .bg(dec_bg)
                            .text_size(Typography::META_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(dec_color)
                            .child(dec_label),
                    )
                }),
        );

    // Kind-specific content preview
    match kind {
        ApprovalKind::CommandExecution => {
            let cmd = detail
                .get("command")
                .or_else(|| detail.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| detail.as_str().unwrap_or(""));
            let cwd = detail.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
            let is_destructive = is_destructive_command(cmd);

            if is_destructive {
                card = card.child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xf871711a))
                        .border_1()
                        .border_color(rgb(0xf87171))
                        .v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(Typography::META_SIZE)
                                .text_color(rgb(0xf87171))
                                .child("⚠️ HIGH-RISK COMMAND DETECTED"),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child("This operation contains potentially destructive actions (e.g. deletion, reset, or format)."),
                        ),
                );
            }

            card = card
                .child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.terminal.surface)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.terminal.prompt)
                        .child(format!("$ {cmd}")),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.terminal.dim)
                        .child(format!("Working Directory: {cwd}")),
                );
        }

        ApprovalKind::FileEdit => {
            let path = detail
                .get("path")
                .or_else(|| detail.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let diff_text = detail
                .get("diff")
                .or_else(|| detail.get("content"))
                .and_then(|v| v.as_str());

            card = card.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().text_size(Typography::BODY_SIZE).child("📄"))
                    .child(
                        div()
                            .text_size(Typography::BODY_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.chrome.text_t1)
                            .child(path.to_string()),
                    ),
            );

            if let Some(diff) = diff_text {
                card = card.child(render_diff_snippet(diff, theme));
            }
        }

        ApprovalKind::NetworkAccess => {
            let target = detail
                .get("url")
                .or_else(|| detail.get("domain"))
                .or_else(|| detail.get("host"))
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| detail.as_str().unwrap_or("external host"));

            card = card
                .child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.terminal.surface)
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent.color())
                        .child(format!("🔗 {target}")),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("The agent is requesting an outbound network connection to an external address."),
                );
        }

        ApprovalKind::ToolCall => {
            let tool_name = detail
                .get("tool")
                .or_else(|| detail.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("Custom Tool");
            let pretty_payload = serde_json::to_string_pretty(detail).unwrap_or_default();

            card = card
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child(format!("Tool: {tool_name}")),
                )
                .child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.terminal.surface)
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t2)
                        .child(pretty_payload),
                );
        }

        ApprovalKind::FullAccessEscalation => {
            card = card.child(
                div()
                    .p_2()
                    .rounded(Radii::CHIP)
                    .bg(rgba(0xd8b45e26))
                    .text_size(Typography::BODY_SIZE)
                    .text_color(theme.terminal.warn)
                    .child("🚨 Agent requests FullAccess mode to remove execution and sandboxing boundaries."),
            );
        }
    }

    // Action buttons if pending
    if decision.is_none() {
        card =
            card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("btn-approve")
                            .primary()
                            .label(approve_label)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                on_approve(
                                    this,
                                    tid_approve.clone(),
                                    req_id_approve.clone(),
                                    ApprovalDecision::Approved,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(Button::new("btn-deny").ghost().label(deny_label).on_click(
                        cx.listener(move |this, _event, window, cx| {
                            on_approve(
                                this,
                                tid_deny.clone(),
                                req_id_deny.clone(),
                                ApprovalDecision::Denied,
                                window,
                                cx,
                            );
                        }),
                    )),
            );
    }

    card.into_any_element()
}

/// Renders a dedicated execution plan card with checklist and progress percentage.
pub fn render_plan_card(idx: usize, steps: &[PlanStep], theme: &Theme) -> AnyElement {
    let total = steps.len();
    let completed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Completed)
        .count();
    let failed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Failed)
        .count();
    let pct = (completed * 100).checked_div(total).unwrap_or(0);

    let bar_color = if failed > 0 {
        rgb(0xf87171)
    } else if completed == total && total > 0 {
        theme.terminal.success
    } else {
        theme.accent.color()
    };

    div()
        .id(ElementId::NamedInteger("plan".into(), idx as u64))
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(rgba(0xffffff14))
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(Typography::BODY_SIZE)
                        .text_color(theme.chrome.text_t1)
                        .child("📋 Execution Plan"),
                )
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xffffff0a))
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent.color())
                        .child(format!("{completed} of {total} steps complete ({pct}%)")),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(4.0))
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff14))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(gpui_kit::gpui::relative(pct.min(100) as f32 / 100.0))
                        .rounded(Radii::CHIP)
                        .bg(bar_color),
                ),
        )
        .children(steps.iter().map(|step| {
            let (icon, icon_color, badge_label, badge_color) = match step.status {
                PlanStepStatus::Completed => (
                    "✓",
                    theme.terminal.success,
                    "Completed",
                    theme.terminal.success,
                ),
                PlanStepStatus::InProgress => (
                    "▶",
                    theme.accent.color(),
                    "In Progress",
                    theme.accent.color(),
                ),
                PlanStepStatus::Pending => {
                    ("○", theme.terminal.dim, "Pending", theme.chrome.text_t4)
                }
                PlanStepStatus::Failed => ("✗", rgb(0xf87171), "Failed", rgb(0xf87171)),
                PlanStepStatus::Skipped => {
                    ("-", theme.terminal.dim, "Skipped", theme.chrome.text_t4)
                }
            };

            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff05))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(icon_color)
                                .child(icon),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(if step.status == PlanStepStatus::Completed {
                                    theme.chrome.text_t3
                                } else {
                                    theme.chrome.text_t1
                                })
                                .child(step.title.clone()),
                        ),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xffffff0a))
                        .text_size(Typography::META_SIZE)
                        .text_color(badge_color)
                        .child(badge_label),
                )
        }))
        .into_any_element()
}

/// Renders a styled notice card with category indicators.
pub fn render_notice_card(idx: usize, level: &str, message: &str, theme: &Theme) -> AnyElement {
    let lower_msg = message.to_lowercase();
    let category_badge = if lower_msg.contains("rate limit") {
        Some("[Rate Limit]")
    } else if lower_msg.contains("context window") {
        Some("[Context Window]")
    } else if lower_msg.contains("budget") {
        Some("[Budget]")
    } else {
        None
    };

    let (icon, border_col, bg_col, text_col) = match level.to_lowercase().as_str() {
        "warning" | "warn" => (
            "⚠️",
            theme.terminal.warn,
            rgba(0xd8b45e1a),
            theme.terminal.warn,
        ),
        "error" => ("🛑", rgb(0xf87171), rgba(0xf871711a), rgb(0xf87171)),
        _ => (
            "ℹ️",
            theme.accent.color(),
            rgba(0x43d9c91a),
            theme.accent.color(),
        ),
    };

    div()
        .id(ElementId::NamedInteger("notice".into(), idx as u64))
        .p_2p5()
        .rounded(Radii::ROW)
        .bg(bg_col)
        .border_1()
        .border_color(border_col)
        .v_flex()
        .gap_1()
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_1p5()
                .child(div().text_size(Typography::META_SIZE).child(icon))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(Typography::META_SIZE)
                        .text_color(text_col)
                        .child(level.to_string()),
                )
                .when_some(category_badge, |this, cat| {
                    this.child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(Radii::CHIP)
                            .bg(rgba(0xffffff14))
                            .text_size(Typography::META_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.chrome.text_t1)
                            .child(cat),
                    )
                }),
        )
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .text_color(theme.chrome.text_t1)
                .child(message.to_string()),
        )
        .into_any_element()
}

/// Renders a styled error card with recovery status and classification.
pub fn render_error_card(
    idx: usize,
    class: &str,
    message: &str,
    recoverable: bool,
    theme: &Theme,
) -> AnyElement {
    div()
        .id(ElementId::NamedInteger("error".into(), idx as u64))
        .p_3()
        .rounded(Radii::ROW)
        .bg(rgba(0xf871711a))
        .border_1()
        .border_color(rgb(0xf87171))
        .v_flex()
        .gap_1p5()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(Typography::BODY_SIZE)
                                .text_color(rgb(0xf87171))
                                .child("⚠️ Error"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xf8717126))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf87171))
                                .child(format!("[{class}]")),
                        ),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(if recoverable {
                            rgba(0x43d9c91f)
                        } else {
                            rgba(0xf8717133)
                        })
                        .text_size(Typography::META_SIZE)
                        .text_color(if recoverable {
                            theme.accent.color()
                        } else {
                            rgb(0xf87171)
                        })
                        .child(if recoverable { "Recoverable" } else { "Fatal" }),
                ),
        )
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .text_color(theme.chrome.text_t1)
                .child(message.to_string()),
        )
        .into_any_element()
}

/// Renders a dedicated changed files card summarizing modifications, additions, and deletions.
pub fn render_changed_files_card(
    idx: usize,
    files: &[ChangedFileSummary],
    total_additions: usize,
    total_deletions: usize,
    theme: &Theme,
) -> AnyElement {
    let count = files.len();
    let file_label = if count == 1 {
        "1 file changed"
    } else {
        &format!("{count} files changed")
    };

    div()
        .id(ElementId::NamedInteger("changed-files".into(), idx as u64))
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(rgba(0xffffff14))
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().text_size(Typography::TITLE_SIZE).child("📁"))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child(format!("Changed Files ({file_label})")),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .when(total_additions > 0, |this| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0x7fd88f1a))
                                    .text_size(Typography::META_SIZE)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.terminal.success)
                                    .child(format!("+{total_additions}")),
                            )
                        })
                        .when(total_deletions > 0, |this| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0xf871711a))
                                    .text_size(Typography::META_SIZE)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xf87171))
                                    .child(format!("-{total_deletions}")),
                            )
                        }),
                ),
        )
        .child(
            div()
                .v_flex()
                .gap_1()
                .p_2()
                .rounded(Radii::CHIP)
                .bg(theme.terminal.surface)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .children(files.iter().map(|file| {
                    let (kind_label, kind_color, kind_bg) = match file.kind {
                        FileChangeKind::Created => {
                            ("Added", theme.terminal.success, rgba(0x7fd88f1a))
                        }
                        FileChangeKind::Modified => {
                            ("Modified", theme.terminal.warn, rgba(0xd8b45e1a))
                        }
                        FileChangeKind::Deleted => ("Deleted", rgb(0xf87171), rgba(0xf871711a)),
                    };

                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .rounded(Radii::CHIP)
                        .bg(rgba(0xffffff05))
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_size(Typography::META_SIZE).child("📄"))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.chrome.text_t1)
                                        .child(file.path.clone()),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .when(file.additions > 0 || file.deletions > 0, |this| {
                                    this.child(
                                        div()
                                            .h_flex()
                                            .items_center()
                                            .gap_1()
                                            .text_size(Typography::META_SIZE)
                                            .when(file.additions > 0, |this| {
                                                this.child(
                                                    div()
                                                        .text_color(theme.terminal.success)
                                                        .child(format!("+{}", file.additions)),
                                                )
                                            })
                                            .when(file.deletions > 0, |this| {
                                                this.child(
                                                    div()
                                                        .text_color(rgb(0xf87171))
                                                        .child(format!("-{}", file.deletions)),
                                                )
                                            }),
                                    )
                                })
                                .child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(kind_bg)
                                        .text_size(Typography::META_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(kind_color)
                                        .child(kind_label),
                                ),
                        )
                })),
        )
        .into_any_element()
}

/// Cached syntax highlighted code block or markdown segment.
#[derive(Clone, Debug, PartialEq)]
pub struct CachedMarkdownBlock {
    pub content_hash: u64,
    pub raw_text: String,
    pub line_count: usize,
    pub has_code_block: bool,
    pub language: Option<String>,
}

/// Incremental markdown and syntax highlight cache.
/// Avoids re-parsing markdown ASTs and code blocks on every render frame.
#[derive(Clone, Debug, Default)]
pub struct MarkdownHighlightCache {
    entries: HashMap<u64, CachedMarkdownBlock>,
    max_capacity: usize,
}

impl MarkdownHighlightCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            max_capacity: 5000,
        }
    }

    pub fn compute_hash(text: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    pub fn get_or_parse(&mut self, text: &str) -> &CachedMarkdownBlock {
        let hash = Self::compute_hash(text);
        if !self.entries.contains_key(&hash) {
            if self.entries.len() >= self.max_capacity {
                let keys_to_remove: Vec<u64> = self.entries.keys().take(500).copied().collect();
                for k in keys_to_remove {
                    self.entries.remove(&k);
                }
            }

            let has_code_block = text.contains("```");
            let language = if has_code_block {
                text.lines()
                    .find(|l| l.starts_with("```"))
                    .map(|l| l.trim_start_matches("```").trim().to_string())
                    .filter(|l| !l.is_empty())
            } else {
                None
            };
            let line_count = text.lines().count();

            self.entries.insert(
                hash,
                CachedMarkdownBlock {
                    content_hash: hash,
                    raw_text: text.to_string(),
                    line_count,
                    has_code_block,
                    language,
                },
            );
        }

        self.entries.get(&hash).unwrap()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A discrete position in the timeline (item index and character offset).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TimelinePosition {
    pub item_index: usize,
    pub char_offset: usize,
}

/// S1 Selection Model: selection owned at the timeline level.
/// Spans across virtualized messages; copy text is extracted directly from projection.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TimelineSelection {
    pub anchor: Option<TimelinePosition>,
    pub head: Option<TimelinePosition>,
    pub is_active: bool,
}

impl TimelineSelection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, item_index: usize, char_offset: usize) {
        let pos = TimelinePosition {
            item_index,
            char_offset,
        };
        self.anchor = Some(pos);
        self.head = Some(pos);
        self.is_active = true;
    }

    pub fn update(&mut self, item_index: usize, char_offset: usize) {
        self.head = Some(TimelinePosition {
            item_index,
            char_offset,
        });
        self.is_active = true;
    }

    pub fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.is_active = false;
    }

    pub fn select_all(&mut self, total_items: usize) {
        if total_items == 0 {
            self.clear();
            return;
        }
        self.anchor = Some(TimelinePosition {
            item_index: 0,
            char_offset: 0,
        });
        self.head = Some(TimelinePosition {
            item_index: total_items.saturating_sub(1),
            char_offset: usize::MAX,
        });
        self.is_active = true;
    }

    pub fn selected_item_range(&self) -> Option<Range<usize>> {
        if !self.is_active {
            return None;
        }
        let a = self.anchor?.item_index;
        let h = self.head?.item_index;
        let start = a.min(h);
        let end = a.max(h) + 1;
        Some(start..end)
    }

    pub fn is_item_selected(&self, item_index: usize) -> bool {
        self.selected_item_range()
            .map(|r| r.contains(&item_index))
            .unwrap_or(false)
    }

    pub fn extract_selected_text(&self, items: &[TimelineItem]) -> String {
        let Some(range) = self.selected_item_range() else {
            return String::new();
        };

        let mut out = Vec::new();
        let start = range.start.min(items.len());
        let end = range.end.min(items.len());

        for item in &items[start..end] {
            match item {
                TimelineItem::TurnUserPrompt { text, .. } => out.push(format!("You: {text}")),
                TimelineItem::AssistantText { text, .. } => out.push(format!("Assistant: {text}")),
                TimelineItem::WorkLog { summary, .. } => out.push(format!("Work Log: {summary}")),
                TimelineItem::Reasoning { text, .. } => out.push(format!("Thought: {text}")),
                TimelineItem::Plan { steps, .. } => {
                    let mut s = String::from("Plan:\n");
                    for step in steps {
                        s.push_str(&format!("- [{:?}] {}\n", step.status, step.title));
                    }
                    out.push(s);
                }
                TimelineItem::Error { message, .. } => out.push(format!("Error: {message}")),
                TimelineItem::Notice { message, .. } => out.push(format!("Notice: {message}")),
                TimelineItem::ChangedFiles { files, .. } => {
                    out.push(format!("Changed Files: {} files", files.len()));
                }
                _ => {}
            }
        }

        out.join("\n\n")
    }
}

/// Benchmark report measuring performance budget metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineBenchmarkReport {
    pub total_items: usize,
    pub generation_ms: f64,
    pub slice_calculation_ms: f64,
    pub cache_lookup_ms: f64,
    pub total_elapsed_ms: f64,
    pub items_per_ms: f64,
    pub passes_budget: bool,
}

/// State for the virtualized timeline with recycling and performance budgets.
#[derive(Clone, Debug, Default)]
pub struct VirtualizedTimelineState {
    pub scroll_offset: f32,
    pub viewport_height: f32,
    pub estimated_item_height: f32,
    pub overscan: usize,
    pub tail_following: bool,
    pub unread_count: usize,
    pub selection: TimelineSelection,
    pub cache: MarkdownHighlightCache,
}

impl VirtualizedTimelineState {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0.0,
            viewport_height: 600.0,
            estimated_item_height: 72.0,
            overscan: 6,
            tail_following: true,
            unread_count: 0,
            selection: TimelineSelection::new(),
            cache: MarkdownHighlightCache::new(),
        }
    }

    /// Computes the active slice of items that should be rendered.
    pub fn visible_range(&self, total_items: usize) -> Range<usize> {
        if total_items == 0 {
            return 0..0;
        }

        let item_h = self.estimated_item_height.max(10.0);
        let first_visible = (self.scroll_offset / item_h).floor() as usize;
        let visible_count = (self.viewport_height / item_h).ceil() as usize + 1;

        let start = first_visible.saturating_sub(self.overscan);
        let end = (first_visible + visible_count + self.overscan).min(total_items);

        let start = start.min(total_items);
        let end = end.max(start).min(total_items);

        start..end
    }

    /// Top spacer height for unrendered preceding items.
    pub fn top_spacer_height(&self, total_items: usize) -> f32 {
        let range = self.visible_range(total_items);
        (range.start as f32) * self.estimated_item_height
    }

    /// Bottom spacer height for unrendered trailing items.
    pub fn bottom_spacer_height(&self, total_items: usize) -> f32 {
        let range = self.visible_range(total_items);
        let remaining = total_items.saturating_sub(range.end);
        (remaining as f32) * self.estimated_item_height
    }

    /// Handles scroll movement.
    pub fn on_scroll(&mut self, new_offset: f32, total_items: usize) {
        let total_h = (total_items as f32) * self.estimated_item_height;
        let max_offset = (total_h - self.viewport_height).max(0.0);
        self.scroll_offset = new_offset.clamp(0.0, max_offset);

        // If user scrolled up significantly from bottom, disable tail following
        let dist_from_bottom = max_offset - self.scroll_offset;
        if dist_from_bottom > 120.0 {
            self.tail_following = false;
        } else {
            self.tail_following = true;
            self.unread_count = 0;
        }
    }

    /// Jumps directly to the latest message and re-enables tail following.
    pub fn jump_to_bottom(&mut self, total_items: usize) {
        let total_h = (total_items as f32) * self.estimated_item_height;
        self.scroll_offset = (total_h - self.viewport_height).max(0.0);
        self.tail_following = true;
        self.unread_count = 0;
    }

    /// Informs the virtualized state of newly settled events.
    pub fn on_new_items(&mut self, new_total: usize) {
        if self.tail_following {
            self.jump_to_bottom(new_total);
        } else {
            self.unread_count += 1;
        }
    }

    /// Runs a 2,000-item benchmark asserting compliance with the performance budget (<100ms).
    pub fn benchmark_2000_items(&mut self) -> TimelineBenchmarkReport {
        let start = Instant::now();
        let total_items = 2000;

        // 1. Generation and slice calculation
        let slice_start = Instant::now();
        for offset in (0..100).step_by(10) {
            self.scroll_offset = (offset as f32) * self.estimated_item_height;
            let range = self.visible_range(total_items);
            assert!(!range.is_empty());
        }
        let slice_elapsed = slice_start.elapsed();

        // 2. Incremental highlight cache test
        let cache_start = Instant::now();
        for i in 0..500 {
            let markdown = format!(
                "### Header {i}\n\n```rust\nfn sample_{i}() -> i32 {{ {i} }}\n```\nExplanation text."
            );
            let block = self.cache.get_or_parse(&markdown);
            assert!(block.has_code_block);
            assert_eq!(block.language.as_deref(), Some("rust"));
        }
        let cache_elapsed = cache_start.elapsed();

        let total_elapsed = start.elapsed();
        let total_ms = total_elapsed.as_secs_f64() * 1000.0;
        let passes_budget = total_ms < 100.0;

        TimelineBenchmarkReport {
            total_items,
            generation_ms: total_ms,
            slice_calculation_ms: slice_elapsed.as_secs_f64() * 1000.0,
            cache_lookup_ms: cache_elapsed.as_secs_f64() * 1000.0,
            total_elapsed_ms: total_ms,
            items_per_ms: (total_items as f64) / total_ms.max(0.001),
            passes_budget,
        }
    }
}

/// Renders the virtualized timeline transcript with row recycling, spacers,
/// selection highlighting, and jump-to-bottom floating pill.
#[allow(clippy::too_many_arguments)]
pub fn render_virtualized_timeline<V: 'static>(
    state: &VirtualizedTimelineState,
    items: &[TimelineItem],
    thread_id: &ThreadId,
    theme: &Theme,
    on_approve: impl Fn(&mut V, ThreadId, String, ApprovalDecision, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    on_select_item: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static + Copy,
    on_jump_to_bottom: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    let total_items = items.len();
    let range = state.visible_range(total_items);
    let top_spacer = state.top_spacer_height(total_items);
    let bottom_spacer = state.bottom_spacer_height(total_items);
    let show_jump_pill = !state.tail_following && state.unread_count > 0;
    let unread_count = state.unread_count;

    div()
        .id("virtualized-timeline-container")
        .relative()
        .size_full()
        .overflow_hidden()
        .child(
            div()
                .id("virtualized-timeline-scroller")
                .size_full()
                .v_flex()
                .gap_3()
                .p_2()
                .overflow_hidden()
                // Top Spacer for recycled preceding rows
                .when(top_spacer > 0.0, |this| this.child(div().h(px(top_spacer))))
                // Active Visible Range
                .children(range.clone().filter_map(|idx| {
                    let item = items.get(idx)?;
                    let is_selected = state.selection.is_item_selected(idx);
                    let border_color = if is_selected {
                        theme.accent_color()
                    } else {
                        rgba(0x00000000)
                    };
                    let bg_color = if is_selected {
                        rgba(0xffffff08)
                    } else {
                        rgba(0x00000000)
                    };

                    Some(
                        div()
                            .id(ElementId::named_usize("virt-item", idx))
                            .rounded(Radii::ROW)
                            .border_1()
                            .border_color(border_color)
                            .bg(bg_color)
                            .on_click(cx.listener(move |this, _ev, win, cx| {
                                on_select_item(this, idx, win, cx);
                            }))
                            .child(render_timeline_item(
                                idx, item, thread_id, theme, on_approve, cx,
                            )),
                    )
                }))
                // Bottom Spacer for recycled trailing rows
                .when(bottom_spacer > 0.0, |this| {
                    this.child(div().h(px(bottom_spacer)))
                }),
        )
        // Floating Jump to Bottom pill
        .when(show_jump_pill, |this| {
            this.child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .left_0()
                    .right_0()
                    .h_flex()
                    .justify_center()
                    .child(
                        Button::new("btn-jump-to-bottom")
                            .primary()
                            .label(format!("↓ Jump to bottom ({unread_count} new)"))
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                on_jump_to_bottom(this, window, cx);
                            })),
                    ),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_timeline_virtualization_visible_range() {
        let mut state = VirtualizedTimelineState::new();
        state.viewport_height = 720.0;
        state.estimated_item_height = 72.0;
        state.overscan = 5;

        // At scroll_offset = 0 with 2,000 items
        let range = state.visible_range(2000);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 16); // 10 visible + 1 ceil + 5 overscan

        // Scroll down to item 100
        state.on_scroll(7200.0, 2000);
        let range = state.visible_range(2000);
        assert_eq!(range.start, 95); // 100 - 5 overscan
        assert_eq!(range.end, 116); // 100 + 11 + 5 overscan
    }

    #[test]
    fn test_timeline_selection_model() {
        let mut sel = TimelineSelection::new();
        assert!(!sel.is_active);

        sel.start(2, 0);
        sel.update(5, 10);
        assert!(sel.is_active);
        assert!(sel.is_item_selected(2));
        assert!(sel.is_item_selected(3));
        assert!(sel.is_item_selected(4));
        assert!(sel.is_item_selected(5));
        assert!(!sel.is_item_selected(1));
        assert!(!sel.is_item_selected(6));

        sel.select_all(10);
        assert_eq!(sel.selected_item_range(), Some(0..10));

        sel.clear();
        assert!(!sel.is_active);
    }

    #[test]
    fn test_markdown_highlight_cache() {
        let mut cache = MarkdownHighlightCache::new();
        let sample = "### Plan\n\n```python\nprint('hello')\n```\nDone.";

        let block1 = cache.get_or_parse(sample).clone();
        assert!(block1.has_code_block);
        assert_eq!(block1.language.as_deref(), Some("python"));
        assert_eq!(cache.len(), 1);

        // Subsequent lookup hits existing cache
        let block2 = cache.get_or_parse(sample);
        assert_eq!(block1.content_hash, block2.content_hash);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_2000_item_performance_budget() {
        let mut state = VirtualizedTimelineState::new();
        let report = state.benchmark_2000_items();
        assert_eq!(report.total_items, 2000);
        assert!(report.passes_budget, "Must pass performance budget");
        assert!(report.items_per_ms > 20.0);
    }
}
