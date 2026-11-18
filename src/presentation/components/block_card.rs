//! Command block card: chrome, header, state pill, clipboard actions, flat
//! styled body with free selection and local scrolling.

mod actions;
mod body;
mod snapshot;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use actions::ActionScope;
pub use state::BlockUiState;
pub use view::{CardCallbacks, CopyHandler, SelectionHandler, render_block_card};

pub(crate) use view::render_header;

#[cfg(test)]
pub use actions::CARD_ACTIONS;
#[cfg(test)]
pub use body::{build_body_text, clamp_selection};
#[cfg(test)]
pub use snapshot::{
    CardSnapshot, StatusTone, card_snapshot, command_preview, status_label, status_tone,
};
