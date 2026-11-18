//! Clipboard actions: the per-block copy buttons rendered in the header.

use gpui::*;

use super::view::CopyHandler;
use crate::domain::block::{BlockId, CopyKind};
use crate::presentation::theme::{TerminalTheme, colors};

/// One clipboard surface offered by a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardAction {
    pub kind: CopyKind,
    pub label: &'static str,
}

/// Whole block, command line, output — the three clipboard surfaces.
pub const CARD_ACTIONS: [CardAction; 3] = [
    CardAction {
        kind: CopyKind::WholeBlock,
        label: "Copy",
    },
    CardAction {
        kind: CopyKind::Command,
        label: "Cmd",
    },
    CardAction {
        kind: CopyKind::Output,
        label: "Out",
    },
];

/// Where a header is painted. The sticky overlay and the in-flow card render
/// the same actions, so their element IDs must stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionScope {
    Flow,
    Pinned,
}

impl ActionScope {
    fn suffix(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Pinned => "pinned",
        }
    }
}

/// Element id unique per block, action and scope; IDs only need to be stable
/// within the header's surface.
pub(super) fn action_id(kind: CopyKind, block: BlockId, scope: ActionScope) -> ElementId {
    let name = match kind {
        CopyKind::WholeBlock => "copy-block",
        CopyKind::Command => "copy-command",
        CopyKind::Output => "copy-output",
    };
    let base: ElementId = (name, block.get()).into();
    (base, scope.suffix()).into()
}

pub(super) fn render_action(
    block: BlockId,
    action: CardAction,
    scope: ActionScope,
    group: &SharedString,
    theme: &TerminalTheme,
    on_copy: &CopyHandler,
) -> impl IntoElement {
    let on_copy = on_copy.clone();
    div()
        .id(action_id(action.kind, block, scope))
        .invisible()
        .group_hover(group.clone(), |style| style.visible())
        .px(px(6.0))
        .py(px(2.0))
        .rounded_sm()
        .text_size(px(theme.font_size * 0.78))
        .text_color(theme.card_meta)
        .cursor_pointer()
        .hover(|style| style.bg(colors::SURFACE1).text_color(theme.foreground))
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            on_copy(action.kind, window, cx);
        })
        .child(action.label)
}
