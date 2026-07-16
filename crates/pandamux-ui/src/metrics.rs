//! Measured terminal cell metrics.
//!
//! The terminal viewport paints a fixed-pitch grid: every layout decision
//! (rows/columns from pixels, hit tests, cursor placement) shares these
//! numbers. The cell width is measured from the font cosmic-text actually
//! resolves for [`theme::MONO_FONT`], so the grid matches what glyphon
//! renders on this machine; the cell height keeps the terminal's historical
//! line-height ratio so the look stays stable. Measurement is headless (no
//! GPU) and in logical pixels, so it is scale-factor independent. The legacy
//! constants remain as the fallback when measurement fails.

use crate::theme;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::{self, Paragraph as _, Text};
use iced::{Pixels, Size};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
}

impl CellMetrics {
    /// The process-wide metrics at the terminal font size, measured once.
    pub fn get() -> Self {
        static METRICS: OnceLock<CellMetrics> = OnceLock::new();
        *METRICS.get_or_init(|| Self::measure(theme::SIZE_TERMINAL))
    }

    /// Measure the monospace advance width at `font_size` logical pixels.
    pub fn measure(font_size: f32) -> Self {
        let height = (font_size * (theme::term::CELL_HEIGHT / theme::SIZE_TERMINAL)).round();
        const SAMPLE_LEN: usize = 64;
        let sample = "M".repeat(SAMPLE_LEN);
        let paragraph = Paragraph::with_text(Text {
            content: sample.as_str(),
            bounds: Size::INFINITE,
            size: Pixels(font_size),
            line_height: text::LineHeight::Absolute(Pixels(height)),
            font: theme::MONO_FONT,
            align_x: text::Alignment::Default,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        });
        let advance = paragraph.min_bounds().width / SAMPLE_LEN as f32;
        let width = if advance.is_finite() && advance >= 1.0 {
            advance
        } else {
            theme::term::CELL_WIDTH
        };
        Self { width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_metrics_are_finite_and_positive() {
        let metrics = CellMetrics::measure(theme::SIZE_TERMINAL);
        assert!(metrics.width.is_finite() && metrics.width >= 1.0);
        assert!(metrics.height.is_finite() && metrics.height >= theme::SIZE_TERMINAL);
    }

    #[test]
    fn height_keeps_the_historical_ratio() {
        let metrics = CellMetrics::measure(theme::SIZE_TERMINAL);
        assert_eq!(metrics.height, theme::term::CELL_HEIGHT);
    }
}
