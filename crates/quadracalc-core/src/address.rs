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

    /// The inverse of [`Self::a1`]: `AB10` → `(9, 27)`. `None` for
    /// anything that isn't letters then a 1-based row number (a range,
    /// a sheet-qualified or `$`-anchored reference).
    pub fn parse(a1: &str) -> Option<Self> {
        let split = a1.find(|c: char| c.is_ascii_digit())?;
        let (letters, digits) = a1.split_at(split);
        if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let row: u32 = digits.parse().ok()?;
        let col = letters.chars().try_fold(0u32, |n, c| {
            n.checked_mul(26)?
                .checked_add(u32::from(c.to_ascii_uppercase() as u8 - b'A') + 1)
        })?;
        Some(Self::new(row.checked_sub(1)?, col - 1))
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
    fn parse_round_trips_a1() {
        for a in [
            Address::new(0, 0),
            Address::new(9, 27),
            Address::new(41, 702),
        ] {
            assert_eq!(Address::parse(&a.a1()), Some(a));
        }
        assert_eq!(Address::parse("b3"), Some(Address::new(2, 1)));
        for bad in ["", "A", "12", "A0", "A1:B2", "$A$1", "Sheet1!A1"] {
            assert_eq!(Address::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a1_is_one_based_rows() {
        assert_eq!(Address::new(0, 0).a1(), "A1");
        assert_eq!(Address::new(9, 27).a1(), "AB10");
    }
}
