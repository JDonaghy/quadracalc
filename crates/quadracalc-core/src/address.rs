//! A1-style cell addresses: zero-based row/column on our side, A1 text on the
//! engine's side.

/// A zero-based cell position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Address {
    pub row: u32,
    pub col: u32,
}

impl Address {
    pub fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// The engine's A1 form, e.g. `(0, 0)` → `A1`, `(9, 27)` → `AB10`.
    pub fn a1(self) -> String {
        format!("{}{}", column_label(self.col), self.row + 1)
    }
}

/// Spreadsheet column letters: 0 → `A`, 25 → `Z`, 26 → `AA`.
pub fn column_label(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII letters")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_labels_roll_over_past_z() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(27), "AB");
        assert_eq!(column_label(701), "ZZ");
        assert_eq!(column_label(702), "AAA");
    }

    #[test]
    fn a1_is_one_based_rows() {
        assert_eq!(Address::new(0, 0).a1(), "A1");
        assert_eq!(Address::new(9, 27).a1(), "AB10");
    }
}
