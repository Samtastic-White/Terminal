//! Command editor canvas: shaped visible lines, selection quads, and a caret
//! bar. The domain editor owns all text and cell math; this component only
//! paints the window it receives and maps pointer positions back to cells.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::domain::editor::EditorView;
use crate::presentation::theme::TerminalTheme;

/// Horizontal inset of the editor canvas inside the dock.
pub const EDITOR_PADDING_X: f32 = 12.0;
/// Vertical inset above and below the canvas inside the dock.
pub const EDITOR_PADDING_Y: f32 = 6.0;
const CARET_WIDTH: f32 = 2.0;

/// Layout captured during prepaint and reused by pointer hit-testing.
pub struct InputEditorState {
    bounds: Option<Bounds<Pixels>>,
    char_width: f32,
    line_height: Pixels,
    line_count: usize,
    dragging: bool,
    shaped: Vec<Option<WrappedLine>>,
}

impl Default for InputEditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputEditorState {
    pub fn new() -> Self {
        Self {
            bounds: None,
            char_width: 0.0,
            line_height: px(1.0),
            line_count: 0,
            dragging: false,
            shaped: Vec::new(),
        }
    }

    /// Maps a window position to `(visual line, cell)` relative to the visible
    /// window. Positions below the last line resolve past the end so the
    /// editor places the caret at the text end.
    pub fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let bounds = self.bounds?;
        if self.char_width <= 0.0 {
            return None;
        }
        let relative_y = (position.y - bounds.top()).max(px(0.0)).as_f32();
        let line = (relative_y / self.line_height.as_f32()) as usize;
        if line >= self.line_count {
            return Some((self.line_count, 0));
        }
        let relative_x = (position.x - bounds.left()).max(px(0.0)).as_f32();
        let cell = (relative_x / self.char_width).floor() as usize;
        Some((line, cell))
    }

    pub fn begin_drag(&mut self) {
        self.dragging = true;
    }

    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }
}

fn text_font(theme: &TerminalTheme) -> Font {
    let mut font = font(theme.font_family);
    font.weight = FontWeight::NORMAL;
    font.style = FontStyle::Normal;
    font
}

fn text_run(len: usize, theme: &TerminalTheme) -> TextRun {
    TextRun {
        len,
        font: text_font(theme),
        color: theme.foreground.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Pixel x of a byte offset inside the line's shaped layout. `None` when the
/// line was not shaped (empty text or a shaper failure); callers fall back to
/// the cell grid.
fn shaped_x(shaped: Option<&WrappedLine>, offset: usize, line_height: Pixels) -> Option<Pixels> {
    shaped?
        .position_for_index(offset, line_height)
        .map(|point| point.x)
}

/// Renders the visible editor lines. Bounds and shapes are refreshed in
/// prepaint; the paint pass only draws the cached layout.
pub fn render_input_editor(
    state: Rc<RefCell<InputEditorState>>,
    view: Rc<EditorView>,
    theme: &TerminalTheme,
    visible_lines: usize,
) -> impl IntoElement {
    let font_size = px(theme.font_size);
    let line_height = px(theme.font_size * theme.line_height);
    let visible_lines = visible_lines.max(1);
    let height = px(line_height.as_f32() * visible_lines as f32);

    let prepaint_state = state.clone();
    let prepaint_view = view.clone();
    let prepaint_theme = theme.clone();
    let paint_state = state;
    let paint_theme = theme.clone();

    canvas(
        move |bounds, window, _cx| {
            let font_id = window
                .text_system()
                .resolve_font(&text_font(&prepaint_theme));
            let char_width = window
                .text_system()
                .ch_advance(font_id, font_size)
                .map(|width| width.as_f32())
                .unwrap_or(prepaint_theme.font_size * 0.6)
                .max(1.0);

            let mut shaped = Vec::with_capacity(prepaint_view.lines.len());
            for line in &prepaint_view.lines {
                if line.text.is_empty() {
                    shaped.push(None);
                    continue;
                }
                let runs = [text_run(line.text.len(), &prepaint_theme)];
                match window.text_system().shape_text(
                    line.text.clone().into(),
                    font_size,
                    &runs,
                    None,
                    None,
                ) {
                    Ok(mut lines) => shaped.push(lines.drain(..).next()),
                    Err(error) => {
                        tracing::warn!(error = %error, "editor text shaping failed");
                        shaped.push(None);
                    }
                }
            }

            let mut state = prepaint_state.borrow_mut();
            state.bounds = Some(bounds);
            state.char_width = char_width;
            state.line_height = line_height;
            state.line_count = prepaint_view.lines.len();
            state.shaped = shaped;
        },
        move |bounds, _state, window, cx| {
            let Ok(state) = paint_state.try_borrow() else {
                return;
            };
            let top = bounds.top();
            for (index, line) in view.lines.iter().enumerate() {
                let line_top = top + px(state.line_height.as_f32() * index as f32);
                let shaped = state.shaped.get(index).and_then(Option::as_ref);
                if let Some(selected) = &line.selected_cells {
                    // Shaped offsets keep wide glyphs from drifting; the cell
                    // range is the fallback when shaping produced no layout.
                    let left = line
                        .selected_offsets
                        .as_ref()
                        .and_then(|offsets| shaped_x(shaped, offsets.start, state.line_height))
                        .unwrap_or(px(state.char_width * selected.start as f32));
                    let mut right = line
                        .selected_offsets
                        .as_ref()
                        .and_then(|offsets| shaped_x(shaped, offsets.end, state.line_height))
                        .unwrap_or(px(state.char_width * selected.end as f32));
                    if line.newline_selected {
                        right += px(state.char_width);
                    }
                    let left = bounds.left() + left;
                    let right = bounds.left() + right;
                    if right > left {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(left, line_top),
                                size(right - left, state.line_height),
                            ),
                            paint_theme.selection_background,
                        ));
                    }
                }
                if let Some(shaped) = shaped {
                    let _ = shaped.paint(
                        point(bounds.left(), line_top),
                        state.line_height,
                        TextAlign::Left,
                        Some(bounds),
                        window,
                        cx,
                    );
                }
                if let Some(cell) = line.caret_cell {
                    let caret_x = line
                        .caret_offset
                        .and_then(|offset| shaped_x(shaped, offset, state.line_height))
                        .unwrap_or(px(state.char_width * cell as f32));
                    let caret_left =
                        (bounds.left() + caret_x).min(bounds.right() - px(CARET_WIDTH));
                    window.paint_quad(fill(
                        Bounds::new(
                            point(caret_left, line_top),
                            size(px(CARET_WIDTH), state.line_height),
                        ),
                        paint_theme.foreground,
                    ));
                }
            }
        },
    )
    .w_full()
    .h(height)
}
