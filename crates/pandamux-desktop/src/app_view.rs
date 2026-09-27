use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use pandamux_client::projections::ThreadProjection;
use pandamux_core::{
    AgentId, ApprovalDecision, EnvironmentId, Thread, ThreadEvent, ThreadId,
    ThreadStatus,
};
use pandamux_protocol::{
    EventEnvelope, ThreadCancelTurnParams, ThreadCreateParams, ThreadRespondApprovalParams,
    ThreadSendTurnParams,
};

use crate::composer::{render_composer, ComposerState};
use crate::picker::PickerState;
use crate::server_bridge::{spawn_server_bridge, ServerBridgeHandle, ServerStatus};
use crate::sidebar::{render_rail, render_sidebar, RailTab};
use crate::theme::{AccentColor, Radii, Spacing, Theme, Typography};
use crate::timeline::render_timeline_item;
use crate::titlebar::CustomTitlebar;

/// The primary application view for PandaMUX Desktop.
pub struct AppView {
    theme: Theme,
    server_status: ServerStatus,
    bridge: Option<ServerBridgeHandle>,
    thread_projections: HashMap<ThreadId, ThreadProjection>,
    thread_order: Vec<ThreadId>,
    active_thread_id: Option<ThreadId>,
    composer: ComposerState,
    picker: PickerState,
    active_rail_tab: RailTab,
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
            composer: ComposerState::new(),
            picker: PickerState::new(),
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

    /// Dispatches create_thread intent to backend server using active picker state.
    pub fn create_default_thread(&mut self, cx: &mut Context<Self>) {
        let title = format!("Thread #{}", self.thread_order.len() + 1);
        let params = ThreadCreateParams {
            project_id: None,
            environment_id: Some(EnvironmentId::from("env-local")),
            title: Some(title),
            provider_instance_id: Some(self.picker.provider_instance_id()),
            model: Some(self.picker.model().to_string()),
            effort: self.picker.effort.clone(),
            access_mode: None,
            agent_id: None,
            cwd: None,
            worktree: None,
        };

        self.dispatch_create_thread(params, cx);
    }

    /// Dispatches create_thread intent assigned to a specific agent.
    pub fn create_agent_thread(&mut self, agent_id: &'static str, cx: &mut Context<Self>) {
        let title = format!("Agent: {agent_id} #{}", self.thread_order.len() + 1);
        let params = ThreadCreateParams {
            project_id: None,
            environment_id: Some(EnvironmentId::from("env-local")),
            title: Some(title),
            provider_instance_id: Some(self.picker.provider_instance_id()),
            model: Some(self.picker.model().to_string()),
            effort: self.picker.effort.clone(),
            access_mode: None,
            agent_id: Some(AgentId::from(agent_id)),
            cwd: None,
            worktree: None,
        };

        self.active_rail_tab = RailTab::Threads;
        self.dispatch_create_thread(params, cx);
    }

    fn dispatch_create_thread(&mut self, params: ThreadCreateParams, cx: &mut Context<Self>) {
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

    /// Dispatches send_turn intent to backend server with current composer prompt and picker model/effort.
    pub fn send_composer_turn(&mut self, cx: &mut Context<Self>) {
        let prompt = self.composer.text.trim().to_string();
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
            model: Some(self.picker.model().to_string()),
            effort: self.picker.effort.clone(),
        };

        self.composer.clear();

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

    /// Sets the active thread.
    pub fn select_thread(&mut self, thread_id: ThreadId) {
        self.active_thread_id = Some(thread_id);
    }

    /// Sets the active navigation rail tab.
    pub fn select_rail_tab(&mut self, tab: RailTab) {
        self.active_rail_tab = tab;
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
            // 2. Central Layout (52px Rail + 264px Sidebar + Main Workspace)
            .child(
                div()
                    .flex_1()
                    .h_flex()
                    .overflow_hidden()
                    // 2a. 52px Navigation Icon Rail
                    .child(render_rail(
                        self.active_rail_tab,
                        &theme,
                        |this, tab, _win, cx| {
                            this.select_rail_tab(tab);
                            cx.notify();
                        },
                        cx,
                    ))
                    // 2b. 264px Navigation Sidebar with Stubs
                    .child(render_sidebar(
                        self.active_rail_tab,
                        &self.thread_order,
                        &self.thread_projections,
                        self.active_thread_id.as_ref(),
                        &theme,
                        |this, _win, cx| {
                            this.create_default_thread(cx);
                        },
                        |this, tid, _win, cx| {
                            this.select_thread(tid);
                            cx.notify();
                        },
                        |this, agent_id, _win, cx| {
                            this.create_agent_thread(agent_id, cx);
                        },
                        cx,
                    ))
                    // 2c. Main Surface (Timeline + Header + Composer)
                    .child(self.render_main_surface(&theme, cx)),
            )
            // 3. 26px Status Bar
            .child(self.render_statusbar(&theme))
    }
}

impl AppView {
    /// Renders the central workspace with timeline and composer.
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
                // Top Header Bar
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
                                                .text_color(theme.accent.color())
                                                .child(format!("Provider: {}", proj.thread.provider_instance_id.as_str())),
                                        )
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
                                .label("● Ready")
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
                            render_timeline_item(
                                idx,
                                item,
                                &thread_id,
                                theme,
                                |this: &mut AppView, tid, req_id, dec, _win, cx| {
                                    this.respond_approval(tid, req_id, dec, cx);
                                },
                                cx,
                            )
                        })),
                )
                // Rich Composer
                .child(render_composer(
                    &self.composer,
                    &self.picker,
                    theme,
                    |this, _win, cx| {
                        this.send_composer_turn(cx);
                    },
                    |this, _win, cx| {
                        this.composer.clear();
                        cx.notify();
                    },
                    |this, _win, cx| {
                        this.picker.cycle_provider();
                        cx.notify();
                    },
                    |this, _win, cx| {
                        this.picker.cycle_model();
                        cx.notify();
                    },
                    |this, _win, cx| {
                        this.picker.cycle_effort();
                        cx.notify();
                    },
                    cx,
                ))
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
                        .text_size(px(28.0))
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
            ServerStatus::Disconnected => "Server Disconnected (Reconnecting with sinceSeq...)".to_string(),
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
