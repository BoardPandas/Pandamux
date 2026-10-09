use pandamux_orchestrator::{LegacyRecordKind, require_explicit_legacy_migration};

#[test]
fn unversioned_records_never_become_canonical_state_silently() {
    let fixtures = [
        (
            LegacyRecordKind::Run,
            include_str!("fixtures/legacy-run.json"),
            "legacy_run_requires_versioned_adapter",
            "Preserve the original JSON and migrate it through the versioned Phase 5 run adapter before dispatch.",
        ),
        (
            LegacyRecordKind::RunEvent,
            include_str!("fixtures/legacy-run-event.json"),
            "legacy_run_event_requires_versioned_adapter",
            "Preserve the original JSON and migrate it together with its legacy run through the versioned Phase 5 run adapter before replay.",
        ),
        (
            LegacyRecordKind::AgentDefinition,
            include_str!("fixtures/legacy-agent-definition.json"),
            "legacy_agent_requires_versioned_adapter",
            "Preserve the original definition and import it through the versioned agent compiler before activation.",
        ),
        (
            LegacyRecordKind::MemoryEntry,
            include_str!("fixtures/legacy-memory-entry.json"),
            "legacy_memory_requires_markdown_migration",
            "Preserve the original entry, discard its derived embedding, and migrate canonical text through the Phase 5 markdown-memory importer.",
        ),
        (
            LegacyRecordKind::ScheduleRecord,
            include_str!("fixtures/legacy-schedule-record.json"),
            "legacy_schedule_deferred_to_phase6",
            "Preserve the raw schedule for Phase 6 import; it must land disabled and Approve-first and may require target repair.",
        ),
    ];

    for (kind, raw, expected_code, expected_next_action) in fixtures {
        let error = require_explicit_legacy_migration(kind, raw)
            .expect_err("unversioned compatibility input must never be accepted directly");
        assert_eq!(error.code, expected_code);
        assert_eq!(error.kind, kind);
        assert!(!error.message.is_empty());
        assert_eq!(error.next_action, expected_next_action);
        assert_eq!(error.line, None);
        assert_eq!(error.column, None);
    }
}

#[test]
fn later_canonical_fields_cannot_reinterpret_frozen_v0_records() {
    let fixtures = [
        (
            LegacyRecordKind::Run,
            include_str!("fixtures/legacy-run.json"),
            "lastSeq",
        ),
        (
            LegacyRecordKind::RunEvent,
            include_str!("fixtures/legacy-run-event.json"),
            "eventId",
        ),
        (
            LegacyRecordKind::AgentDefinition,
            include_str!("fixtures/legacy-agent-definition.json"),
            "tier",
        ),
        (
            LegacyRecordKind::MemoryEntry,
            include_str!("fixtures/legacy-memory-entry.json"),
            "schemaVersion",
        ),
        (
            LegacyRecordKind::ScheduleRecord,
            include_str!("fixtures/legacy-schedule-record.json"),
            "orchestrationPolicy",
        ),
    ];

    for (kind, raw, future_field) in fixtures {
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("valid fixture");
        value
            .as_object_mut()
            .expect("legacy fixture must be an object")
            .insert(future_field.to_string(), serde_json::json!("future"));
        let changed = serde_json::to_string(&value).expect("serialize changed fixture");

        let error = require_explicit_legacy_migration(kind, &changed)
            .expect_err("a later canonical field must not become valid V0 input");
        assert_eq!(error.code, "legacy_record_malformed");
        assert_eq!(error.kind, kind);
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
