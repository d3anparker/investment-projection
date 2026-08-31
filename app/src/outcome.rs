//! One recomputation's result, plus the error→control mapping the UI needs.
//!
//! `calc` reports errors against an index into the investments it was given, but
//! blank rows are filtered out before it sees them (see [`crate::convert`]) — so
//! index 1 is not necessarily the second row on screen. [`Outcome`] carries the
//! surviving `row_ids` so an error can be translated back to the control that
//! caused it (`aria-invalid` + `aria-describedby` + `.field-invalid`).
//!
//! Pure (depends only on `calc` types, no leptos/DOM), so the mapping is
//! unit-tested natively.

use calc::{CalcError, CalcOutput, Field, InvestmentField};
use std::time::Duration;

/// Id of the visible error paragraph. Invalid controls point at it with
/// `aria-describedby`, so the message is read out with the field rather than
/// stranded at the bottom of the form.
pub const ERROR_ID: &str = "calc-error";

/// How long typing must settle before the error is announced. Long enough that
/// a keystroke mid-word doesn't queue a message, short enough to feel prompt.
pub const ANNOUNCE_DELAY: Duration = Duration::from_millis(700);

/// One recomputation's result plus the mapping needed to interpret it.
#[derive(Clone, PartialEq)]
pub struct Outcome {
    pub result: Result<CalcOutput, CalcError>,
    pub row_ids: Vec<usize>,
}

impl Outcome {
    pub fn error(&self) -> Option<&CalcError> {
        self.result.as_ref().err()
    }

    pub fn message(&self) -> Option<String> {
        self.error().map(|e| e.message.clone())
    }

    /// Is the current error about this row's `part`? Maps the error's index
    /// (into the filtered investments) back through `row_ids` to a row id.
    pub fn flags(&self, row_id: usize, part: InvestmentField) -> bool {
        match self.error().and_then(|e| e.field) {
            Some(Field::Investment { index, part: failed }) => {
                failed == part && self.row_ids.get(index).copied() == Some(row_id)
            }
            _ => false,
        }
    }

    /// Is the current error about this form-level control?
    ///
    /// One method for every whole-form field (horizon, drawdown, withdrawal,
    /// other income, age, region, strategy, uprate), rather than eight one-line
    /// copies that differed only in the variant. The per-row investment fields
    /// keep their own [`flags`](Self::flags), which additionally has to map an
    /// index back through `row_ids`. `Field` is `Copy + Eq`, so a caller passes
    /// the variant it means: `o.flags_field(Field::Age)`.
    pub fn flags_field(&self, field: Field) -> bool {
        self.error().and_then(|e| e.field) == Some(field)
    }
}

/// `aria-invalid`/`aria-describedby` values for a control, or `None` to leave
/// both attributes off entirely.
pub fn invalid_attrs(flagged: bool) -> (Option<&'static str>, Option<&'static str>) {
    if flagged {
        (Some("true"), Some(ERROR_ID))
    } else {
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calc::{calculate, CalcInput, InvestmentInput, Plan, Unit};

    fn err_outcome(field: Option<Field>, row_ids: Vec<usize>) -> Outcome {
        Outcome {
            result: Err(CalcError { message: "boom".into(), field }),
            row_ids,
        }
    }

    fn ok_outcome() -> Outcome {
        let input = CalcInput {
            investments: vec![InvestmentInput {
                name: "A".into(),
                value: "1000".into(),
                rate: "7".into(),
                contribution: "0".into(),
                ..Default::default()
            }],
            horizon_value: "10".into(),
            horizon_unit: Unit::Years,
            plan: Plan::Deposits,
            currency: String::new(),
            tax: None,
        };
        Outcome { result: calculate(&input), row_ids: vec![0] }
    }

    #[test]
    fn flags_maps_error_index_through_row_ids() {
        // calc blamed investment index 1; row_ids says that is the on-screen row
        // with id 42. The middle row (id 7) was filtered out before calc ran.
        let out = err_outcome(
            Some(Field::Investment { index: 1, part: InvestmentField::Value }),
            vec![7, 42],
        );
        assert!(out.flags(42, InvestmentField::Value));
        // Wrong row id -> not flagged, even though the part matches.
        assert!(!out.flags(7, InvestmentField::Value));
        // Right row, wrong part -> not flagged.
        assert!(!out.flags(42, InvestmentField::Rate));
    }

    #[test]
    fn flags_false_when_index_out_of_range() {
        let out = err_outcome(
            Some(Field::Investment { index: 5, part: InvestmentField::Rate }),
            vec![0, 1],
        );
        assert!(!out.flags(0, InvestmentField::Rate));
        assert!(!out.flags(1, InvestmentField::Rate));
    }

    #[test]
    fn flags_field_only_for_the_matching_field() {
        assert!(err_outcome(Some(Field::Horizon), vec![]).flags_field(Field::Horizon));
        assert!(!err_outcome(
            Some(Field::Investment { index: 0, part: InvestmentField::Value }),
            vec![0]
        )
        .flags_field(Field::Horizon));
        assert!(!err_outcome(None, vec![]).flags_field(Field::Horizon));
    }

    #[test]
    fn flags_field_distinguishes_neighbouring_controls() {
        assert!(err_outcome(Some(Field::Drawdown), vec![]).flags_field(Field::Drawdown));
        assert!(!err_outcome(Some(Field::Drawdown), vec![]).flags_field(Field::Withdrawal));
        assert!(err_outcome(Some(Field::Withdrawal), vec![]).flags_field(Field::Withdrawal));
        assert!(!err_outcome(Some(Field::Withdrawal), vec![]).flags_field(Field::Drawdown));
        assert!(!err_outcome(Some(Field::Horizon), vec![]).flags_field(Field::Drawdown));
    }

    #[test]
    fn a_successful_outcome_flags_nothing() {
        let out = ok_outcome();
        assert!(out.error().is_none());
        assert!(out.message().is_none());
        assert!(!out.flags(0, InvestmentField::Value));
        assert!(!out.flags_field(Field::Horizon));
    }

    #[test]
    fn message_surfaces_the_error_text() {
        let out = err_outcome(Some(Field::Horizon), vec![]);
        assert_eq!(out.message().as_deref(), Some("boom"));
    }

    #[test]
    fn invalid_attrs_pairs_true_with_the_error_id() {
        assert_eq!(invalid_attrs(true), (Some("true"), Some(ERROR_ID)));
        assert_eq!(invalid_attrs(false), (None, None));
    }
}
