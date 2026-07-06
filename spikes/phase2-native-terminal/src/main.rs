use std::env;
use std::time::Instant;

use anyhow::{Result, anyhow};
use iced::widget::{column, container, text};
use iced::{Element, Length, Theme, application};

mod gpu_render_smoke;
mod iced_terminal_viewport;
mod pty_smoke;
mod russh_smoke;
mod term_grid;
mod text_stack;

#[derive(Debug, Clone)]
enum Message {}

#[derive(Debug)]
struct SpikeApp {
    terminal_lines: Vec<String>,
}

impl Default for SpikeApp {
    fn default() -> Self {
        let mut grid = term_grid::TerminalGrid::new(80, 24);
        grid.advance(
            b"PandaMUX Phase 2\r\n\
              native Iced shell + alacritty_terminal grid\r\n\
              Unicode: ascii, box \xE2\x94\x80, CJK \xE7\x8C\xAB, emoji \xF0\x9F\x9A\x80\r\n",
        );

        Self {
            terminal_lines: grid.snapshot_text().lines().map(str::to_owned).collect(),
        }
    }
}

fn main() {
    let result = match command() {
        SpikeCommand::IcedShell => run_iced_shell(),
        SpikeCommand::PtySmoke => (|| -> Result<()> {
            let output = pty_smoke::run_pty_smoke()?;
            print!("{output}");
            Ok(())
        })(),
        SpikeCommand::GridSmoke => run_grid_smoke(),
        SpikeCommand::IcedWidgetSmoke => run_iced_widget_smoke(),
        SpikeCommand::BurstSmoke { lines } => run_burst_smoke(lines),
        SpikeCommand::TextStackSmoke => run_text_stack_smoke(),
        SpikeCommand::RusshGalahadSmoke { key_path } => run_russh_galahad_smoke(key_path),
        SpikeCommand::RusshGalahadAgentSmoke => run_russh_galahad_agent_smoke(),
        SpikeCommand::RusshGalahadOnePasswordSmoke => run_russh_galahad_one_password_smoke(),
        SpikeCommand::RusshGalahadPasswordSmoke { password } => {
            run_russh_galahad_password_smoke(password)
        }
        SpikeCommand::GpuRenderSmoke => run_gpu_render_smoke(),
        SpikeCommand::VisualQaSmoke { output_path } => run_visual_qa_smoke(output_path),
    };

    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

enum SpikeCommand {
    IcedShell,
    PtySmoke,
    GridSmoke,
    IcedWidgetSmoke,
    BurstSmoke { lines: usize },
    TextStackSmoke,
    RusshGalahadSmoke { key_path: String },
    RusshGalahadAgentSmoke,
    RusshGalahadOnePasswordSmoke,
    RusshGalahadPasswordSmoke { password: String },
    GpuRenderSmoke,
    VisualQaSmoke { output_path: String },
}

fn command() -> SpikeCommand {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--pty-smoke") => SpikeCommand::PtySmoke,
        Some("--grid-smoke") => SpikeCommand::GridSmoke,
        Some("--iced-widget-smoke") => SpikeCommand::IcedWidgetSmoke,
        Some("--text-stack-smoke") => SpikeCommand::TextStackSmoke,
        Some("--russh-galahad-smoke") => {
            let key_path = args
                .next()
                .or_else(|| env::var("PANDAMUX_GALAHAD_KEY").ok())
                .unwrap_or_else(|| {
                    format!(
                        "{}\\.ssh\\galahad",
                        env::var("USERPROFILE").unwrap_or_default()
                    )
                });
            SpikeCommand::RusshGalahadSmoke { key_path }
        }
        Some("--russh-galahad-agent-smoke") => SpikeCommand::RusshGalahadAgentSmoke,
        Some("--russh-galahad-1password-smoke") => SpikeCommand::RusshGalahadOnePasswordSmoke,
        Some("--russh-galahad-password-smoke") => {
            let password = args
                .next()
                .or_else(|| env::var("PANDAMUX_GALAHAD_PASSWORD").ok())
                .unwrap_or_default();
            SpikeCommand::RusshGalahadPasswordSmoke { password }
        }
        Some("--gpu-render-smoke") => SpikeCommand::GpuRenderSmoke,
        Some("--visual-qa-smoke") => {
            let output_path = args
                .next()
                .unwrap_or_else(|| "phase2-visual-qa.bmp".to_string());
            SpikeCommand::VisualQaSmoke { output_path }
        }
        Some("--burst-smoke") => {
            let lines = args
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(2_000);
            SpikeCommand::BurstSmoke { lines }
        }
        _ => SpikeCommand::IcedShell,
    }
}

fn run_iced_shell() -> Result<()> {
    application(|| SpikeApp::default(), update, view)
        .title("PandaMUX Phase 2 Terminal Spike")
        .theme(theme)
        .run()
        .map_err(|error| anyhow!("{error}"))
}

fn update(_state: &mut SpikeApp, _message: Message) {}

fn theme(_state: &SpikeApp) -> Theme {
    Theme::Dark
}

fn view(state: &SpikeApp) -> Element<'_, Message> {
    let content = column![
        text("PandaMUX Phase 2").size(24),
        text("Native terminal spike shell").size(16),
        text("The text below is parsed through alacritty_terminal, then projected into Iced.")
            .size(14),
        iced_terminal_viewport::terminal_viewport(state.terminal_lines.clone(), 80, 24),
    ]
    .spacing(12);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(24)
        .into()
}

fn run_grid_smoke() -> Result<()> {
    let output = term_grid::render_bytes_to_text(
        b"alpha\r\n\x1b[31mred\x1b[0m\r\nwide:\xE7\x8C\xAB\r\nemoji:\xF0\x9F\x9A\x80\r\n",
        20,
        8,
    );

    if !output.contains("alpha")
        || !output.contains("red")
        || !output.contains("wide:")
        || !output.contains("emoji:")
    {
        return Err(anyhow!("grid smoke missing expected text: {output:?}"));
    }

    println!("{output}");
    Ok(())
}

fn run_iced_widget_smoke() -> Result<()> {
    let app = SpikeApp::default();
    if app.terminal_lines.len() < 3
        || !app
            .terminal_lines
            .iter()
            .any(|line| line.contains("Unicode"))
    {
        return Err(anyhow!(
            "Iced terminal viewport sample is missing expected rows"
        ));
    }

    let _viewport =
        iced_terminal_viewport::TerminalViewport::new(app.terminal_lines.clone(), 80, 24);
    println!(
        "PANDAMUX_ICED_WIDGET_SMOKE_OK\nlines={}\ncolumns=80\nrows=24",
        app.terminal_lines.len()
    );
    Ok(())
}

fn run_text_stack_smoke() -> Result<()> {
    let report = text_stack::run_text_stack_smoke()?;
    println!("{}", report.summary());
    Ok(())
}

fn run_russh_galahad_smoke(key_path: String) -> Result<()> {
    let config = russh_smoke::default_galahad_config(key_path);
    let report = russh_smoke::run_galahad_smoke(config)?;
    println!("{}", report.summary());
    Ok(())
}

fn run_russh_galahad_agent_smoke() -> Result<()> {
    let config = russh_smoke::default_galahad_agent_config();
    let report = russh_smoke::run_galahad_smoke(config)?;
    println!("{}", report.summary());
    Ok(())
}

fn run_russh_galahad_one_password_smoke() -> Result<()> {
    let config = russh_smoke::default_galahad_one_password_config();
    let report = russh_smoke::run_galahad_smoke(config)?;
    println!("{}", report.summary());
    Ok(())
}

fn run_russh_galahad_password_smoke(password: String) -> Result<()> {
    if password.is_empty() {
        return Err(anyhow!(
            "missing password; pass it as an argument or set PANDAMUX_GALAHAD_PASSWORD"
        ));
    }
    let config = russh_smoke::default_galahad_password_config(password);
    let report = russh_smoke::run_galahad_smoke(config)?;
    println!("{}", report.summary());
    Ok(())
}

fn run_gpu_render_smoke() -> Result<()> {
    let report = gpu_render_smoke::run_gpu_render_smoke()?;
    println!("{}", report.summary());
    Ok(())
}

fn run_visual_qa_smoke(output_path: String) -> Result<()> {
    let report = gpu_render_smoke::run_visual_qa_smoke(output_path)?;
    println!("{}", report.summary());
    Ok(())
}

fn run_burst_smoke(lines: usize) -> Result<()> {
    let start = Instant::now();
    let (output, pty_elapsed) = pty_smoke::run_pty_burst(lines)?;
    let mut grid = term_grid::TerminalGrid::new(120, 40);
    grid.advance(output.as_bytes());
    let text = grid.snapshot_text();
    let total_elapsed = start.elapsed();

    if !text.contains("PANDAMUX_BURST_") {
        return Err(anyhow!("burst output did not reach the terminal grid"));
    }

    println!(
        "burst_lines={lines} pty_ms={} total_ms={} bytes={} last_screen_contains_marker=true",
        pty_elapsed.as_millis(),
        total_elapsed.as_millis(),
        output.len()
    );
    Ok(())
}
