//! The strategy comparison: the panel that actually answers "what is the best
//! way to draw this down?".
//!
//! # It reports several axes and ranks none of them
//!
//! This is the load-bearing design decision, not a styling choice. "Least tax"
//! is one axis, and optimising it hard is worse on every other one — the
//! lowest-tax order will happily drain a tax-free account early and leave a
//! wholly taxable one to come out at the top rate. A single "best" figure would
//! read as a recommendation however it were captioned, whereas a table of
//! consequences with no winner leaves the judgement where it belongs.
//!
//! So: rows in a fixed order, never sorted by outcome, no "recommended" badge,
//! no highlighted cell, no "you would save £X by switching".
//!
//! # Why it is a separate memo
//!
//! Following the precedent set by the goal answer: a strategy that empties the
//! pot early is not an *input* error, so it must never mark the form stale or
//! dim the projection panels. It renders its own outcome and leaves the rest of
//! the page alone.

use calc::{calculate, CalcInput, CalcOutput, Plan, Strategy};
use leptos::*;

use crate::convert::TAX_SYSTEM;
use crate::format::{fmt_money, month_label};

/// One strategy's outcome, reduced to the figures the table shows.
#[derive(Clone, PartialEq, Debug)]
pub struct Comparison {
    pub label: String,
    /// A one-line plain-English description of how this strategy takes the
    /// money, surfaced as the row's `title` tooltip. Generated from the
    /// strategy so it stays in step with the behaviour, never written per row.
    pub hint: String,
    /// What the strategy could not do, if anything. Rendered in place of the
    /// figures rather than as an error.
    pub problem: Option<String>,
    pub tax: String,
    pub kept: String,
    pub lasts: String,
    pub left: String,
    pub unused_allowance: String,
    pub accounts: String,
}

/// The em-dash every absent figure reads as.
const DASH: &str = "\u{2014}";

impl Comparison {
    /// A row with no figures in it. The error arm fills in `label` and
    /// `problem` over the top, so the dash literal lives in one place rather
    /// than once per column.
    fn blank() -> Comparison {
        Comparison {
            label: String::new(),
            hint: String::new(),
            problem: None,
            tax: DASH.into(),
            kept: DASH.into(),
            lasts: DASH.into(),
            left: DASH.into(),
            unused_allowance: DASH.into(),
            accounts: DASH.into(),
        }
    }
}

/// The strategies offered, in a fixed presentation order.
///
/// Labels are descriptive, never evaluative. The ordered one composes its label
/// from the account names in the order itself, so even that string is generated
/// from the tax system rather than written here.
pub fn candidates() -> Vec<(String, Strategy)> {
    let conventional: Vec<String> = TAX_SYSTEM
        .conventional_order()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    vec![
        ("Split across everything".to_string(), Strategy::ProRata),
        (conventional_label(&conventional), Strategy::Ordered { order: conventional }),
        ("Lowest tax this month".to_string(), Strategy::CheapestFirst),
        ("Longest-lasting pot".to_string(), Strategy::PreserveGrowth),
    ]
}

/// "Spend Cash, then Gains, then Income" — built from whatever the tax system's
/// conventional order actually contains.
///
/// Only spelled out for a short order. A catalogue of a dozen accounts truncated
/// to the first two reads as though those two were the point ("Spend GIA, then
/// VCT/EIS, then the rest"), which is worse than not naming them at all.
fn conventional_label(order: &[String]) -> String {
    let names: Vec<&str> = order
        .iter()
        .filter_map(|id| TAX_SYSTEM.account_kind(id))
        .map(|k| k.short_label)
        .collect();
    match names.len() {
        1..=3 => format!("Spend {}", names.join(", then ")),
        _ => "In the conventional order".to_string(),
    }
}

/// A plain-English gloss of what each strategy actually does, shown as the
/// row's `title` tooltip. Keyed off the strategy so it never drifts from the
/// behaviour, and descriptive rather than evaluative for the same reason the
/// labels are — no strategy is called good or bad here.
fn strategy_hint(strategy: &Strategy) -> String {
    match strategy {
        Strategy::ProRata => "Take a slice from every holding at once, in proportion to \
             its size. This is the app's default, and the amount is taken before tax."
            .to_string(),
        Strategy::Ordered { .. } => "Empty each kind of account in turn, in the order an \
             adviser conventionally suggests \u{2014} the simpler, lower-tax pots first."
            .to_string(),
        Strategy::CheapestFirst => "Each month, draw from whichever account is taxed least \
             on the next pound. It minimises this month's tax, not your lifetime tax."
            .to_string(),
        Strategy::PreserveGrowth => "Drain your lowest-returning holding first, leaving the \
             best compounder untouched for as long as possible. Needs no tax details."
            .to_string(),
        Strategy::RateCapped { .. } => "Draw from each account only while its tax rate stays \
             at or below the cap, then move on to the next."
            .to_string(),
    }
}

/// Run every candidate over the same input and reduce each to its row.
///
/// The strategy in `input` is replaced per candidate; everything else — the
/// holdings, the periods, the withdrawal, the tax details — is held constant, so
/// the rows differ in exactly one thing.
pub fn compare(input: &CalcInput) -> Vec<Comparison> {
    if !matches!(input.plan, Plan::Drawdown { .. }) {
        return Vec::new();
    }

    candidates()
        .into_iter()
        .map(|(label, strategy)| {
            let mut probe = input.clone();
            if let Plan::Drawdown { strategy: s, .. } = &mut probe.plan {
                *s = strategy.clone();
            }
            // Pro-rata ignores tax, so handing it a context would make its row
            // claim a tax year it never used. Same rule as `convert::tax_from`,
            // asked of the strategy rather than spelled out again.
            if !strategy.withdrawal_is_net() {
                probe.tax = None;
            }
            match calculate(&probe) {
                Ok(out) => row_for(label, &strategy, &out),
                Err(e) => Comparison {
                    label,
                    hint: strategy_hint(&strategy),
                    problem: Some(e.message),
                    ..Comparison::blank()
                },
            }
        })
        .collect()
}

fn row_for(label: String, strategy: &Strategy, out: &CalcOutput) -> Comparison {
    // A pro-rata withdrawal is gross and every other one is net, so the two are
    // not answering the same question. Say so *and* withhold the two figures
    // that would invite the comparison: printing £0.00 tax and a gross total
    // under "kept" is precisely the incomparable number the caveat warns about,
    // and a reader who skims the columns would take it as the best row.
    let untaxed = !strategy.withdrawal_is_net();
    let problem = if untaxed {
        Some("Takes this amount before tax, so it is not directly comparable.".to_string())
    } else if out.rate_cap_breached {
        Some("The rate cap had to be exceeded to deliver this income.".to_string())
    } else {
        None
    };

    let lasts = match out.depletion_month {
        Some(m) => format!("Runs out at {}", month_label(m)),
        None => "Lasts the whole period".to_string(),
    };
    // The average across the drawdown, not the total: "3 accounts a year" is a
    // measure of fiddliness, whereas a sum over thirty years is a big number
    // that means nothing.
    let accounts = out
        .accounts_touched_typical
        .map_or_else(|| DASH.to_string(), |a| a.to_string());

    let dash = DASH.to_string();
    Comparison {
        label,
        hint: strategy_hint(strategy),
        problem,
        tax: if untaxed { dash.clone() } else { fmt_money(out.tax_paid_total) },
        kept: if untaxed { dash.clone() } else { fmt_money(out.net_withdrawn_total) },
        lasts,
        left: fmt_money(out.projected_total),
        unused_allowance: if out.unused_allowance_total.is_zero() {
            DASH.to_string()
        } else {
            fmt_money(out.unused_allowance_total)
        },
        accounts,
    }
}

/// The comparison table.
#[component]
pub fn StrategyPanel(rows: Memo<Vec<Comparison>>) -> impl IntoView {
    view! {
        {move || {
            let rows = rows.get();
            (!rows.is_empty()).then(|| view! {
                <div class="strategy-compare">
                    <div class="table-scroll">
                        <table class="breakdown">
                            <caption class="table-note">
                                "Ways of taking the same income, and what each costs. \
                                 They are not ranked: which one is \u{201c}best\u{201d} depends on \
                                 whether you care most about tax, about how long the money lasts, \
                                 about what is left at the end, or about how little you want to \
                                 think about it."
                            </caption>
                            <thead>
                                <tr>
                                    <th scope="col"
                                        title="How the monthly income is taken from your holdings. Hover a row's name for what it does.">
                                        "Take it"
                                    </th>
                                    <th scope="col"
                                        title="Total tax paid across the whole drawdown period.">
                                        "Tax"
                                    </th>
                                    <th scope="col"
                                        title="Net income you actually receive, after tax, across the whole drawdown.">
                                        "Kept"
                                    </th>
                                    <th scope="col"
                                        title="Whether the pot lasts the full drawdown period, or the month it runs out.">
                                        "How long"
                                    </th>
                                    <th scope="col"
                                        title="What the portfolio is still worth when the drawdown period ends.">
                                        "Left at the end"
                                    </th>
                                    <th scope="col"
                                        title="Tax-free allowance left unclaimed \u{2014} headroom you could have drawn tax-free but did not. Lower means more of your allowances were used.">
                                        "Allowance unused"
                                    </th>
                                    <th scope="col"
                                        title="How many separate accounts you draw from in a typical year \u{2014} a measure of how fiddly the strategy is to run.">
                                        "Accounts a year"
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.into_iter().map(|r| view! {
                                    <tr>
                                        <th scope="row" title=r.hint>
                                            {r.label}
                                            {r.problem.map(|p| view! {
                                                <span class="strategy-note">{p}</span>
                                            })}
                                        </th>
                                        <td class="num">{r.tax}</td>
                                        <td class="num">{r.kept}</td>
                                        <td>{r.lasts}</td>
                                        <td class="num">{r.left}</td>
                                        <td class="num">{r.unused_allowance}</td>
                                        <td class="num">{r.accounts}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                </div>
            })
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calc::{InvestmentInput, Unit};

    fn drawdown_input(strategy: Strategy) -> CalcInput {
        CalcInput {
            investments: vec![InvestmentInput {
                name: "Fund".into(),
                value: "300000".into(),
                rate: "5".into(),
                contribution: "0".into(),
                ..Default::default()
            }],
            horizon_value: "1".into(),
            horizon_unit: Unit::Months,
            plan: Plan::Drawdown {
                drawdown_value: "120".into(),
                drawdown_unit: Unit::Months,
                withdrawal: "1000".into(),
                strategy,
            },
            tax: None,
        }
    }

    #[test]
    fn every_candidate_produces_a_row_in_a_fixed_order() {
        let first = compare(&drawdown_input(Strategy::ProRata));
        let second = compare(&drawdown_input(Strategy::CheapestFirst));
        assert_eq!(first.len(), candidates().len());
        let labels: Vec<&str> = first.iter().map(|r| r.label.as_str()).collect();
        let again: Vec<&str> = second.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels, again,
            "row order must not depend on the strategy in the form, or on the figures"
        );
    }

    #[test]
    fn a_strategy_that_cannot_run_reports_instead_of_failing_the_comparison() {
        // No tax details, so the tax-aware orders cannot run. They must come back
        // as rows carrying a reason, not blow up the whole table.
        let rows = compare(&drawdown_input(Strategy::ProRata));
        assert_eq!(rows.len(), candidates().len(), "every candidate still gets a row");
        assert!(
            rows.iter().any(|r| r.problem.is_some()),
            "the ones that could not run must say why"
        );
    }

    #[test]
    fn the_untaxed_row_withholds_the_figures_that_are_not_comparable() {
        // Its withdrawal is gross where every other row's is net, so printing
        // "£0.00 tax" and a gross total under "kept" would read as the best row
        // to anyone skimming the columns.
        let rows = compare(&drawdown_input(Strategy::ProRata));
        let pro_rata = rows.first().expect("pro-rata is the first candidate");
        assert!(pro_rata.problem.is_some(), "and it says why");
        assert_eq!(pro_rata.tax, "\u{2014}");
        assert_eq!(pro_rata.kept, "\u{2014}");
        // How long it lasts and what is left *are* comparable, so they stay.
        assert_ne!(pro_rata.lasts, "\u{2014}");
        assert_ne!(pro_rata.left, "\u{2014}");
    }

    #[test]
    fn deposits_mode_has_nothing_to_compare() {
        let mut input = drawdown_input(Strategy::ProRata);
        input.plan = Plan::Deposits;
        assert!(compare(&input).is_empty());
    }

    #[test]
    fn a_short_order_names_its_accounts_from_the_catalogue() {
        // Never written out: the accounts named have to be ones the tax system
        // actually advertises, or the label would drift from the behaviour.
        let ids: Vec<String> = TAX_SYSTEM
            .account_kinds()
            .iter()
            .take(2)
            .map(|k| k.id.to_string())
            .collect();
        let label = conventional_label(&ids);
        for id in &ids {
            let k = TAX_SYSTEM.account_kind(id).expect("taken from the catalogue");
            assert!(label.contains(k.short_label), "{label}");
        }
    }

    #[test]
    fn a_long_order_falls_back_rather_than_naming_the_first_two() {
        // Truncating a dozen accounts to two reads as though those two were the
        // point, which is more misleading than not naming any.
        let all: Vec<String> = TAX_SYSTEM
            .conventional_order()
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        if all.len() > 3 {
            assert_eq!(conventional_label(&all), "In the conventional order");
        }
        assert_eq!(conventional_label(&[]), "In the conventional order");
    }
}
