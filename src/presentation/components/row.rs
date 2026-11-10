//! Row rendering component — converts domain rows/spans into GPUI elements.

use gpui::*;

use crate::domain::output::Row;
use crate::presentation::theme::TerminalTheme;

/// Renders one committed row as a run of styled spans. Rows are already
/// wrapped by the writer, so no re-flow is applied here.
fn render_row(row: &Row, theme: &TerminalTheme) -> impl IntoElement {
    let mut line = div()
        .flex()
        .flex_row()
        .items_start()
        .whitespace_nowrap()
        .min_h(px(theme.font_size * theme.line_height))
        .text_color(theme.foreground);

    if row.spans.is_empty() {
        return line.child(div().h(px(theme.font_size * theme.line_height)));
    }

    for span in &row.spans {
        let mut span_element = div()
            .flex()
            .items_start()
            .text_color(theme.resolve_color(&span.style.fg))
            .child(span.text.clone());

        if let Some(background) = theme.resolve_bg(&span.style.bg) {
            span_element = span_element.bg(background);
        }
        if span.style.bold {
            span_element = span_element.font_weight(FontWeight::BOLD);
        }
        if span.style.italic {
            span_element = span_element.italic();
        }
        if span.style.underline {
            span_element = span_element.underline();
        }

        line = line.child(span_element);
    }

    line
}

/// Renders the bounded session document. The window sets the font metrics
/// once for the whole block; spans only override colors and attributes.
pub fn render_rows(
    rows: &std::collections::VecDeque<Row>,
    theme: &TerminalTheme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_start()
        .font_family(theme.font_family)
        .text_size(px(theme.font_size))
        .line_height(px(theme.font_size * theme.line_height))
        .children(rows.iter().map(|row| render_row(row, theme)))
}
