//! Card view: header, body canvas with selection handlers, and the
//! top-level `render_block_card` composition.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use gpui::*;

use super::actions::{CARD_ACTIONS, render_action};
use super::body::BodySlice;
use super::snapshot::{CardSnapshot, StatusTone};
use super::state::{BlockUiState, BodyLayout, PendingBody};
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::presentation::theme::TerminalTheme;

/// Click callback routed by the view to `SessionService::copy_block`.
pub type CopyHandler = Rc<dyn Fn(CopyKind, &mut Window, &mut App)>;

pub(super) fn body_text_runs(slice: &BodySlice, theme: &TerminalTheme) -> Vec<TextRun> {
    slice
        .runs()
        .iter()
        .map(|run| {
            let style = run.style();
            let mut text_font = font(theme.font_family);
            text_font.weight = if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };
            text_font.style = if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            };
            let color: Hsla = theme.resolve_color(&style.fg).into();
            let background_color: Option<Hsla> = if run.selected() {
                Some(theme.selection_background.into())
            } else {
                theme.resolve_bg(&style.bg).map(Into::into)
            };
            TextRun {
                len: run.range().len(),
                font: text_font,
                color,
                background_color,
                underline: style.underline.then(|| UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            }
        })
        .collect()
}

fn tone_color(tone: StatusTone, theme: &TerminalTheme) -> Rgba {
    match tone {
        StatusTone::Running => theme.badge_running,
        StatusTone::Success => theme.badge_success,
        StatusTone::Failure => theme.badge_failure,
        StatusTone::Interrupted => theme.badge_interrupted,
    }
}

fn render_header(
    snapshot: &CardSnapshot,
    block: BlockId,
    theme: &TerminalTheme,
    on_copy: &CopyHandler,
) -> impl IntoElement {
    let group: SharedString = format!("block-{}", block.get()).into();
    let pill = div()
        .flex_none()
        .px(px(8.0))
        .py(px(2.0))
        .rounded_full()
        .bg(tone_color(snapshot.tone, theme))
        .text_color(theme.badge_text)
        .text_size(px(theme.font_size * 0.72))
        .font_weight(FontWeight::MEDIUM)
        .child(snapshot.status_label.clone());

    let actions = div().flex().flex_row().items_center().gap_1().children(
        CARD_ACTIONS
            .into_iter()
            .map(|action| render_action(block, action, &group, theme, on_copy)),
    );

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px(px(12.0))
        .py(px(8.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.card_header)
                .child(snapshot.command.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_color(theme.card_meta)
                .text_size(px(theme.font_size * 0.82))
                .child(snapshot.meta.clone()),
        )
        .child(pill)
        .child(actions)
}

/// Paints the visible body slice. Layout bounds are refreshed in prepaint so
/// selection mapping stays correct while the outer list scrolls.
fn render_body(
    height: Pixels,
    ui: Rc<RefCell<BlockUiState>>,
    block: BlockId,
    view: EntityId,
    font_size: Pixels,
    line_height: Pixels,
) -> impl IntoElement {
    let prepaint_ui = ui.clone();
    let paint_ui = ui.clone();

    let body = canvas(
        move |bounds, window, _cx| {
            let mut state = prepaint_ui.borrow_mut();
            if let Some(pending) = state.pending.take() {
                let PendingBody {
                    text,
                    runs,
                    line_height,
                    slice_start_byte,
                    slice_end_byte,
                } = pending;
                let shaped =
                    window
                        .text_system()
                        .shape_text(text.into(), font_size, &runs, None, None);
                let lines = match shaped {
                    Ok(lines) => lines.into_iter().collect(),
                    Err(error) => {
                        tracing::warn!(
                            block_id = block.get(),
                            error = %error,
                            "block body shaping failed"
                        );
                        Vec::new()
                    }
                };
                state.layout = Some(BodyLayout {
                    lines,
                    bounds: Some(bounds),
                    line_height,
                    slice_start_byte,
                    slice_end_byte,
                });
            } else if let Some(layout) = state.layout.as_mut() {
                layout.bounds = Some(bounds);
            }
        },
        move |bounds, _, window, cx| {
            let Ok(state) = paint_ui.try_borrow() else {
                return;
            };
            let Some(layout) = state.layout.as_ref() else {
                return;
            };
            let mut origin = bounds.origin;
            for line in &layout.lines {
                let _ = line.paint_background(
                    origin,
                    layout.line_height,
                    TextAlign::Left,
                    Some(bounds),
                    window,
                    cx,
                );
                let _ = line.paint(
                    origin,
                    layout.line_height,
                    TextAlign::Left,
                    Some(bounds),
                    window,
                    cx,
                );
                origin.y += line.size(layout.line_height).height;
            }
        },
    )
    .h(height)
    .w_full();

    div()
        .id(("block-body", block.get()))
        .relative()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .cursor_text()
        .on_mouse_down(MouseButton::Left, {
            let ui = ui.clone();
            move |event, _window, cx| {
                if ui.borrow_mut().begin_selection(event.position) {
                    cx.notify(view);
                }
            }
        })
        .on_mouse_move({
            let ui = ui.clone();
            move |event, _window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if ui.borrow_mut().extend_selection(event.position) {
                    cx.notify(view);
                }
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let ui = ui.clone();
            move |_event, _window, cx| {
                if ui.borrow_mut().end_selection() {
                    cx.notify(view);
                }
            }
        })
        .on_scroll({
            move |event, _window, cx| {
                let delta = event.delta.pixel_delta(line_height);
                if ui.borrow_mut().scroll_by(delta.y.as_f32(), line_height) {
                    cx.stop_propagation();
                    cx.notify(view);
                }
            }
        })
        .child(body)
}

/// One block card. Empty output renders the header only.
pub fn render_block_card(
    block: &Block,
    theme: &TerminalTheme,
    now: SystemTime,
    ui: Rc<RefCell<BlockUiState>>,
    view: EntityId,
    on_copy: CopyHandler,
) -> impl IntoElement {
    let line_height = px(theme.font_size * theme.line_height);
    let (snapshot, body_height) = {
        let mut state = ui.borrow_mut();
        state.sync_body(block);
        let snapshot = state.header_snapshot(block, now);
        let body_height = if snapshot.has_body {
            state.prepare_body(theme)
        } else {
            None
        };
        (snapshot, body_height)
    };

    let mut card = div()
        .id(("block-card", block.id().get()))
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .mb_3()
        .rounded_md()
        .border_1()
        .border_color(theme.card_border)
        .bg(theme.card_background)
        .shadow_sm()
        .group(format!("block-{}", block.id().get()))
        .child(render_header(&snapshot, block.id(), theme, &on_copy));

    if let Some(height) = body_height {
        card = card.child(div().px(px(12.0)).pb(px(10.0)).child(render_body(
            height,
            ui,
            block.id(),
            view,
            px(theme.font_size),
            line_height,
        )));
    }

    card
}
