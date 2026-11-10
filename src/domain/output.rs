#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb { r: u8, g: u8, b: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl TextStyle {
    #[allow(dead_code)]
    pub fn is_plain(&self) -> bool {
        self.fg == Color::Default
            && self.bg == Color::Default
            && !self.bold
            && !self.italic
            && !self.underline
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub spans: Vec<Span>,
}

impl Row {
    #[allow(dead_code)]
    pub fn is_blank(&self) -> bool {
        self.spans.iter().all(|span| span.text.is_empty())
    }
}
