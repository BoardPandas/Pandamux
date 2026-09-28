#![recursion_limit = "256"]

pub mod app_view;
pub mod composer;
pub mod diff_view;
pub mod picker;
pub mod server_bridge;
pub mod settings_view;
pub mod sidebar;
pub mod theme;
pub mod timeline;
pub mod titlebar;

pub use app_view::AppView;
pub use composer::{ComposerState, render_composer};
pub use diff_view::{
    DiffFile, DiffHunk, DiffLine, DiffLineKind, DiffViewMode, DiffViewerState, DiffWord,
    SplitDiffRow, parse_unified_diff, render_diff_file, render_diff_viewer,
};
pub use picker::{ModelChoice, PickerState, ProviderChoice};
pub use server_bridge::{
    BridgeCommand, RuntimeInfo, ServerBridgeHandle, ServerStatus, discover_server_runtime,
    spawn_server_bridge,
};
pub use settings_view::{SettingsCategory, SettingsViewState, render_settings_view};
pub use sidebar::{AgentRosterItem, DEFAULT_AGENTS, RailTab, render_rail, render_sidebar};
pub use theme::{AccentColor, ChromePalette, Radii, Spacing, Theme, ThemeMode, Typography};
pub use timeline::render_timeline_item;
pub use titlebar::CustomTitlebar;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui::px;
    use pandamux_core::{
        ApprovalDecision, ApprovalKind, EnvironmentId, ProviderInstanceId, Thread, ThreadEvent,
        ThreadEventKind, ThreadId, ThreadStatus, ThreadWorkspace, TurnId, TurnInput, TurnOutcome,
    };
    use pandamux_protocol::EventEnvelope;

    #[test]
    fn test_gpui_kit_highlighter_theme() {
        let dark_theme = gpui_kit::component::highlighter::HighlightTheme::default_dark();
        assert_eq!(dark_theme.appearance, gpui_kit::component::ThemeMode::Dark);
    }

    #[test]
    fn test_theme_section_12_tokens() {
        let dark = Theme::dark(AccentColor::Teal);
        assert_eq!(dark.mode, ThemeMode::Dark);
        assert_eq!(dark.accent, AccentColor::Teal);

        // Verify key Section 12 dimensions
        assert_eq!(Spacing::TITLEBAR_HEIGHT, px(40.0));
        assert_eq!(Spacing::STATUSBAR_HEIGHT, px(26.0));
        assert_eq!(Spacing::WORKSPACE_PADDING, px(10.0));
        assert_eq!(Spacing::RAIL_WIDTH, px(52.0));
        assert_eq!(Spacing::SIDEBAR_WIDTH, px(264.0));
        assert_eq!(Spacing::SIDEBAR_COMPACT_WIDTH, px(216.0));

        // Verify Section 12 radii
        assert_eq!(Radii::PANE, px(12.0));
        assert_eq!(Radii::OVERLAY, px(16.0));
        assert_eq!(Radii::ROW, px(8.0));
        assert_eq!(Radii::CHIP, px(6.0));
        assert_eq!(Radii::RAIL_BUTTON, px(10.0));

        // Verify typography sizes
        assert_eq!(Typography::TITLE_SIZE, px(13.0));
        assert_eq!(Typography::BODY_SIZE, px(12.5));
        assert_eq!(Typography::SECONDARY_SIZE, px(11.0));
        assert_eq!(Typography::META_SIZE, px(10.0));

        let light = Theme::light(AccentColor::Gold);
        assert_eq!(light.mode, ThemeMode::Light);
        assert_eq!(light.accent, AccentColor::Gold);
    }

    #[test]
    fn test_runtime_info_round_trip() {
        let json = r#"{
            "pid": 12345,
            "role": "hub",
            "protocolVersion": 3,
            "serverVersion": "0.53.16",
            "pipePath": "/tmp/test.sock",
            "token": "secret-token",
            "startedAtMs": 1700000000000
        }"#;

        let info: RuntimeInfo = serde_json::from_str(json).expect("deserialize RuntimeInfo");
        assert_eq!(info.pid, 12345);
        assert_eq!(info.role, "hub");
        assert_eq!(info.protocol_version, 3);
        assert_eq!(info.server_version, "0.53.16");
        assert_eq!(info.pipe_path, "/tmp/test.sock");
        assert_eq!(info.token, "secret-token");
        assert_eq!(info.started_at_ms, 1700000000000);
    }

    #[test]
    fn test_thread_projection_event_accumulation() {
        let thread = Thread {
            id: ThreadId::from("thread-desktop-1"),
            project_id: None,
            environment_id: EnvironmentId::from("env-local"),
            parent_thread_id: None,
            title: "Desktop Projection Thread".to_string(),
            provider_instance_id: ProviderInstanceId::from("claude"),
            model: "claude-3-7-sonnet".to_string(),
            effort: None,
            access_mode: Default::default(),
            workspace: ThreadWorkspace {
                cwd: "/repo".to_string(),
                worktree: None,
            },
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };

        let mut proj = pandamux_client::projections::ThreadProjection::new(thread);
        assert_eq!(proj.thread.status, ThreadStatus::Idle);

        // TurnRequested
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 1,
            at_ms: 1010,
            kind: ThreadEventKind::TurnRequested {
                turn_id: TurnId::from("turn-1"),
                input: TurnInput {
                    text: "Hello desktop agent".to_string(),
                    attachment_ids: vec![],
                    attachments: vec![],
                    model: None,
                    effort: None,
                },
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Working);
        assert_eq!(proj.items.len(), 1);

        // AssistantText
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 2,
            at_ms: 1020,
            kind: ThreadEventKind::AssistantText {
                item_id: "item-1".to_string(),
                text: "Hello from Claude!".to_string(),
            },
        });
        assert_eq!(proj.items.len(), 2);

        // ApprovalRequested
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 3,
            at_ms: 1030,
            kind: ThreadEventKind::ApprovalRequested {
                request_id: "req-appr-1".to_string(),
                kind: ApprovalKind::CommandExecution,
                detail: serde_json::json!({ "command": "cargo test" }),
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::AwaitingApproval);
        assert_eq!(proj.items.len(), 3);

        // ApprovalResolved
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 4,
            at_ms: 1040,
            kind: ThreadEventKind::ApprovalResolved {
                request_id: "req-appr-1".to_string(),
                decision: ApprovalDecision::Approved,
                by: "user".to_string(),
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Working);

        // TurnCompleted
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 5,
            at_ms: 1050,
            kind: ThreadEventKind::TurnCompleted {
                outcome: TurnOutcome::Success,
            },
        });

        // TurnSettled
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-desktop-1"),
            seq: 6,
            at_ms: 1060,
            kind: ThreadEventKind::TurnSettled {
                changed_files: vec!["file1.rs".to_string()],
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Idle);
        assert_eq!(proj.changed_files, vec!["file1.rs".to_string()]);
        assert_eq!(proj.last_seq, 6);
    }

    #[test]
    fn test_picker_cycling_and_selection() {
        let mut picker = PickerState::new();
        assert_eq!(picker.provider_id, "claude");
        assert_eq!(picker.model_id, "claude-3-7-sonnet");
        assert_eq!(picker.effort_str(), Some("high"));

        // Cycle provider: claude -> codex
        picker.cycle_provider();
        assert_eq!(picker.provider_id, "codex");
        assert_eq!(picker.model_id, "o3-mini");

        // Cycle model for codex: o3-mini -> gpt-4.5-preview
        picker.cycle_model();
        assert_eq!(picker.model_id, "gpt-4.5-preview");

        // Cycle provider again: codex -> antigravity
        picker.cycle_provider();
        assert_eq!(picker.provider_id, "antigravity");
        assert_eq!(picker.model_id, "gemini-2.0-flash");

        // Direct selection
        picker.select_provider("claude");
        picker.select_model("claude-3-5-haiku");
        assert_eq!(
            picker.provider_instance_id(),
            ProviderInstanceId::from("claude")
        );
        assert_eq!(picker.model(), "claude-3-5-haiku");

        // Effort cycling: high -> max -> None -> low -> medium -> high
        picker.cycle_effort();
        assert_eq!(picker.effort_str(), Some("max"));
        picker.cycle_effort();
        assert_eq!(picker.effort_str(), None);
        picker.cycle_effort();
        assert_eq!(picker.effort_str(), Some("low"));
    }

    #[test]
    fn test_composer_state() {
        let mut comp = ComposerState::new();
        assert!(comp.is_empty());
        assert_eq!(comp.text.len(), 0);

        comp.set_text("Hello agent");
        assert!(!comp.is_empty());
        assert_eq!(comp.text, "Hello agent");

        comp.append_char('!');
        assert_eq!(comp.text, "Hello agent!");

        comp.clear();
        assert!(comp.is_empty());

        // Test attachment management in composer
        let att1 = pandamux_core::AttachmentRecord {
            id: "att-1".to_string(),
            thread_id: ThreadId::from("t-1"),
            file_name: "test.png".to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 50000,
            file_path: "/path/to/test.png".to_string(),
            created_at_ms: 1000,
        };
        comp.add_attachment(att1.clone());
        assert!(!comp.is_empty());
        assert_eq!(comp.attachments.len(), 1);
        assert_eq!(comp.total_attachment_bytes(), 50000);

        let att2 = pandamux_core::AttachmentRecord {
            id: "att-2".to_string(),
            thread_id: ThreadId::from("t-1"),
            file_name: "test.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 1000,
            file_path: "/path/to/test.txt".to_string(),
            created_at_ms: 1005,
        };
        comp.add_attachment(att2);
        assert_eq!(comp.attachments.len(), 2);
        assert_eq!(comp.total_attachment_bytes(), 51000);

        // Remove attachment
        comp.remove_attachment("att-1");
        assert_eq!(comp.attachments.len(), 1);
        assert_eq!(comp.attachments[0].id, "att-2");

        // Clear attachments
        comp.clear_attachments();
        assert!(comp.is_empty());
    }

    #[test]
    fn test_sidebar_agents_roster() {
        assert_eq!(DEFAULT_AGENTS.len(), 8);
        let ids: Vec<&str> = DEFAULT_AGENTS.iter().map(|a| a.id).collect();
        assert!(ids.contains(&"orchestrator"));
        assert!(ids.contains(&"architect"));
        assert!(ids.contains(&"builder"));
        assert!(ids.contains(&"reviewer"));
        assert!(ids.contains(&"tester"));
        assert!(ids.contains(&"security"));
        assert!(ids.contains(&"performance"));
        assert!(ids.contains(&"ux-reviewer"));
    }

    #[test]
    fn test_since_seq_tracking_with_envelopes() {
        let (event_tx, _event_rx) = smol::channel::unbounded::<EventEnvelope>();
        let (status_tx, _status_rx) = smol::channel::unbounded::<ServerStatus>();

        let bridge = spawn_server_bridge(event_tx, status_tx);
        assert_eq!(bridge.last_seen_seq(), 0);
    }
}
