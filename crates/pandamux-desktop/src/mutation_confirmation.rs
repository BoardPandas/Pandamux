use gpui_kit::base::{Disableable as _, StyledExt as _, TestSupportExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::MutationConfirmationProjection;

use crate::theme::{Radii, Theme, Typography};

/// A terminal confirmation action that remains visibly disabled and reports
/// the same state through AccessKit.
///
/// GPUI Kit's disabled button is correctly inert and leaves keyboard focus,
/// but its current public API does not expose AccessKit's disabled property.
/// Wrapping the presentational button lets native accessibility clients see
/// the control as disabled without duplicating any action handler.
struct DisabledConfirmationAction {
    element: Stateful<Div>,
}

impl DisabledConfirmationAction {
    fn new(
        id: SharedString,
        label: &'static str,
        accessibility_label: String,
        primary: bool,
    ) -> Self {
        let visual_id = SharedString::from(format!("{id}-visual"));
        let button = Button::new(visual_id)
            .role(None::<Role>)
            .label(label)
            .disabled(true);
        let button = if primary {
            button.primary()
        } else {
            button.ghost()
        };

        Self {
            element: div()
                .id(id)
                .flex_shrink_0()
                .role(Role::Button)
                .aria_label(accessibility_label)
                .child(button),
        }
    }
}

impl IntoElement for DisabledConfirmationAction {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DisabledConfirmationAction {
    type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Div> as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.element)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.element.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.element
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }

    fn a11y_role(&self) -> Option<Role> {
        self.element.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.element.write_a11y_info(node);
        node.set_disabled();
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        Element::a11y_synthetic_children(&mut self.element, prepaint, builder);
    }
}

impl InteractiveElement for DisabledConfirmationAction {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

fn disabled_confirmation_action(
    id: SharedString,
    label: &'static str,
    accessibility_label: String,
    primary: bool,
) -> impl IntoElement {
    DisabledConfirmationAction::new(id, label, accessibility_label, primary).test_support()
}

pub fn confirmation_accessibility_summary(proposal: &MutationConfirmationProjection) -> String {
    format!(
        "Configuration confirmation. Status: {}. Caller: {}, {}. Tool: {}. Environment: {}. Target: {}. Expected revision: {}. Values are redacted. {} changed sections, {} affected scope entries, {} access changes, {} FullAccess changes, {} budget changes, {} tracking changes, {} trust changes, {} issues. Expires at {}.",
        proposal.status_label,
        proposal.caller_name,
        proposal.caller_role,
        proposal.tool_name,
        proposal.environment_id,
        proposal.target_label,
        proposal.expected_revision,
        proposal.diff.changed_sections.len(),
        proposal.affected_scopes.len(),
        proposal.access_changes.len(),
        proposal.full_access_changes.len(),
        proposal.budget_changes.len(),
        proposal.tracking_changes.len(),
        proposal.trust_changes.len(),
        proposal.issues.len(),
        proposal.expires_at_ms,
    )
}

pub fn render_mutation_confirmation<V: 'static>(
    proposal: &MutationConfirmationProjection,
    theme: &Theme,
    on_approve: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    on_reject: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    let proposal_id = proposal.proposal_id.clone();
    let approve_id = proposal_id.clone();
    let reject_id = proposal_id.clone();
    let can_resolve = proposal.can_resolve;
    let summary = confirmation_accessibility_summary(proposal);
    let detail_rows = proposal
        .affected_scopes
        .iter()
        .chain(proposal.access_changes.iter())
        .chain(proposal.full_access_changes.iter())
        .chain(proposal.budget_changes.iter())
        .chain(proposal.tracking_changes.iter())
        .chain(proposal.trust_changes.iter())
        .chain(proposal.issues.iter())
        .cloned()
        .collect::<Vec<_>>();

    div()
        .id(SharedString::from(format!("mutation-card-{proposal_id}")))
        .tab_group()
        .role(Role::Group)
        .aria_label(summary)
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(Radii::ROW)
        .border_1()
        .border_color(theme.accent.color())
        .bg(theme.chrome.panel2)
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_size(Typography::BODY_SIZE)
                .child("Review configuration change"),
        )
        .child(format!("Status: {}", proposal.status_label))
        .child(format!(
            "Caller: {} ({})",
            proposal.caller_name, proposal.caller_role
        ))
        .child(format!("Tool: {}", proposal.tool_name))
        .child(format!("Environment: {}", proposal.environment_id))
        .child(format!("Target: {}", proposal.target_label))
        .child(format!("Expected revision: {}", proposal.expected_revision))
        .child(format!(
            "Diff: {} changed section(s), values redacted",
            proposal.diff.changed_sections.len()
        ))
        .children(
            proposal
                .diff
                .changed_sections
                .iter()
                .map(|section| div().child(format!("Changed: {section}"))),
        )
        .children(detail_rows.into_iter().map(|detail| div().child(detail)))
        .child(format!("Expires at: {}", proposal.expires_at_ms))
        .child(
            div()
                .h_flex()
                .gap_2()
                .when(can_resolve, |actions| {
                    actions
                        .child(
                            Button::new(SharedString::from(format!("approve-{proposal_id}")))
                                .primary()
                                .label("Approve once")
                                .accessibility_label(format!(
                                    "Approve configuration proposal {proposal_id} once"
                                ))
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_approve(this, approve_id.clone(), window, cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("reject-{proposal_id}")))
                                .ghost()
                                .label("Reject")
                                .accessibility_label(format!(
                                    "Reject configuration proposal {proposal_id} without changes"
                                ))
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_reject(this, reject_id.clone(), window, cx);
                                })),
                        )
                })
                .when(!can_resolve, |actions| {
                    actions
                        .child(disabled_confirmation_action(
                            SharedString::from(format!("approve-{proposal_id}")),
                            "Approve once",
                            format!("Approve configuration proposal {proposal_id} once"),
                            true,
                        ))
                        .child(disabled_confirmation_action(
                            SharedString::from(format!("reject-{proposal_id}")),
                            "Reject",
                            format!("Reject configuration proposal {proposal_id} without changes"),
                            false,
                        ))
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt as _;
    use pandamux_client::SafeMutationDiff;

    struct ConfirmationHarness {
        proposal: MutationConfirmationProjection,
        approvals: usize,
        rejections: usize,
    }

    impl Render for ConfirmationHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            render_mutation_confirmation(
                &self.proposal,
                &Theme::dark(crate::theme::AccentColor::Teal),
                |this, _, _, _| this.approvals += 1,
                |this, _, _, _| this.rejections += 1,
                cx,
            )
        }
    }

    fn projection(can_resolve: bool, status: &str) -> MutationConfirmationProjection {
        MutationConfirmationProjection {
            proposal_id: "proposal-one".to_string(),
            caller_name: "Local assistant".to_string(),
            caller_role: "Authenticated MCP adapter".to_string(),
            tool_name: "propose_agent_access".to_string(),
            environment_id: "environment-local".to_string(),
            target_label: "Trusted settings".to_string(),
            expected_revision: 4,
            diff: SafeMutationDiff {
                old_digest: "a".repeat(64),
                new_digest: "b".repeat(64),
                changed_sections: vec!["Trusted configuration".to_string()],
                values_redacted: true,
            },
            affected_scopes: vec!["Agent agent-one".to_string()],
            access_changes: vec!["Enable agent".to_string()],
            full_access_changes: vec!["Grant FullAccess".to_string()],
            budget_changes: vec!["No budget change".to_string()],
            tracking_changes: vec!["Change tracked branch".to_string()],
            trust_changes: vec!["Trust exact project content".to_string()],
            issues: vec!["Review exact scope".to_string()],
            created_at_ms: 1,
            expires_at_ms: 2,
            status_label: status.to_string(),
            can_resolve,
        }
    }

    #[test]
    fn accessible_summary_names_status_and_every_review_dimension() {
        let summary =
            confirmation_accessibility_summary(&projection(true, "Pending local confirmation"));
        for expected in [
            "Pending local confirmation",
            "Local assistant",
            "Authenticated MCP adapter",
            "propose_agent_access",
            "environment-local",
            "Trusted settings",
            "Expected revision: 4",
            "1 changed sections",
            "1 affected scope entries",
            "1 access changes",
            "1 FullAccess changes",
            "1 budget changes",
            "1 tracking changes",
            "1 trust changes",
            "1 issues",
            "Expires at 2",
        ] {
            assert!(summary.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn terminal_cards_are_explicitly_not_resolvable() {
        let expired = projection(false, "Expired without changes");
        let disconnected = projection(false, "Cancelled after caller disconnected");
        assert!(!expired.can_resolve);
        assert!(!disconnected.can_resolve);
        assert_ne!(expired.status_label, disconnected.status_label);
    }

    #[gpui_kit::test]
    fn confirmation_actions_are_tab_reachable_and_keyboard_activatable(cx: &mut TestAppContext) {
        fn activate(cx: &mut VisualTestContext, key: &str) {
            let keystroke = Keystroke::parse(key).expect("valid test keystroke");
            cx.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(KeyUpEvent { keystroke });
        }

        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| ConfirmationHarness {
            proposal: projection(true, "Pending local confirmation"),
            approvals: 0,
            rejections: 0,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.update(|window, cx| {
            window.focus_next(cx);
            window.draw(cx).clear(cx);
            assert_eq!(window.find("approve-proposal-one").focused(), Some(true));
        });
        activate(cx, "enter");
        cx.update(|_, cx| assert_eq!(view.read(cx).approvals, 1));

        cx.update(|window, cx| {
            window.focus_next(cx);
            window.draw(cx).clear(cx);
            assert_eq!(window.find("reject-proposal-one").focused(), Some(true));
        });
        activate(cx, "space");
        cx.update(|_, cx| assert_eq!(view.read(cx).rejections, 1));
    }

    #[gpui_kit::test]
    fn terminal_confirmation_actions_are_accessibly_disabled(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| ConfirmationHarness {
            proposal: projection(false, "Approved and applied"),
            approvals: 0,
            rejections: 0,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.update(|window, _| {
            let approve = window.find("approve-proposal-one");
            assert_eq!(approve.role(), Some(Role::Button));
            assert_eq!(approve.disabled(), Some(true));
            assert_eq!(approve.focused(), None);
            assert_eq!(
                approve.label(),
                Some("Approve configuration proposal proposal-one once")
            );

            let reject = window.find("reject-proposal-one");
            assert_eq!(reject.role(), Some(Role::Button));
            assert_eq!(reject.disabled(), Some(true));
            assert_eq!(reject.focused(), None);
            assert_eq!(
                reject.label(),
                Some("Reject configuration proposal proposal-one without changes")
            );
        });
        cx.update(|_, cx| {
            assert_eq!(view.read(cx).approvals, 0);
            assert_eq!(view.read(cx).rejections, 0);
        });
    }
}
