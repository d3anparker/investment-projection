//! Turning the raw form strings into a `calc::CalcInput`.
//!
//! This is the marshalling layer between the reactive UI and the pure `calc`
//! core: it applies the UI's own conventions (blank means zero, the two
//! `<select>` string values map to `calc` enums) and — critically — filters out
//! blank rows *before* `calc` sees them. That filtering breaks the index
//! correspondence between `CalcInput::investments` and the rows on screen, so
//! [`build_input`] also returns the surviving rows' ids in order; the memo in
//! `main.rs` pairs that with a [`crate::outcome::Outcome`] so a `calc` error can
//! be mapped back to the right control.
//!
//! All of this is pure (no signals, no DOM), so it is unit-tested natively — the
//! `RowData` snapshots the reactive `Row` down to plain strings for exactly that
//! reason.

use calc::{CalcInput, InvestmentInput, Mode, Unit};

/// A plain-string snapshot of one editor row, decoupled from the reactive
/// `Row`'s signals so the input-building logic can be tested without a runtime.
pub struct RowData {
    pub id: usize,
    pub name: String,
    pub value: String,
    pub mode: String,
    pub rate: String,
    pub contribution: String,
}

/// Build the `calc` input from the current rows, dropping blank ones. Returns
/// the input alongside the ids of the rows that survived the filter, in order,
/// so a `CalcError`'s index (into `CalcInput::investments`) can be mapped back to
/// the row on screen that caused it.
pub fn build_input(
    rows: &[RowData],
    horizon_value: &str,
    horizon_unit: &str,
) -> (CalcInput, Vec<usize>) {
    let mut row_ids = Vec::new();
    let investments: Vec<InvestmentInput> = rows
        .iter()
        .filter_map(|r| {
            // Skip blank rows so a half-typed row doesn't error the form. A row
            // counts as present if it has *any* of value/rate/contribution.
            if r.value.trim().is_empty()
                && r.rate.trim().is_empty()
                && r.contribution.trim().is_empty()
            {
                return None;
            }
            row_ids.push(r.id);
            Some(InvestmentInput {
                name: if r.name.trim().is_empty() {
                    "Investment".into()
                } else {
                    r.name.clone()
                },
                value: blank_zero(&r.value),
                mode: mode_from(&r.mode),
                rate: blank_zero(&r.rate),
                contribution: blank_zero(&r.contribution),
            })
        })
        .collect();

    let input = CalcInput {
        investments,
        horizon_value: blank_zero(horizon_value),
        horizon_unit: unit_from(horizon_unit),
    };
    (input, row_ids)
}

/// A blank (or whitespace-only) field means "nothing here" — send `calc` a
/// literal `"0"` so it parses rather than erroring on an empty string.
pub fn blank_zero(s: &str) -> String {
    if s.trim().is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// The horizon unit `<select>`'s string value → `calc::Unit`. Anything other
/// than `"months"` is years (the default option).
pub fn unit_from(s: &str) -> Unit {
    if s == "months" {
        Unit::Months
    } else {
        Unit::Years
    }
}

/// A row's return-basis `<select>` value → `calc::Mode`. Anything other than
/// `"total"` is the annualised default.
pub fn mode_from(s: &str) -> Mode {
    if s == "total" {
        Mode::Total
    } else {
        Mode::Annual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: usize, name: &str, value: &str, rate: &str, contribution: &str) -> RowData {
        RowData {
            id,
            name: name.into(),
            value: value.into(),
            mode: "annual".into(),
            rate: rate.into(),
            contribution: contribution.into(),
        }
    }

    #[test]
    fn blank_zero_maps_empty_and_whitespace_to_zero() {
        assert_eq!(blank_zero(""), "0");
        assert_eq!(blank_zero("   "), "0");
        assert_eq!(blank_zero("\t"), "0");
        // Anything non-blank passes straight through, untrimmed.
        assert_eq!(blank_zero("10000"), "10000");
        assert_eq!(blank_zero(" 10,000 "), " 10,000 ");
    }

    #[test]
    fn unit_from_defaults_to_years() {
        // `calc::Unit`/`Mode` don't derive `Debug`, so compare with `==` rather
        // than `assert_eq!` (which would need it for its failure message).
        assert!(unit_from("months") == Unit::Months);
        assert!(unit_from("years") == Unit::Years);
        // Unknown strings fall back to the default option, not an error.
        assert!(unit_from("") == Unit::Years);
        assert!(unit_from("decades") == Unit::Years);
    }

    #[test]
    fn mode_from_defaults_to_annual() {
        assert!(mode_from("total") == Mode::Total);
        assert!(mode_from("annual") == Mode::Annual);
        assert!(mode_from("") == Mode::Annual);
        assert!(mode_from("yearly") == Mode::Annual);
    }

    #[test]
    fn build_input_drops_fully_blank_rows() {
        let rows = vec![
            row(0, "A", "1000", "7", ""),
            row(1, "", "", "", ""), // fully blank -> dropped
            row(2, "C", "", "", "50"), // kept: has a contribution only
        ];
        let (input, ids) = build_input(&rows, "10", "years");
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![0, 2]);
    }

    #[test]
    fn build_input_keeps_a_row_present_by_any_single_field() {
        // Each row is non-blank via exactly one of value / rate / contribution.
        for (v, r, c) in [("1", "", ""), ("", "5", ""), ("", "", "10")] {
            let (input, ids) = build_input(&[row(9, "", v, r, c)], "10", "years");
            assert_eq!(input.investments.len(), 1, "value/rate/contrib = {v:?}/{r:?}/{c:?}");
            assert_eq!(ids, vec![9]);
        }
    }

    #[test]
    fn build_input_row_ids_survive_a_dropped_middle_row() {
        // The load-bearing case: dropping the middle row must not shift the
        // mapping. calc will index investments 0,1; row_ids must map those back
        // to the on-screen ids 11 and 13, skipping the blank 12.
        let rows = vec![
            row(11, "A", "1000", "", ""),
            row(12, "", "", "", ""), // dropped
            row(13, "C", "2000", "", ""),
        ];
        let (input, ids) = build_input(&rows, "10", "years");
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![11, 13]);
    }

    #[test]
    fn build_input_defaults_blanks_and_names() {
        let rows = vec![row(0, "   ", "1000", "", "")];
        let (input, _) = build_input(&rows, "  ", "years");
        let inv = &input.investments[0];
        // Blank name -> the placeholder calc labels errors with.
        assert_eq!(inv.name, "Investment");
        // Blank numeric fields -> "0", not empty (which calc would reject).
        assert_eq!(inv.rate, "0");
        assert_eq!(inv.contribution, "0");
        // Present field passes through verbatim.
        assert_eq!(inv.value, "1000");
        // Blank horizon -> "0" too.
        assert_eq!(input.horizon_value, "0");
    }

    #[test]
    fn build_input_passes_horizon_through() {
        let (input, _) = build_input(&[row(0, "A", "1000", "7", "")], "36", "months");
        assert_eq!(input.horizon_value, "36");
        assert!(input.horizon_unit == Unit::Months);
    }
}
