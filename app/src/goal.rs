//! The goal-seek form, marshalled between the reactive UI and `calc::solve`.
//!
//! Like [`crate::convert`], this is the pure string-in/string-out layer around a
//! `calc` entry point: the two `<select>` values map to a `calc::Goal` variant,
//! and a returned `calc::Solution` maps to the sentence the summary panel shows.
//! Both halves are natively unit-tested — no signals, no DOM.
//!
//! There are two directions of question. The "reach a target" pair asks what it
//! takes to *get* to a figure (a monthly top-up, or a span of time); the "draw it
//! down" pair is their inverse for a portfolio being spent (the monthly
//! withdrawal that still leaves a floor at the horizon, or how long a given
//! withdrawal lasts). One amount box serves all four — target, floor, or
//! withdrawal — which is why the wording around it moves with the kind.

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

/// The goal-seek question, parsed from the kind `<select>`'s string value. The
/// single interpreter of that string — the counterpart to
/// [`crate::convert::mode_from`]/`unit_from`/`flow_from` for the other selects —
/// so the known set and the "unknown → top-up" default live here once, rather
/// than being re-spelled at every site that reacts to the kind: [`build_goal`],
/// the option-selected predicate, and the box's label and placeholder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalKind {
    /// Reach a target with a monthly top-up. The default, so an unknown value
    /// (a shared link from a newer build) lands here rather than nowhere.
    TopUp,
    /// Reach a target on a fixed contribution — solve for the time it takes.
    Time,
    /// The largest monthly withdrawal that still leaves a floor at the horizon.
    Withdrawal,
    /// How long a fixed monthly withdrawal lasts before the pot runs dry.
    Lasts,
}

impl GoalKind {
    /// Map the kind `<select>`'s value to a variant; anything unrecognised is
    /// the top-up default (the same fallback [`build_goal`] applies).
    pub fn parse(kind: &str) -> GoalKind {
        match kind {
            "time" => GoalKind::Time,
            "withdrawal" => GoalKind::Withdrawal,
            "lasts" => GoalKind::Lasts,
            _ => GoalKind::TopUp,
        }
    }

    /// The sentence-initial label for the single amount box, which means a
    /// different thing per kind: a target to *reach*, a floor to *leave*, or a
    /// sum to *withdraw*.
    pub fn label(self) -> &'static str {
        match self {
            GoalKind::Withdrawal => "Leave",
            GoalKind::Lasts => "Withdraw",
            GoalKind::TopUp | GoalKind::Time => "Reach",
        }
    }

    /// The amount box's placeholder. Under a withdrawal a blank box is a real
    /// answer ("leave nothing"), so it shows the value an empty box stands for
    /// rather than a suggestion.
    pub fn placeholder(self) -> &'static str {
        match self {
            GoalKind::Withdrawal => "0",
            GoalKind::Lasts => "1,500",
            GoalKind::TopUp | GoalKind::Time => "500,000",
        }
    }

    /// Whether the amount box names a *recurring* sum — only "how long it lasts"
    /// does, so only it earns the "a month" suffix beside the box.
    pub fn is_recurring(self) -> bool {
        matches!(self, GoalKind::Lasts)
    }
}

/// Build the `calc::Goal` from the goal form's strings, or `None` when the goal
/// is inert.
///
/// `kind` is the goal `<select>`'s value (`"time"`, `"withdrawal"` and `"lasts"`
/// name the three non-default questions, anything else the monthly-top-up
/// default); `scope` is the scope `<select>`'s value ([`PORTFOLIO`] or a holding
/// index into the *filtered* `calc` investments), the same in every goal kind.
///
/// The blank-box rule is **kind-aware**. Three of the four kinds need a figure to
/// aim at, so an empty box keeps the feature inert rather than surfacing an error
/// the user never asked for. "Spend it down to nothing by the horizon" is a
/// perfectly good question though, so a blank box under `"withdrawal"` *is* the
/// floor £0 — selecting the kind answers immediately instead of demanding a zero
/// be typed in.
pub fn build_goal(kind: &str, target: &str, scope: &str) -> Option<Goal> {
    let blank = target.trim().is_empty();
    let kind = GoalKind::parse(kind);
    // The withdrawal kind is the one where a blank box is itself the answer
    // (floor £0 — see the doc comment); every other kind needs a figure to aim at.
    if blank && kind != GoalKind::Withdrawal {
        return None;
    }
    let scope = parse_scope(scope);
    let text = target.to_string();
    Some(match kind {
        GoalKind::Time => Goal::TimeToTarget { target: text, scope },
        GoalKind::Withdrawal => {
            Goal::MaxWithdrawal { floor: if blank { "0".to_string() } else { text }, scope }
        }
        GoalKind::Lasts => Goal::TimeToDeplete { amount: text, scope },
        GoalKind::TopUp => Goal::MonthlyTopUp { target: text, scope },
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
/// (e.g. `"10 years"`); the two "solve for an amount" answers name it because the
/// figure is solved *for that period* and is meaningless without it, whereas the
/// two "solve for a span" answers compute their own period and ignore it.
/// `target` is the one amount box — a target, a floor or a monthly withdrawal
/// depending on the kind — echoed back through the same `fmt_money` the rest of
/// the UI uses, so it reads `£500,000.00`, not the raw keystrokes.
///
/// The `Solution` variant alone decides the sentence shape; the kind is never
/// passed in, because `calc` has already answered the question that was asked.
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
        // Here `target` is the *floor* left at the horizon. Leaving nothing
        // behind is a different sentence, not a "£0.00" one — saying "still leave
        // £0.00" would read as a rounding artefact rather than the intent.
        Ok(Solution::MaxWithdrawal(amount)) if floor_is_zero(target) => format!(
            "You can take out {} a month from {} and spend it to zero by {}.",
            fmt_money(*amount),
            subject,
            horizon
        ),
        Ok(Solution::MaxWithdrawal(amount)) => format!(
            "You can take out {} a month from {} and still leave {} after {}.",
            fmt_money(*amount),
            subject,
            target_txt,
            horizon
        ),
        // For both depletion answers `target` is the monthly withdrawal being
        // tested, which has to be restated — "runs dry in 8 years" means nothing
        // without the sum being drawn.
        Ok(Solution::Depletes(months)) => {
            format!("Drawing {} a month, {} runs dry in {}.", target_txt, subject, duration_label(*months))
        }
        Ok(Solution::NeverDepletes) => format!(
            "Drawing {} a month, {} never runs dry \u{2014} its returns cover the withdrawals.",
            target_txt, subject
        ),
        Err(msg) => msg.clone(),
    }
}

/// Whether a max-withdrawal floor means "leave nothing behind": a blank box
/// (which [`build_goal`] reads as £0) or a typed zero. A wording branch only —
/// every reported figure still comes from `calc`, and the parse goes through
/// `calc::parse_number` so `"0"`, `"£0"` and `"0.00"` all read alike.
fn floor_is_zero(floor: &str) -> bool {
    parse_number(floor).map_or(true, |d| d.is_zero())
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
        // "How long does £nothing a month last?" is not a question.
        assert!(build_goal("lasts", "", "0").is_none());
    }

    #[test]
    fn blank_floor_still_asks_the_max_withdrawal_question() {
        // The one kind where an empty box means something: spend it to nothing.
        match build_goal("withdrawal", "  ", PORTFOLIO) {
            Some(Goal::MaxWithdrawal { floor, scope }) => {
                assert_eq!(floor, "0");
                assert_eq!(scope, Scope::Portfolio);
            }
            _ => panic!("blank withdrawal floor -> MaxWithdrawal with floor 0"),
        }
    }

    #[test]
    fn build_goal_maps_the_drawdown_kinds() {
        match build_goal("withdrawal", "50000", "1") {
            Some(Goal::MaxWithdrawal { floor, scope }) => {
                assert_eq!(floor, "50000");
                assert_eq!(scope, Scope::Holding(1));
            }
            _ => panic!("withdrawal -> MaxWithdrawal"),
        }
        match build_goal("lasts", "1500", PORTFOLIO) {
            Some(Goal::TimeToDeplete { amount, scope }) => {
                assert_eq!(amount, "1500");
                assert_eq!(scope, Scope::Portfolio);
            }
            _ => panic!("lasts -> TimeToDeplete"),
        }
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
    fn goal_kind_parses_and_defaults_to_top_up() {
        assert_eq!(GoalKind::parse("time"), GoalKind::Time);
        assert_eq!(GoalKind::parse("withdrawal"), GoalKind::Withdrawal);
        assert_eq!(GoalKind::parse("lasts"), GoalKind::Lasts);
        assert_eq!(GoalKind::parse("topup"), GoalKind::TopUp);
        // Unknown/empty is the default — the same rule build_goal's fallback uses.
        assert_eq!(GoalKind::parse(""), GoalKind::TopUp);
        assert_eq!(GoalKind::parse("future-kind"), GoalKind::TopUp);
        // Only the depletion question is recurring (drives the "a month" suffix).
        assert!(GoalKind::Lasts.is_recurring());
        assert!(!GoalKind::Withdrawal.is_recurring());
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
    fn describe_reads_the_drawdown_answers() {
        // A zero floor (typed or blank) is the "spend it to nothing" wording,
        // and the withdrawal names the horizon it was solved over.
        assert_eq!(
            describe(&Ok(Solution::MaxWithdrawal(dec("2450.13"))), "0", "your whole portfolio", "25 years"),
            "You can take out \u{00a3}2,450.13 a month from your whole portfolio and spend it to zero by 25 years."
        );
        assert_eq!(
            describe(&Ok(Solution::MaxWithdrawal(dec("1000"))), "", "Bonds", "10 years"),
            "You can take out \u{00a3}1,000.00 a month from Bonds and spend it to zero by 10 years."
        );
        // A floor left behind is stated, through the same money formatting.
        assert_eq!(
            describe(&Ok(Solution::MaxWithdrawal(dec("1875.40"))), "50000", "Global Equity Fund", "20 years"),
            "You can take out \u{00a3}1,875.40 a month from Global Equity Fund and still leave \u{00a3}50,000.00 after 20 years."
        );
        // A depletion answer computes its own span and restates the draw.
        assert_eq!(
            describe(&Ok(Solution::Depletes(83)), "1500", "your whole portfolio", "10 years"),
            "Drawing \u{00a3}1,500.00 a month, your whole portfolio runs dry in 6 years 11 months."
        );
        assert_eq!(
            describe(&Ok(Solution::NeverDepletes), "1500", "Bonds", "10 years"),
            "Drawing \u{00a3}1,500.00 a month, Bonds never runs dry \u{2014} its returns cover the withdrawals."
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
