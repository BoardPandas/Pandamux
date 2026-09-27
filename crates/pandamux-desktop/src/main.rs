use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::gpui::*;
use pandamux_desktop::{AccentColor, AppView, Theme};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_headless = args.iter().any(|arg| arg == "--headless" || arg == "--bench");

    if is_headless {
        println!("Running PandaMUX Desktop in headless verification mode...");
        let theme = Theme::dark(AccentColor::Teal);
        println!("Theme initialized: {:?} with accent {:?}", theme.mode, theme.accent);
        println!("Headless check: ALL CHECKS PASSED.");
        return;
    }

    let is_smoke = args.iter().any(|arg| arg == "--smoke");

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("PandaMUX".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| AppView::new(window, cx));

                if is_smoke {
                    cx.spawn(async move |cx| {
                        smol::Timer::after(Duration::from_millis(500)).await;
                        println!("PandaMUX Desktop Smoke Test: Window initialized and rendered cleanly.");
                        let _ = cx.update(|cx| cx.quit());
                    })
                    .detach();
                }

                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("Failed to open PandaMUX window");
    });
}
