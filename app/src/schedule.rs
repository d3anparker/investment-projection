//! The month-by-month panel: how one withdrawal strategy would take the money
//! out, holding by holding, a year at a time.
//!
//! # A panel, not a page
//!
//! It was first planned as a page of its own, reached through the location
//! fragment. Most of what that cost was navigation machinery whose only job was
//! to undo the side effects of leaving: a router, the projection kept alive but
//! hidden, a history rewrite, focus put back on return. A page would also be a
//! snapshot, where a panel follows the form as it is edited. So it is a
//! collapsible panel below the comparison, opened from any comparison row.
//!
//! # Numbers
//!
//! Every figure comes from `calc::schedule`, which rounds each month so that it
//! adds up; this module only formats. It is a separate memo for the same reason
//! the comparison is: a strategy that cannot run, or empties the pot early, is
//! not an input error, so it must never dim the projection or flag a control.
//!
//! # Wording
//!
//! The comparison ranks nothing, and neither does this. It describes what a
//! strategy *would* take, never what anyone *should* withdraw.

use std::collections::BTreeSet;

use calc::{CalcInput, Schedule, ScheduleFigures, Strategy};
use leptos::*;
use rust_decimal::Decimal;
use taxkit::{SimpleDate, TaxSystem};

use crate::convert::StrategyChoice;
use crate::format::{fmt_money, fmt_pct, horizon_label, money_or_dash};
use crate::strategy::{probe, strategy_hint, strategy_label, strategy_options};

/// What the panel shows: the strategy, and its schedule — or the reason it
/// could not be drawn up.
#[derive(Clone, PartialEq, Debug)]
pub struct Shown {
    pub strategy: Strategy,
    pub result: Result<Schedule, String>,
}

/// The strategy the panel shows: the one it was pointed at, or, when it has not
/// been pointed at one, whatever the "Take it" picker says.
pub fn chosen(shown: &str, picked: &str) -> StrategyChoice {
    StrategyChoice::from_id(if shown.trim().is_empty() { picked } else { shown })
}

/// Project `input` under `strategy` and read it back month by month, through the
/// comparison's own [`probe`], so a strategy is projected here exactly as its
/// comparison row was.
pub fn show(input: &CalcInput, strategy: Strategy) -> Shown {
    let result = calc::schedule(&probe(input, &strategy)).map_err(|e| e.message);
    Shown { strategy, result }
}

/// "Year 3", or "Year 31 (6 months)" for a part year at the end.
pub fn year_label(year: u32, months: usize) -> String {
    match months {
        12 => format!("Year {year}"),
        m => format!("Year {year} ({})", horizon_label(m as u32)),
    }
}

/// The sentence restating what the strategy is asked to deliver. A net order
/// takes whatever it needs to leave the amount after tax; pro-rata takes the
/// amount itself, before tax, and this model does not tax it.
pub fn withdrawal_sentence(net: bool, amount: Decimal) -> String {
    if amount.is_zero() {
        "No monthly withdrawal is set, so nothing would be taken out.".to_string()
    } else if net {
        format!(
            "Each month this would take whatever it needs, before tax, to leave {} after tax.",
            fmt_money(amount)
        )
    } else {
        format!(
            "Each month this would take {} before tax, split across the holdings in \
             proportion to their value. Tax on it is not worked out here.",
            fmt_money(amount)
        )
    }
}

/// The columns after the holdings, in order. One list drives the heading, every
/// row's cells and the closing row's span, so a column cannot be added to one
/// and missed in another.
#[derive(Clone, Copy, PartialEq)]
enum Col {
    TakenOut,
    Tax,
    Received,
    Charged,
    Left,
}

impl Col {
    fn heading(self) -> &'static str {
        match self {
            Col::TakenOut => "Taken out",
            Col::Tax => "Tax",
            Col::Received => "You receive",
            Col::Charged => "Charged while invested",
            Col::Left => "Left in the portfolio",
        }
    }

    fn cell(self, f: &ScheduleFigures) -> String {
        match self {
            Col::TakenOut => money_or_dash(f.gross),
            Col::Tax => money_or_dash(f.tax),
            Col::Received => money_or_dash(f.net),
            Col::Charged => money_or_dash(f.charged),
            // A balance, not a flow: an empty pot reads as nothing left.
            Col::Left => fmt_money(f.value_after),
        }
    }
}

/// The columns this schedule needs. Tax only when tax was actually charged, so
/// a pro-rata or untaxed schedule keeps a narrow table — the breakdown's own
/// rule — and the holding charge only when one was levied in the drawdown.
fn columns(schedule: &Schedule) -> Vec<Col> {
    let has_tax = !schedule.output.tax_paid_total.is_zero();
    let has_charge = schedule.years.iter().any(|y| !y.figures.charged.is_zero());
    let mut cols = vec![Col::TakenOut];
    if has_tax {
        cols.extend([Col::Tax, Col::Received]);
    }
    if has_charge {
        cols.push(Col::Charged);
    }
    cols.push(Col::Left);
    cols
}

/// The month-by-month panel. Collapsed, it computes nothing: `shown` is `None`
/// and the body is empty.
///
/// `toggle` is the disclosure button, created by the caller so a comparison
/// row can move focus to it when it opens the panel.
#[component]
pub fn SchedulePanel(
    open: RwSignal<bool>,
    /// The `StrategyChoice` id the panel was pointed at; blank follows the
    /// picker. Written by the panel's select.
    chosen_id: RwSignal<String>,
    /// The strategy the panel shows, resolved by the caller from `chosen_id`
    /// and the picker.
    #[prop(into)]
    choice: Signal<StrategyChoice>,
    shown: Memo<Option<Shown>>,
    #[prop(into)] jurisdiction: Signal<String>,
    today: SimpleDate,
    toggle: NodeRef<html::Button>,
) -> impl IntoView {
    // Which years are open. Outside the table's render closure, so a schedule
    // that recomputes while the reader types keeps the years they opened.
    let expanded = create_rw_signal(BTreeSet::from([1u32]));
    let system = move || crate::jurisdiction::system_from(&jurisdiction.get());

    view! {
        <section class="panel panel-schedule" aria-labelledby="schedule-h">
            <h2 id="schedule-h" class="disclosure-h">
                <button type="button" class="disclosure" node_ref=toggle
                    aria-expanded=move || open.get().to_string()
                    aria-controls="schedule-body"
                    on:click=move |_| open.update(|o| *o = !*o)>
                    <span class="disclosure-mark" aria-hidden="true"></span>
                    "Month by month"
                </button>
            </h2>
            // Always in the DOM so the toggle's `aria-controls` names something;
            // its contents exist only while open.
            <div id="schedule-body" class="schedule-body" hidden=move || !open.get()>
                // The picker sits outside the closure that rebuilds on every
                // recomputation, so changing it never replaces the focused node.
                {move || open.get().then(|| view! {
                    <div class="period-row schedule-pick">
                        <label for="schedule-strategy">"Show months for"</label>
                        <select id="schedule-strategy"
                            on:change=move |ev| chosen_id.set(event_target_value(&ev))>
                            {strategy_options(move |c| choice.get() == c)}
                        </select>
                    </div>
                })}
                {move || {
                    // Subscribe to the currency so a jurisdiction switch
                    // re-renders the figures, as `panel::stale_body` does.
                    let _ = crate::format::currency();
                    let sys = system();
                    shown.with(|s| s.as_ref().map(|s| body(s, sys, today, expanded)))
                }}
            </div>
        </section>
    }
}

/// The strategy's description, the notes, and the table — or the reason there
/// is no table.
fn body(
    shown: &Shown,
    system: &'static dyn TaxSystem,
    today: SimpleDate,
    expanded: RwSignal<BTreeSet<u32>>,
) -> View {
    let intro = view! {
        <p class="schedule-strategy">
            <strong>{strategy_label(&shown.strategy)}</strong>
            {format!(". {}", strategy_hint(&shown.strategy))}
        </p>
    };
    let schedule = match &shown.result {
        Err(message) => {
            return view! {
                {intro}
                <p class="schedule-problem">{message.clone()}</p>
            }
            .into_view()
        }
        Ok(s) => s,
    };
    if schedule.years.is_empty() {
        return view! {
            {intro}
            <p class="schedule-problem">"This projection does not draw anything down."</p>
        }
        .into_view();
    }

    let out = &schedule.output;
    let sentence = withdrawal_sentence(shown.strategy.withdrawal_is_net(), schedule.withdrawal);
    let runs_out = schedule.runs_out.map(|r| {
        view! {
            <p class="depletion-note" role="note">
                <strong>{format!("The money runs out in Month {} (Year {}).", r.month, r.year)}</strong>
            </p>
        }
    });
    let breached = out.rate_cap_breached.then(|| view! {
        <p class="strategy-note">"The rate cap would have to be exceeded to deliver this income."</p>
    });

    view! {
        {intro}
        <p class="schedule-sentence">{sentence}</p>
        {runs_out}
        {breached}
        {crate::freshness::rules_note(system, out, today)}
        {table(schedule, system, expanded)}
    }
    .into_view()
}

/// The schedule as one table: a row for the starting values, then a row per
/// year that opens onto its months.
fn table(
    schedule: &Schedule,
    system: &'static dyn TaxSystem,
    expanded: RwSignal<BTreeSet<u32>>,
) -> impl IntoView {
    let out = &schedule.output;
    let cols = columns(schedule);

    // After the pot is empty every row would be a line of dashes; stop at the
    // year it runs out and say so once instead.
    let last = schedule.runs_out.map(|r| r.year);
    let shown_years = schedule.years.iter().take_while(|y| last.map_or(true, |l| y.year <= l));
    let ended = last.filter(|l| schedule.years.len() > *l as usize);

    let headings = out
        .investments
        .iter()
        .map(|inv| {
            let account = system.account_kind(&inv.account_kind).map(|k| k.short_label);
            let sub = match account {
                Some(a) => format!("{a} \u{00b7} {} a year", fmt_pct(inv.annualised)),
                None => format!("{} a year", fmt_pct(inv.annualised)),
            };
            view! {
                <th scope="col" class="num">
                    {inv.name.clone()}
                    <span class="col-sub">{sub}</span>
                </th>
            }
        })
        .collect_view();

    // The values the drawdown starts from: today's, when there is no growth
    // phase, otherwise the handover pot. Keyed on the figure, as elsewhere.
    let start_label = if out.horizon_months == 0 { "Value today" } else { "Value at the start" };
    let start_cells = out
        .investments
        .iter()
        .map(|inv| {
            let v = inv.handover_value.unwrap_or(inv.current_value);
            view! { <td class="num">{fmt_money(v)}</td> }
        })
        .collect_view();
    let start_total = fmt_money(out.handover_total.unwrap_or(out.current_total));
    // Nothing has moved yet, so only the balance column has a figure.
    let start_tail = cols
        .iter()
        .map(|c| view! { <td class="num">{(*c == Col::Left).then(|| start_total.clone())}</td> })
        .collect_view();

    let years = shown_years
        .map(|y| {
            let year = y.year;
            // A memo per year, so opening one notifies that year alone rather
            // than every year's button and months reading the whole set.
            let is_open = create_memo(move |_| expanded.with(|e| e.contains(&year)));
            let months = y.months.clone();
            let month_cols = cols.clone();
            let id = format!("schedule-y{year}");
            view! {
                <tbody class="schedule-year">
                    <tr>
                        <th scope="row">
                            <button type="button" class="year-toggle"
                                aria-expanded=move || is_open.get().to_string()
                                aria-controls=id.clone()
                                on:click=move |_| expanded.update(|e| {
                                    if !e.remove(&year) {
                                        e.insert(year);
                                    }
                                })>
                                <span class="disclosure-mark" aria-hidden="true"></span>
                                {year_label(year, y.months.len())}
                            </button>
                        </th>
                        {figures(&y.figures, &cols)}
                    </tr>
                </tbody>
                // The months, in a body of their own that the year's button
                // controls. Only this closure re-runs when the year is opened, so
                // the button keeps its node — and the reader keeps their focus.
                <tbody id=id class="schedule-months">
                    {move || is_open.get().then(|| months.iter().map(|m| view! {
                        <tr>
                            <th scope="row">{format!("Month {}", m.drawdown_month)}</th>
                            {figures(&m.figures, &month_cols)}
                        </tr>
                    }).collect_view())}
                </tbody>
            }
        })
        .collect_view();

    let span = 1 + out.investments.len() + cols.len();
    view! {
        <div class="table-scroll">
            <table class="breakdown schedule">
                <caption class="table-note">
                    "What this strategy would take from each holding, before tax, if the \
                     returns you entered held exactly. Open a year to see its months; each \
                     month\u{2019}s holdings add up to its total. An illustration of the \
                     projection, not an instruction."
                </caption>
                <thead>
                    <tr>
                        <th scope="col">"Period"</th>
                        {headings}
                        {cols.iter().map(|c| view! {
                            <th scope="col" class="num">{c.heading()}</th>
                        }).collect_view()}
                    </tr>
                </thead>
                <tbody class="schedule-start">
                    <tr>
                        <th scope="row">{start_label}</th>
                        {start_cells}
                        {start_tail}
                    </tr>
                </tbody>
                {years}
                {ended.map(|last| view! {
                    <tbody>
                        <tr class="schedule-end">
                            <td colspan=span.to_string()>
                                {format!("Nothing left to take after Year {last}.")}
                            </td>
                        </tr>
                    </tbody>
                })}
            </table>
        </div>
    }
}

/// One row's figures: each holding's cell, then the tail columns. Shared by the
/// year and month rows, so the two can never fall out of step with the header.
fn figures(f: &ScheduleFigures, cols: &[Col]) -> impl IntoView {
    view! {
        {f.drawn.iter().map(|d| view! { <td class="num">{money_or_dash(*d)}</td> }).collect_view()}
        {cols.iter().map(|c| view! { <td class="num">{c.cell(f)}</td> }).collect_view()}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calc::Plan;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn a_blank_choice_follows_the_picker() {
        assert_eq!(chosen("", StrategyChoice::Capped.id()), StrategyChoice::Capped);
        assert_eq!(chosen("  ", ""), StrategyChoice::ProRata);
        assert_eq!(
            chosen(StrategyChoice::Preserve.id(), StrategyChoice::Capped.id()),
            StrategyChoice::Preserve,
            "a strategy the panel was pointed at outranks the picker"
        );
    }

    #[test]
    fn a_part_year_says_how_long_it_is() {
        assert_eq!(year_label(3, 12), "Year 3");
        assert_eq!(year_label(31, 6), "Year 31 (6 months)");
        assert_eq!(year_label(2, 1), "Year 2 (1 month)");
    }

    #[test]
    fn the_sentence_says_whether_the_amount_is_before_or_after_tax() {
        let after = withdrawal_sentence(true, d("2000"));
        assert!(after.contains("\u{00a3}2,000.00 after tax"), "{after}");
        let before = withdrawal_sentence(false, d("2000"));
        assert!(before.contains("\u{00a3}2,000.00 before tax"), "{before}");
        assert!(withdrawal_sentence(true, d("0")).contains("nothing would be taken"));
    }

    #[test]
    fn nothing_in_the_panel_reads_as_advice() {
        // The same posture as the comparison: describe, never recommend.
        for text in [
            withdrawal_sentence(true, d("1500")),
            withdrawal_sentence(false, d("1500")),
            withdrawal_sentence(true, d("0")),
        ] {
            let lower = text.to_lowercase();
            for banned in ["should", "recommend", "best", "you must"] {
                assert!(!lower.contains(banned), "'{banned}' in: {text}");
            }
        }
    }

    #[test]
    fn show_projects_through_the_comparisons_probe() {
        let input = CalcInput {
            investments: vec![calc::InvestmentInput {
                name: "Fund".into(),
                value: "12000".into(),
                rate: "0".into(),
                contribution: "0".into(),
                ..Default::default()
            }],
            horizon_value: "0".into(),
            horizon_unit: calc::Unit::Months,
            plan: Plan::Drawdown {
                drawdown_value: "12".into(),
                drawdown_unit: calc::Unit::Months,
                withdrawal: "1,000".into(),
                strategy: Strategy::pro_rata(),
            },
            currency: String::new(),
            tax: None,
        };
        let s = show(&input, Strategy::preserve_growth());
        assert_eq!(s.strategy, Strategy::preserve_growth());
        let schedule = s.result.expect("an untaxed order-by-return projects");
        assert_eq!(schedule.withdrawal, d("1000"), "parsed by calc, as typed");
        assert_eq!(schedule.years.len(), 1);
        assert_eq!(schedule.years[0].figures.gross, d("12000.00"));
        assert_eq!(schedule.runs_out.map(|r| r.month), Some(12));
        // No tax and no charge: the narrow table.
        assert!(columns(&schedule) == [Col::TakenOut, Col::Left]);
    }
}
