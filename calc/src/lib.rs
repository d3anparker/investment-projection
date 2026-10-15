//! Investment projection core.
//!
//! Pure, exact base-10 `Decimal` arithmetic (via `rust_decimal`) — no UI, no
//! WASM bindings, no floating point. The Leptos front end calls [`calculate`]
//! directly with these types and only *formats* the `Decimal`s it gets back; it
//! performs no financial arithmetic of its own.
//!
//! A projection runs in one of two modes, carried by [`Plan`]:
//!
//! * **Deposits** — grow every holding from its value today, adding each row's
//!   optional monthly deposit, over the horizon. The classic accumulation.
//! * **Drawdown** — the same accumulation for `horizon_months`, then a second
//!   phase of `drawdown_months` in which a single *portfolio-level* monthly
//!   withdrawal is taken, apportioned across the holdings pro-rata by their
//!   current value and rebalanced every month. Monthly deposits stop at the
//!   handover; the only cash flow in the drawdown phase is that withdrawal.
//!
//! The whole thing is one continuous month-by-month series — `series[horizon_months]`
//! is the pot at the start of drawdown ([`CalcOutput::handover_total`]) — so the
//! UI never has to stitch two projections together or work out the handover value
//! for itself.
//!
//! [`schedule`] reads the same projection back month by month: what each
//! drawdown month takes from each holding, rounded so that every month and every
//! year adds up on screen. It runs exactly the projection [`calculate`] does, so
//! the two never disagree.

mod engine;
mod parse;
mod schedule;
mod solve;
mod strategy;
mod tax;
mod types;

#[cfg(test)]
mod tests;

pub use engine::calculate;
pub use parse::parse_number;
pub use schedule::schedule;
pub use solve::{solve, Goal, Solution};
pub use strategy::{Limit, Order, Strategy};
pub use types::{
    CalcError, CalcInput, CalcOutput, Field, InvestmentField, InvestmentInput, InvestmentResult,
    Plan, RunsOut, Schedule, ScheduleFigures, ScheduleMonth, ScheduleYear, TaxContext, Unit,
};

/// The 100-year projection cap, in months. `calculate` rejects any period past
/// this, and the time-based solvers project out to exactly it.
pub(crate) const MAX_HORIZON_MONTHS: u32 = 1200;
