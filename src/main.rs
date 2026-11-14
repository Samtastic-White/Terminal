use gpui::*;

mod application;
mod common;
mod domain;
mod infrastructure;
mod presentation;
mod probe;

#[cfg(test)]
mod tests;

use application::session::SessionService;
use domain::pty::PtyPort;
use domain::size::TerminalSize;
use gpui_platform::application;
use infrastructure::parser::BlockTextWriter;
use infrastructure::pty::PtyAdapter;
use presentation::session_view::TerminalWindow;

fn main() {
    common::logging::init();

    application().run(|cx: &mut App| {
        let measure = probe::measure_enabled();

        let mut pty_adapter = PtyAdapter::new();
        let block_writer = BlockTextWriter::new(80);
        let initial_size = TerminalSize::new(80, 24);

        if let Err(error) = pty_adapter.spawn(initial_size) {
            tracing::error!(error = %error, "shell spawn failed");
            return;
        }
        let Some(pty_receiver) = pty_adapter.take_output() else {
            tracing::error!("pty channel unavailable");
            return;
        };
        let session_service = SessionService::new(Box::new(pty_adapter), Box::new(block_writer));

        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |_window, cx| cx.new(|cx| TerminalWindow::new(session_service, pty_receiver, cx)),
        );
        if let Err(error) = window {
            tracing::error!(error = %error, "window open failed");
            return;
        }

        cx.activate(true);

        if measure {
            let backend = probe::backend_name();
            std::thread::Builder::new()
                .name("nt-idle-probe".into())
                .spawn(move || probe::run_probe(backend))
                .ok();
        }
    });
}
