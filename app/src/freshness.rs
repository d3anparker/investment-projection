//! Whether the tax figures on screen are still current.
//!
//! The *judgement* belongs to the tax system, not here: what counts as out of
//! date depends on the jurisdiction's own cycle, so this module only reads the
//! clock and asks. That keeps the UI free of any rule about April, tax years, or
//! how long is too long.
//!
//! Reading the clock is deliberately kept out of `calc`. A projection must stay
//! a pure function of the form — otherwise the same shared link would produce
//! different numbers depending on when it was opened, and the headless browser
//! suite would start failing on a date rather than on a change. So staleness is
//! computed here, at the edge, and never feeds back into the arithmetic.
//!
//! The clock read itself is the only `js_sys` in the module; [`describe`] takes
//! the date as an argument so it can be tested off the browser.

use taxkit::{SimpleDate, Staleness, TaxSystem};

/// Today, from the browser's clock, in local time.
///
/// Local rather than UTC on purpose: a tax year boundary is a local-calendar
/// fact, and a user just past midnight on 6 April should be told the year has
/// turned over.
pub fn today() -> SimpleDate {
    let now = js_sys::Date::new_0();
    SimpleDate::new(
        now.get_full_year() as u16,
        // `get_month` is zero-based; `get_date` is the day of the month.
        (now.get_month() + 1) as u8,
        now.get_date() as u8,
    )
}

/// The always-visible line naming the rules a projection used and when they were
/// last checked.
pub fn as_of_line(system: &dyn TaxSystem, rules: &str, checked: SimpleDate) -> String {
    format!(
        "{} {} tax rules. Figures last checked {}.",
        system.label(),
        rules,
        checked
    )
}

/// The warning shown when the figures look out of date, or `None` while they
/// still look current.
///
/// Deliberately a *warning* and not a refusal: the projection still runs, no
/// control is marked invalid, and nothing is disabled. Stale rates make the tax
/// figures indicative rather than meaningless, and blocking the whole page over
/// them would be a worse answer than showing them with a caveat.
pub fn stale_note(system: &dyn TaxSystem, today: SimpleDate) -> Option<String> {
    match system.staleness(today) {
        Staleness::Fresh => None,
        Staleness::Stale { current_period } => Some(format!(
            "These are {} rules, last checked {}. The current period is {}, so allowances and \
             rates have probably changed since. The projection still runs \u{2014} treat the tax \
             figures as indicative.",
            system.rules_label(),
            system.as_of(),
            current_period,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::active_system;

    #[test]
    fn the_as_of_line_names_the_system_the_rules_and_the_date() {
        let line = as_of_line(active_system(), "2026/27", SimpleDate::new(2026, 4, 6));
        assert!(line.contains(active_system().label()), "{line}");
        assert!(line.contains("2026/27"), "{line}");
        assert!(line.contains("6 April 2026"), "{line}");
    }

    #[test]
    fn figures_are_not_flagged_while_the_system_calls_them_fresh() {
        // Whatever the tax system's own cycle is, the day it was checked is
        // certainly not stale.
        assert!(stale_note(active_system(), active_system().as_of()).is_none());
    }

    #[test]
    fn a_stale_warning_names_both_periods_and_never_refuses() {
        // Far enough ahead that any jurisdiction's rules have turned over.
        let much_later = SimpleDate::new(active_system().as_of().year + 5, 6, 1);
        let note = stale_note(active_system(), much_later).expect("five years on must be stale");
        assert!(note.contains(active_system().rules_label()), "names what it used: {note}");
        assert!(
            note.contains("still runs"),
            "the warning must not read as a refusal: {note}"
        );
    }
}
