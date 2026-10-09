use pandamux_orchestrator::{LegacyRecordKind, require_explicit_legacy_migration};

#[test]
fn unversioned_records_never_become_canonical_state_silently() {
    let fixtures = [
        (
            LegacyRecordKind::Run,
            include_str!("fixtures/legacy-run.json"),
            "legacy_run_requires_versioned_adapter",
        ),
        (
            LegacyRecordKind::AgentDefinition,
            include_str!("fixtures/legacy-agent-definition.json"),
            "legacy_agent_requires_versioned_adapter",
        ),
        (
            LegacyRecordKind::MemoryEntry,
            include_str!("fixtures/legacy-memory-entry.json"),
            "legacy_memory_requires_markdown_migration",
        ),
        (
            LegacyRecordKind::ScheduleRecord,
            include_str!("fixtures/legacy-schedule-record.json"),
            "legacy_schedule_deferred_to_phase6",
        ),
    ];

    for (kind, raw, expected_code) in fixtures {
        let error = require_explicit_legacy_migration(kind, raw)
            .expect_err("unversioned compatibility input must never be accepted directly");
        assert_eq!(error.code, expected_code);
        assert_eq!(error.kind, kind);
        assert!(!error.message.is_empty());
        assert!(!error.next_action.is_empty());
        assert_eq!(error.line, None);
        assert_eq!(error.column, None);
    }
}

#[test]
fn malformed_legacy_json_reports_location_without_echoing_payload() {
    let raw = r#"{"id":"run-legacy", "kind": }"#;
    let error = require_explicit_legacy_migration(LegacyRecordKind::Run, raw)
        .expect_err("malformed legacy JSON must fail closed");

    assert_eq!(error.code, "legacy_record_malformed");
    assert!(error.line.is_some());
    assert!(error.column.is_some());
    assert!(!error.to_string().contains(raw));
}
