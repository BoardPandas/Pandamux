use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::projections::ThreadProjection;
use pandamux_core::{
    AgentId, ApprovalDecision, EnvironmentId, Thread, ThreadEvent, ThreadId, ThreadStatus,
};
use pandamux_protocol::{
    AttachmentImportPathParams, AttachmentImportResult, AttachmentPutChunkParams,
    AttachmentPutResult, EventEnvelope, GitCommitParams, GitCommitResult, GitCreatePrParams,
    GitCreatePrResult, GitPushParams, GitPushResult, GitStatusParams, GitStatusResult,
    ProviderHealthResult, SettingsGetResult, SettingsSetParams, ThreadCancelTurnParams,
    ThreadCreateParams, ThreadRespondApprovalParams, ThreadSendTurnParams,
};

use crate::composer::{ComposerState, render_composer};
use crate::picker::PickerState;
use crate::server_bridge::{ServerBridgeHandle, ServerStatus, spawn_server_bridge};
use crate::settings_view::{SettingsViewState, render_settings_view};
use crate::sidebar::{RailTab, render_rail, render_sidebar};
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
    pub settings_view: SettingsViewState,
    pub git_status: Option<GitStatusResult>,
    pub git_action_msg: Option<String>,
    pub is_git_busy: bool,
}

impl AppView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = smol::channel::unbounded::<EventEnvelope>();
        let (status_tx, status_rx) = smol::channel::unbounded::<ServerStatus>();

        let bridge = spawn_server_bridge(event_tx, status_tx);

        // Spawn async listener for server status updates
        cx.spawn(async move |this, cx| {
            while let Ok(status) = status_rx.recv().await {
                cx.update(|cx| {
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
                cx.update(|cx| {
                    let _ = this.update(cx, |this: &mut AppView, cx| {
                        this.handle_event_envelope(envelope);
                        cx.notify();
                    });
                });
            }
        })
        .detach();

        let mut app = Self {
            theme: Theme::dark(AccentColor::Teal),
            server_status: ServerStatus::Connecting,
            bridge: Some(bridge),
            thread_projections: HashMap::new(),
            thread_order: Vec::new(),
            active_thread_id: None,
            composer: ComposerState::new(),
            picker: PickerState::new(),
            active_rail_tab: RailTab::Threads,
            settings_view: SettingsViewState::new(),
            git_status: None,
            git_action_msg: None,
            is_git_busy: false,
        };

        app.load_settings(cx);
        app.run_health_checks(cx);
        app
    }

    /// Ingests incoming EventEnvelope into the corresponding client-side projection.
    pub fn handle_event_envelope(&mut self, envelope: EventEnvelope) {
        if let Some(thread_id) = &envelope.thread_id
            && let Some(proj) = self.thread_projections.get_mut(thread_id)
        {
            let event = ThreadEvent {
                thread_id: thread_id.clone(),
                seq: envelope.seq,
                at_ms: envelope.at_ms,
                kind: envelope.kind,
            };
            proj.apply_event(&event);
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
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(thread) = serde_json::from_value::<Thread>(val)
                {
                    cx.update(|cx| {
                        let _ = this.update(cx, |this: &mut AppView, cx| {
                            this.add_thread(thread);
                            cx.notify();
                        });
                    });
                }
            })
            .detach();
        }
    }

    /// Dispatches send_turn intent to backend server with current composer prompt and picker model/effort.
    /// Dispatches send_turn intent to backend server with current composer prompt and picker model/effort.
    pub fn send_composer_turn(&mut self, cx: &mut Context<Self>) {
        let prompt = self.composer.text.trim().to_string();
        let attachments = self.composer.attachments.clone();
        if prompt.is_empty() && attachments.is_empty() {
            return;
        }

        let thread_id = match &self.active_thread_id {
            Some(id) => id.clone(),
            None => return,
        };

        let attachment_ids: Vec<String> = attachments.into_iter().map(|a| a.id).collect();

        let effective_text = if prompt.is_empty() {
            "Please inspect the attached files.".to_string()
        } else {
            prompt
        };

        let params = ThreadSendTurnParams {
            thread_id: thread_id.clone(),
            text: effective_text,
            attachment_ids,
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

    /// Imports a file from a local filesystem path as an attachment for the active thread.
    pub fn import_attachment_path(&mut self, path: String, cx: &mut Context<Self>) {
        let thread_id = match &self.active_thread_id {
            Some(id) => id.clone(),
            None => return,
        };

        let params = AttachmentImportPathParams { thread_id, path };

        if let Some(bridge) = &self.bridge {
            let rx =
                bridge.send_request("attachment.import_path", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(import_res) = serde_json::from_value::<AttachmentImportResult>(val)
                {
                    let _ = this.update(cx, |app, cx| {
                        app.composer.add_attachment(import_res.attachment);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// Uploads raw bytes as an attachment in 256 KiB chunks (e.g., from clipboard images).
    pub fn upload_attachment_chunked(
        &mut self,
        file_name: String,
        mime_type: String,
        data: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let thread_id = match &self.active_thread_id {
            Some(id) => id.clone(),
            None => return,
        };

        let cmd_tx = match &self.bridge {
            Some(b) => b.request_sender(),
            None => return,
        };

        let id = format!("att-clip-{}", uuid::Uuid::new_v4().simple());
        let chunk_size = pandamux_core::ATTACHMENT_CHUNK_SIZE_BYTES;
        let total_chunks = if data.is_empty() {
            1
        } else {
            data.len().div_ceil(chunk_size) as u32
        };

        use base64::Engine;
        use base64::engine::general_purpose::STANDARD as BASE64;

        cx.spawn(async move |this, cx| {
            for chunk_idx in 0..total_chunks {
                let start = (chunk_idx as usize) * chunk_size;
                let end = ((chunk_idx as usize + 1) * chunk_size).min(data.len());
                let slice = &data[start..end];
                let b64 = BASE64.encode(slice);

                let params = AttachmentPutChunkParams {
                    thread_id: thread_id.clone(),
                    id: id.clone(),
                    chunk_index: chunk_idx,
                    total_chunks,
                    data_base64: b64,
                    mime_type: mime_type.clone(),
                    file_name: file_name.clone(),
                };

                let rx = crate::server_bridge::send_request_channel(
                    &cmd_tx,
                    "attachment.put",
                    serde_json::to_value(&params).ok(),
                );
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(put_res) = serde_json::from_value::<AttachmentPutResult>(val)
                    && put_res.is_complete
                    && let Some(record) = put_res.attachment
                {
                    let _ = this.update(cx, |app, cx| {
                        app.composer.add_attachment(record);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Invokes native file dialog or path prompt to pick attachments.
    pub fn pick_attachment(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });

        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                for path in paths {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = this.update(cx, |app, cx| {
                        app.import_attachment_path(path_str, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// Checks the system clipboard for file paths or image data URLs to attach.
    pub fn paste_clipboard_attachment(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard()
            && let Some(text) = item.text()
        {
            let trimmed = text.trim();

            // 1. Support data:image/...;base64,... images from clipboard
            if trimmed.starts_with("data:image/")
                && let Some((mime, rest)) = trimmed
                    .strip_prefix("data:")
                    .and_then(|s| s.split_once(";base64,"))
            {
                use base64::Engine;
                use base64::engine::general_purpose::STANDARD as BASE64;
                if let Ok(bytes) = BASE64.decode(rest.trim()) {
                    let ext = match mime {
                        "image/png" => "png",
                        "image/jpeg" => "jpg",
                        "image/gif" => "gif",
                        "image/webp" => "webp",
                        _ => "img",
                    };
                    self.upload_attachment_chunked(
                        format!("pasted_image.{ext}"),
                        mime.to_string(),
                        bytes,
                        cx,
                    );
                    return;
                }
            }

            // 2. Support local filesystem paths
            let path = std::path::Path::new(trimmed);
            if path.is_file() {
                self.import_attachment_path(trimmed.to_string(), cx);
            }
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
            let rx = bridge.send_request(
                "thread.respond_approval",
                serde_json::to_value(&params).ok(),
            );
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

    /// Sets the active thread and queries Git status.
    pub fn select_thread(&mut self, thread_id: ThreadId, cx: &mut Context<Self>) {
        self.active_thread_id = Some(thread_id);
        self.refresh_git_status(cx);
    }

    /// Sets the active navigation rail tab.
    pub fn select_rail_tab(&mut self, tab: RailTab) {
        self.active_rail_tab = tab;
    }

    /// Synchronizes the active GPUI Theme from the loaded user settings.
    pub fn sync_theme_from_settings(&mut self) {
        let accent = match self.settings_view.settings.ui.accent.as_str() {
            "gold" => AccentColor::Gold,
            "blue" => AccentColor::Blue,
            "purple" => AccentColor::Purple,
            _ => AccentColor::Teal,
        };
        let is_light = self.settings_view.settings.ui.theme == "light";
        self.theme = if is_light {
            Theme::light(accent)
        } else {
            Theme::dark(accent)
        };
    }

    /// Loads settings from the server daemon.
    pub fn load_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(bridge) = &self.bridge {
            let rx = bridge.send_request("settings.get", None);
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(res) = serde_json::from_value::<SettingsGetResult>(val)
                {
                    let _ = this.update(cx, |app, cx| {
                        app.settings_view.settings = res.settings;
                        app.sync_theme_from_settings();
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// Saves settings to the server daemon.
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(bridge) = &self.bridge {
            self.settings_view.is_saving = true;
            let params = SettingsSetParams {
                settings: Some(self.settings_view.settings.clone()),
                key: None,
                value: None,
            };
            let rx = bridge.send_request("settings.set", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await
                    && resp.is_success()
                {
                    let _ = this.update(cx, |app, cx| {
                        app.settings_view.is_saving = false;
                        app.settings_view.status_banner =
                            Some(("Settings saved successfully".to_string(), false));
                        app.sync_theme_from_settings();
                        cx.notify();
                    });
                } else {
                    let _ = this.update(cx, |app, cx| {
                        app.settings_view.is_saving = false;
                        app.settings_view.status_banner =
                            Some(("Failed to save settings".to_string(), true));
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// Triggers zero-spawn, offline health checks for all configured providers.
    pub fn run_health_checks(&mut self, cx: &mut Context<Self>) {
        if let Some(bridge) = &self.bridge {
            self.settings_view.is_checking_health = true;
            let rx = bridge.send_request("provider.health", None);
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(res) = serde_json::from_value::<ProviderHealthResult>(val)
                {
                    let _ = this.update(cx, |app, cx| {
                        app.settings_view.health_reports = res.reports;
                        app.settings_view.is_checking_health = false;
                        app.settings_view.status_banner = Some((
                            "Offline health checks complete (Rule 1: zero-spawn, zero-auth)"
                                .to_string(),
                            false,
                        ));
                        cx.notify();
                    });
                } else {
                    let _ = this.update(cx, |app, cx| {
                        app.settings_view.is_checking_health = false;
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// Renders the Settings workspace surface.
    fn render_settings_surface(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let server_status_label = format!("{}", self.server_status);
        render_settings_view(
            &self.settings_view,
            theme,
            server_status_label,
            |this, _win, cx| {
                this.save_settings(cx);
            },
            |this, _win, cx| {
                this.run_health_checks(cx);
            },
            |this, modifier, _win, cx| {
                modifier(&mut this.settings_view.settings);
                this.sync_theme_from_settings();
                cx.notify();
            },
            cx,
        )
    }

    /// Queries current Git status for the active thread worktree.
    pub fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        if let Some(thread_id) = &self.active_thread_id
            && let Some(bridge) = &self.bridge
        {
            let params = GitStatusParams {
                thread_id: thread_id.clone(),
            };
            let rx = bridge.send_request("git.status", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(resp)) = rx.await
                    && let Some(val) = resp.result
                    && let Ok(res) = serde_json::from_value::<GitStatusResult>(val)
                {
                    let _ = this.update(cx, |app, cx| {
                        app.git_status = Some(res);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// Stages all changes and creates a Git commit in the thread worktree.
    pub fn commit_git(&mut self, message: Option<String>, cx: &mut Context<Self>) {
        if let Some(thread_id) = &self.active_thread_id
            && let Some(bridge) = &self.bridge
        {
            self.is_git_busy = true;
            let params = GitCommitParams {
                thread_id: thread_id.clone(),
                message,
            };
            let rx = bridge.send_request("git.commit", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                let res = rx.await;
                let _ = this.update(cx, |app, cx| {
                    app.is_git_busy = false;
                    if let Ok(Ok(resp)) = res
                        && let Some(val) = resp.result
                        && let Ok(commit_res) = serde_json::from_value::<GitCommitResult>(val)
                    {
                        let short_hash = if commit_res.commit_hash.len() >= 7 {
                            &commit_res.commit_hash[..7]
                        } else {
                            &commit_res.commit_hash
                        };
                        app.git_action_msg =
                            Some(format!("Committed {short_hash}: {}", commit_res.message));
                        app.refresh_git_status(cx);
                    } else {
                        app.git_action_msg = Some("Git commit failed or no changes".to_string());
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Pushes committed changes in the thread worktree to remote origin.
    pub fn push_git(&mut self, cx: &mut Context<Self>) {
        if let Some(thread_id) = &self.active_thread_id
            && let Some(bridge) = &self.bridge
        {
            self.is_git_busy = true;
            let params = GitPushParams {
                thread_id: thread_id.clone(),
                remote: None,
                branch: None,
            };
            let rx = bridge.send_request("git.push", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                let res = rx.await;
                let _ = this.update(cx, |app, cx| {
                    app.is_git_busy = false;
                    if let Ok(Ok(resp)) = res
                        && let Some(val) = resp.result
                        && let Ok(push_res) = serde_json::from_value::<GitPushResult>(val)
                    {
                        app.git_action_msg = Some(push_res.message);
                        app.refresh_git_status(cx);
                    } else {
                        app.git_action_msg = Some("Git push failed".to_string());
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Creates a Pull Request using gh CLI if available, or generates a compare URL.
    pub fn create_pr_git(&mut self, cx: &mut Context<Self>) {
        if let Some(thread_id) = &self.active_thread_id
            && let Some(bridge) = &self.bridge
        {
            self.is_git_busy = true;
            let params = GitCreatePrParams {
                thread_id: thread_id.clone(),
                title: None,
                body: None,
            };
            let rx = bridge.send_request("git.create_pr", serde_json::to_value(&params).ok());
            cx.spawn(async move |this, cx| {
                let res = rx.await;
                let _ = this.update(cx, |app, cx| {
                    app.is_git_busy = false;
                    if let Ok(Ok(resp)) = res
                        && let Some(val) = resp.result
                        && let Ok(pr_res) = serde_json::from_value::<GitCreatePrResult>(val)
                    {
                        app.git_action_msg = Some(format!("PR: {}", pr_res.url));
                        app.refresh_git_status(cx);
                    } else {
                        app.git_action_msg = Some("Create PR failed".to_string());
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Renders the Git Actions bar showing branch, ahead/behind, changes, and quick actions.
    fn render_git_actions_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let git_status = self.git_status.as_ref();
        let is_repo = git_status.map(|s| s.is_repo).unwrap_or(false);
        let branch = git_status.map(|s| s.branch.as_str()).unwrap_or("none");
        let ahead = git_status.map(|s| s.ahead).unwrap_or(0);
        let behind = git_status.map(|s| s.behind).unwrap_or(0);
        let file_count = git_status.map(|s| s.files.len()).unwrap_or(0);
        let drafted_msg = git_status
            .map(|s| s.drafted_commit_message.as_str())
            .unwrap_or("");
        let is_busy = self.is_git_busy;

        div()
            .w_full()
            .p_2()
            .rounded(Radii::ROW)
            .bg(theme.chrome.panel2)
            .border_1()
            .border_color(rgba(0xffffff0d))
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
                            .gap_2()
                            // Branch indicator
                            .child(
                                div()
                                    .h_flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0xffffff0a))
                                    .text_size(Typography::META_SIZE)
                                    .text_color(theme.chrome.text_t1)
                                    .child(if is_repo {
                                        format!("🌱 {branch}")
                                    } else {
                                        "Non-Git Workspace".to_string()
                                    }),
                            )
                            // Ahead / Behind pills
                            .when(ahead > 0 || behind > 0, |this| {
                                this.child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.accent.color())
                                        .child(format!("↑{ahead} ↓{behind}")),
                                )
                            })
                            // Change count badge
                            .when(is_repo, |this| {
                                this.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(if file_count > 0 {
                                            rgba(0xd8b45e22)
                                        } else {
                                            rgba(0x7fd88f22)
                                        })
                                        .text_size(Typography::META_SIZE)
                                        .text_color(if file_count > 0 {
                                            rgb(0xd8b45e)
                                        } else {
                                            rgb(0x7fd88f)
                                        })
                                        .child(if file_count > 0 {
                                            format!("● {file_count} changed")
                                        } else {
                                            "✓ Clean".to_string()
                                        }),
                                )
                            })
                            // Feedback toast
                            .when_some(self.git_action_msg.as_ref(), |this, msg| {
                                this.child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.accent.color())
                                        .child(msg.clone()),
                                )
                            }),
                    )
                    // Action buttons
                    .when(is_repo, |this| {
                        this.child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(Button::new("btn-git-refresh").ghost().label("🔄").on_click(
                                    cx.listener(|this, _event, _window, cx| {
                                        this.refresh_git_status(cx);
                                    }),
                                ))
                                .child(
                                    Button::new("btn-git-commit")
                                        .ghost()
                                        .label(if is_busy { "Committing..." } else { "Commit" })
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.commit_git(None, cx);
                                        })),
                                )
                                .child(Button::new("btn-git-push").ghost().label("Push").on_click(
                                    cx.listener(|this, _event, _window, cx| {
                                        this.push_git(cx);
                                    }),
                                ))
                                .child(
                                    Button::new("btn-git-pr")
                                        .ghost()
                                        .label("Create PR")
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.create_pr_git(cx);
                                        })),
                                ),
                        )
                    }),
            )
            // Draft message preview when changes exist
            .when(file_count > 0 && !drafted_msg.is_empty(), |this| {
                this.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("Draft:")
                        .child(
                            div()
                                .text_color(theme.chrome.text_t2)
                                .font_family(Typography::MONO_FAMILY)
                                .child(drafted_msg.to_string()),
                        ),
                )
            })
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
                            this.select_thread(tid, cx);
                            cx.notify();
                        },
                        |this, agent_id, _win, cx| {
                            this.create_agent_thread(agent_id, cx);
                        },
                        self.settings_view.active_category,
                        |this, cat, _win, cx| {
                            this.settings_view.set_category(cat);
                            cx.notify();
                        },
                        cx,
                    ))
                    // 2c. Main Surface (Timeline + Header + Composer OR Settings View)
                    .child(if self.active_rail_tab == RailTab::Settings {
                        self.render_settings_surface(&theme, cx).into_any_element()
                    } else {
                        self.render_main_surface(&theme, cx).into_any_element()
                    }),
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
                                                .child(format!(
                                                    "Provider: {}",
                                                    proj.thread.provider_instance_id.as_str()
                                                )),
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
                            Button::new("btn-ready").ghost().label("● Ready")
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
                        .children(proj.grouped_items().iter().enumerate().map(|(idx, item)| {
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
                // Rich Composer with Drag & Drop file attachment support
                .child(
                    div()
                        .w_full()
                        .on_drop(cx.listener(
                            |this: &mut AppView, paths: &ExternalPaths, _window, cx| {
                                for path in paths.paths() {
                                    let path_str = path.to_string_lossy().to_string();
                                    this.import_attachment_path(path_str, cx);
                                }
                            },
                        ))
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
                            |this, win, cx| {
                                this.pick_attachment(win, cx);
                            },
                            |this, att_id, _win, cx| {
                                this.composer.remove_attachment(&att_id);
                                cx.notify();
                            },
                            cx,
                        )),
                )
                // Git actions bar
                .child(self.render_git_actions_bar(theme, cx))
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
            ServerStatus::Disconnected => {
                "Server Disconnected (Reconnecting with sinceSeq...)".to_string()
            }
            ServerStatus::Failed(e) => format!("Server Error: {e}"),
        };

        let git_info = self
            .git_status
            .as_ref()
            .map(|s| {
                if s.is_repo {
                    let clean_dirty = if s.files.is_empty() {
                        "✓ Clean"
                    } else {
                        "● Modified"
                    };
                    format!("🌱 {} · {}", s.branch, clean_dirty)
                } else {
                    "Local Workspace".to_string()
                }
            })
            .unwrap_or_else(|| "PandaMUX 1.0".to_string());

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
            .child(div().child(git_info))
    }
}
