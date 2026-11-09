//! Pure output primitives: the interpreted terminal stream is stored as
//! structured styled spans, never as a raw monolithic `String`
//! (architecture Common Mistakes #4).
//!
//! These types are plain data — Presentation converts them to GPUI
//! rendering; nothing here knows about gpui, PTY, or escapes.

/// Terminal color model covering the Phase 1 writer subset (spec BR-5):
/// default, palette indices (16-color included), and 24-bit RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// Terminal default color (SGR 39 / 49 reset).
    #[default]
    Default,
    /// Palette index: `0..=15` are the 16 ANSI colors, `16..=255` the
    /// xterm 256 palette.
    Indexed(u8),
    /// True (24-bit) color.
    Rgb { r: u8, g: u8, b: u8 },
}

/// Text style of a span: color pair plus the attribute flags the subset
/// supports (bold / italic / underline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl TextStyle {
    /// True when nothing is set: colors default, no attributes. Fast path
    /// for rendering unstyled runs.
    #[allow(dead_code)]
    pub fn is_plain(&self) -> bool {
        self.fg == Color::Default
            && self.bg == Color::Default
            && !self.bold
            && !self.italic
            && !self.underline
    }
}

/// One styled run of terminal text. SGR attribute changes split spans;
/// erase and rewrite machinery never cross a span boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub style: TextStyle,
    pub text: String,
}

impl Span {
    #[allow(dead_code)]
    pub fn new(style: TextStyle, text: impl Into<String>) -> Self {
        Self {
            style,
            text: text.into(),
        }
    }
}

/// One committed wrap-unit line: what the writer produced at a given PTY
/// width.
///
/// Rows keep the wrap width they were created with — no retroactive
/// re-wrap on window resize (spec BR-7; reflow is a Phase 4 behavior).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub spans: Vec<Span>,
}

impl Row {
    /// A row is blank when it holds no spans, or only empty-text spans.
    #[allow(dead_code)]
    pub fn is_blank(&self) -> bool {
        self.spans.iter().all(|span| span.text.is_empty())
    }
}
