//! Line-glyph icon set for the native shell chrome.
//!
//! The design calls for crisp, thin-stroke line icons (no icon font, no glyph
//! placeholders): every icon here is drawn as a small [`canvas::Program`] whose
//! paths are stroked (never filled) at roughly a 1.3px width. [`icon`] is the
//! single entry point other chrome modules should reach for; [`Icon`] enumerates
//! the glyphs currently in use.
//!
//! All paths are authored in a normalized `[0, 1]` box (occasionally spilling a
//! little outside it, e.g. the zoom arrow ticks) that [`IconProgram::draw`] maps
//! onto the canvas bounds, inset by a small padding, so one glyph definition
//! renders crisply at any requested size.

use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};

/// The set of chrome icons drawn as line glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Search,
    Bell,
    Settings,
    Sessions,
    Palette,
    Plus,
    Git,
    Terminal,
    SplitRight,
    SplitDown,
    ZoomIn,
    ZoomOut,
    Close,
    Minimize,
    Maximize,
    Folder,
    Home,
    Drive,
}

/// A line icon drawn into a `size` x `size` canvas in `color`. Stroke ~1.3px.
pub fn icon<'a, Message: 'a>(kind: Icon, size: f32, color: Color) -> Element<'a, Message> {
    canvas::Canvas::new(IconProgram { kind, color })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// The `canvas::Program` backing [`icon`]. Holds nothing but the glyph and its
/// color; the geometry is fully determined by the bounds it is drawn into.
struct IconProgram {
    kind: Icon,
    color: Color,
}

impl<Message> canvas::Program<Message> for IconProgram {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Inset the drawing box a little so strokes never touch the canvas
        // edge, then map normalized (0..1) coordinates onto it.
        let pad = bounds.width * 0.18;
        let grid = Grid {
            pad,
            inner_w: bounds.width - pad * 2.0,
            inner_h: bounds.height - pad * 2.0,
        };

        let stroke_width = (bounds.width / 16.0 * 1.3).max(1.0);
        let stroke = canvas::Stroke::default()
            .with_color(self.color)
            .with_width(stroke_width)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);

        for path in paths_for(self.kind, &grid) {
            frame.stroke(&path, stroke);
        }

        vec![frame.into_geometry()]
    }
}

/// Maps normalized `[0, 1]` coordinates (occasionally spilling a little
/// outside that range, e.g. the zoom arrow ticks) onto the padded canvas box.
struct Grid {
    pad: f32,
    inner_w: f32,
    inner_h: f32,
}

impl Grid {
    fn p(&self, nx: f32, ny: f32) -> Point {
        Point::new(self.pad + nx * self.inner_w, self.pad + ny * self.inner_h)
    }

    /// A normalized radius, scaled against the box width (icons are square).
    fn r(&self, nr: f32) -> f32 {
        nr * self.inner_w
    }

    /// A normalized width/height pair, for rectangles anchored at `p(nx, ny)`.
    fn size(&self, nw: f32, nh: f32) -> iced::Size {
        iced::Size::new(nw * self.inner_w, nh * self.inner_h)
    }
}

/// Builds the stroked paths for `kind` on `grid`. Kept as a free function
/// (rather than a method) since it only needs the grid, not `self`.
fn paths_for(kind: Icon, grid: &Grid) -> Vec<canvas::Path> {
    match kind {
        Icon::Search => vec![
            canvas::Path::circle(grid.p(0.4, 0.4), grid.r(0.28)),
            canvas::Path::line(grid.p(0.6, 0.6), grid.p(0.95, 0.95)),
        ],
        Icon::Bell => vec![
            canvas::Path::new(|b| {
                b.move_to(grid.p(0.2, 0.55));
                b.quadratic_curve_to(grid.p(0.2, 0.08), grid.p(0.5, 0.05));
                b.quadratic_curve_to(grid.p(0.8, 0.08), grid.p(0.8, 0.55));
                b.line_to(grid.p(0.88, 0.62));
                b.line_to(grid.p(0.12, 0.62));
                b.close();
            }),
            canvas::Path::line(grid.p(0.5, 0.62), grid.p(0.5, 0.72)),
            canvas::Path::circle(grid.p(0.5, 0.8), grid.r(0.04)),
        ],
        Icon::Settings => vec![
            canvas::Path::line(grid.p(0.0, 0.2), grid.p(1.0, 0.2)),
            canvas::Path::circle(grid.p(0.35, 0.2), grid.r(0.06)),
            canvas::Path::line(grid.p(0.0, 0.5), grid.p(1.0, 0.5)),
            canvas::Path::circle(grid.p(0.65, 0.5), grid.r(0.06)),
            canvas::Path::line(grid.p(0.0, 0.8), grid.p(1.0, 0.8)),
            canvas::Path::circle(grid.p(0.45, 0.8), grid.r(0.06)),
        ],
        Icon::Sessions => vec![
            canvas::Path::line(grid.p(0.05, 0.2), grid.p(0.95, 0.2)),
            canvas::Path::line(grid.p(0.05, 0.5), grid.p(0.95, 0.5)),
            canvas::Path::line(grid.p(0.05, 0.8), grid.p(0.95, 0.8)),
        ],
        Icon::Palette => vec![
            canvas::Path::rectangle(grid.p(0.05, 0.05), grid.size(0.38, 0.38)),
            canvas::Path::rectangle(grid.p(0.57, 0.05), grid.size(0.38, 0.38)),
            canvas::Path::rectangle(grid.p(0.05, 0.57), grid.size(0.38, 0.38)),
            canvas::Path::rectangle(grid.p(0.57, 0.57), grid.size(0.38, 0.38)),
        ],
        Icon::Plus => vec![
            canvas::Path::line(grid.p(0.5, 0.0), grid.p(0.5, 1.0)),
            canvas::Path::line(grid.p(0.0, 0.5), grid.p(1.0, 0.5)),
        ],
        Icon::Git => vec![
            canvas::Path::line(grid.p(0.25, 0.12), grid.p(0.25, 0.88)),
            canvas::Path::new(|b| {
                b.move_to(grid.p(0.25, 0.5));
                b.quadratic_curve_to(grid.p(0.5, 0.55), grid.p(0.75, 0.55));
            }),
            canvas::Path::circle(grid.p(0.25, 0.15), grid.r(0.09)),
            canvas::Path::circle(grid.p(0.25, 0.85), grid.r(0.09)),
            canvas::Path::circle(grid.p(0.75, 0.55), grid.r(0.09)),
        ],
        Icon::Terminal => vec![
            canvas::Path::rounded_rectangle(
                grid.p(0.0, 0.0),
                grid.size(1.0, 1.0),
                iced::border::Radius::from(2.0),
            ),
            canvas::Path::new(|b| {
                b.move_to(grid.p(0.18, 0.35));
                b.line_to(grid.p(0.4, 0.5));
                b.line_to(grid.p(0.18, 0.65));
            }),
            canvas::Path::line(grid.p(0.48, 0.65), grid.p(0.7, 0.65)),
        ],
        Icon::SplitRight => vec![
            canvas::Path::rounded_rectangle(
                grid.p(0.0, 0.0),
                grid.size(1.0, 1.0),
                iced::border::Radius::from(2.0),
            ),
            canvas::Path::line(grid.p(0.5, 0.1), grid.p(0.5, 0.9)),
        ],
        Icon::SplitDown => vec![
            canvas::Path::rounded_rectangle(
                grid.p(0.0, 0.0),
                grid.size(1.0, 1.0),
                iced::border::Radius::from(2.0),
            ),
            canvas::Path::line(grid.p(0.1, 0.5), grid.p(0.9, 0.5)),
        ],
        Icon::ZoomIn => vec![
            // Top-left bracket, opening outward, with a short diagonal tick
            // past the corner standing in for an arrowhead.
            canvas::Path::line(grid.p(0.05, 0.3), grid.p(0.05, 0.05)),
            canvas::Path::line(grid.p(0.05, 0.05), grid.p(0.3, 0.05)),
            canvas::Path::line(grid.p(0.05, 0.05), grid.p(-0.05, -0.05)),
            // Bottom-right bracket, mirrored.
            canvas::Path::line(grid.p(0.95, 0.7), grid.p(0.95, 0.95)),
            canvas::Path::line(grid.p(0.95, 0.95), grid.p(0.7, 0.95)),
            canvas::Path::line(grid.p(0.95, 0.95), grid.p(1.05, 1.05)),
        ],
        Icon::ZoomOut => vec![
            // Same brackets pulled toward the center, tick pointing further
            // inward, so the pair reads as a collapse rather than an expand.
            canvas::Path::line(grid.p(0.3, 0.3), grid.p(0.3, 0.05)),
            canvas::Path::line(grid.p(0.3, 0.3), grid.p(0.05, 0.3)),
            canvas::Path::line(grid.p(0.3, 0.3), grid.p(0.22, 0.22)),
            canvas::Path::line(grid.p(0.7, 0.7), grid.p(0.7, 0.95)),
            canvas::Path::line(grid.p(0.7, 0.7), grid.p(0.95, 0.7)),
            canvas::Path::line(grid.p(0.7, 0.7), grid.p(0.78, 0.78)),
        ],
        Icon::Close => vec![
            canvas::Path::line(grid.p(0.15, 0.15), grid.p(0.85, 0.85)),
            canvas::Path::line(grid.p(0.85, 0.15), grid.p(0.15, 0.85)),
        ],
        Icon::Minimize => vec![canvas::Path::line(grid.p(0.25, 0.85), grid.p(0.75, 0.85))],
        Icon::Maximize => vec![canvas::Path::rounded_rectangle(
            grid.p(0.0, 0.0),
            grid.size(1.0, 1.0),
            iced::border::Radius::from(2.0),
        )],
        Icon::Folder => vec![canvas::Path::new(|b| {
            // Folder outline with a raised tab across the top-left.
            b.move_to(grid.p(0.02, 0.88));
            b.line_to(grid.p(0.02, 0.18));
            b.line_to(grid.p(0.36, 0.18));
            b.line_to(grid.p(0.46, 0.32));
            b.line_to(grid.p(0.98, 0.32));
            b.line_to(grid.p(0.98, 0.88));
            b.close();
        })],
        Icon::Home => vec![
            canvas::Path::new(|b| {
                // Roof.
                b.move_to(grid.p(0.02, 0.5));
                b.line_to(grid.p(0.5, 0.08));
                b.line_to(grid.p(0.98, 0.5));
            }),
            canvas::Path::new(|b| {
                // Walls and a door opening.
                b.move_to(grid.p(0.15, 0.45));
                b.line_to(grid.p(0.15, 0.92));
                b.line_to(grid.p(0.4, 0.92));
                b.line_to(grid.p(0.4, 0.62));
                b.line_to(grid.p(0.6, 0.62));
                b.line_to(grid.p(0.6, 0.92));
                b.line_to(grid.p(0.85, 0.92));
                b.line_to(grid.p(0.85, 0.45));
            }),
        ],
        Icon::Drive => vec![
            canvas::Path::rounded_rectangle(
                grid.p(0.0, 0.25),
                grid.size(1.0, 0.5),
                iced::border::Radius::from(2.0),
            ),
            canvas::Path::circle(grid.p(0.78, 0.5), grid.r(0.05)),
            canvas::Path::line(grid.p(0.12, 0.5), grid.p(0.45, 0.5)),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 18] = [
        Icon::Search,
        Icon::Bell,
        Icon::Settings,
        Icon::Sessions,
        Icon::Palette,
        Icon::Plus,
        Icon::Git,
        Icon::Terminal,
        Icon::SplitRight,
        Icon::SplitDown,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::Close,
        Icon::Minimize,
        Icon::Maximize,
        Icon::Folder,
        Icon::Home,
        Icon::Drive,
    ];

    #[test]
    fn every_icon_builds_without_panic() {
        for kind in ALL {
            let _element = icon::<()>(kind, 16.0, Color::WHITE);
        }
    }
}
