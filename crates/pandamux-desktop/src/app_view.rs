use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::projections::{ThreadProjection, TimelineItem};
use pandamux_core::{
    ApprovalDecision, EnvironmentId, ProviderInstanceId, Thread, ThreadEvent, ThreadId,
    ThreadStatus,
};
use pandamux_protocol::{
    EventEnvelope, ThreadCancelTurnParams, ThreadCreateParams, ThreadRespondApprovalParams,
    ThreadSendTurnParams,
};

use crate::server_bridge::{spawn_server_bridge, ServerBridgeHandle, ServerStatus};
use crate::theme::{AccentColor, Radii, Spacing, Theme, Typography};
use crate::titlebar::CustomTitlebar;

/// The primary application view for PandaMUX Desktop.
pub struct AppView {
    theme: Theme,
    server_status: ServerStatus,
    bridge: Option<ServerBridgeHandle>,
    thread_projections: HashMap<ThreadId, ThreadProjection>,
    thread_order: Vec<ThreadId>,
    active_thread_id: Option<ThreadId>,
    composer_text: String,
    active_rail_tab: RailTab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RailTab {
    #[default]
    Threads,
    Agents,
    Terminal,
    Settings,
}

impl AppView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = smol::channel::unbounded::<EventEnvelope>();
        let (status_tx, status_rx) = smol::channel::unbounded::<ServerStatus>();

        let bridge = spawn_server_bridge(event_tx, status_tx);

        // Spawn async listener for server status updates
        cx.spawn(async move |this, cx| {
            while let Ok(status) = status_rx.recv().await {
                let _ = cx.update(|cx| {
                    let _ = this.update(cx, |this: &mut AppView, cx| {
                        this.server_status = status;
                        cx.notify();
                    });
                });
            }
        })
        .detach();

        // Spawn async listener for live thread events from server
        cx.spawn(async move |this, cx| {
            while let Ok(envelope) = event_rx.recv().await {
                let _ = cx.update(|cx| {
                    let _ = this.update(cx, |this: &mut AppView, cx| {
                        this.handle_event_envelope(envelope);
                        cx.notify();
                    });
                });
            }
        })
        .detach();

        Self {
            theme: Theme::dark(AccentColor::Teal),
            server_status: ServerStatus::Connecting,
            bridge: Some(bridge),
            thread_projections: HashMap::new(),
            thread_order: Vec::new(),
            active_thread_id: None,
            composer_text: String::new(),
            active_rail_tab: RailTab::Threads,
        }
    }

    /// Ingests incoming EventEnvelope into the corresponding client-side projection.
    pub fn handle_event_envelope(&mut self, envelope: EventEnvelope) {
        if let Some(thread_id) = &envelope.thread_id {
            if let Some(proj) = self.thread_projections.get_mut(thread_id) {
                let event = ThreadEvent {
                    thread_id: thread_id.clone(),
                    seq: envelope.seq,
                    at_ms: envelope.at_ms,
                    kind: envelope.kind,
                };
                proj.apply_event(&event);
            }
        }
    }

    /// Registers a new thread projection locally and tracks it.
    pub fn add_thread(&mut self, thread: Thread) {
        let id = thread.id.clone();
        if !self.thread_order.contains(&id) {
            self.thread_order.push(id.clone());
        }
        self.thread_projections
            .insert(id.clone(), ThreadProjection::new(thread));
        if self.active_thread_id.is_none() {
            self.active_thread_id = Some(id);
        }
    }

    /// Dispatches create_thread intent to backend server.
    pub fn create_default_thread(&mut self, cx: &mut Context<Self>) {
        let title = format!("Thread #{}", self.thread_order.len() + 1);
        let params = ThreadCreateParams {
            project_id: None,
            environment_id: Some(EnvironmentId::from("env-local")),
            title: Some(title.clone()),
            provider_instance_id: Some(ProviderInstanceId::from("claude")),
            model: Some("claude-3-7-sonnet".to_string()),
            effort: None,
            access_mode: None,
            agent_id: None,
            cwd: None,
            worktree: None,
        };

        if let Some(bridge) = &self.bridge {
            let rx = bridge.send_request("thread.create", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await {
                    if let Some(val) = resp.result {
                        if let Ok(thread) = serde_json::from_value::<Thread>(val) {
                            let _ = cx.update(|cx| {
                                let _ = this.update(cx, |this: &mut AppView, cx| {
                                    this.add_thread(thread);
                                    cx.notify();
                                });
                            });
                        }
                    }
                }
            })
            .detach();
        }
    }

    /// Dispatches send_turn intent to backend server.
    pub fn send_composer_turn(&mut self, cx: &mut Context<Self>) {
        let prompt = self.composer_text.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        let thread_id = match &self.active_thread_id {
            Some(id) => id.clone(),
            None => return,
        };

        let params = ThreadSendTurnParams {
            thread_id: thread_id.clone(),
            text: prompt,
            attachment_ids: vec![],
            model: None,
            effort: None,
        };

        self.composer_text.clear();

        if let Some(bridge) = &self.bridge {
            let rx = bridge.send_request("thread.send_turn", serde_json::to_value(&params).ok());
            cx.spawn(async move |_this, _cx| {
                let _ = rx.await;
            })
            .detach();
        }
    }

    /// Responds to an approval request.
    pub fn respond_approval(
        &mut self,
        thread_id: ThreadId,
        request_id: String,
        decision: ApprovalDecision,
        _cx: &mut Context<Self>,
    ) {
        let params = ThreadRespondApprovalParams {
            thread_id,
            request_id,
            decision,
        };

        if let Some(bridge) = &self.bridge {
            let rx =
                bridge.send_request("thread.respond_approval", serde_json::to_value(&params).ok());
            drop(rx);
        }
    }

    /// Cancels or interrupts the active turn.
    pub fn cancel_turn(&mut self, thread_id: ThreadId, _cx: &mut Context<Self>) {
        let params = ThreadCancelTurnParams {
            thread_id,
            turn_id: None,
        };

        if let Some(bridge) = &self.bridge {
            let rx = bridge.send_request("thread.cancel_turn", serde_json::to_value(&params).ok());
            drop(rx);
        }
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_title = self.active_thread_id.as_ref().and_then(|id| {
            self.thread_projections
                .get(id)
                .map(|p| p.thread.title.as_str())
        });

        let theme = self.theme.clone();
        let server_status = self.server_status.clone();

        div()
            .size_full()
            .v_flex()
            .bg(theme.chrome.bg_base_start)
            .text_color(theme.chrome.text_t1)
            // 1. Custom 40px Titlebar
            .child(CustomTitlebar::render(
                &theme,
                &server_status,
                active_title,
                window,
                cx,
            ))
            // 2. Central Layout (Rail + Sidebar + Main Surface)
            .child(
                div()
                    .flex_1()
                    .h_flex()
                    .overflow_hidden()
                    // 2a. 52px Navigation Icon Rail
                    .child(self.render_rail(&theme, cx))
                    // 2b. 264px Session Sidebar
                    .child(self.render_sidebar(&theme, cx))
                    // 2c. Main Chat & Workspace Surface
                    .child(self.render_main_surface(&theme, cx)),
            )
            // 3. 26px Status Bar
            .child(self.render_statusbar(&theme))
    }
}

impl AppView {
    /// Renders the 52px icon rail.
    fn render_rail(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(Spacing::RAIL_WIDTH)
            .h_full()
            .v_flex()
            .items_center()
            .py_2()
            .gap_3()
            .bg(theme.chrome.panel2)
            .border_r_1()
            .border_color(rgba(0xffffff0d))
            // Threads tab
            .child(
                Button::new("rail-threads")
                    .ghost()
                    .label("💬")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.active_rail_tab = RailTab::Threads;
                        cx.notify();
                    })),
            )
            // Agents tab
            .child(
                Button::new("rail-agents")
                    .ghost()
                    .label("🤖")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.active_rail_tab = RailTab::Agents;
                        cx.notify();
                    })),
            )
            // Terminal tab
            .child(
                Button::new("rail-terminal")
                    .ghost()
                    .label("📟")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.active_rail_tab = RailTab::Terminal;
                        cx.notify();
                    })),
            )
            // Spacer
            .child(div().flex_1())
            // Settings tab
            .child(
                Button::new("rail-settings")
                    .ghost()
                    .label("⚙️")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.active_rail_tab = RailTab::Settings;
                        cx.notify();
                    })),
            )
    }

    /// Renders the 264px session sidebar listing all threads.
    fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active_id = self.active_thread_id.clone();
        let thread_count = self.thread_order.len();

        div()
            .w(Spacing::SIDEBAR_WIDTH)
            .h_full()
            .v_flex()
            .bg(theme.chrome.panel)
            .border_r_1()
            .border_color(rgba(0xffffff0d))
            // Sidebar Header
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .border_b_1()
                    .border_color(rgba(0xffffff0a))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(Typography::TITLE_SIZE)
                                    .font_weight(FontWeight::BOLD)
                                    .child("Threads"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0xffffff14))
                                    .text_size(Typography::META_SIZE)
                                    .text_color(theme.chrome.text_t3)
                                    .child(format!("{thread_count}")),
                            ),
                    )
                    .child(
                        Button::new("btn-new-thread")
                            .primary()
                            .label("+ New")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.create_default_thread(cx);
                            })),
                    ),
            )
            // Thread rows list
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .p_2()
                    .gap_1()
                    .overflow_hidden()
                    .children(self.thread_order.iter().filter_map(|id| {
                        let proj = self.thread_projections.get(id)?;
                        let is_active = Some(id) == active_id.as_ref();
                        let thread_id_clone = id.clone();

                        let status_dot_color = match proj.thread.status {
                            ThreadStatus::Working => theme.accent.color(),
                            ThreadStatus::AwaitingApproval => theme.terminal.warn,
                            ThreadStatus::Idle => theme.terminal.success,
                            ThreadStatus::Errored => rgb(0xf87171),
                            ThreadStatus::Paused => theme.terminal.dim,
                            ThreadStatus::Archived => rgba(0xffffff20),
                        };

                        Some(
                            div()
                                .id(ElementId::NamedInteger(
                                    "thread-row".into(),
                                    proj.thread.id.as_str().len() as u64,
                                ))
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .px_2p5()
                                .py_2()
                                .rounded(Radii::ROW)
                                .bg(if is_active {
                                    rgba(0xffffff14)
                                } else {
                                    rgba(0x00000000)
                                })
                                .border_1()
                                .border_color(if is_active {
                                    theme.accent.color()
                                } else {
                                    rgba(0x00000000)
                                })
                                .cursor_pointer()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _event, _window, cx| {
                                        this.active_thread_id = Some(thread_id_clone.clone());
                                        cx.notify();
                                    }),
                                )
                                .child(
                                    div()
                                        .v_flex()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_size(Typography::BODY_SIZE)
                                                .font_weight(if is_active {
                                                    FontWeight::BOLD
                                                } else {
                                                    FontWeight::NORMAL
                                                })
                                                .child(proj.thread.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .h_flex()
                                                .gap_1p5()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .w_1p5()
                                                        .h_1p5()
                                                        .rounded_full()
                                                        .bg(status_dot_color),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(Typography::META_SIZE)
                                                        .text_color(theme.chrome.text_t3)
                                                        .child(format!(
                                                            "{} · {} turns",
                                                            proj.thread.provider_instance_id.as_str(),
                                                            proj.turns.len()
                                                        )),
                                                ),
                                        ),
                                ),
                        )
                    })),
            )
    }

    /// Renders the central workspace and timeline.
    fn render_main_surface(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active_proj = self
            .active_thread_id
            .as_ref()
            .and_then(|id| self.thread_projections.get(id));

        if let Some(proj) = active_proj {
            let thread_id = proj.thread.id.clone();
            let is_busy = matches!(
                proj.thread.status,
                ThreadStatus::Working | ThreadStatus::AwaitingApproval
            );

            div()
                .flex_1()
                .h_full()
                .v_flex()
                .p_3()
                .gap_3()
                // Header bar
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .child(
                            div()
                                .v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(Typography::TITLE_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .child(proj.thread.title.clone()),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(format!("Model: {}", proj.thread.model)),
                                        )
                                        .child(
                                            div()
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(format!(
                                                    "CWD: {}",
                                                    proj.thread.workspace.effective_path()
                                                )),
                                        ),
                                ),
                        )
                        .child(if is_busy {
                            let tid = thread_id.clone();
                            Button::new("btn-cancel-turn")
                                .ghost()
                                .label("⏹ Interrupt")
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.cancel_turn(tid.clone(), cx);
                                }))
                        } else {
                            Button::new("btn-ready")
                                .ghost()
                                .label("Idle")
                        }),
                )
                // Timeline messages area
                .child(
                    div()
                        .flex_1()
                        .v_flex()
                        .p_2()
                        .gap_3()
                        .overflow_hidden()
                        .children(proj.items.iter().enumerate().map(|(idx, item)| {
                            self.render_timeline_item(idx, item, &thread_id, theme, cx)
                        })),
                )
                // Composer at bottom
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .p_2()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(rgba(0xffffff14))
                        .child(
                            div()
                                .flex_1()
                                .child("Type a prompt to send to the AI agent..."),
                        )
                        .child(
                            Button::new("btn-send-turn")
                                .primary()
                                .label("Send")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.send_composer_turn(cx);
                                })),
                        ),
                )
        } else {
            // Welcome empty state
            div()
                .flex_1()
                .h_full()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_4()
                .child(
                    div()
                        .text_size(px(24.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent.color())
                        .child("🐼 PandaMUX 1.0"),
                )
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .text_color(theme.chrome.text_t2)
                        .child("Native Terminal Multiplexer and AI Agent Orchestrator"),
                )
                .child(
                    Button::new("btn-create-first-thread")
                        .primary()
                        .label("+ Create Thread")
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.create_default_thread(cx);
                        })),
                )
        }
    }

    /// Renders an individual timeline item (user turn, assistant text, tool call, approval).
    fn render_timeline_item(
        &self,
        idx: usize,
        item: &TimelineItem,
        thread_id: &ThreadId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match item {
            TimelineItem::TurnUserPrompt { text, .. } => div()
                .id(ElementId::NamedInteger("user-turn".into(), idx as u64))
                .p_3()
                .rounded(Radii::ROW)
                .bg(rgba(0x43d9c91f))
                .border_1()
                .border_color(rgba(0x43d9c940))
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
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
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .child(text.clone()),
                )
                .into_any_element(),

            TimelineItem::ToolCall {
                name,
                input,
                status,
                output,
                ..
            } => div()
                .id(ElementId::NamedInteger("tool-call".into(), idx as u64))
                .p_2p5()
                .rounded(Radii::ROW)
                .bg(theme.chrome.bgc_knockout)
                .border_1()
                .border_color(rgba(0xffffff14))
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
                                .child(format!("🔧 Tool: {name}")),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.accent.color())
                                .child(format!("{status:?}")),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("Input: {input}")),
                )
                .when_some(output.as_ref(), |this, out| {
                    this.child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.terminal.text)
                            .child(format!("Output: {out}")),
                    )
                })
                .into_any_element(),

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
                            .child(format!("⚠️ Approval Requested: {kind:?}")),
                    )
                    .child(
                        div()
                            .text_size(Typography::BODY_SIZE)
                            .child(format!("{detail}")),
                    )
                    .child(if let Some(dec) = decision {
                        div()
                            .text_size(Typography::META_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .child(format!("Decision: {dec:?}"))
                    } else {
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("btn-approve")
                                    .primary()
                                    .label("Approve")
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.respond_approval(
                                            tid_approve.clone(),
                                            req_id_approve.clone(),
                                            ApprovalDecision::Approved,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("btn-deny")
                                    .ghost()
                                    .label("Deny")
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.respond_approval(
                                            tid_deny.clone(),
                                            req_id_deny.clone(),
                                            ApprovalDecision::Denied,
                                            cx,
                                        );
                                    })),
                            )
                    })
                    .into_any_element()
            }

            TimelineItem::Notice { message, .. } => div()
                .p_2()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff0a))
                .child(message.clone())
                .into_any_element(),

            TimelineItem::Error { message, .. } => div()
                .p_2()
                .rounded(Radii::CHIP)
                .bg(rgba(0xf871711a))
                .border_1()
                .border_color(rgb(0xf87171))
                .child(format!("Error: {message}"))
                .into_any_element(),

            _ => div().into_any_element(),
        }
    }

    /// Renders the 26px status bar at bottom.
    fn render_statusbar(&self, theme: &Theme) -> impl IntoElement {
        let server_desc = match &self.server_status {
            ServerStatus::Connected {
                pipe_path,
                pid,
                server_version,
            } => {
                format!("Server v{server_version} (PID {pid}) · {pipe_path}")
            }
            ServerStatus::Connecting => "Connecting to PandaMUX Server...".to_string(),
            ServerStatus::Disconnected => "Server Offline".to_string(),
            ServerStatus::Failed(e) => format!("Server Error: {e}"),
        };

        div()
            .h(Spacing::STATUSBAR_HEIGHT)
            .w_full()
            .h_flex()
            .items_center()
            .justify_between()
            .px_3()
            .bg(theme.chrome.bgc_knockout)
            .border_t_1()
            .border_color(rgba(0xffffff0d))
            .text_size(Typography::STATUS_SIZE)
            .text_color(theme.chrome.text_t3)
            .child(div().child(server_desc))
            .child(div().child("PandaMUX 1.0 · Protocol v3"))
    }
}
