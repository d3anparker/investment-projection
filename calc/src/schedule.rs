//! The drawdown schedule: the projection read back month by month, holding by
//! holding, rounded so that every month a reader looks at adds up.
//!
//! No new arithmetic about money lives here. The month loop in
//! [`crate::engine::project`] records what it moved; this module only rounds
//! those raw flows to the penny and groups them into years.

use rust_decimal::{Decimal, RoundingStrategy};

use crate::engine::calculate_recorded;
use crate::parse::round2;
use crate::types::{
    CalcError, CalcInput, RunsOut, Schedule, ScheduleFigures, ScheduleMonth, ScheduleYear,
};

/// Months in a schedule year. A calendar fact, not a fiscal one: the tax
/// system's own period length is [`crate::CalcOutput::period_months`].
const MONTHS_PER_YEAR: usize = 12;

/// Project `input` and read the drawdown back month by month.
///
/// `output` is exactly what [`crate::calculate`] returns for the same input.
/// A deposits plan has nothing to draw down, so its `years` are empty.
///
/// # Rounding
///
/// Each month is rounded on its own, so a month always adds up:
///
/// * `gross` and `net` are the month's raw totals rounded to the penny, and
///   `tax` is `gross - net`, so `gross == net + tax` holds exactly.
/// * The per-holding cells split the rounded `gross` by the largest-remainder
///   method (see [`apportion`]): every cell is within a penny of its raw value,
///   none is negative, a holding that gave nothing reads exactly zero, and the
///   cells sum exactly to `gross`. A pro-rata £1,000 withdrawal therefore reads
///   £1,000.00 every month, never £999.99.
/// * A year is the plain sum of its rounded months, so it adds up too.
///
/// The cost, accepted on purpose: summed over the *whole* drawdown, a holding's
/// cells can differ by a few pence from its `InvestmentResult::withdrawn`,
/// which is rounded once from the raw total. The month is what a reader acts
/// on, so the month is what is made exact.
pub fn schedule(input: &CalcInput) -> Result<Schedule, CalcError> {
    let (output, recording) = calculate_recorded(input, true)?;
    let horizon = output.horizon_months as usize;
    let holdings = output.investments.len();

    // One pass: each month is rounded, then folded into the year it opens or
    // continues, so no month is built twice.
    let mut years: Vec<ScheduleYear> = Vec::new();
    for (i, flows) in recording.months.into_iter().enumerate() {
        let k = i + 1;
        let raw_gross: Decimal = flows.gross.iter().sum();
        let gross = round2(raw_gross);
        let net = round2(raw_gross - flows.tax);
        let month = ScheduleMonth {
            drawdown_month: k as u32,
            figures: ScheduleFigures {
                drawn: apportion(&flows.gross, gross),
                gross,
                tax: gross - net,
                net,
                charged: round2(flows.charged),
                value_after: output.series[horizon + k],
            },
        };
        if i % MONTHS_PER_YEAR == 0 {
            years.push(ScheduleYear {
                year: year_of(k as u32),
                months: Vec::with_capacity(MONTHS_PER_YEAR),
                figures: ScheduleFigures::zero(holdings),
            });
        }
        let year = years.last_mut().expect("a year is opened at its first month");
        year.figures.add(&month.figures);
        year.months.push(month);
    }

    // `depletion_month` indexes `series`, so the drawdown month it falls in is
    // that index less the growth period. A pot cannot empty before the first
    // withdrawal, but `checked_sub` and the filter make that a `None` rather
    // than a wrapped number if it ever did.
    let runs_out = output
        .depletion_month
        .and_then(|m| m.checked_sub(output.horizon_months))
        .filter(|k| *k >= 1)
        .map(|month| RunsOut { month, year: year_of(month) });

    Ok(Schedule { output, withdrawal: recording.withdrawal, years, runs_out })
}

/// The schedule year (from 1) a drawdown month (from 1) falls in.
fn year_of(drawdown_month: u32) -> u32 {
    drawdown_month.saturating_sub(1) / MONTHS_PER_YEAR as u32 + 1
}

impl ScheduleFigures {
    /// A period with nothing in it yet, for `holdings` holdings.
    fn zero(holdings: usize) -> Self {
        ScheduleFigures {
            drawn: vec![Decimal::ZERO; holdings],
            gross: Decimal::ZERO,
            tax: Decimal::ZERO,
            net: Decimal::ZERO,
            charged: Decimal::ZERO,
            value_after: Decimal::ZERO,
        }
    }

    /// Extend this period by a later one: the flows add up, and the portfolio
    /// is whatever the later one left.
    fn add(&mut self, later: &ScheduleFigures) {
        for (sum, cell) in self.drawn.iter_mut().zip(&later.drawn) {
            *sum += *cell;
        }
        self.gross += later.gross;
        self.tax += later.tax;
        self.net += later.net;
        self.charged += later.charged;
        self.value_after = later.value_after;
    }
}

/// Split `total` (already rounded to the penny) into penny cells as close as
/// possible to `raw`, by the largest-remainder method.
///
/// Every cell starts at its raw value rounded *down* to the penny; the pennies
/// left over go one each to the cells with the largest remainders, ties to the
/// earlier holding. That keeps each cell within a penny of its raw value, never
/// negative, exactly zero where nothing was drawn, and the cells sum exactly to
/// `total`.
///
/// The leftover is never more than the number of cells with a remainder: the
/// floors sum to at most `floor(Σ raw)`, and a total rounded to the nearest
/// penny is at most `ceil(Σ raw)`.
pub(crate) fn apportion(raw: &[Decimal], total: Decimal) -> Vec<Decimal> {
    let penny = Decimal::new(1, 2);
    let mut cells: Vec<Decimal> = raw
        .iter()
        .map(|r| (*r).max(Decimal::ZERO).round_dp_with_strategy(2, RoundingStrategy::ToZero))
        .collect();
    let remainders: Vec<Decimal> = raw
        .iter()
        .zip(&cells)
        .map(|(r, c)| (*r).max(Decimal::ZERO) - *c)
        .collect();

    let mut order: Vec<usize> = (0..raw.len()).filter(|j| remainders[*j] > Decimal::ZERO).collect();
    order.sort_by(|a, b| remainders[*b].cmp(&remainders[*a]).then(a.cmp(b)));

    let mut left = total - cells.iter().sum::<Decimal>();
    for j in order {
        if left < penny {
            break;
        }
        cells[j] += penny;
        left -= penny;
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::apportion;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn ds(v: &[&str]) -> Vec<Decimal> {
        v.iter().map(|s| d(s)).collect()
    }

    /// The properties every split must have, whatever the inputs.
    fn check(raw: &[Decimal], total: Decimal) -> Vec<Decimal> {
        let cells = apportion(raw, total);
        assert_eq!(cells.iter().sum::<Decimal>(), total, "cells {cells:?} must sum to {total}");
        for (c, r) in cells.iter().zip(raw) {
            assert!(*c >= Decimal::ZERO, "a cell is never negative: {cells:?}");
            assert!((*c - *r).abs() < d("0.01"), "cell {c} strays a penny or more from {r}");
            if r.is_zero() {
                assert!(c.is_zero(), "a holding that gave nothing reads exactly zero");
            }
        }
        cells
    }

    #[test]
    fn thirds_put_the_spare_penny_on_the_earliest_tie() {
        let third = d("1000") / d("3");
        let cells = check(&[third, third, third], d("1000.00"));
        assert_eq!(cells, ds(&["333.34", "333.33", "333.33"]));
    }

    #[test]
    fn the_largest_remainder_takes_the_spare_penny() {
        let cells = check(&ds(&["1.004", "2.008", "3.001"]), d("6.01"));
        assert_eq!(cells, ds(&["1.00", "2.01", "3.00"]));
    }

    #[test]
    fn exact_figures_and_zeros_pass_straight_through() {
        let cells = check(&ds(&["600", "0", "400"]), d("1000.00"));
        assert_eq!(cells, ds(&["600.00", "0.00", "400.00"]));
    }

    #[test]
    fn dust_too_small_to_show_rounds_to_nothing() {
        let cells = check(&ds(&["0.003", "0.001"]), d("0.00"));
        assert_eq!(cells, ds(&["0.00", "0.00"]));
    }

    #[test]
    fn a_total_rounded_up_spreads_its_pennies_across_the_remainders() {
        // 0.006 + 0.006 + 0.006 = 0.018, which rounds to 0.02: two pennies to
        // place, and each of the first two cells takes one.
        let six = d("0.006");
        let cells = check(&[six, six, six], d("0.02"));
        assert_eq!(cells, ds(&["0.01", "0.01", "0.00"]));
    }
}
