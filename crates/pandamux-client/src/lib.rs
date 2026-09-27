pub mod client;
pub mod projections;
pub mod token_queue;
pub mod transport;

pub use client::PandamuxClient;
pub use projections::{
    ChangedFileSummary, RunProjection, SubAgentProjection, ThreadProjection, TimelineItem,
    WorkLogEntry, WorkLogStatus, format_duration,
};
pub use token_queue::TokenSmoothingQueue;
pub use transport::{MockTransport, MockTransportPeer, TransportError};

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        ApprovalDecision, ApprovalKind, EnvironmentId, ProviderInstanceId, Run, RunEvent, RunKind,
        RunStatus, RunTask, RunTaskStatus, Thread, ThreadEvent, ThreadEventKind, ThreadId,
        ThreadStatus, ThreadWorkspace, ToolCallStatus, TurnId, TurnInput, TurnOutcome, TurnStatus,
        TurnUsage,
    };
    use pandamux_protocol::EventEnvelope;
    use serde_json::json;

    fn mock_thread(id: &str) -> Thread {
        Thread {
            id: ThreadId::from(id),
            project_id: None,
            environment_id: EnvironmentId::from("env-local"),
            parent_thread_id: None,
            title: "Test Thread".to_string(),
            provider_instance_id: ProviderInstanceId::from("prov-claude"),
            model: "claude-3-7-sonnet".to_string(),
            effort: None,
            access_mode: Default::default(),
            workspace: ThreadWorkspace {
                cwd: "/home/user/project".to_string(),
                worktree: None,
            },
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[test]
    fn thread_projection_turn_lifecycle_reducer() {
        let thread = mock_thread("thread-1");
        let mut proj = ThreadProjection::new(thread);
        assert_eq!(proj.thread.status, ThreadStatus::Idle);

        // 1. TurnRequested
        let turn_id = TurnId::from("turn-1");
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 1,
            at_ms: 1010,
            kind: ThreadEventKind::TurnRequested {
                turn_id: turn_id.clone(),
                input: TurnInput {
                    text: "Calculate 2+2 and run tests".to_string(),
                    attachment_ids: vec![],
                    model: None,
                    effort: None,
                },
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Working);
        assert_eq!(proj.turns.len(), 1);
        assert_eq!(proj.turns[0].status, TurnStatus::Requested);
        assert_eq!(proj.items.len(), 1);

        // 2. TurnStarted
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 2,
            at_ms: 1020,
            kind: ThreadEventKind::TurnStarted {
                turn_id: turn_id.clone(),
            },
        });
        assert_eq!(proj.turns[0].status, TurnStatus::Running);

        // 3. Streaming text coalescing
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 3,
            at_ms: 1030,
            kind: ThreadEventKind::AssistantText {
                item_id: "msg-1".to_string(),
                text: "I will ".to_string(),
            },
        });
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 4,
            at_ms: 1040,
            kind: ThreadEventKind::AssistantText {
                item_id: "msg-1".to_string(),
                text: "help you calculate 2+2.".to_string(),
            },
        });
        // Both text chunks coalesced into the same item!
        assert_eq!(proj.items.len(), 2);
        if let TimelineItem::AssistantText { id, text } = &proj.items[1] {
            assert_eq!(id, "msg-1");
            assert_eq!(text, "I will help you calculate 2+2.");
        } else {
            panic!("expected AssistantText");
        }

        // 4. ToolCall lifecycle
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 5,
            at_ms: 1050,
            kind: ThreadEventKind::ToolCall {
                item_id: "tool-1".to_string(),
                name: "calc".to_string(),
                input: json!({"expr": "2+2"}),
                status: ToolCallStatus::Executing,
                output: None,
            },
        });
        assert_eq!(proj.items.len(), 3);

        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 6,
            at_ms: 1060,
            kind: ThreadEventKind::ToolCall {
                item_id: "tool-1".to_string(),
                name: "calc".to_string(),
                input: json!({"expr": "2+2"}),
                status: ToolCallStatus::Success,
                output: Some("4".to_string()),
            },
        });
        assert_eq!(proj.items.len(), 3); // Updated in-place
        if let TimelineItem::ToolCall { status, output, .. } = &proj.items[2] {
            assert_eq!(*status, ToolCallStatus::Success);
            assert_eq!(output.as_deref(), Some("4"));
        } else {
            panic!("expected ToolCall");
        }

        // 5. Approval flow
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 7,
            at_ms: 1070,
            kind: ThreadEventKind::ApprovalRequested {
                request_id: "appr-1".to_string(),
                kind: ApprovalKind::CommandExecution,
                detail: json!({"cmd": "cargo test"}),
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::AwaitingApproval);

        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 8,
            at_ms: 1080,
            kind: ThreadEventKind::ApprovalResolved {
                request_id: "appr-1".to_string(),
                decision: ApprovalDecision::Approved,
                by: "user".to_string(),
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Working);

        // 6. SubAgent events
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 9,
            at_ms: 1090,
            kind: ThreadEventKind::SubAgentSpawned {
                sub_agent_id: "sub-1".to_string(),
                parent_sub_agent_id: None,
                parent_item_id: Some("tool-1".to_string()),
                title: "Test Subagent".to_string(),
                agent_type: "tester".to_string(),
                model: "claude-3-5-haiku".to_string(),
                effort: None,
            },
        });
        assert!(proj.sub_agents.contains_key("sub-1"));
        assert_eq!(proj.sub_agents["sub-1"].title, "Test Subagent");

        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 10,
            at_ms: 1100,
            kind: ThreadEventKind::SubAgentActivity {
                sub_agent_id: "sub-1".to_string(),
                preview: "Running unit tests...".to_string(),
            },
        });
        assert_eq!(
            proj.sub_agents["sub-1"].latest_activity.as_deref(),
            Some("Running unit tests...")
        );

        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 11,
            at_ms: 1110,
            kind: ThreadEventKind::SubAgentFinished {
                sub_agent_id: "sub-1".to_string(),
                outcome: "All tests passed".to_string(),
                elapsed_ms: 500,
                summary: Some("82 tests run".to_string()),
            },
        });
        assert!(proj.sub_agents["sub-1"].finished);

        // 7. Token Usage & Completion
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 12,
            at_ms: 1120,
            kind: ThreadEventKind::TokenUsage {
                usage: TurnUsage {
                    input_tokens: 150,
                    output_tokens: 45,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    cost_usd: Some(0.002),
                },
            },
        });
        assert_eq!(proj.turns[0].usage.as_ref().unwrap().input_tokens, 150);

        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 13,
            at_ms: 1130,
            kind: ThreadEventKind::TurnCompleted {
                outcome: TurnOutcome::Success,
            },
        });
        assert_eq!(proj.turns[0].status, TurnStatus::Completed);

        // 8. TurnSettled
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-1"),
            seq: 14,
            at_ms: 1140,
            kind: ThreadEventKind::TurnSettled {
                changed_files: vec!["src/main.rs".to_string()],
            },
        });
        assert_eq!(proj.thread.status, ThreadStatus::Idle);
        assert_eq!(proj.changed_files, vec!["src/main.rs"]);
        assert_eq!(proj.last_seq, 14);
    }

    #[test]
    fn run_projection_reducer() {
        use pandamux_core::RunId;

        let run = Run {
            id: RunId::from("run-1"),
            kind: RunKind::Orchestrator,
            status: RunStatus::Planned,
            trigger: "manual".to_string(),
            started_at_ms: 1000,
            ended_at_ms: None,
            tasks: vec![],
            blackboard: Default::default(),
            usage: None,
            outcome: None,
        };
        let mut proj = RunProjection::new(run);
        assert_eq!(proj.run.status, RunStatus::Planned);

        // Planned event
        let task = RunTask {
            task_id: "task-1".to_string(),
            target_project_id: None,
            agent_id: None,
            thread_id: None,
            status: RunTaskStatus::Pending,
            depends_on: vec![],
            summary: None,
        };
        proj.apply_event(&RunEvent::Planned {
            run_id: RunId::from("run-1"),
            tasks: vec![task],
        });
        assert_eq!(proj.run.tasks.len(), 1);

        // Dispatched event
        proj.apply_event(&RunEvent::TaskDispatched {
            run_id: RunId::from("run-1"),
            task_id: "task-1".to_string(),
            target_project_id: None,
            agent_id: None,
            thread_id: ThreadId::from("thread-task-1"),
        });
        assert_eq!(proj.run.status, RunStatus::Running);
        assert_eq!(proj.run.tasks[0].status, RunTaskStatus::Dispatched);

        // Finished event
        proj.apply_event(&RunEvent::Finished {
            run_id: RunId::from("run-1"),
            status: RunStatus::Succeeded,
        });
        assert_eq!(proj.run.status, RunStatus::Succeeded);
    }

    #[test]
    fn token_smoothing_queue_frame_draining() {
        let mut queue = TokenSmoothingQueue::new(120);
        assert_eq!(queue.target_fps(), 120);
        assert!(queue.is_empty());

        queue.push("msg-1", "Hello, ");
        queue.push("msg-1", "wonderful ");
        queue.push("msg-1", "world!");
        assert_eq!(queue.pending_chars(), 23);

        // Drain up to 10 characters (split across words)
        let frame1 = queue.drain_frame(10);
        let text1: String = frame1.into_iter().map(|(_, t)| t).collect();
        assert_eq!(text1, "Hello, won");
        assert_eq!(queue.pending_chars(), 13);

        // Flush remaining characters
        let remaining = queue.flush();
        let text2: String = remaining.into_iter().map(|(_, t)| t).collect();
        assert_eq!(text2, "derful world!");
        assert!(queue.is_empty());
        assert_eq!(queue.pending_chars(), 0);
    }

    #[test]
    fn client_envelope_ingestion_and_since_seq() {
        let (mock_transport, _peer) = MockTransport::pair();
        let mut client = PandamuxClient::new(mock_transport);

        let thread = mock_thread("thread-42");
        client.track_thread(thread);

        let envelope = EventEnvelope {
            subscription_id: "sub-1".to_string(),
            environment_id: EnvironmentId::from("env-local"),
            thread_id: Some(ThreadId::from("thread-42")),
            run_id: None,
            seq: 101,
            at_ms: 2000,
            kind: ThreadEventKind::TurnRequested {
                turn_id: TurnId::from("turn-42"),
                input: TurnInput {
                    text: "Verify since_seq replay".to_string(),
                    attachment_ids: vec![],
                    model: None,
                    effort: None,
                },
            },
        };

        client.ingest_envelope(&envelope);

        let proj = client
            .get_thread_projection(&ThreadId::from("thread-42"))
            .expect("projection found");
        assert_eq!(proj.last_seq, 101);
        assert_eq!(proj.thread.status, ThreadStatus::Working);
        assert_eq!(proj.turns.len(), 1);
        assert_eq!(proj.turns[0].input.text, "Verify since_seq replay");
    }

    #[test]
    fn format_duration_formatting() {
        assert_eq!(format_duration(500), "<1s");
        assert_eq!(format_duration(12_000), "12s");
        assert_eq!(format_duration(60_000), "1m 0s");
        assert_eq!(format_duration(680_000), "11m 20s");
    }

    #[test]
    fn work_log_grouping_and_timing() {
        use pandamux_core::FileChangeKind;

        let thread = mock_thread("thread-wl");
        let mut proj = ThreadProjection::new(thread);

        // Turn requested at 1000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 1,
            at_ms: 1000,
            kind: ThreadEventKind::TurnRequested {
                turn_id: TurnId::from("turn-wl"),
                input: TurnInput {
                    text: "Refactor database queries and test".to_string(),
                    attachment_ids: vec![],
                    model: None,
                    effort: None,
                },
            },
        });

        // Reasoning at 2000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 2,
            at_ms: 2000,
            kind: ThreadEventKind::Reasoning {
                item_id: "reason-1".to_string(),
                text: "Analyzing slow queries...".to_string(),
            },
        });

        // Tool call at 3000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 3,
            at_ms: 3000,
            kind: ThreadEventKind::ToolCall {
                item_id: "tool-db".to_string(),
                name: "query_index".to_string(),
                input: json!({"table": "users"}),
                status: ToolCallStatus::Executing,
                output: None,
            },
        });

        // While thread is working and tool is executing, grouped items show Running work log
        let running_items = proj.grouped_items();
        assert_eq!(running_items.len(), 2); // TurnUserPrompt + WorkLog
        if let TimelineItem::WorkLog {
            summary,
            entries,
            status,
            duration_ms,
            ..
        } = &running_items[1]
        {
            assert_eq!(*status, WorkLogStatus::Running);
            assert_eq!(entries.len(), 2);
            assert!(summary.starts_with("Thinking and executing tools"));
            assert_eq!(*duration_ms, Some(1000)); // 3000 - 2000
        } else {
            panic!("expected WorkLog item");
        }

        // Tool finishes at 5000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 4,
            at_ms: 5000,
            kind: ThreadEventKind::ToolCall {
                item_id: "tool-db".to_string(),
                name: "query_index".to_string(),
                input: json!({"table": "users"}),
                status: ToolCallStatus::Success,
                output: Some("12 indexes analyzed".to_string()),
            },
        });

        // Command run at 7000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 5,
            at_ms: 7000,
            kind: ThreadEventKind::CommandRun {
                item_id: "cmd-test".to_string(),
                command: "cargo test --test db".to_string(),
                cwd: "/repo".to_string(),
                exit_code: Some(0),
                output_tail: Some("test result: ok".to_string()),
            },
        });

        // File change at 8000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 6,
            at_ms: 8000,
            kind: ThreadEventKind::FileChange {
                item_id: "file-schema".to_string(),
                path: "src/schema.rs".to_string(),
                kind: FileChangeKind::Modified,
            },
        });

        // Assistant final text at 9000
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 7,
            at_ms: 9000,
            kind: ThreadEventKind::AssistantText {
                item_id: "msg-ans".to_string(),
                text: "Refactored queries and all tests pass.".to_string(),
            },
        });

        // Turn completed
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 8,
            at_ms: 9500,
            kind: ThreadEventKind::TurnCompleted {
                outcome: TurnOutcome::Success,
            },
        });

        // Settle thread to Idle
        proj.apply_event(&ThreadEvent {
            thread_id: ThreadId::from("thread-wl"),
            seq: 9,
            at_ms: 9600,
            kind: ThreadEventKind::TurnSettled {
                changed_files: vec!["src/schema.rs".to_string()],
            },
        });

        let completed_items = proj.grouped_items();
        // Should be: [TurnUserPrompt, WorkLog, AssistantText, ChangedFiles]
        assert_eq!(completed_items.len(), 4);

        if let TimelineItem::WorkLog {
            summary,
            entries,
            duration_ms,
            status,
            ..
        } = &completed_items[1]
        {
            assert_eq!(*status, WorkLogStatus::Completed);
            assert_eq!(entries.len(), 4); // Reasoning, ToolCall, CommandRun, FileChange
            assert_eq!(*duration_ms, Some(6000)); // 8000 - 2000
            assert_eq!(summary, "Worked for 6s (4 items)");
        } else {
            panic!("expected completed WorkLog");
        }

        if let TimelineItem::ChangedFiles { files, .. } = &completed_items[3] {
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, "src/schema.rs");
        } else {
            panic!("expected ChangedFiles item");
        }
    }

    #[test]
    fn changed_file_summary_parsing() {
        use pandamux_core::FileChangeKind;

        let stat1 = ChangedFileSummary::parse("src/main.rs (+14 -3)");
        assert_eq!(stat1.path, "src/main.rs");
        assert_eq!(stat1.additions, 14);
        assert_eq!(stat1.deletions, 3);
        assert_eq!(stat1.kind, FileChangeKind::Modified);

        let stat2 = ChangedFileSummary::parse("A\tsrc/new_module.rs");
        assert_eq!(stat2.path, "src/new_module.rs");
        assert_eq!(stat2.kind, FileChangeKind::Created);

        let stat3 = ChangedFileSummary::parse("D\tsrc/deprecated.rs");
        assert_eq!(stat3.path, "src/deprecated.rs");
        assert_eq!(stat3.kind, FileChangeKind::Deleted);
    }
}
