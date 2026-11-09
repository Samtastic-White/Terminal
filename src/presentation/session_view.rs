//! Session view — Phase 1 render surface, key capture, and resize hook
//! (`knowledge/specs/pty-raw-output/design.md` § 1, § 5, § 6).
//!
//! The window bootstrap and the infrastructure adapters live in the
//! composition root (`src/main.rs`); this view only depends on the
//! Application service and Domain types. Presentation never holds a PTY
//! handle (architecture Common Mistake #3).

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui::*;

use crate::application::session::SessionService;
use crate::domain::pty::PtySize;
use crate::presentation::components::row::render_rows;
use crate::presentation::theme::TerminalTheme;

/// Drain-task idle wake (`design.md` § 4: POLL_INTERVAL).
const POLL_INTERVAL: Duration = Duration::from_millis(8);
/// Maximum chunks parsed per wake; remaining chunks wait for the next
/// timer tick so output ingestion never monopolizes a frame.
const MAX_CHUNKS_PER_WAKE: usize = 8;

/// Horizontal padding of the output area (`px_3` on each side).
const OUTPUT_PADDING_X: f32 = 24.0;
/// Vertical padding of the output area (`py_2` on each side).
const OUTPUT_PADDING_Y: f32 = 16.0;

/// Main terminal window state: one Phase 1 session rendered as styled rows.
pub struct TerminalWindow {
    focus_handle: FocusHandle,
    session_service: SessionService,
    pty_receiver: Receiver<Vec<u8>>,
    theme: TerminalTheme,
    scroll: ScrollHandle,
    last_pty_size: Option<PtySize>,
    /// BR-10: auto-follow is active until the user scrolls back up.
    follow_tail: bool,
    /// The reader channel closed and the final row was flushed (BR-9).
    stream_closed: bool,
    did_focus: bool,
    /// Foreground drain task; dropping it would stop output ingestion.
    _drain_task: Task<()>,
}

impl TerminalWindow {
    /// Build the session view around an already spawned session service.
    pub fn new(
        session_service: SessionService,
        pty_receiver: Receiver<Vec<u8>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = TerminalTheme::default();

        // Drain task (`design.md` § 2/§ 3): bounded channel consumer that
        // never runs on the render thread. `try_recv` drains what is
        // available, then parks on the timer; every batch notifies the
        // entity so the next frame renders the new rows.
        let _drain_task = cx.spawn(async move |this, cx| {
            loop {
                let closed = this.update(cx, |this, cx| {
                    let (changed, closed) = this.drain_output();
                    if changed {
                        cx.notify();
                    }
                    closed
                });
                match closed {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) => break, // view released
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            session_service,
            pty_receiver,
            theme,
            scroll: ScrollHandle::new(),
            last_pty_size: None,
            follow_tail: true,
            stream_closed: false,
            did_focus: false,
            _drain_task,
        }
    }

    /// Consume every chunk currently buffered. Returns `(changed, closed)`:
    /// `changed` means the document grew; `closed` means the reader
    /// channel disconnected and the final partial row was flushed.
    fn drain_output(&mut self) -> (bool, bool) {
        let mut changed = false;
        let mut drained = 0;
        while drained < MAX_CHUNKS_PER_WAKE {
            match self.pty_receiver.try_recv() {
                Ok(chunk) => {
                    self.session_service.ingest_bytes(&chunk);
                    changed = true;
                    drained += 1;
                }
                Err(TryRecvError::Empty) => return (changed, false),
                Err(TryRecvError::Disconnected) => {
                    if !self.stream_closed {
                        self.session_service.finalize_stream();
                        self.stream_closed = true;
                        changed = true;
                    }
                    return (changed, true);
                }
            }
        }
        (changed, false)
    }

    /// Phase 1 keyboard passthrough (`design.md` § 5): the shell keeps its
    /// own prompt and line editing, so keys go to the PTY as bytes — no
    /// local input buffer and no ghost prompt.
    fn handle_key(&mut self, event: &KeyDownEvent) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;

        if modifiers.control {
            match keystroke.key.as_str() {
                "c" => self.write_keys(b"\x03"),
                "d" => self.write_keys(b"\x04"),
                "l" => self.write_keys(b"\x0c"),
                _ => {}
            }
            return;
        }

        match keystroke.key.as_str() {
            "enter" => self.write_keys(b"\r"),
            "backspace" => self.write_keys(b"\x7f"),
            "up" => self.write_keys(b"\x1b[A"),
            "down" => self.write_keys(b"\x1b[B"),
            "right" => self.write_keys(b"\x1b[C"),
            "left" => self.write_keys(b"\x1b[D"),
            _ => {
                // Plain printable text. Modified keys outside the Phase 1
                // table are dropped silently (never wrongly encoded).
                if modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                if let Some(text) = keystroke.key_char.as_deref()
                    && !text.is_empty()
                    && text.chars().all(|c| !c.is_control())
                {
                    self.write_keys(text.as_bytes());
                }
            }
        }
    }

    /// Route bytes through the Application layer (never the PTY directly).
    fn write_keys(&mut self, bytes: &[u8]) {
        let _ = self.session_service.write_keystrokes(bytes);
    }

    /// BR-7/T9: recompute cols/rows from font metrics and forward only when
    /// the size actually changed (no debounce, but no per-frame ioctl
    /// either). Stored rows keep their creation-time wrapping.
    fn update_pty_size(&mut self, window: &Window) {
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&gpui::font(self.theme.font_family));
        let char_width = text_system
            .ch_advance(font_id, px(self.theme.font_size))
            .map(|width| width.as_f32())
            .unwrap_or(self.theme.font_size * 0.6)
            .max(1.0);
        let line_height = self.theme.font_size * self.theme.line_height;

        let bounds = window.bounds();
        let available_width = (bounds.size.width.as_f32() - OUTPUT_PADDING_X).max(1.0);
        let available_height = (bounds.size.height.as_f32() - OUTPUT_PADDING_Y).max(1.0);
        let cols = (available_width / char_width)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16;
        let rows = (available_height / line_height)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16;
        let size = PtySize::new(cols, rows);

        if self.last_pty_size != Some(size) {
            self.last_pty_size = Some(size);
            let _ = self.session_service.resize(size);
        }
    }

    fn is_at_bottom(&self) -> bool {
        self.scroll.offset().y + self.scroll.max_offset().y <= px(4.0)
    }
}

impl Focusable for TerminalWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.did_focus {
            window.focus(&self.focus_handle, cx);
            self.did_focus = true;
        }

        self.update_pty_size(window);

        // BR-10: follow the tail while docked; a manual scroll suspends it
        // and returning to the end resumes it.
        if self.follow_tail {
            self.scroll.scroll_to_bottom();
        } else if self.is_at_bottom() {
            self.follow_tail = true;
            self.scroll.scroll_to_bottom();
        }

        let rows = self.session_service.document().rows();

        div()
            .id("terminal-window")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _window, _cx| {
                this.handle_key(event);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .on_scroll(cx.listener(|this, _event, _window, cx| {
                this.follow_tail = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("terminal-output")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_3()
                    .py_2()
                    .child(render_rows(rows, &self.theme)),
            )
    }
}
