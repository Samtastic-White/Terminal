//! Drop-oldest bounded row buffer shared by blocks and prompt rings.

use std::collections::VecDeque;

use crate::domain::output::Row;

/// A row buffer that discards the oldest row once its cap is reached.
#[derive(Debug, Clone)]
pub struct BoundedRows {
    rows: VecDeque<Row>,
    cap: usize,
}

impl BoundedRows {
    /// A cap of zero keeps the buffer permanently empty.
    pub fn new(cap: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            cap,
        }
    }

    /// Appends a row, returning true when the oldest row was evicted.
    pub fn push(&mut self, row: Row) -> bool {
        if self.cap == 0 {
            return false;
        }

        let evicted = self.rows.len() == self.cap;
        if evicted {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
        evicted
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Row> {
        self.rows.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::output::{Row, Span, TextStyle};

    fn row(text: &str) -> Row {
        Row {
            spans: vec![Span {
                style: TextStyle::default(),
                text: text.into(),
            }],
        }
    }

    fn texts(rows: &BoundedRows) -> Vec<String> {
        rows.iter()
            .flat_map(|row| row.spans.iter().map(|span| span.text.clone()))
            .collect()
    }

    #[test]
    fn keeps_rows_below_the_cap() {
        let mut rows = BoundedRows::new(3);

        assert!(!rows.push(row("a")));
        assert!(!rows.push(row("b")));

        assert_eq!(rows.len(), 2);
        assert_eq!(texts(&rows), vec!["a", "b"]);
    }

    #[test]
    fn drops_the_oldest_row_at_the_cap() {
        let mut rows = BoundedRows::new(2);

        rows.push(row("a"));
        rows.push(row("b"));
        let evicted = rows.push(row("c"));

        assert!(evicted);
        assert_eq!(rows.len(), 2);
        assert_eq!(texts(&rows), vec!["b", "c"]);
    }

    #[test]
    fn a_zero_cap_never_stores_rows() {
        let mut rows = BoundedRows::new(0);

        assert!(!rows.push(row("a")));

        assert!(rows.is_empty());
    }

    #[test]
    fn repeated_pushes_never_exceed_the_cap() {
        let mut rows = BoundedRows::new(3);

        for index in 0..10_000 {
            rows.push(row(&index.to_string()));
        }

        assert_eq!(rows.len(), 3);
        assert_eq!(texts(&rows), vec!["9997", "9998", "9999"]);
    }

    #[test]
    fn clear_empties_the_buffer() {
        let mut rows = BoundedRows::new(2);
        rows.push(row("a"));

        rows.clear();

        assert!(rows.is_empty());
    }
}
