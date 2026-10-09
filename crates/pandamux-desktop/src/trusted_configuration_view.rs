use gpui_kit::base::{Disableable as _, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::TrustedConfigurationProjection;

use crate::mutation_confirmation::render_mutation_confirmation;
use crate::theme::{Radii, Theme, Typography};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustedConfigurationIntent {
    Refresh,
    ApproveMutation(String),
    RejectMutation(String),
    ApproveCandidate,
    TrustProject,
    RevokeProject,
    RevertPersonal(String),
    RevertBundle(String),
}

pub type TrustedConfigurationHandler<V> =
    fn(&mut V, TrustedConfigurationIntent, &mut Window, &mut Context<V>);

pub fn source_accessibility_summary(state: &TrustedConfigurationProjection) -> String {
    state.source.as_ref().map_or_else(
        || "No instructions source configured".to_string(),
        |source| {
            format!(
                "Source {} at revision {}. Active source: {}. Active bundle: {}.",
                source.id,
                source.revision,
                source
                    .active_source_commit_sha
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "none".to_string()),
                source
                    .active_bundle_sha
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "none".to_string()),
            )
        },
    )
}

pub fn candidate_accessibility_summary(state: &TrustedConfigurationProjection) -> String {
    state.candidate.as_ref().map_or_else(
        || "No candidate is awaiting review".to_string(),
        |candidate| {
            format!(
                "Candidate {} at source {}. State: {:?}. Safety: {:?}. Activatable: {}. Validation issues: {}.",
                candidate.id,
                candidate.observed_source_commit_sha,
                candidate.state,
                candidate.safety,
                candidate.activatable,
                state
                    .candidate_diff
                    .as_ref()
                    .map_or(0, |diff| diff.issues.len()),
            )
        },
    )
}

pub fn candidate_approval_enabled(state: &TrustedConfigurationProjection) -> bool {
    state.candidate.as_ref().is_some_and(|candidate| {
        !state.is_loading
            && candidate.activatable
            && state.repository.is_some()
            && matches!(
                candidate.state,
                pandamux_core::InstructionCandidateState::AwaitingApproval
                    | pandamux_core::InstructionCandidateState::Approved
            )
    })
}

pub fn safe_diff_category_labels(value: &serde_json::Value) -> Vec<&'static str> {
    let mut labels = value
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .map(|key| match key.as_str() {
            "added" | "modified" | "deleted" | "files" => "Files",
            "agents" => "Agents",
            "access" | "accessMode" => "Access",
            "fullAccess" => "FullAccess",
            "tools" => "Tools",
            "environments" => "Environments",
            "capabilities" | "requiredCapabilities" => "Capabilities",
            "prompts" => "Prompts",
            "knowledge" => "Knowledge",
            "classes" => "Classes",
            "routing" => "Routing",
            "verification" => "Verification",
            "schedules" | "scheduledAgents" => "Schedules",
            "sourceIdentities" | "dependencies" => "Source identities",
            "trust" | "grants" => "Trust",
            "settings" => "Settings",
            "origins" => "Origins",
            _ => "Other reviewed changes",
        })
        .collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    labels
}

pub fn render_trusted_configuration<V: 'static>(
    state: &TrustedConfigurationProjection,
    theme: &Theme,
    on_intent: TrustedConfigurationHandler<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    let source_status = source_accessibility_summary(state);
    let project_status = state.repository.as_ref().map_or_else(
        || "No project repository is available for the active thread".to_string(),
        |repository| {
            format!(
                "Project {} repository revision {}. Trust status: {}.",
                repository.project_id,
                repository.revision,
                if state.project_trust.is_some() {
                    "trusted for the exact displayed content"
                } else {
                    "not trusted"
                }
            )
        },
    );
    let status_section = section(
        theme,
        "Instructions and last-good activation",
        source_status.clone(),
    );
    let candidate_panel = render_candidate_panel(state, theme, on_intent, cx);
    let project_panel = render_project_panel(state, theme, project_status.clone(), on_intent, cx);
    let confirmation_cards = state
        .confirmations
        .iter()
        .map(|proposal| {
            let mut proposal = proposal.clone();
            proposal.can_resolve &= !state.is_loading;
            render_mutation_confirmation(
                &proposal,
                theme,
                move |this, id, window, cx| {
                    on_intent(
                        this,
                        TrustedConfigurationIntent::ApproveMutation(id),
                        window,
                        cx,
                    );
                },
                move |this, id, window, cx| {
                    on_intent(
                        this,
                        TrustedConfigurationIntent::RejectMutation(id),
                        window,
                        cx,
                    );
                },
                cx,
            )
        })
        .collect::<Vec<_>>();
    let confirmation_panel = div()
        .v_flex()
        .gap_2()
        .child(section_title("Pending confirmations"))
        .when(state.confirmations.is_empty(), |this| {
            this.child("No pending or recent configuration confirmations")
        })
        .children(confirmation_cards)
        .into_any_element();
    let configuration_history = render_configuration_history(state, theme, on_intent, cx);
    let personal_history = render_personal_history(state, theme, on_intent, cx);

    div()
        .id("trusted-configuration-panel")
        .role(Role::Group)
        .aria_label(format!(
            "Trusted configuration. {source_status} {project_status}"
        ))
        .v_flex()
        .gap_4()
        .p_1()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(Typography::TITLE_SIZE)
                        .child("Trusted configuration status"),
                )
                .child(
                    Button::new("trusted-configuration-refresh")
                        .ghost()
                        .label(if state.is_loading {
                            "Refreshing"
                        } else {
                            "Refresh"
                        })
                        .accessibility_label("Refresh trusted configuration status and history")
                        .disabled(state.is_loading)
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_intent(this, TrustedConfigurationIntent::Refresh, window, cx);
                        })),
                ),
        )
        .when_some(state.status_message.as_ref(), |this, message| {
            this.child(
                div()
                    .id("trusted-configuration-status-message")
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .p_2()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel2)
                    .child(message.clone()),
            )
        })
        .child(status_section)
        .child(candidate_panel)
        .child(project_panel)
        .child(confirmation_panel)
        .child(configuration_history)
        .child(personal_history)
        .into_any_element()
}

fn render_candidate_panel<V: 'static>(
    state: &TrustedConfigurationProjection,
    theme: &Theme,
    on_intent: TrustedConfigurationHandler<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    let candidate_summary = candidate_accessibility_summary(state);
    let issues = state
        .candidate_diff
        .as_ref()
        .map(|diff| diff.issues.clone())
        .unwrap_or_default();
    div()
        .id("instructions-candidate-panel")
        .role(Role::Group)
        .aria_label(format!("Instructions candidate diff. {candidate_summary}"))
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .child(section_title("Candidate diff and validation"))
        .child(candidate_summary)
        .when_some(state.candidate_diff.as_ref(), |this, diff| {
            let file_categories = safe_diff_category_labels(&diff.file_diff);
            let behavioral_categories = safe_diff_category_labels(&diff.behavioral_diff);
            this.child(format!(
                "Diff digest: {}. Validation issues: {}. File changes: {}. Behavioral changes: {}.",
                diff.diff_digest,
                diff.issues.len(),
                if file_categories.is_empty() {
                    "none".to_string()
                } else {
                    file_categories.join(", ")
                },
                if behavioral_categories.is_empty() {
                    "none".to_string()
                } else {
                    behavioral_categories.join(", ")
                },
            ))
        })
        .children(issues.into_iter().map(|issue| {
            div()
                .child(format!(
                    "Issue: {}",
                    issue
                        .get("safeReason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Candidate validation issue")
                ))
                .into_any_element()
        }))
        .child(
            Button::new("approve-instructions-candidate")
                .primary()
                .label("Approve exact candidate")
                .accessibility_label("Approve the exact displayed instructions candidate once")
                .disabled(!candidate_approval_enabled(state))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    on_intent(
                        this,
                        TrustedConfigurationIntent::ApproveCandidate,
                        window,
                        cx,
                    );
                })),
        )
        .into_any_element()
}

fn render_project_panel<V: 'static>(
    state: &TrustedConfigurationProjection,
    theme: &Theme,
    project_status: String,
    on_intent: TrustedConfigurationHandler<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    let has_project = state.repository.is_some() && state.project_content_sha256.is_some();
    let trusted = state.project_trust.is_some();
    div()
        .id("project-trust-panel")
        .role(Role::Group)
        .aria_label(format!("Project layer trust. {project_status}"))
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .child(section_title("Project layer trust and diff"))
        .child(project_status)
        .when(state.project_visible_but_inactive, |this| {
            this.child(
                "Project configuration is visible but inactive until exact trust is approved",
            )
        })
        .when_some(state.project_diff.as_ref(), |this, diff| {
            let categories = safe_diff_category_labels(&diff.behavioral_diff);
            this.child(format!(
                "Project content: {}. Validation issues: {}. Behavioral changes: {}.",
                diff.project_content_sha256,
                diff.issues.len(),
                if categories.is_empty() {
                    "none".to_string()
                } else {
                    categories.join(", ")
                },
            ))
        })
        .children(
            state
                .project_diff
                .iter()
                .flat_map(|diff| diff.issues.iter())
                .map(|issue| {
                    div().child(format!(
                        "Issue: {}",
                        issue
                            .get("safeReason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Project validation issue")
                    ))
                }),
        )
        .child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    Button::new("trust-project-layer")
                        .primary()
                        .label("Propose exact trust")
                        .accessibility_label(
                            "Propose trust for the exact displayed project content",
                        )
                        .disabled(state.is_loading || !has_project || trusted)
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_intent(this, TrustedConfigurationIntent::TrustProject, window, cx);
                        })),
                )
                .child(
                    Button::new("revoke-project-layer")
                        .ghost()
                        .label("Propose trust revocation")
                        .accessibility_label("Propose revocation of active project layer trust")
                        .disabled(state.is_loading || !trusted)
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_intent(this, TrustedConfigurationIntent::RevokeProject, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

fn render_configuration_history<V: 'static>(
    state: &TrustedConfigurationProjection,
    theme: &Theme,
    on_intent: TrustedConfigurationHandler<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .id("configuration-history-panel")
        .role(Role::Group)
        .aria_label("Activation and configuration history")
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .child(section_title("Activation and configuration history"))
        .when(state.configuration_history.is_empty(), |this| {
            this.child("No configuration audit entries")
        })
        .children(state.configuration_history.iter().map(|entry| {
            let bundle = entry.bundle_sha.as_ref().map(ToString::to_string);
            let callback_bundle = bundle.clone();
            let is_active = entry.bundle_sha.as_ref()
                == state
                    .source
                    .as_ref()
                    .and_then(|source| source.active_bundle_sha.as_ref());
            div()
                .v_flex()
                .gap_1()
                .child(format!(
                    "{} by {} at {}",
                    entry.kind, entry.actor, entry.created_at_ms
                ))
                .when_some(bundle, |this, bundle| {
                    this.child(
                        Button::new(SharedString::from(format!("revert-bundle-{}", entry.id)))
                            .ghost()
                            .label(format!("Propose bundle revert {bundle}"))
                            .accessibility_label(format!(
                                "Propose reactivation of retained bundle {bundle}"
                            ))
                            .disabled(state.is_loading || is_active)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                if let Some(bundle) = &callback_bundle {
                                    on_intent(
                                        this,
                                        TrustedConfigurationIntent::RevertBundle(bundle.clone()),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                })
                .into_any_element()
        }))
        .into_any_element()
}

fn render_personal_history<V: 'static>(
    state: &TrustedConfigurationProjection,
    theme: &Theme,
    on_intent: TrustedConfigurationHandler<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .id("personal-configuration-history-panel")
        .role(Role::Group)
        .aria_label("Personal configuration history and forward-only revert actions")
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .child(section_title("Personal configuration history"))
        .when(state.personal_history.is_empty(), |this| {
            this.child("No accepted personal configuration edits")
        })
        .children(state.personal_history.iter().map(|entry| {
            let commit = entry.source_commit_sha.to_string();
            let callback_commit = commit.clone();
            let is_active =
                state.personal_head_source_commit_sha.as_ref() == Some(&entry.source_commit_sha);
            div()
                .v_flex()
                .gap_1()
                .child(format!(
                    "Commit {} accepted at {} by {}",
                    entry.source_commit_sha, entry.accepted_at_ms, entry.caller
                ))
                .child(
                    Button::new(SharedString::from(format!(
                        "revert-personal-{}",
                        entry.source_commit_sha
                    )))
                    .ghost()
                    .label("Propose forward-only revert")
                    .accessibility_label(format!(
                        "Propose a new personal configuration commit matching {commit}"
                    ))
                    .disabled(state.is_loading || is_active)
                    .on_click(cx.listener(
                        move |this, _event, window, cx| {
                            on_intent(
                                this,
                                TrustedConfigurationIntent::RevertPersonal(callback_commit.clone()),
                                window,
                                cx,
                            );
                        },
                    )),
                )
                .into_any_element()
        }))
        .into_any_element()
}

fn section(theme: &Theme, title: &str, body: String) -> AnyElement {
    div()
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .child(section_title(title))
        .child(body)
        .into_any_element()
}

fn section_title(title: &str) -> AnyElement {
    div()
        .font_weight(FontWeight::BOLD)
        .text_size(Typography::BODY_SIZE)
        .child(title.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use pandamux_core::{
        BundleSha, ConfigurationRevision, ConfigurationUpdateSafety, ContentSha256,
        InstructionCandidateId, InstructionCandidateRecord, InstructionCandidateState,
        InstructionSourceId, InstructionsSourceRecord, InstructionsUpdatePolicy, SourceCommitSha,
        TrackingRef,
    };
    use pandamux_protocol::InstructionsDiffResult;

    #[test]
    fn blocked_candidate_remains_named_beside_last_good_status() {
        let active_source = SourceCommitSha::parse("a".repeat(40)).expect("source SHA");
        let active_bundle = BundleSha::parse("b".repeat(64)).expect("bundle SHA");
        let candidate_source = SourceCommitSha::parse("c".repeat(40)).expect("candidate SHA");
        let digest = ContentSha256::parse("d".repeat(64)).expect("diff digest");
        let candidate_id = InstructionCandidateId::new("candidate-blocked");
        let state = TrustedConfigurationProjection {
            source: Some(InstructionsSourceRecord {
                id: InstructionSourceId::new("source-one"),
                canonical_remote_identity: "example.invalid/private".to_string(),
                tracking: TrackingRef::Branch("refs/heads/main".to_string()),
                policy: InstructionsUpdatePolicy::ApproveEach,
                require_signed_commit: true,
                revision: ConfigurationRevision::new(4).expect("revision"),
                last_checked_at_ms: Some(10),
                last_error_reason: None,
                active_source_commit_sha: Some(active_source.clone()),
                active_bundle_sha: Some(active_bundle.clone()),
            }),
            candidate: Some(InstructionCandidateRecord {
                id: candidate_id.clone(),
                source_id: InstructionSourceId::new("source-one"),
                observed_source_commit_sha: candidate_source.clone(),
                observed_tag_object_sha: None,
                state: InstructionCandidateState::Blocked,
                safety: ConfigurationUpdateSafety::AccessWidening,
                diff_digest: digest.clone(),
                activatable: false,
                created_at_ms: 20,
                updated_at_ms: 20,
            }),
            candidate_diff: Some(InstructionsDiffResult {
                candidate_id,
                observed_source_commit_sha: candidate_source,
                diff_digest: digest,
                safety: ConfigurationUpdateSafety::AccessWidening,
                file_diff: serde_json::json!({}),
                behavioral_diff: serde_json::json!({}),
                issues: vec![serde_json::json!({ "safeReason": "Signature is invalid" })],
                activatable: false,
            }),
            ..TrustedConfigurationProjection::default()
        };

        let source = source_accessibility_summary(&state);
        assert!(source.contains(active_source.as_str()));
        assert!(source.contains(active_bundle.as_str()));
        let candidate = candidate_accessibility_summary(&state);
        for expected in [
            "candidate-blocked",
            "Blocked",
            "AccessWidening",
            "Activatable: false",
            "Validation issues: 1",
        ] {
            assert!(candidate.contains(expected), "missing {expected}");
        }
        assert!(!candidate_approval_enabled(&state));
    }

    #[test]
    fn diff_categories_never_echo_unknown_keys_or_values() {
        let labels = safe_diff_category_labels(&serde_json::json!({
            "agents": { "changed": true },
            "secretTokenName": "secret-value-sentinel",
        }));
        assert_eq!(labels, vec!["Agents", "Other reviewed changes"]);
        let rendered = labels.join(", ");
        assert!(!rendered.contains("secretTokenName"));
        assert!(!rendered.contains("secret-value-sentinel"));
    }
}
