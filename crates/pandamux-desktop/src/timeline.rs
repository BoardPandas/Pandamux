use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::projections::TimelineItem;
use pandamux_core::{ApprovalDecision, ApprovalKind, FileChangeKind, ThreadId, ToolCallStatus};

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
        TimelineItem::TurnUserPrompt { text, .. } => div()
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
        } => {
            let req_id_approve = request_id.clone();
            let req_id_deny = request_id.clone();
            let tid_approve = thread_id.clone();
            let tid_deny = thread_id.clone();

            let kind_title = match kind {
                ApprovalKind::CommandExecution => "Command Execution",
                ApprovalKind::FileEdit => "File Edit",
                ApprovalKind::NetworkAccess => "Network Access",
                ApprovalKind::ToolCall => "Tool Call",
                ApprovalKind::FullAccessEscalation => "Full Access Escalation",
            };

            let detail_text = if detail.is_string() {
                detail.as_str().unwrap_or("").to_string()
            } else {
                serde_json::to_string_pretty(detail).unwrap_or_default()
            };

            div()
                .id(ElementId::NamedInteger("approval".into(), idx as u64))
                .p_3()
                .rounded(Radii::ROW)
                .bg(rgba(0xd8b45e1a))
                .border_1()
                .border_color(theme.terminal.warn)
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.terminal.warn)
                        .child(format!("⚠️ Approval Requested: {kind_title}")),
                )
                .child(div().text_size(Typography::BODY_SIZE).child(detail_text))
                .child(if let Some(dec) = decision {
                    let dec_color = match dec {
                        ApprovalDecision::Approved => theme.terminal.success,
                        ApprovalDecision::Denied => rgb(0xf87171),
                        ApprovalDecision::TimedOut => theme.terminal.dim,
                    };
                    div()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(dec_color)
                        .child(format!("Decision: {dec:?}"))
                } else {
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("btn-approve")
                                .primary()
                                .label("Approve")
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
                        .child(
                            Button::new("btn-deny")
                                .ghost()
                                .label("Deny")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_approve(
                                        this,
                                        tid_deny.clone(),
                                        req_id_deny.clone(),
                                        ApprovalDecision::Denied,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                })
                .into_any_element()
        }

        TimelineItem::Plan { steps } => div()
            .id(ElementId::NamedInteger("plan".into(), idx as u64))
            .p_2p5()
            .rounded(Radii::ROW)
            .bg(theme.chrome.panel2)
            .border_1()
            .border_color(rgba(0xffffff14))
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(Typography::BODY_SIZE)
                    .child(format!("📋 Execution Plan ({} steps)", steps.len())),
            )
            .children(steps.iter().map(|step| {
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .rounded(Radii::CHIP)
                    .bg(rgba(0xffffff08))
                    .child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .child(step.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.accent.color())
                            .child(format!("{:?}", step.status)),
                    )
            }))
            .into_any_element(),

        TimelineItem::Notice { message, .. } => div()
            .id(ElementId::NamedInteger("notice".into(), idx as u64))
            .p_2()
            .rounded(Radii::CHIP)
            .bg(rgba(0xffffff0a))
            .text_size(Typography::META_SIZE)
            .text_color(theme.chrome.text_t2)
            .child(format!("ℹ️ {message}"))
            .into_any_element(),

        TimelineItem::Error { message, .. } => div()
            .id(ElementId::NamedInteger("error".into(), idx as u64))
            .p_2()
            .rounded(Radii::CHIP)
            .bg(rgba(0xf871711a))
            .border_1()
            .border_color(rgb(0xf87171))
            .text_size(Typography::META_SIZE)
            .text_color(rgb(0xf87171))
            .child(format!("⚠️ Error: {message}"))
            .into_any_element(),
    }
}
