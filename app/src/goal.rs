//! The goal-seek form, marshalled between the reactive UI and `calc::solve`.
//!
//! Like [`crate::convert`], this is the pure string-in/string-out layer around a
//! `calc` entry point: the kind `<select>`'s value maps to a `calc::Goal` variant,
//! and a returned `calc::Solution` maps to the sentence the summary panel shows.
//! Both halves are natively unit-tested — no signals, no DOM.
//!
//! The available questions depend on the top-level mode. In **deposits** mode you
//! ask what it takes to *reach* a figure (a monthly top-up, or a span of time). In
//! **drawdown** mode you ask about spending the projected pot down (the largest
//! monthly withdrawal that empties it exactly at the end of the drawdown period,
//! or how long a given withdrawal lasts). Every goal is about the whole portfolio.

use crate::format::{fmt_money, horizon_label};
use calc::{parse_number, Goal, Solution};

/// The goal-seek question. Two kinds belong to each mode; [`parse`](GoalKind::parse)
/// resolves the kind `<select>`'s value *within the current mode*, so a value left
/// over from the other mode (or a newer build) falls back to that mode's default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalKind {
    /// Deposits: reach a target with a monthly top-up. The deposits default.
    TopUp,
    /// Deposits: reach a target on the current deposits — solve for the time.
    Time,
    /// Drawdown: the largest monthly withdrawal that spends the pot to £0 by the
    /// end of the drawdown period. The drawdown default.
    Withdrawal,
    /// Drawdown: how long a fixed monthly withdrawal lasts before the pot runs dry.
    Lasts,
}

impl GoalKind {
    /// Resolve the kind `<select>`'s value within `plan` (`"drawdown"` or, by
    /// default, deposits). A value that doesn't belong to the current mode maps to
    /// that mode's default, so switching mode never leaves a stale kind selected.
    pub fn parse(kind: &str, plan: &str) -> GoalKind {
        let drawdown = plan == "drawdown";
        match kind {
            "time" if !drawdown => GoalKind::Time,
            "lasts" if drawdown => GoalKind::Lasts,
            _ if drawdown => GoalKind::Withdrawal,
            _ => GoalKind::TopUp,
        }
    }
}

/// Build the `calc::Goal` from the goal form's strings, or `None` when the goal is
/// inert. `kind` is the kind `<select>`'s value; `plan` the top-level mode;
/// `target` the deposits-mode goal amount box; `withdrawal` the drawdown-panel
/// monthly withdrawal.
///
/// The deposits goals need a figure to aim at, so a blank target keeps them inert.
/// `MaxWithdrawal` is always live in drawdown mode — "how much can I take and
/// spend it to zero?" is always answerable — while `TimeToDeplete` needs a
/// withdrawal to be about, so a blank withdrawal box leaves it inert.
pub fn build_goal(kind: &str, plan: &str, target: &str, withdrawal: &str) -> Option<Goal> {
    match GoalKind::parse(kind, plan) {
        GoalKind::TopUp => (!target.trim().is_empty()).then(|| Goal::MonthlyTopUp { target: target.into() }),
        GoalKind::Time => (!target.trim().is_empty()).then(|| Goal::TimeToTarget { target: target.into() }),
        GoalKind::Withdrawal => Some(Goal::MaxWithdrawal),
        GoalKind::Lasts => (!withdrawal.trim().is_empty()).then_some(Goal::TimeToDeplete),
    }
}

/// One line of plain English for a solved goal, or the error message when the goal
/// could not be met. Every goal is about the whole portfolio, so the subject is
/// fixed. `amount` is the relevant on-screen figure — the target in deposits mode,
/// the monthly withdrawal in drawdown mode — echoed through the same `fmt_money`
/// the rest of the UI uses. `horizon` is the growth-period label (a top-up is
/// solved *for that period*); `drawdown` is the drawdown-period label (a
/// max-withdrawal empties the pot over exactly that span).
///
/// The `Solution` variant alone decides the sentence shape — `calc` has already
/// answered the question that was asked.
pub fn describe(solution: &Result<Solution, String>, amount: &str, horizon: &str, drawdown: &str) -> String {
    let amount_txt = parse_number(amount).map(fmt_money).unwrap_or_else(|| amount.to_string());
    match solution {
        Ok(Solution::MonthlyTopUp(x)) => {
            format!("{} a month gets your portfolio to {} over {}.", fmt_money(*x), amount_txt, horizon)
        }
        Ok(Solution::Months(m)) => {
            format!("Your portfolio reaches {} in {}.", amount_txt, duration_label(*m))
        }
        Ok(Solution::AlreadyMet) => {
            format!("Your portfolio is already on track to reach {amount_txt}.")
        }
        Ok(Solution::MaxWithdrawal(x)) => format!(
            "You can take out {} a month and spend your portfolio to zero over {} of drawdown.",
            fmt_money(*x),
            drawdown
        ),
        Ok(Solution::Depletes(m)) => format!(
            "Drawing {} a month, your portfolio runs dry after {} of drawdown.",
            amount_txt,
            duration_label(*m)
        ),
        Ok(Solution::NeverDepletes) => format!(
            "Drawing {} a month, your portfolio never runs dry \u{2014} its returns cover the withdrawals.",
            amount_txt
        ),
        Err(msg) => msg.clone(),
    }
}

/// A span of whole months as years-and-months prose: `83 -> "6 years 11 months"`,
/// `12 -> "1 year"`, `7 -> "7 months"`, `0 -> "no time at all"`. Distinct from
/// `format::horizon_label`, which never breaks a non-round span into years.
pub fn duration_label(months: u32) -> String {
    if months == 0 {
        return "no time at all".to_string();
    }
    match (months / 12, months % 12) {
        (0, _) | (_, 0) => horizon_label(months),
        (y, m) => format!("{} {}", horizon_label(y * 12), horizon_label(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn goal_kind_parses_within_its_mode() {
        // Deposits mode: the two deposits kinds resolve, drawdown kinds fall back.
        assert_eq!(GoalKind::parse("topup", "deposits"), GoalKind::TopUp);
        assert_eq!(GoalKind::parse("time", "deposits"), GoalKind::Time);
        assert_eq!(GoalKind::parse("withdrawal", "deposits"), GoalKind::TopUp);
        assert_eq!(GoalKind::parse("", "deposits"), GoalKind::TopUp);
        // Drawdown mode: the two drawdown kinds resolve, deposits kinds fall back.
        assert_eq!(GoalKind::parse("withdrawal", "drawdown"), GoalKind::Withdrawal);
        assert_eq!(GoalKind::parse("lasts", "drawdown"), GoalKind::Lasts);
        assert_eq!(GoalKind::parse("topup", "drawdown"), GoalKind::Withdrawal);
        assert_eq!(GoalKind::parse("", "drawdown"), GoalKind::Withdrawal);
    }

    #[test]
    fn build_goal_deposits_kinds_need_a_target() {
        assert!(build_goal("topup", "deposits", "", "").is_none());
        assert!(build_goal("time", "deposits", "   ", "").is_none());
        assert!(matches!(build_goal("topup", "deposits", "1000", ""), Some(Goal::MonthlyTopUp { .. })));
        assert!(matches!(build_goal("time", "deposits", "1000", ""), Some(Goal::TimeToTarget { .. })));
    }

    #[test]
    fn build_goal_drawdown_kinds() {
        // Max withdrawal is always live in drawdown mode, target box irrelevant.
        assert!(matches!(build_goal("withdrawal", "drawdown", "", ""), Some(Goal::MaxWithdrawal)));
        // "How long does it last" needs a withdrawal to be about.
        assert!(build_goal("lasts", "drawdown", "", "").is_none());
        assert!(matches!(build_goal("lasts", "drawdown", "", "2000"), Some(Goal::TimeToDeplete)));
    }

    #[test]
    fn describe_reads_the_deposits_answers() {
        assert_eq!(
            describe(&Ok(Solution::MonthlyTopUp(dec("412.60"))), "500000", "10 years", "30 years"),
            "\u{00a3}412.60 a month gets your portfolio to \u{00a3}500,000.00 over 10 years."
        );
        assert_eq!(
            describe(&Ok(Solution::Months(83)), "50000", "10 years", "30 years"),
            "Your portfolio reaches \u{00a3}50,000.00 in 6 years 11 months."
        );
        assert_eq!(
            describe(&Ok(Solution::AlreadyMet), "40000", "10 years", "30 years"),
            "Your portfolio is already on track to reach \u{00a3}40,000.00."
        );
        assert_eq!(describe(&Err("nope".to_string()), "1", "10 years", "30 years"), "nope");
    }

    #[test]
    fn describe_reads_the_drawdown_answers() {
        assert_eq!(
            describe(&Ok(Solution::MaxWithdrawal(dec("2450.13"))), "", "10 years", "30 years"),
            "You can take out \u{00a3}2,450.13 a month and spend your portfolio to zero over 30 years of drawdown."
        );
        assert_eq!(
            describe(&Ok(Solution::Depletes(83)), "1500", "10 years", "30 years"),
            "Drawing \u{00a3}1,500.00 a month, your portfolio runs dry after 6 years 11 months of drawdown."
        );
        assert_eq!(
            describe(&Ok(Solution::NeverDepletes), "1500", "10 years", "30 years"),
            "Drawing \u{00a3}1,500.00 a month, your portfolio never runs dry \u{2014} its returns cover the withdrawals."
        );
    }

    #[test]
    fn duration_label_breaks_a_span_into_years_and_months() {
        assert_eq!(duration_label(0), "no time at all");
        assert_eq!(duration_label(1), "1 month");
        assert_eq!(duration_label(7), "7 months");
        assert_eq!(duration_label(12), "1 year");
        assert_eq!(duration_label(24), "2 years");
        assert_eq!(duration_label(13), "1 year 1 month");
        assert_eq!(duration_label(83), "6 years 11 months");
    }
}
