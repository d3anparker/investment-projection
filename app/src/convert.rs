//! Turning the raw form strings into a `calc::CalcInput`.
//!
//! This is the marshalling layer between the reactive UI and the pure `calc`
//! core: it applies the UI's own conventions (blank means zero, the `<select>`
//! string values map to `calc` enums) and — critically — filters out blank rows
//! *before* `calc` sees them. That filtering breaks the index correspondence
//! between `CalcInput::investments` and the rows on screen, so [`build_input`]
//! also returns the surviving rows' ids in order; the memo in `main.rs` pairs
//! that with a [`crate::outcome::Outcome`] so a `calc` error can be mapped back
//! to the right control.
//!
//! All of this is pure (no signals, no DOM), so it is unit-tested natively — the
//! [`FormInput`]/[`RowData`] snapshot the reactive form down to plain strings for
//! exactly that reason.

use calc::{CalcInput, InvestmentInput, Plan, Unit};

/// A plain-string snapshot of one editor row, decoupled from the reactive
/// `Row`'s signals so the input-building logic can be tested without a runtime.
/// `Clone` so it can also carry a projection's form state into [`crate::share`].
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct RowData {
    // The id is positional bookkeeping for the reactive layer, not part of the
    // shared state: the codec drops it on the way out and reassigns it by row
    // position on the way back in, so it never bloats the link.
    #[serde(skip)]
    pub id: usize,
    pub name: String,
    pub value: String,
    pub rate: String,
    pub contribution: String,
}

/// A plain-string snapshot of the whole form, decoupled from the signals. The
/// single argument to [`build_input`], so adding a form-level control means one
/// new field here rather than another positional string parameter. Deliberately
/// *not* `serde` and deliberately without the goal fields: it feeds the
/// projection memo, whose dependency set must stay as narrow as it is — typing in
/// the goal box must not re-run `calculate`.
pub struct FormInput {
    pub rows: Vec<RowData>,
    pub horizon_value: String,
    pub horizon_unit: String,
    /// `"deposits"` or `"drawdown"` — the top-level mode.
    pub plan: String,
    pub drawdown_value: String,
    pub drawdown_unit: String,
    pub withdrawal: String,
}

/// Build the `calc` input from the current form, dropping blank rows. Returns the
/// input alongside the ids of the rows that survived the filter, in order, so a
/// `CalcError`'s index (into `CalcInput::investments`) can be mapped back to the
/// row on screen that caused it.
pub fn build_input(f: &FormInput) -> (CalcInput, Vec<usize>) {
    let mut row_ids = Vec::new();
    let investments: Vec<InvestmentInput> = f
        .rows
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
                rate: blank_zero(&r.rate),
                contribution: blank_zero(&r.contribution),
            })
        })
        .collect();

    let input = CalcInput {
        investments,
        horizon_value: blank_zero(&f.horizon_value),
        horizon_unit: unit_from(&f.horizon_unit),
        plan: plan_from(&f.plan, &f.drawdown_value, &f.drawdown_unit, &f.withdrawal),
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

/// A period unit `<select>`'s string value → `calc::Unit`. Anything other than
/// `"months"` is years (the default option).
pub fn unit_from(s: &str) -> Unit {
    if s == "months" {
        Unit::Months
    } else {
        Unit::Years
    }
}

/// The top-level mode → `calc::Plan`. Anything other than `"drawdown"` (including
/// a blank from a pre-mode shared link) is the deposits default, so the drawdown
/// period and withdrawal are only read when actually drawing down.
pub fn plan_from(plan: &str, drawdown_value: &str, drawdown_unit: &str, withdrawal: &str) -> Plan {
    if plan == "drawdown" {
        Plan::Drawdown {
            drawdown_value: blank_zero(drawdown_value),
            drawdown_unit: unit_from(drawdown_unit),
            // A blank withdrawal is a flat drawdown (zero), which `calc` accepts.
            withdrawal: blank_zero(withdrawal),
        }
    } else {
        Plan::Deposits
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
            rate: rate.into(),
            contribution: contribution.into(),
        }
    }

    fn form(rows: Vec<RowData>, horizon_value: &str, horizon_unit: &str) -> FormInput {
        FormInput {
            rows,
            horizon_value: horizon_value.into(),
            horizon_unit: horizon_unit.into(),
            plan: "deposits".into(),
            drawdown_value: "30".into(),
            drawdown_unit: "years".into(),
            withdrawal: String::new(),
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
        // `calc::Unit` compares with `==`.
        assert!(unit_from("months") == Unit::Months);
        assert!(unit_from("years") == Unit::Years);
        assert!(unit_from("") == Unit::Years);
        assert!(unit_from("decades") == Unit::Years);
    }

    #[test]
    fn plan_from_defaults_to_deposits() {
        assert!(plan_from("deposits", "30", "years", "2000") == Plan::Deposits);
        // Blank (a pre-mode shared link) and anything unknown are deposits.
        assert!(plan_from("", "30", "years", "2000") == Plan::Deposits);
        assert!(plan_from("nonsense", "30", "years", "2000") == Plan::Deposits);
    }

    #[test]
    fn plan_from_carries_the_drawdown_fields() {
        let p = plan_from("drawdown", "30", "years", "2000");
        assert!(
            p == Plan::Drawdown {
                drawdown_value: "30".into(),
                drawdown_unit: Unit::Years,
                withdrawal: "2000".into(),
            }
        );
        // A blank withdrawal becomes "0" (a flat drawdown), never an empty string.
        let blank = plan_from("drawdown", "30", "months", "  ");
        assert!(
            blank == Plan::Drawdown {
                drawdown_value: "30".into(),
                drawdown_unit: Unit::Months,
                withdrawal: "0".into(),
            }
        );
    }

    #[test]
    fn build_input_drops_fully_blank_rows() {
        let rows = vec![
            row(0, "A", "1000", "7", ""),
            row(1, "", "", "", ""), // fully blank -> dropped
            row(2, "C", "", "", "50"), // kept: has a contribution only
        ];
        let (input, ids) = build_input(&form(rows, "10", "years"));
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![0, 2]);
    }

    #[test]
    fn build_input_keeps_a_row_present_by_any_single_field() {
        for (v, r, c) in [("1", "", ""), ("", "5", ""), ("", "", "10")] {
            let (input, ids) = build_input(&form(vec![row(9, "", v, r, c)], "10", "years"));
            assert_eq!(input.investments.len(), 1, "value/rate/contrib = {v:?}/{r:?}/{c:?}");
            assert_eq!(ids, vec![9]);
        }
    }

    #[test]
    fn build_input_row_ids_survive_a_dropped_middle_row() {
        // The load-bearing case: dropping the middle row must not shift the
        // mapping.
        let rows = vec![
            row(11, "A", "1000", "", ""),
            row(12, "", "", "", ""), // dropped
            row(13, "C", "2000", "", ""),
        ];
        let (input, ids) = build_input(&form(rows, "10", "years"));
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![11, 13]);
    }

    #[test]
    fn build_input_defaults_blanks_and_names() {
        let (input, _) = build_input(&form(vec![row(0, "   ", "1000", "", "")], "  ", "years"));
        let inv = &input.investments[0];
        assert_eq!(inv.name, "Investment");
        assert_eq!(inv.rate, "0");
        assert_eq!(inv.contribution, "0");
        assert_eq!(inv.value, "1000");
        assert_eq!(input.horizon_value, "0");
    }

    #[test]
    fn build_input_threads_the_drawdown_plan_through() {
        let mut f = form(vec![row(0, "A", "1000", "7", "")], "10", "years");
        f.plan = "drawdown".into();
        f.withdrawal = "2000".into();
        let (input, _) = build_input(&f);
        assert!(
            input.plan == Plan::Drawdown {
                drawdown_value: "30".into(),
                drawdown_unit: Unit::Years,
                withdrawal: "2000".into(),
            }
        );
    }

    #[test]
    fn build_input_passes_horizon_through() {
        let (input, _) = build_input(&form(vec![row(0, "A", "1000", "7", "")], "36", "months"));
        assert_eq!(input.horizon_value, "36");
        assert!(input.horizon_unit == Unit::Months);
    }
}
