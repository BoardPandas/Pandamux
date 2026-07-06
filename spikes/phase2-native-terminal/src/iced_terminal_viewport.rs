use iced::widget::canvas;
use iced::{Color, Element, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};

const CELL_WIDTH: f32 = 9.0;
const CELL_HEIGHT: f32 = 20.0;
const PADDING: f32 = 16.0;

#[derive(Debug, Clone)]
pub struct TerminalViewport {
    lines: Vec<String>,
    columns: usize,
    rows: usize,
}

impl TerminalViewport {
    pub fn new(lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            lines,
            columns,
            rows,
        }
    }

    fn preferred_width(&self) -> f32 {
        PADDING * 2.0 + self.columns as f32 * CELL_WIDTH
    }

    fn preferred_height(&self) -> f32 {
        PADDING * 2.0 + self.rows as f32 * CELL_HEIGHT
    }
}

impl<Message> canvas::Program<Message> for TerminalViewport {
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
        let background = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, Color::from_rgb8(11, 15, 21));

        let terminal_size = Size::new(
            self.preferred_width().min(bounds.width),
            self.preferred_height().min(bounds.height),
        );
        let terminal = canvas::Path::rectangle(Point::new(0.0, 0.0), terminal_size);
        frame.fill(&terminal, Color::from_rgb8(15, 23, 32));

        let cursor_row = self
            .lines
            .len()
            .saturating_sub(1)
            .min(self.rows.saturating_sub(1));
        let cursor = canvas::Path::rectangle(
            Point::new(PADDING, PADDING + cursor_row as f32 * CELL_HEIGHT + 2.0),
            Size::new(CELL_WIDTH, CELL_HEIGHT - 4.0),
        );
        frame.fill(&cursor, Color::from_rgb8(45, 72, 95));

        for row in 0..=self.rows {
            let y = PADDING + row as f32 * CELL_HEIGHT;
            let rule = canvas::Path::rectangle(
                Point::new(PADDING, y),
                Size::new(
                    (self.columns as f32 * CELL_WIDTH).min(bounds.width - PADDING),
                    1.0,
                ),
            );
            frame.fill(&rule, Color::from_rgba8(80, 96, 112, 0.18));
        }

        for (row, line) in self.lines.iter().take(self.rows).enumerate() {
            frame.fill_text(canvas::Text {
                content: line.clone(),
                position: Point::new(PADDING, PADDING + row as f32 * CELL_HEIGHT),
                max_width: (self.columns as f32 * CELL_WIDTH).min(bounds.width - PADDING * 2.0),
                color: Color::from_rgb8(230, 238, 248),
                size: Pixels(15.0),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(CELL_HEIGHT)),
                font: Font::MONOSPACE,
                shaping: iced::widget::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }
}

pub fn terminal_viewport<'a, Message: 'a>(
    lines: Vec<String>,
    columns: usize,
    rows: usize,
) -> Element<'a, Message> {
    canvas::Canvas::new(TerminalViewport::new(lines, columns, rows))
        .width(Length::Fill)
        .height(Length::Fixed(PADDING * 2.0 + rows as f32 * CELL_HEIGHT))
        .into()
}
