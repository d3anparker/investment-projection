//! The goal-seek: [`solve`] a projection for a [`Goal`].
//!
//! Bisection and series scans over [`crate::engine`]'s [`project`]/[`calculate`],
//! sharing their never-panic contract — unreachable targets come back as `Err`,
//! never a panic or a hang, and all arithmetic stays exact `Decimal`.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::engine::{
    calculate, drawdown_months_of, groups_for, horizon_months_of, open_if_ordered, prepare_holdings,
    project, rate_cap_of, withdrawal_of, Prepared, Run,
};
use crate::parse::{fmt_money_plain, parse_number, round2};
use crate::strategy::Strategy;
use crate::types::{CalcError, CalcInput, Plan, Unit};
use crate::MAX_HORIZON_MONTHS;

/// What to solve the projection *for*. The two deposits-mode goals carry the
/// user's target as a raw string, parsed here the way [`calculate`] parses
/// inputs. The two drawdown-mode goals carry nothing: the drawdown period and the
/// monthly withdrawal they reason about are already in [`CalcInput`], so there is
/// no extra number to pass. Every goal is measured against the whole portfolio.
pub enum Goal {
    /// Solve for the recurring monthly deposit that makes the portfolio reach
    /// `target` over the horizon. Deposits mode.
    MonthlyTopUp { target: String },
    /// Solve for the time, in whole months, until the portfolio first reaches
    /// `target` at its current rates and deposits. Deposits mode.
    TimeToTarget { target: String },
    /// Solve for the largest monthly withdrawal the portfolio can sustain and
    /// still reach exactly £0 at the end of the drawdown period in the input.
    /// Drawdown mode.
    MaxWithdrawal,
    /// Solve for how long, in whole months *of drawdown*, the monthly withdrawal
    /// in the input lasts before the portfolio runs dry. Drawdown mode.
    TimeToDeplete,
}

/// The answer to a [`Goal`].
#[derive(Clone, Debug, PartialEq)]
pub enum Solution {
    /// The monthly deposit required, rounded up to the penny so it genuinely
    /// reaches the target rather than landing a fraction short.
    MonthlyTopUp(Decimal),
    /// Whole months until the portfolio first reaches the target.
    Months(u32),
    /// The target is already met by the inputs as they stand — no deposit needed,
    /// or the portfolio is already worth at least the target today.
    AlreadyMet,
    /// The largest sustainable monthly withdrawal, rounded *down* to the penny so
    /// the figure reported is one the pot genuinely supports.
    MaxWithdrawal(Decimal),
    /// Whole months *of drawdown* until the portfolio runs dry under the
    /// withdrawal asked about. This counts from the start of drawdown, not from
    /// today — [`CalcOutput::depletion_month`] is the absolute index.
    Depletes(u32),
    /// The returns cover the withdrawal, so the pot never runs dry within the
    /// 100-year cap — there is no month to report rather than a very large one.
    NeverDepletes,
}

/// The largest monthly figure the bracket search will consider before declaring a
/// target unreachable, for both the top-up and the withdrawal searches. A billion
/// a month is comfortably past any real use and keeps the doubling bounded.
const MAX_TOP_UP: i64 = 1_000_000_000;

/// Solve the projection for a [`Goal`]. Shares [`calculate`]'s never-panic
/// contract: invalid input and unreachable targets come back as `Err`, never a
/// panic or a hang, and all arithmetic stays exact `Decimal`.
pub fn solve(input: &CalcInput, goal: &Goal) -> Result<Solution, CalcError> {
    match goal {
        Goal::MonthlyTopUp { target } => solve_top_up(input, target),
        Goal::TimeToTarget { target } => solve_time(input, target),
        Goal::MaxWithdrawal => solve_max_withdrawal(input),
        Goal::TimeToDeplete => solve_time_to_deplete(input),
    }
}

/// A goal needs at least one holding to be about.
fn require_holdings(input: &CalcInput) -> Result<(), CalcError> {
    if input.investments.is_empty() {
        Err(CalcError::new("Add a holding before solving a goal.", None))
    } else {
        Ok(())
    }
}

/// The target field of a goal is not one of the form's investment controls, so
/// its errors carry no `Field` — they surface as a portfolio-level message.
fn parse_target(target: &str) -> Result<Decimal, CalcError> {
    let t = parse_number(target)
        .ok_or_else(|| CalcError::new("Enter a valid target amount.", None))?;
    if t < Decimal::ZERO {
        return Err(CalcError::new("The target amount cannot be negative.", None));
    }
    Ok(t)
}

/// Prepare a projection's holdings once, exactly as [`calculate`] does before
/// its own run, so a solver can re-`project` them across a bracket search without
/// re-parsing every input string and recomputing every twelfth root per probe.
fn prepare_for(input: &CalcInput) -> Result<Vec<Prepared>, CalcError> {
    let catalogue: &'static [taxkit::AccountKind] =
        input.tax.as_ref().map_or(&[], |t| t.system.account_kinds());
    let default_kind = input.tax.as_ref().and_then(|t| t.system.default_account_kind());
    prepare_holdings(&input.investments, catalogue, default_kind)
}

/// A deposits-only clone of `input` over the 100-year cap — the base for both
/// deposits-mode solvers, which scan or bracket a long accumulation regardless of
/// the horizon (or drawdown plan) the user actually entered.
fn long_deposits(input: &CalcInput) -> CalcInput {
    CalcInput {
        investments: input.investments.clone(),
        horizon_value: MAX_HORIZON_MONTHS.to_string(),
        horizon_unit: Unit::Months,
        plan: Plan::Deposits,
        currency: input.currency.clone(),
        tax: input.tax.clone(),
    }
}

/// Bisection on the monthly top-up. The projected value rises monotonically with
/// the top-up, so a doubling bracket plus binary search converges on the least
/// top-up that reaches the target.
fn solve_top_up(input: &CalcInput, target: &str) -> Result<Solution, CalcError> {
    let target = parse_target(target)?;
    require_holdings(input)?;

    // Prepare once; each probe only overwrites the deposit and re-projects, so
    // the bracket search never re-parses a string or recomputes a twelfth root.
    let horizon_months = horizon_months_of(input)?;
    let mut prepared = prepare_for(input)?;
    let n = Decimal::from(prepared.len());

    // The portfolio value a candidate monthly top-up reaches over the user's
    // horizon, spread evenly across the holdings ("a portfolio top-up is shared
    // equally"). Deposits-only — no drawdown, no session — whatever mode the
    // input came from, because a top-up is a deposit.
    let mut projected_with = |top_up: Decimal| -> Result<Decimal, CalcError> {
        let each = top_up / n;
        for p in prepared.iter_mut() {
            p.contribution = each;
        }
        // A charging system (Germany) taxes the holding even while accumulating,
        // so a top-up goal must feel that drag too; a withdrawal-only system opens
        // no session here and the run is untaxed exactly as before.
        let mut plan = open_if_ordered(&input.tax, &Strategy::pro_rata(), false)?;
        let run = project(&prepared, horizon_months, 0, Decimal::ZERO, &Strategy::pro_rata(), &[], None, &mut plan)?;
        Ok(round2(*run.totals.last().expect("horizon >= 1 guarantees a point")))
    };

    if projected_with(Decimal::ZERO)? >= target {
        return Ok(Solution::AlreadyMet);
    }

    let cap = Decimal::from(MAX_TOP_UP);
    let mut hi = Decimal::ONE;
    while projected_with(hi)? < target {
        hi *= Decimal::from(2u32);
        if hi > cap {
            return Err(CalcError::new(
                format!(
                    "No monthly top-up reaches {} in this time; extend the horizon or lower the target.",
                    fmt_money_plain(target, input.currency_symbol())
                ),
                None,
            ));
        }
    }

    let mut lo = Decimal::ZERO;
    let cent = Decimal::new(1, 2); // 0.01
    for _ in 0..80 {
        if hi - lo <= cent {
            break;
        }
        let mid = (lo + hi) / Decimal::from(2u32);
        if projected_with(mid)? >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let answer = hi.round_dp_with_strategy(2, RoundingStrategy::AwayFromZero);
    Ok(Solution::MonthlyTopUp(answer))
}

/// Time until the portfolio reaches the target. With annualised-only rates, a
/// row's rate no longer depends on the horizon, so this simply projects the
/// deposits over the 100-year cap and scans the series for the first month that
/// clears the target.
fn solve_time(input: &CalcInput, target: &str) -> Result<Solution, CalcError> {
    let target = parse_target(target)?;
    require_holdings(input)?;

    let out = calculate(&long_deposits(input))?;

    match out.series.iter().position(|v| *v >= target) {
        Some(0) => Ok(Solution::AlreadyMet),
        Some(i) => Ok(Solution::Months(i as u32)),
        None => Err(CalcError::new(
            format!(
                "The portfolio does not reach {} within 100 years; raise the returns or the contributions.",
                fmt_money_plain(target, input.currency_symbol())
            ),
            None,
        )),
    }
}

/// Bisection on the monthly withdrawal: the largest draw that still leaves the
/// portfolio solvent to the end of the drawdown period (i.e. reaches £0 no sooner
/// than the final month). Feasibility is downward-closed — drawing less is always
/// at least as safe — so the feasible side of the bracket is the low one and the
/// answer is rounded down.
fn solve_max_withdrawal(input: &CalcInput) -> Result<Solution, CalcError> {
    require_holdings(input)?;
    let Plan::Drawdown { drawdown_value, drawdown_unit, strategy, .. } = &input.plan else {
        return Err(CalcError::new("This goal only applies while drawing the portfolio down.", None));
    };
    // Parse and prepare once; each probe below only varies the withdrawal.
    let horizon_months = horizon_months_of(input)?;
    let drawdown_months = drawdown_months_of(drawdown_value, *drawdown_unit, horizon_months)?;
    let rate_cap = rate_cap_of(strategy)?;
    let prepared = prepare_for(input)?;
    let groups = groups_for(strategy, &prepared);

    // Re-project the pre-parsed holdings under a candidate withdrawal. Each probe
    // opens a fresh session — cheap next to the month loop — and skips the series
    // rounding and per-row assembly a full `CalcOutput` would carry.
    let run_with = |w: Decimal| -> Result<Run, CalcError> {
        let mut plan = open_if_ordered(&input.tax, strategy, true)?;
        project(&prepared, horizon_months, drawdown_months, w, strategy, &groups, rate_cap, &mut plan)
    };
    // A draw is feasible if it survives the whole drawdown period without the
    // portfolio ever hitting £0.
    let feasible = |w: Decimal| -> Result<bool, CalcError> { Ok(run_with(w)?.depletion_month.is_none()) };

    // The pot at the start of drawdown, drawing nothing. If it is already empty
    // there is nothing to spend down.
    if round2(run_with(Decimal::ZERO)?.totals[horizon_months as usize]) <= Decimal::ZERO {
        return Err(CalcError::new(
            "The portfolio has nothing left to draw down at the end of the growth period.",
            None,
        ));
    }

    if !feasible(Decimal::ZERO)? {
        return Err(CalcError::new(
            "The portfolio runs dry before the drawdown ends even with no withdrawals.",
            None,
        ));
    }

    let cap = Decimal::from(MAX_TOP_UP);
    let mut hi = Decimal::ONE;
    while feasible(hi)? {
        if hi >= cap {
            return Err(CalcError::new(
                format!(
                    "The portfolio can sustain more than {} a month, which is beyond the range this solver reports.",
                    fmt_money_plain(cap, input.currency_symbol())
                ),
                None,
            ));
        }
        hi = (hi * Decimal::from(2u32)).min(cap);
    }

    let mut lo = Decimal::ZERO;
    let cent = Decimal::new(1, 2); // 0.01
    for _ in 0..80 {
        if hi - lo <= cent {
            break;
        }
        let mid = (lo + hi) / Decimal::from(2u32);
        if feasible(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // Round down, then settle the last penny against `project` itself: step up
    // while the next penny still holds. Bounded regardless — the never-hang
    // contract outranks the extra penny.
    let mut answer = lo.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    for _ in 0..3 {
        if !feasible(answer + cent)? {
            break;
        }
        answer += cent;
    }

    // Under pro-rata, feasibility is downward-closed and the figure above always
    // holds. Under a tax-aware order it is only *empirically* so — drawing more
    // this month changes which account is cheapest next month, so the balances
    // are not pointwise ordered and the bisection can settle a hair too high.
    // Rather than assert a monotonicity that cannot be proved, verify and step
    // back down. Bounded, so it can fail loudly but never hang.
    for _ in 0..8 {
        if feasible(answer)? {
            break;
        }
        answer -= cent;
    }
    if !feasible(answer)? {
        return Err(CalcError::new(
            "Could not settle on a sustainable withdrawal for this withdrawal order.",
            None,
        ));
    }

    Ok(Solution::MaxWithdrawal(answer))
}

/// How long the drawdown withdrawal lasts. Re-projects the drawdown over the
/// whole remaining span to the 100-year cap (the pot may outlast the period on
/// screen), reads the month the portfolio first hits £0, and reports it relative
/// to the *start of drawdown*.
fn solve_time_to_deplete(input: &CalcInput) -> Result<Solution, CalcError> {
    require_holdings(input)?;
    let Plan::Drawdown { withdrawal, strategy, .. } = &input.plan else {
        return Err(CalcError::new("This goal only applies while drawing the portfolio down.", None));
    };

    let horizon_months = horizon_months_of(input)?;
    // No room left to draw down within the cap: nothing can run out.
    if horizon_months >= MAX_HORIZON_MONTHS {
        return Ok(Solution::NeverDepletes);
    }
    // Project the user's withdrawal over the long span. Only the period is
    // lengthened; a blank/zero draw never depletes.
    let span = MAX_HORIZON_MONTHS - horizon_months;
    let withdrawal = withdrawal_of(withdrawal)?;
    let rate_cap = rate_cap_of(strategy)?;
    let prepared = prepare_for(input)?;
    let groups = groups_for(strategy, &prepared);
    let mut plan = open_if_ordered(&input.tax, strategy, true)?;
    let out = project(&prepared, horizon_months, span, withdrawal, strategy, &groups, rate_cap, &mut plan)?;

    Ok(match out.depletion_month {
        Some(m) => Solution::Depletes(m - horizon_months),
        None => Solution::NeverDepletes,
    })
}
