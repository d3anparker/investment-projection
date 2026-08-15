//! The "reach a target" goal-seek form, marshalled between the reactive UI and
//! `calc::solve`.
//!
//! Like [`crate::convert`], this is the pure string-in/string-out layer around a
//! `calc` entry point: the two `<select>` values map to a `calc::Goal` variant,
//! and a returned `calc::Solution` maps to the sentence the summary panel shows.
//! Both halves are natively unit-tested — no signals, no DOM.

use crate::format::{fmt_money, horizon_label};
use calc::{parse_number, Goal, Scope, Solution};

/// The picker value that means "the whole portfolio" rather than a single
/// holding. Anything else is parsed as a holding index. Shared with `main.rs`
/// (the picker option) and `share.rs` (the default), so the string lives once.
pub const PORTFOLIO: &str = "portfolio";

/// Map the scope `<select>`'s value to a `calc::Scope`: the sentinel
/// [`PORTFOLIO`], or a holding index. An unparseable value falls back to the
/// whole portfolio — the always-valid choice — rather than an arbitrary holding.
pub fn parse_scope(scope: &str) -> Scope {
    match scope {
        PORTFOLIO => Scope::Portfolio,
        other => other.parse::<usize>().map(Scope::Holding).unwrap_or(Scope::Portfolio),
    }
}

/// Build the `calc::Goal` from the goal form's strings, or `None` when the
/// target is blank — the feature is inert until the user types a target, so an
/// empty box must not surface an error.
///
/// `kind` is the goal `<select>`'s value (`"time"` selects "time to reach it",
/// anything else the monthly-top-up default); `scope` is the scope `<select>`'s
/// value ([`PORTFOLIO`] or a holding index into the *filtered* `calc`
/// investments), the same in both goal kinds.
pub fn build_goal(kind: &str, target: &str, scope: &str) -> Option<Goal> {
    if target.trim().is_empty() {
        return None;
    }
    let scope = parse_scope(scope);
    Some(match kind {
        "time" => Goal::TimeToTarget { target: target.to_string(), scope },
        _ => Goal::MonthlyTopUp { target: target.to_string(), scope },
    })
}

/// The noun phrase naming what a goal is about, for the answer sentence: the
/// holding's own name, or "your whole portfolio". `holdings` is the picker's
/// `(index, name)` list. Routes through [`parse_scope`] so the scope-string
/// grammar (the sentinel, the index, the portfolio fallback) is interpreted in
/// exactly one place; a `Holding` index with no matching row falls back to the
/// portfolio phrase too.
pub fn subject_label(scope: &str, holdings: &[(usize, String)]) -> String {
    match parse_scope(scope) {
        Scope::Holding(i) => holdings.iter().find(|(j, _)| *j == i).map(|(_, name)| name.clone()),
        Scope::Portfolio => None,
    }
    .unwrap_or_else(|| "your whole portfolio".to_string())
}

/// One line of plain English for a solved goal, or the error message when the
/// goal could not be met. `subject` names what the goal is about (a holding, or
/// "your whole portfolio") so the answer states its own scope rather than
/// leaving the reader to guess it. `horizon` is the projection-horizon label
/// (e.g. `"10 years"`); a monthly-top-up answer names it because the top-up is
/// solved *for that period* — the figure is meaningless without it — whereas a
/// time-to-target answer computes its own period and ignores it. `target` is
/// echoed back through the same `fmt_money` the rest of the UI uses, so it reads
/// `£500,000.00`, not the raw keystrokes.
pub fn describe(solution: &Result<Solution, String>, target: &str, subject: &str, horizon: &str) -> String {
    let target_txt = parse_number(target).map(fmt_money).unwrap_or_else(|| target.to_string());
    match solution {
        Ok(Solution::MonthlyTopUp(amount)) => {
            format!("{} a month gets {} to {} over {}.", fmt_money(*amount), subject, target_txt, horizon)
        }
        Ok(Solution::Months(months)) => {
            format!("{} reaches {} in {}.", capitalise(subject), target_txt, duration_label(*months))
        }
        Ok(Solution::AlreadyMet) => {
            format!("{} is already on track to reach {}.", capitalise(subject), target_txt)
        }
        Err(msg) => msg.clone(),
    }
}

/// Capitalise the first character for a sentence-initial subject. Holding names
/// are usually already capitalised (idempotent here); it's the "your whole
/// portfolio" phrase that needs the lift to "Your whole portfolio".
fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
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
        // A pure run of months, or a whole number of years, reads exactly like a
        // horizon label — reuse it rather than repeat the pluralisation.
        (0, _) | (_, 0) => horizon_label(months),
        // The mixed span is the one shape `horizon_label` never produces.
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
    fn blank_target_makes_the_goal_inert() {
        assert!(build_goal("topup", "", "0").is_none());
        assert!(build_goal("time", "   ", "0").is_none());
    }

    #[test]
    fn build_goal_maps_the_select_to_a_variant() {
        // calc's Goal doesn't derive Debug/PartialEq for these, so match on it.
        match build_goal("time", "1000", PORTFOLIO) {
            Some(Goal::TimeToTarget { target, scope }) => {
                assert_eq!(target, "1000");
                assert_eq!(scope, Scope::Portfolio);
            }
            _ => panic!("time -> TimeToTarget"),
        }
        match build_goal("topup", "1000", "2") {
            Some(Goal::MonthlyTopUp { target, scope }) => {
                assert_eq!(target, "1000");
                assert_eq!(scope, Scope::Holding(2));
            }
            _ => panic!("topup -> MonthlyTopUp"),
        }
        // Unknown kind falls back to the top-up default, like the other selects.
        assert!(matches!(build_goal("", "1", "0"), Some(Goal::MonthlyTopUp { .. })));
    }

    #[test]
    fn parse_scope_reads_the_picker_value() {
        assert_eq!(parse_scope(PORTFOLIO), Scope::Portfolio);
        assert_eq!(parse_scope("0"), Scope::Holding(0));
        assert_eq!(parse_scope("3"), Scope::Holding(3));
        // A nonsense value is the whole portfolio, never a stray holding.
        assert_eq!(parse_scope("nope"), Scope::Portfolio);
    }

    #[test]
    fn subject_label_names_the_holding_or_the_portfolio() {
        let holdings = vec![(0, "Global Equity Fund".to_string()), (1, "Bonds".to_string())];
        assert_eq!(subject_label(PORTFOLIO, &holdings), "your whole portfolio");
        assert_eq!(subject_label("1", &holdings), "Bonds");
        // An out-of-range index falls back rather than panicking.
        assert_eq!(subject_label("9", &holdings), "your whole portfolio");
    }

    #[test]
    fn describe_reads_as_a_sentence_and_names_the_subject() {
        // A top-up answer names the period it was solved over.
        assert_eq!(
            describe(&Ok(Solution::MonthlyTopUp(dec("412.60"))), "500000", "Global Equity Fund", "10 years"),
            "\u{00a3}412.60 a month gets Global Equity Fund to \u{00a3}500,000.00 over 10 years."
        );
        // A time answer states its own computed span and ignores the horizon.
        assert_eq!(
            describe(&Ok(Solution::Months(83)), "50000", "your whole portfolio", "10 years"),
            "Your whole portfolio reaches \u{00a3}50,000.00 in 6 years 11 months."
        );
        assert_eq!(
            describe(&Ok(Solution::AlreadyMet), "40000", "Bonds", "10 years"),
            "Bonds is already on track to reach \u{00a3}40,000.00."
        );
        // An error passes straight through.
        assert_eq!(describe(&Err("nope".to_string()), "1", "your whole portfolio", "10 years"), "nope");
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
