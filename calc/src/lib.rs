//! Investment projection core.
//!
//! Pure, exact base-10 `Decimal` arithmetic (via `rust_decimal`) — no UI, no
//! WASM bindings, no floating point. The Leptos front end calls [`calculate`]
//! directly with these types and only *formats* the `Decimal`s it gets back; it
//! performs no financial arithmetic of its own.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use std::str::FromStr;

/// The unit a period value is expressed in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Years,
    Months,
}

/// How a row's `rate` should be interpreted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `rate` is the cumulative total return (%) expected over the whole
    /// projection horizon.
    Total,
    /// `rate` is the annualised return (%).
    Annual,
}

/// One investment as entered in the UI. Numbers arrive as strings (exactly as
/// typed) and are parsed here, so parsing and validation live in one place.
pub struct InvestmentInput {
    pub name: String,
    /// Today's value of the whole holding (principal plus any historical
    /// compounding already baked in). This is the figure projected forward.
    pub value: String,
    pub mode: Mode,
    pub rate: String,
    /// Optional recurring amount added to this holding every month going
    /// forward (an ongoing monthly investment). Blank/`"0"` means none.
    pub contribution: String,
}

pub struct CalcInput {
    pub investments: Vec<InvestmentInput>,
    pub horizon_value: String,
    pub horizon_unit: Unit,
}

/// Which part of an investment row an error belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestmentField {
    Value,
    Rate,
    Contribution,
}

/// The specific input a [`CalcError`] is about, so the UI can mark that control
/// invalid and describe it by the message rather than stranding a sentence at
/// the bottom of the form. `Investment::index` indexes
/// [`CalcInput::investments`] — the caller filters blank rows out before calling,
/// so it is the caller's job to map that back to its own row identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Investment { index: usize, part: InvestmentField },
    Horizon,
}

/// A validation or overflow failure. `field` is `None` when the problem is with
/// the portfolio as a whole rather than one control the user could go and fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalcError {
    pub message: String,
    pub field: Option<Field>,
}

impl CalcError {
    fn new(message: impl Into<String>, field: Option<Field>) -> Self {
        Self { message: message.into(), field }
    }

    fn at(message: impl Into<String>, index: usize, part: InvestmentField) -> Self {
        Self::new(message, Some(Field::Investment { index, part }))
    }
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Per-investment results. Monetary figures are rounded to 2 dp; `annualised`
/// is a growth *fraction* (e.g. 0.07 for 7%), left unrounded.
#[derive(Clone, Debug, PartialEq)]
pub struct InvestmentResult {
    pub name: String,
    pub current_value: Decimal,
    pub annualised: Decimal,
    /// Total this holding's monthly top-ups add over the horizon. Reported per
    /// row so `projected_value` reconciles: without it a holding with top-ups
    /// looks like `current_value` grew at `annualised`, which it did not.
    pub contributed: Decimal,
    pub projected_value: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalcOutput {
    pub investments: Vec<InvestmentResult>,
    /// Portfolio total value at each month from 0 (today) to the horizon.
    pub series: Vec<Decimal>,
    /// Cumulative contributions deposited by each month from 0 (today) to the
    /// horizon. Starts at 0 and ends at `contributed_total`; parallels `series`.
    pub contributions_series: Vec<Decimal>,
    pub horizon_months: u32,
    pub current_total: Decimal,
    /// Total of all future monthly contributions added over the horizon.
    pub contributed_total: Decimal,
    pub projected_total: Decimal,
    /// Projected investment gain: the final value less today's value *and* less
    /// the money you contribute along the way, so it reflects returns only.
    pub growth: Decimal,
    /// `growth` as a fraction of the capital deployed (today's value plus total
    /// contributions). A simple return on capital, not an IRR.
    pub growth_pct: Decimal,
    /// The capital `growth_pct` is measured against: `current_total +
    /// contributed_total`. Reported so the UI can state the basis instead of
    /// leaving a bare percentage the reader has to guess the denominator for.
    pub deployed: Decimal,
}

/// Project a portfolio forward. Returns a user-facing message on any invalid
/// input rather than panicking.
pub fn calculate(input: &CalcInput) -> Result<CalcOutput, CalcError> {
    if input.investments.is_empty() {
        return Err(CalcError::new("Add at least one investment.", None));
    }

    let hundred = Decimal::from(100u32);
    let twelve = Decimal::from(12u32);

    let horizon_months = to_months(&input.horizon_value, input.horizon_unit, "The projection horizon")
        .map_err(|m| CalcError::new(m, Some(Field::Horizon)))?;
    if horizon_months < 1 {
        return Err(CalcError::new(
            "Enter a projection horizon of at least 1 month.",
            Some(Field::Horizon),
        ));
    }
    // Guard against runaway series sizes.
    if horizon_months > 1200 {
        return Err(CalcError::new(
            "Projection horizon is limited to 100 years (1200 months).",
            Some(Field::Horizon),
        ));
    }
    let horizon = horizon_months as usize;

    // Total return (when used) is spread over the whole horizon.
    let horizon_dec = Decimal::from(horizon_months);

    let mut results: Vec<InvestmentResult> = Vec::with_capacity(input.investments.len());
    // Running portfolio total for each projected month.
    let mut totals: Vec<Decimal> = vec![Decimal::ZERO; horizon + 1];
    // Cumulative contributions deposited by each month, parallel to `totals`.
    let mut contribs: Vec<Decimal> = vec![Decimal::ZERO; horizon + 1];
    let mut contributed_total = Decimal::ZERO;

    for (index, inv) in input.investments.iter().enumerate() {
        use InvestmentField::{Contribution, Rate, Value};
        let too_large = |part| CalcError::at(too_large_msg(&inv.name), index, part);

        // Today's value of the holding — projected forward as-is (no historical
        // compounding: any past growth is already reflected in this figure).
        let current_value = parse_number(&inv.value)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid amount.", inv.name), index, Value))?;
        if current_value < Decimal::ZERO {
            return Err(CalcError::at(
                format!("'{}' has a negative amount.", inv.name),
                index,
                Value,
            ));
        }

        // Optional ongoing monthly contribution (blank means none).
        let contribution = parse_number(&inv.contribution).ok_or_else(|| {
            CalcError::at(
                format!("'{}' has an invalid monthly contribution.", inv.name),
                index,
                Contribution,
            )
        })?;
        if contribution < Decimal::ZERO {
            return Err(CalcError::at(
                format!("'{}' has a negative monthly contribution.", inv.name),
                index,
                Contribution,
            ));
        }

        let rate = parse_number(&inv.rate)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid rate.", inv.name), index, Rate))?
            / hundred;

        // 1 + rate must stay positive for real-valued compounding / roots.
        if rate <= Decimal::NEGATIVE_ONE {
            return Err(CalcError::at(
                format!("'{}' has a return of -100% or worse, which cannot be projected.", inv.name),
                index,
                Rate,
            ));
        }

        // Derive the annualised growth fraction that drives the projection. An
        // "annualised" input is used directly; a "total return" input is the
        // total expected over the whole horizon, so we spread it into an
        // equivalent annualised rate: (1 + total)^(12 / horizon_months) - 1.
        // All arithmetic below is *checked*: extreme-but-reachable inputs (e.g.
        // 100% annualised over 100 years) exceed the Decimal maximum, and an
        // unchecked `*`/`powd` would panic. On overflow we return a clear error.
        // Overflow here is always driven by the return being too steep for the
        // horizon, so these point the user at the rate — the field the message
        // actually asks them to lower.
        let one_plus = Decimal::ONE + rate;
        let annual = match inv.mode {
            Mode::Total => one_plus
                .checked_powd(twelve / horizon_dec)
                .ok_or_else(|| too_large(Rate))?
                - Decimal::ONE,
            Mode::Annual => rate,
        };

        // One `powd` per investment for the monthly factor, then a cheap
        // iterative multiply to build the series.
        let monthly = (Decimal::ONE + annual)
            .checked_powd(Decimal::ONE / twelve)
            .ok_or_else(|| too_large(Rate))?;

        let mut value = current_value;
        let mut projected = current_value;
        // Contributions this holding has deposited by the current month (none at
        // month 0; one more added at each month end below).
        let mut inv_contributed = Decimal::ZERO;
        for (i, month) in totals.iter_mut().enumerate() {
            *month = month
                .checked_add(value)
                .ok_or_else(|| too_large(Rate))?;
            contribs[i] = contribs[i]
                .checked_add(inv_contributed)
                .ok_or_else(|| too_large(Contribution))?;
            if i == horizon {
                projected = value;
            } else {
                // Advance one month: compound the running value, then add this
                // month's contribution at month end. Skip past the horizon so a
                // value we never use can't spuriously overflow at the endpoint.
                value = value
                    .checked_mul(monthly)
                    .ok_or_else(|| too_large(Rate))?
                    .checked_add(contribution)
                    .ok_or_else(|| too_large(Contribution))?;
                inv_contributed = inv_contributed
                    .checked_add(contribution)
                    .ok_or_else(|| too_large(Contribution))?;
                contributed_total = contributed_total
                    .checked_add(contribution)
                    .ok_or_else(|| too_large(Contribution))?;
            }
        }

        results.push(InvestmentResult {
            name: inv.name.clone(),
            current_value: round2(current_value),
            annualised: annual,
            contributed: round2(inv_contributed),
            projected_value: round2(projected),
        });
    }

    let series: Vec<Decimal> = totals.iter().map(|v| round2(*v)).collect();
    let contributions_series: Vec<Decimal> = contribs.iter().map(|v| round2(*v)).collect();
    let current_total = round2(*totals.first().expect("horizon >= 1 guarantees a point"));
    let projected_total = round2(*totals.last().expect("horizon >= 1 guarantees a point"));
    let contributed_total = round2(contributed_total);
    // Gain from returns only: strip out both today's value and the money the
    // user adds over the horizon. Percentage is measured against the capital
    // actually deployed (today's value plus all contributions).
    //
    // Checked: each total is individually capped near the Decimal maximum by the
    // loop above, so the portfolio-summary subtraction can underflow past the
    // minimum and the addition can overflow the maximum. An unchecked `+`/`-`
    // would panic, so on overflow we return an error instead.
    let growth = projected_total
        .checked_sub(current_total)
        .and_then(|g| g.checked_sub(contributed_total))
        .ok_or_else(|| CalcError::new(portfolio_too_large(), None))?;
    let deployed = current_total
        .checked_add(contributed_total)
        .ok_or_else(|| CalcError::new(portfolio_too_large(), None))?;
    // `checked_div` folds both edge cases into one: a zero divisor (no capital
    // deployed) and an overflowing quotient (pennies of capital against a
    // near-maximum projection) each yield `None`, so both report 0% rather than
    // panicking — honouring the never-panic contract.
    let growth_pct = growth.checked_div(deployed).unwrap_or(Decimal::ZERO);

    Ok(CalcOutput {
        investments: results,
        series,
        contributions_series,
        horizon_months,
        current_total,
        contributed_total,
        projected_total,
        growth,
        growth_pct,
        deployed,
    })
}

/// Convert a `value` + `unit` into whole months, doing the ×12 and the rounding
/// of fractional periods entirely in `Decimal`. `field` names the input for
/// error messages.
fn to_months(value: &str, unit: Unit, field: &str) -> Result<u32, String> {
    let v = Decimal::from_str(value.trim())
        .map_err(|_| format!("{field} is not a valid number."))?;
    if v.is_sign_negative() {
        return Err(format!("{field} cannot be negative."));
    }
    let months = match unit {
        Unit::Months => v,
        // Checked: a years value near the Decimal maximum would overflow the
        // ×12, and an unchecked `*` would panic — `calculate` must return an
        // error on any invalid input instead.
        Unit::Years => v
            .checked_mul(Decimal::from(12u32))
            .ok_or_else(|| format!("{field} is too large."))?,
    };
    // Monthly compounding works on whole months; round the fractional period.
    months
        .round()
        .to_u32()
        .ok_or_else(|| format!("{field} is too large."))
}

/// Parse a number the way a user actually types one: `1,234.56`, `£1,234`,
/// `7 %` and `1 234` all parse. Grouping separators, whitespace, a leading
/// currency symbol and a trailing percent sign are noise here — the field
/// already tells us what the number means, and rejecting a pasted `10,000` (the
/// format this app's own output uses) was a dead end for the user.
///
/// This only widens what is accepted; it never guesses. Anything that is not a
/// plain decimal number once the noise is gone still fails. Note the en-GB
/// assumption baked in: `,` is a thousands separator, not a decimal point.
fn parse_number(s: &str) -> Option<Decimal> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '_' | '\u{00a3}' | '$' | '\u{20ac}' | '%'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    Decimal::from_str(&cleaned).ok()
}

fn round2(d: Decimal) -> Decimal {
    d.round_dp(2)
}

fn too_large_msg(name: &str) -> String {
    format!("'{name}' grows too large to project; lower the return or the horizon.")
}

/// Portfolio-level counterpart to [`too_large`], for the summary arithmetic that
/// combines already-bounded per-holding totals rather than a single holding.
fn portfolio_too_large() -> String {
    "The portfolio total is too large to project; lower the values or the horizon.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn one(value: &str, mode: Mode, rate: &str, horizon: &str, hunit: Unit) -> CalcInput {
        with_contribution(value, mode, rate, "0", horizon, hunit)
    }

    fn with_contribution(
        value: &str,
        mode: Mode,
        rate: &str,
        contribution: &str,
        horizon: &str,
        hunit: Unit,
    ) -> CalcInput {
        CalcInput {
            investments: vec![InvestmentInput {
                name: "X".into(),
                value: value.into(),
                mode,
                rate: rate.into(),
                contribution: contribution.into(),
            }],
            horizon_value: horizon.into(),
            horizon_unit: hunit,
        }
    }

    #[test]
    fn annualised_projection_matches_hand_calculation() {
        // Value today 10,000, 7% p.a., 10y -> 10000 * 1.07^10 = 19,671.51. The
        // entered value is today's value, projected forward unchanged at month 0.
        let out = calculate(&one("10000", Mode::Annual, "7", "10", Unit::Years)).unwrap();
        assert_eq!(out.horizon_months, 120);
        assert_eq!(out.investments[0].current_value, d("10000.00"));
        assert_eq!(out.investments[0].projected_value, d("19671.51"));
        assert_eq!(out.current_total, d("10000.00"));
    }

    #[test]
    fn total_return_applies_over_the_horizon() {
        // 1,000 with 50% total return over a 10-year horizon: value today stays
        // 1,000.00, the horizon value is exactly 1,500.00, and the derived
        // annualised rate is 1.5^(1/10) - 1.
        let out = calculate(&one("1000", Mode::Total, "50", "10", Unit::Years)).unwrap();
        assert_eq!(out.current_total, d("1000.00"));
        assert_eq!(out.projected_total, d("1500.00"));
        let expected_annual = d("1.5").powd(Decimal::ONE / Decimal::from(10u32)) - Decimal::ONE;
        assert_eq!(out.investments[0].annualised, expected_annual);
    }

    #[test]
    fn years_and_months_agree() {
        let a = calculate(&one("100", Mode::Annual, "7", "3", Unit::Years)).unwrap();
        let b = calculate(&one("100", Mode::Annual, "7", "36", Unit::Months)).unwrap();
        assert_eq!(a.projected_total, b.projected_total);
    }

    #[test]
    fn fractional_years_round_to_whole_months_in_decimal() {
        // 1.1 years -> 13.2 months -> rounds to 13.
        let out = calculate(&one("100", Mode::Annual, "0", "1.1", Unit::Years)).unwrap();
        assert_eq!(out.horizon_months, 13);
    }

    #[test]
    fn zero_return_leaves_value_unchanged() {
        let out = calculate(&one("500", Mode::Annual, "0", "5", Unit::Years)).unwrap();
        assert_eq!(out.current_total, d("500.00"));
        assert_eq!(out.projected_total, d("500.00"));
        assert_eq!(out.growth, d("0.00"));
    }

    #[test]
    fn guards_reject_bad_input() {
        assert!(calculate(&one("100", Mode::Annual, "7", "0", Unit::Months))
            .unwrap_err()
            .message
            .contains("at least 1 month"));
        assert!(calculate(&one("100", Mode::Annual, "-150", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("-100%"));
        assert!(calculate(&one("-100", Mode::Annual, "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("negative amount"));
        assert!(calculate(&one("abc", Mode::Annual, "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("invalid amount"));
    }

    #[test]
    fn accepts_the_ways_people_actually_type_numbers() {
        // Comma grouping is the format this crate's own output uses, so pasting
        // it back in has to work.
        let grouped = calculate(&one("10,000", Mode::Annual, "7", "10", Unit::Years)).unwrap();
        let plain = calculate(&one("10000", Mode::Annual, "7", "10", Unit::Years)).unwrap();
        assert_eq!(grouped, plain);

        // Currency symbol, spaces and a stray percent sign are all noise the
        // field itself already accounts for.
        for value in ["\u{00a3}10,000", " 10000 ", "10 000", "\u{00a3} 10,000.00"] {
            assert_eq!(
                calculate(&one(value, Mode::Annual, "7", "10", Unit::Years)).unwrap(),
                plain,
                "{value} should parse as 10000"
            );
        }
        assert_eq!(
            calculate(&one("10000", Mode::Annual, "7%", "10", Unit::Years)).unwrap(),
            plain
        );
        assert_eq!(
            calculate(&with_contribution("10000", Mode::Annual, "7", "1,000", "10", Unit::Years))
                .unwrap()
                .contributed_total,
            d("120000.00")
        );

        // Decimals and negatives survive the cleanup.
        assert_eq!(
            calculate(&one("1,234.56", Mode::Annual, "7", "10", Unit::Years))
                .unwrap()
                .current_total,
            d("1234.56")
        );
        assert!(calculate(&one("-1,000", Mode::Annual, "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("negative amount"));
    }

    #[test]
    fn lenient_parsing_still_rejects_nonsense() {
        // Widening what's accepted must not turn into guessing.
        for bad in ["abc", "1.2.3", "--5", "\u{00a3}", "1/2", ""] {
            assert!(
                calculate(&one(bad, Mode::Annual, "7", "10", Unit::Years)).is_err(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn errors_point_at_the_field_that_caused_them() {
        use InvestmentField::{Contribution, Rate, Value};
        let field = |i: &CalcInput| calculate(i).unwrap_err().field;

        assert_eq!(
            field(&one("abc", Mode::Annual, "7", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Value })
        );
        assert_eq!(
            field(&one("100", Mode::Annual, "abc", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Rate })
        );
        assert_eq!(
            field(&with_contribution("100", Mode::Annual, "7", "-5", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Contribution })
        );
        assert_eq!(
            field(&one("100", Mode::Annual, "7", "0", Unit::Months)),
            Some(Field::Horizon)
        );
        // Portfolio-level problems have no single control to blame.
        assert_eq!(
            field(&CalcInput {
                investments: vec![],
                horizon_value: "10".into(),
                horizon_unit: Unit::Years
            }),
            None
        );
    }

    #[test]
    fn error_index_identifies_which_row_failed() {
        // The index must track the position in `investments`, not just report 0.
        let input = CalcInput {
            investments: vec![
                InvestmentInput { name: "A".into(), value: "10000".into(), mode: Mode::Annual, rate: "7".into(), contribution: "0".into() },
                InvestmentInput { name: "B".into(), value: "5000".into(), mode: Mode::Annual, rate: "7".into(), contribution: "0".into() },
                InvestmentInput { name: "C".into(), value: "oops".into(), mode: Mode::Annual, rate: "7".into(), contribution: "0".into() },
            ],
            horizon_value: "10".into(),
            horizon_unit: Unit::Years,
        };
        let err = calculate(&input).unwrap_err();
        assert_eq!(
            err.field,
            Some(Field::Investment { index: 2, part: InvestmentField::Value })
        );
        assert!(err.message.contains('C'));
    }

    #[test]
    fn extreme_growth_errors_instead_of_panicking() {
        // 100% annualised over 100 years overflows the Decimal maximum; the
        // core must return an error rather than panic.
        let out = calculate(&one("10000", Mode::Annual, "100", "100", Unit::Years));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn huge_horizon_in_years_errors_instead_of_panicking() {
        // A years value near the Decimal maximum overflows the ×12 conversion;
        // the core must return an error rather than panic.
        let out = calculate(&one(
            "100",
            Mode::Annual,
            "7",
            "9999999999999999999999999999",
            Unit::Years,
        ));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn horizon_over_100_years_is_rejected() {
        assert!(calculate(&one("100", Mode::Annual, "7", "101", Unit::Years))
            .unwrap_err()
            .message
            .contains("100 years"));
    }

    #[test]
    fn portfolio_sums_across_investments() {
        let input = CalcInput {
            investments: vec![
                InvestmentInput { name: "A".into(), value: "10000".into(), mode: Mode::Annual, rate: "7".into(), contribution: "0".into() },
                InvestmentInput { name: "B".into(), value: "5000".into(), mode: Mode::Total, rate: "80".into(), contribution: "0".into() },
            ],
            horizon_value: "10".into(),
            horizon_unit: Unit::Years,
        };
        let out = calculate(&input).unwrap();
        // Entered values are today's values; the portfolio total is their sum.
        assert_eq!(out.current_total, d("15000.00"));
        // B's 80% total over the horizon lands it at 9,000; A grows to 19,671.51.
        assert_eq!(out.projected_total, d("28671.51"));
        // series is inclusive of both endpoints.
        assert_eq!(out.series.len(), 121);
        assert_eq!(*out.series.first().unwrap(), out.current_total);
        assert_eq!(*out.series.last().unwrap(), out.projected_total);
    }

    #[test]
    fn contributions_add_up_and_are_excluded_from_growth() {
        // 1,000 now, 0% return, +100/month for 12 months (month-end).
        let out = calculate(&with_contribution(
            "1000", Mode::Annual, "0", "100", "12", Unit::Months,
        ))
        .unwrap();
        assert_eq!(out.current_total, d("1000.00"));
        assert_eq!(out.contributed_total, d("1200.00")); // 12 * 100
        assert_eq!(out.projected_total, d("2200.00")); // 1000 + 12 * 100
        // No return, so despite the balance more than doubling, the *gain* from
        // returns is zero — contributions are not counted as growth.
        assert_eq!(out.growth, d("0"));
        assert_eq!(out.growth_pct, d("0"));
        // The percentage's denominator is reported so the UI can state it.
        assert_eq!(out.deployed, d("2200.00")); // 1,000 today + 1,200 added
    }

    #[test]
    fn deployed_is_the_denominator_of_growth_pct() {
        let out = calculate(&with_contribution(
            "10000", Mode::Annual, "7", "200", "10", Unit::Years,
        ))
        .unwrap();
        assert_eq!(out.deployed, out.current_total + out.contributed_total);
        assert_eq!(out.deployed, d("34000.00"));
        // growth_pct really is growth / deployed, not growth / current_total.
        assert_eq!((out.growth / out.deployed).round_dp(6), out.growth_pct.round_dp(6));
    }

    #[test]
    fn per_row_contributed_is_that_rows_own_top_ups() {
        // Only A has top-ups, so the per-row figure must not spread the
        // portfolio total across both rows.
        let input = CalcInput {
            investments: vec![
                InvestmentInput { name: "A".into(), value: "10000".into(), mode: Mode::Annual, rate: "7".into(), contribution: "200".into() },
                InvestmentInput { name: "B".into(), value: "5000".into(), mode: Mode::Total, rate: "80".into(), contribution: "0".into() },
            ],
            horizon_value: "10".into(),
            horizon_unit: Unit::Years,
        };
        let out = calculate(&input).unwrap();
        assert_eq!(out.investments[0].contributed, d("24000.00")); // 120 * 200
        assert_eq!(out.investments[1].contributed, d("0.00"));
        // The rows sum to the portfolio figure the summary panel reports.
        assert_eq!(
            out.investments.iter().map(|r| r.contributed).sum::<Decimal>(),
            out.contributed_total
        );
    }

    #[test]
    fn contributed_reconciles_a_row_that_value_alone_cannot() {
        // The reason the column exists: 10,000 at 7% for 10 years is 19,671.51,
        // nowhere near the 53,881.86 projection. Value today plus contributions
        // must account for all of it bar the returns on those contributions.
        let out = calculate(&with_contribution(
            "10000", Mode::Annual, "7", "200", "10", Unit::Years,
        ))
        .unwrap();
        let row = &out.investments[0];
        assert_eq!(row.current_value, d("10000.00"));
        assert_eq!(row.contributed, d("24000.00"));
        assert_eq!(row.projected_value, d("53881.86"));
        // Deployed capital is a floor on the projection under a positive return.
        assert!(row.projected_value > row.current_value + row.contributed);
    }

    #[test]
    fn contributions_series_accumulates_month_by_month() {
        // 1,000 now, 0% return, +100/month for 12 months (month-end).
        let out = calculate(&with_contribution(
            "1000", Mode::Annual, "0", "100", "12", Unit::Months,
        ))
        .unwrap();
        // Parallels the value series, one point per month inclusive of endpoints.
        assert_eq!(out.contributions_series.len(), out.series.len());
        // Nothing deposited today; one contribution accrues per elapsed month.
        assert_eq!(out.contributions_series[0], d("0.00"));
        assert_eq!(out.contributions_series[1], d("100.00"));
        assert_eq!(out.contributions_series[6], d("600.00"));
        // The final point equals the total contributed over the horizon.
        assert_eq!(*out.contributions_series.last().unwrap(), out.contributed_total);
    }

    #[test]
    fn contributions_series_is_all_zero_without_top_ups() {
        let out = calculate(&one("1000", Mode::Annual, "12", "24", Unit::Months)).unwrap();
        assert!(out.contributions_series.iter().all(|c| c.is_zero()));
    }

    #[test]
    fn contributions_increase_the_projection_but_not_today() {
        let base = calculate(&one("1000", Mode::Annual, "12", "24", Unit::Months)).unwrap();
        let with = calculate(&with_contribution(
            "1000", Mode::Annual, "12", "50", "24", Unit::Months,
        ))
        .unwrap();
        // Contributions lift the horizon value and get some return themselves.
        assert!(with.projected_total > base.projected_total);
        assert_eq!(with.contributed_total, d("1200.00")); // 24 * 50
        // Today's value is unaffected by money not yet invested.
        assert_eq!(with.series[0], base.series[0]);
        assert_eq!(with.current_total, base.current_total);
    }

    #[test]
    fn portfolio_summary_overflow_errors_instead_of_panicking() {
        // A near-maximum value today plus large contributions: each total fits
        // on its own (the value shrinks under a negative return, so the loop
        // never overflows), but the summary `current + contributed` exceeds the
        // Decimal maximum. Must error, not panic.
        let out = calculate(&with_contribution(
            "79000000000000000000000000000", // ~7.9e28, just under Decimal::MAX
            Mode::Annual,
            "-50",
            "10000000000000000000000000", // 1e25 per month
            "1200",
            Unit::Months,
        ));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn zero_deployed_capital_reports_zero_growth_pct() {
        // A row with no value and no contributions (kept in the form by a rate)
        // deploys no capital, so growth_pct divides by zero — must report 0%,
        // not panic.
        let out = calculate(&one("0", Mode::Annual, "7", "10", Unit::Years)).unwrap();
        assert_eq!(out.current_total, d("0.00"));
        assert_eq!(out.growth_pct, Decimal::ZERO);
    }

    #[test]
    fn negative_contribution_is_rejected() {
        assert!(calculate(&with_contribution(
            "1000", Mode::Annual, "5", "-50", "10", Unit::Years,
        ))
        .unwrap_err()
            .message
        .contains("negative monthly contribution"));
    }
}
