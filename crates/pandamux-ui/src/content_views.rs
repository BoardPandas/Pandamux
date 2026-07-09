//! Renderers for non-terminal surface content (markdown / diff).
//!
//! Both render inside the fixed-dark pane box (like the terminal viewport), so
//! they use theme-independent light-on-dark colors and never `palette.ov`
//! (which flips with the chrome theme and would vanish on the dark pane). Only
//! the accent (theme-independent) is taken from the palette.
//!
//! The markdown renderer is a pragmatic line-based pass (headings, bullets,
//! blockquotes, fenced code, rules, paragraphs) rather than a full CommonMark
//! tree; it covers the orchestrator dashboard and doc surfaces without the
//! stateful `iced::widget::markdown` link-handling machinery.

use crate::theme::{self, Palette};
use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Color, Element, Length, Padding};

/// Heading / emphasis color on the dark pane (near-white).
const HEADING: Color = Color::from_rgb8(0xe8, 0xee, 0xee);
/// Diff removal red (soft, readable on dark).
const DIFF_REMOVE: Color = Color::from_rgb8(0xe0, 0x6c, 0x75);

/// Render markdown text into a scrollable view.
pub fn markdown_view<'a, Message: 'a>(content: &str, palette: Palette) -> Element<'a, Message> {
    let mut body = column![].spacing(4).width(Length::Fill);
    let mut in_code = false;

    for raw in content.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }

        if in_code {
            body = body.push(code_line(line));
            continue;
        }

        if line.trim().is_empty() {
            body = body.push(Space::new().height(Length::Fixed(6.0)));
        } else if let Some(level) = heading_level(trimmed) {
            body = body.push(heading(trimmed[level..].trim_start(), level, palette));
        } else if is_rule(trimmed) {
            body = body.push(rule());
        } else if let Some(rest) = bullet_rest(trimmed) {
            body = body.push(bullet(rest));
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            body = body.push(
                text(rest.to_string())
                    .size(theme::SIZE_BODY)
                    .color(theme::term::DIM),
            );
        } else {
            body = body.push(
                text(line.to_string())
                    .size(theme::SIZE_BODY)
                    .color(theme::term::TEXT),
            );
        }
    }

    scroll(body.into())
}

/// Render a unified diff into a scrollable, monospace, per-line-colored view.
pub fn diff_view<'a, Message: 'a>(content: &str, _palette: Palette) -> Element<'a, Message> {
    let mut body = column![].spacing(0).width(Length::Fill);
    for raw in content.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let color = diff_line_color(line);
        body = body.push(
            text(line.to_string())
                .size(theme::SIZE_TERMINAL)
                .font(theme::MONO_FONT)
                .color(color),
        );
    }
    scroll(body.into())
}

fn scroll<'a, Message: 'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(scrollable(content).width(Length::Fill).height(Length::Fill))
        .padding(Padding::from([10.0, 14.0]))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

fn heading<'a, Message: 'a>(content: &str, level: usize, palette: Palette) -> Element<'a, Message> {
    let size = match level {
        1 => 18.0,
        2 => 15.5,
        3 => 13.5,
        _ => 12.5,
    };
    let color = if level == 1 { palette.accent } else { HEADING };
    text(content.to_string())
        .size(size)
        .font(theme::ui(iced::font::Weight::Semibold))
        .color(color)
        .into()
}

fn bullet_rest(line: &str) -> Option<&str> {
    line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
}

fn bullet<'a, Message: 'a>(rest: &str) -> Element<'a, Message> {
    row![
        text("\u{2022}")
            .size(theme::SIZE_BODY)
            .color(theme::term::DIM),
        text(rest.to_string())
            .size(theme::SIZE_BODY)
            .color(theme::term::TEXT),
    ]
    .spacing(8)
    .padding(Padding {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 6.0,
    })
    .into()
}

fn code_line<'a, Message: 'a>(line: &str) -> Element<'a, Message> {
    container(
        text(line.to_string())
            .size(theme::SIZE_TERMINAL)
            .font(theme::MONO_FONT)
            .color(theme::term::SUCCESS),
    )
    .width(Length::Fill)
    .padding(Padding::from([1.0, 6.0]))
    .style(|_theme| container::Style {
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.04).into()),
        ..Default::default()
    })
    .into()
}

fn rule<'a, Message: 'a>() -> Element<'a, Message> {
    container(Space::new().height(Length::Fixed(1.0)).width(Length::Fill))
        .padding(Padding::from([6.0, 0.0]))
        .style(|_theme| container::Style {
            background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.10).into()),
            ..Default::default()
        })
        .into()
}

fn is_rule(line: &str) -> bool {
    matches!(line, "---" | "***" | "___")
}

fn diff_line_color(line: &str) -> Color {
    if line.starts_with("@@") {
        theme::term::GOLD
    } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
        theme::term::DIM
    } else if line.starts_with('+') {
        theme::term::SUCCESS
    } else if line.starts_with('-') {
        DIFF_REMOVE
    } else {
        theme::term::TEXT
    }
}
