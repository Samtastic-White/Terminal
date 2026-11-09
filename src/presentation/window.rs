//! Main terminal window — ties together session rendering + input editor.

use gpui::*;
use crate::application::session::SessionService;
use crate::domain::pty::PtyPort;
use crate::domain::pty::PtySize;
use crate::infrastructure::parser::BlockTextWriter;
use crate::infrastructure::pty::PtyAdapter;
use crate::presentation::components::{input_editor::InputEditor, row::render_rows};
use crate::presentation::theme::TerminalTheme;
use crate::common::errors::Result;
use std::sync::{mpsc::Receiver, Arc, Mutex};

/// Main terminal window state
pub struct TerminalWindow {
    focus_handle: FocusHandle,
    session_service: Arc<Mutex<SessionService>>,
    pty_receiver: Receiver<Vec<u8>>,
    theme: TerminalTheme,
    input_editor: Entity<InputEditor>,
}

impl TerminalWindow {
    /// Create a new terminal window with PTY adapter and writer
    pub fn new(cx: &mut Context<Self>) -> Entity<Self> {
        let theme = TerminalTheme::default();
        
        // Create infrastructure adapters FIRST, spawn, then extract receiver
        let mut pty_adapter = PtyAdapter::new();
        let block_writer = BlockTextWriter::new(80); // Initial cols, will resize on first render
        
        // Spawn the shell immediately to get the receiver
        let initial_size = PtySize::new(80, 24);
        PtyPort::spawn(&mut pty_adapter, initial_size).expect("Failed to spawn PTY");
        let pty_receiver = pty_adapter.take_output().expect("PTY receiver not available");
        
        // NOW box the adapter and create session service
        let session_service = Arc::new(Mutex::new(SessionService::new(
            Box::new(pty_adapter),
            Box::new(block_writer),
        )));

        cx.new(|cx| {
            let session_service = session_service.clone();
            // Create input editor with callbacks
            let input_editor = cx.new(|cx| InputEditor::new(
                theme.clone(),
                // on_submit: send keystrokes to PTY
                {
                    let session_service = session_service.clone();
                    move |text: String| {
                        let mut svc = session_service.lock().unwrap();
                        svc.write_keystrokes(text.as_bytes())
                    }
                },
                // on_ctrl_c: send Ctrl+C
                {
                    let session_service = session_service.clone();
                    move || {
                        let mut svc = session_service.lock().unwrap();
                        svc.write_keystrokes(b"\x03")
                    }
                },
                // on_ctrl_d: send Ctrl+D (EOF)
                {
                    let session_service = session_service.clone();
                    move || {
                        let mut svc = session_service.lock().unwrap();
                        svc.write_keystrokes(b"\x04")
                    }
                },
                // on_ctrl_l: send Ctrl+L (clear screen)
                {
                    let session_service = session_service.clone();
                    move || {
                        let mut svc = session_service.lock().unwrap();
                        svc.write_keystrokes(b"\x0c")
                    }
                },
                cx,
            ));

            Self {
                focus_handle: cx.focus_handle(),
                session_service,
                pty_receiver,
                theme,
                input_editor,
            }
        })
    }

    /// Process pending PTY output (non-blocking)
    fn process_pty_output(&mut self) {
        // Try to receive all available chunks
        while let Ok(chunk) = self.pty_receiver.try_recv() {
            let mut svc = self.session_service.lock().unwrap();
            svc.ingest_bytes(&chunk);
        }
    }

    /// Handle window resize
    fn handle_resize(&mut self, size: PtySize) -> Result<()> {
        let mut svc = self.session_service.lock().unwrap();
        svc.resize(size)
    }
}

impl Focusable for TerminalWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending PTY output
        self.process_pty_output();

        // Get current document rows
        let rows = {
            let svc = self.session_service.lock().unwrap();
            let document = svc.document();
            document.rows().iter().cloned().collect::<Vec<_>>()
        };

        // Calculate size from window bounds
        let bounds = window.bounds();
        let char_width = self.theme.font_size * 0.6; // Approximate monospace char width
        let cols = (bounds.size.width / px(char_width)).max(1.0) as u16;
        let rows_count = (bounds.size.height / px(self.theme.font_size * self.theme.line_height)).max(1.0) as u16;
        let size = PtySize::new(cols, rows_count);

        // Handle resize
        let _ = self.handle_resize(size);

        div()
            .id("terminal-window")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, window, cx| {
                this.input_editor.update(cx, |editor, cx| {
                    editor.on_key_down(event, window, cx);
                });
            }))
            .child(
                // Output area - scrollable
                div()
                    .id("output-area")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .px_3()
                    .py_2()
                    .child(render_rows(&rows, &self.theme))
            )
            .child(
                // Input editor at bottom
                self.input_editor.clone()
            )
    }
}

/// App state holding the terminal window
pub struct TerminalApp {
    window: Option<Entity<TerminalWindow>>,
}

impl TerminalApp {
    pub fn new(cx: &mut App) -> Self {
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |cx| TerminalWindow::new(cx),
        ).ok();

        Self { window }
    }
}

impl Render for TerminalApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // App-level render - the window renders itself
        div().size_full()
    }
}