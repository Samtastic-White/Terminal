//! Sticky header geometry: a pure resolver fed by the tracked scroll handle.
//!
//! GPUI has no sticky primitive, so the window collects each card's unscrolled
//! layout bounds and asks this module which header to paint over the viewport.

/// Vertical extent of one card in unscrolled layout coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardGeometry {
    pub top: f32,
    pub bottom: f32,
}

/// The card header pinned to the top edge and its upward yield offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pin {
    /// Card index in block order.
    pub index: usize,
    /// `<= 0`; rises as the next card reaches the pinned header.
    pub offset: f32,
}

/// Header height used for the yield calculation. T0 measured the rendered
/// header at ~42 px for the default 14 px / 1.5 theme, while the bare formula
/// gives 37 px; the calibration keeps the value theme-derived.
pub fn header_height(font_size: f32, line_height: f32) -> f32 {
    const HEADER_PADDING_Y: f32 = 8.0;
    const MEASURED_TEXT_EXTRA_Y: f32 = 5.0;
    font_size * line_height + 2.0 * HEADER_PADDING_Y + MEASURED_TEXT_EXTRA_Y
}

/// Returns the card crossing `viewport_top`, if any.
pub fn resolve_pin(
    cards: &[CardGeometry],
    offset_y: f32,
    viewport_top: f32,
    header_height: f32,
) -> Option<Pin> {
    let index = cards
        .iter()
        .rposition(|card| card.top + offset_y <= viewport_top)?;
    if cards[index].bottom + offset_y <= viewport_top {
        return None;
    }

    let next_top = cards.get(index + 1).map(|next| next.top + offset_y);
    let yield_offset =
        next_top.map_or(0.0, |next| (next - (viewport_top + header_height)).min(0.0));

    Some(Pin {
        index,
        offset: yield_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(tops: &[(f32, f32)]) -> Vec<CardGeometry> {
        tops.iter()
            .map(|(top, bottom)| CardGeometry {
                top: *top,
                bottom: *bottom,
            })
            .collect()
    }

    #[test]
    fn empty_geometry_has_no_pin() {
        assert_eq!(resolve_pin(&[], 0.0, 0.0, 40.0), None);
    }

    #[test]
    fn the_first_card_pins_once_its_top_crosses_the_viewport() {
        let cards = cards(&[(10.0, 110.0), (110.0, 210.0)]);

        assert_eq!(resolve_pin(&cards, 0.0, 0.0, 40.0), None);
        assert_eq!(
            resolve_pin(&cards, -10.0, 0.0, 40.0),
            Some(Pin {
                index: 0,
                offset: 0.0
            })
        );
    }

    #[test]
    fn a_gap_above_the_first_crossing_card_has_no_pin() {
        let cards = cards(&[(0.0, 20.0), (40.0, 100.0)]);

        assert_eq!(resolve_pin(&cards, -30.0, 0.0, 40.0), None);
    }

    #[test]
    fn the_pinned_header_yields_to_the_next_card() {
        let cards = cards(&[(0.0, 100.0), (100.0, 200.0)]);

        assert_eq!(
            resolve_pin(&cards, -70.0, 0.0, 42.0),
            Some(Pin {
                index: 0,
                offset: -12.0
            })
        );
    }

    #[test]
    fn the_last_card_never_yields() {
        let cards = cards(&[(0.0, 100.0), (100.0, 200.0)]);

        assert_eq!(
            resolve_pin(&cards, -150.0, 0.0, 42.0),
            Some(Pin {
                index: 1,
                offset: 0.0
            })
        );
    }

    #[test]
    fn stale_geometry_beyond_the_card_list_has_no_pin() {
        let cards = cards(&[(0.0, 100.0)]);

        assert_eq!(resolve_pin(&cards, -150.0, 0.0, 42.0), None);
    }

    #[test]
    fn the_calibrated_header_height_matches_the_measured_default() {
        assert_eq!(header_height(14.0, 1.5), 42.0);
    }
}
