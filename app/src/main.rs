//! Leptos (CSR) front end for the investment projection tool.
//!
//! This layer owns the reactive form state and *formats* the `Decimal`s that
//! the `calc` crate returns. It performs no financial arithmetic itself — every
//! number is produced by `calc::calculate`.

mod chart;
mod format;

use calc::{
    calculate, CalcError, CalcInput, CalcOutput, Field, InvestmentField, InvestmentInput, Mode, Unit,
};
use chart::{chart_svg, PLOT_BOTTOM_FRAC, PLOT_LEFT_FRAC, PLOT_TOP_FRAC, PLOT_WIDTH_FRAC};
use format::{fmt_money, fmt_pct, fmt_signed_money, horizon_label, month_label};
use leptos::leptos_dom::helpers::TimeoutHandle;
use leptos::*;
use std::time::Duration;

/// Id of the visible error paragraph. Invalid controls point at it with
/// `aria-describedby`, so the message is read out with the field rather than
/// stranded at the bottom of the form.
const ERROR_ID: &str = "calc-error";

/// How long typing must settle before the error is announced. Long enough that
/// a keystroke mid-word doesn't queue a message, short enough to feel prompt.
const ANNOUNCE_DELAY: Duration = Duration::from_millis(700);

/// One recomputation's result plus the mapping needed to interpret it.
///
/// `calc` reports errors against an index into the investments it was given,
/// but blank rows are filtered out before it sees them — so index 1 is not
/// necessarily the second row on screen. `row_ids` translates back.
#[derive(Clone, PartialEq)]
struct Outcome {
    result: Result<CalcOutput, CalcError>,
    row_ids: Vec<usize>,
}

impl Outcome {
    fn error(&self) -> Option<&CalcError> {
        self.result.as_ref().err()
    }

    fn message(&self) -> Option<String> {
        self.error().map(|e| e.message.clone())
    }

    /// Is the current error about this row's `part`?
    fn flags(&self, row_id: usize, part: InvestmentField) -> bool {
        match self.error().and_then(|e| e.field) {
            Some(Field::Investment { index, part: failed }) => {
                failed == part && self.row_ids.get(index).copied() == Some(row_id)
            }
            _ => false,
        }
    }

    fn flags_horizon(&self) -> bool {
        matches!(self.error().and_then(|e| e.field), Some(Field::Horizon))
    }
}

/// `aria-invalid`/`aria-describedby` values for a control, or `None` to leave
/// both attributes off entirely.
fn invalid_attrs(flagged: bool) -> (Option<&'static str>, Option<&'static str>) {
    if flagged {
        (Some("true"), Some(ERROR_ID))
    } else {
        (None, None)
    }
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });
}

/// One editable row. Every field is its own signal so typing in one cell does
/// not disturb the others. `Copy` because all fields are `Copy`.
#[derive(Clone, Copy)]
struct Row {
    id: usize,
    name: RwSignal<String>,
    value: RwSignal<String>,
    mode: RwSignal<String>,
    rate: RwSignal<String>,
    contribution: RwSignal<String>,
    /// The row's own remove button. Held here rather than created inside the
    /// `For` body so a *sibling* row's handler can reach it to place focus once
    /// this row is gone — see `remove_row`.
    remove_btn: NodeRef<html::Button>,
}

fn new_row(
    counter: StoredValue<usize>,
    name: &str,
    value: &str,
    mode: &str,
    rate: &str,
    contribution: &str,
) -> Row {
    let id = counter.get_value();
    counter.set_value(id + 1);
    Row {
        id,
        name: create_rw_signal(name.to_string()),
        value: create_rw_signal(value.to_string()),
        mode: create_rw_signal(mode.to_string()),
        rate: create_rw_signal(rate.to_string()),
        contribution: create_rw_signal(contribution.to_string()),
        remove_btn: create_node_ref(),
    }
}

/// Tooltip and accessible name for a row's remove button. Every button would
/// otherwise read as a bare "Remove investment", leaving them indistinguishable
/// in a screen reader's element list — which matters more now that focus lands
/// on one of them after a removal.
///
/// Named rows get "Remove Global Equity Fund"; the fallback numbers the row by
/// position so several *unnamed* rows are still told apart. Reading both signals
/// keeps the label live: renaming a holding, or removing one above it, updates it.
fn remove_label(r: Row, rows: RwSignal<Vec<Row>>) -> String {
    let name = r.name.get();
    let name = name.trim();
    if !name.is_empty() {
        return format!("Remove {name}");
    }
    match rows.with(|v| v.iter().position(|x| x.id == r.id)) {
        Some(i) => format!("Remove investment {}", i + 1),
        None => "Remove investment".to_string(),
    }
}

/// Remove the row with `id` and move focus somewhere sensible. Without this the
/// button the user just activated is torn out of the DOM and focus falls back to
/// `<body>`, dropping a keyboard user at the top of the page with the whole form
/// to tab through again.
///
/// Focus lands on the remove button of the row that slid into the vacated slot,
/// or the row above when the last one went, or `add_btn` when the list is empty.
/// No `request_animation_frame` needed: the `For` is keyed by `Row::id`, so every
/// surviving row keeps its DOM node and its `NodeRef` is already populated by the
/// time `update` returns.
fn remove_row(rows: RwSignal<Vec<Row>>, id: usize, add_btn: NodeRef<html::Button>) {
    let mut successor = None;
    rows.update(|v| {
        let Some(i) = v.iter().position(|x| x.id == id) else {
            return;
        };
        v.remove(i);
        successor = v.get(i).or_else(|| v.last()).copied();
    });
    match successor.and_then(|s| s.remove_btn.get_untracked()) {
        Some(btn) => {
            let _ = btn.focus();
        }
        None => {
            if let Some(btn) = add_btn.get_untracked() {
                let _ = btn.focus();
            }
        }
    }
}

/// Bind a text `<input>`'s value to `sig` without the caret-reset a plain
/// reactive `prop:value` causes. On every signal change Leptos would re-assign
/// `input.value`, which browsers treat as a fresh value and bounce the caret to
/// the end — disruptive when editing mid-string. Here the effect writes the DOM
/// value only when it actually differs from the signal, so ordinary typing
/// (where the DOM is already in sync, the edit having come *from* the input)
/// never triggers a write and the caret stays put; an external `sig.set(..)`
/// still updates the field.
fn bind_value(sig: RwSignal<String>) -> NodeRef<html::Input> {
    let node = create_node_ref::<html::Input>();
    create_effect(move |_| {
        let v = sig.get();
        // Tracked `get()` so the effect re-runs once the node mounts and applies
        // the initial value.
        if let Some(input) = node.get() {
            if input.value() != v {
                input.set_value(&v);
            }
        }
    });
    node
}

#[component]
fn App() -> impl IntoView {
    let counter = store_value(0usize);
    let rows = create_rw_signal(vec![
        new_row(counter, "Global Equity Fund", "10000", "annual", "7", "200"),
        new_row(counter, "Government Bond Fund", "5000", "total", "80", "0"),
    ]);
    let horizon_value = create_rw_signal("10".to_string());
    let horizon_unit = create_rw_signal("years".to_string());

    // Single source of computed truth. Reading every field's signal here means
    // the projection recomputes whenever any input changes; the memo caches the
    // result so `calculate` runs once even though we read it in two places
    // (error line + results panel).
    let outcome = create_memo(move |_| {
        // Collect the row ids alongside the inputs: filtering blank rows breaks
        // the correspondence between calc's indices and the rows on screen, and
        // that mapping is what lets an error mark the right control.
        let mut row_ids = Vec::new();
        let investments: Vec<InvestmentInput> = rows
            .get()
            .iter()
            .filter_map(|r| {
                let value = r.value.get();
                let rate = r.rate.get();
                let contribution = r.contribution.get();
                // Skip blank rows so a half-typed row doesn't error the form.
                if value.trim().is_empty()
                    && rate.trim().is_empty()
                    && contribution.trim().is_empty()
                {
                    return None;
                }
                let name = r.name.get();
                row_ids.push(r.id);
                Some(InvestmentInput {
                    name: if name.trim().is_empty() { "Investment".into() } else { name },
                    value: blank_zero(value),
                    mode: mode_from(&r.mode.get()),
                    rate: blank_zero(rate),
                    contribution: blank_zero(contribution),
                })
            })
            .collect();
        let input = CalcInput {
            investments,
            horizon_value: blank_zero(horizon_value.get()),
            horizon_unit: unit_from(&horizon_unit.get()),
        };
        Outcome { result: calculate(&input), row_ids }
    });

    // Hold the last successful projection. Recomputing on every keystroke means
    // a half-typed number briefly fails, and blanking the whole panel for that
    // is both jarring and misleading — the results are stale, not absent. The
    // memo's own previous value is exactly the "last good" we want.
    let displayed = create_memo(move |prev: Option<&Option<CalcOutput>>| {
        let current = outcome.get();
        match current.result {
            Ok(out) => Some(out),
            // An empty form is genuinely empty, not stale — don't keep showing a
            // projection for holdings the user has just deleted.
            Err(_) if current.row_ids.is_empty() => None,
            Err(_) => prev.cloned().flatten(),
        }
    });

    // The visible message updates immediately; the announcement waits for a
    // pause. Each keystroke cancels the previous pending announcement, so a
    // screen reader hears one settled message instead of a running commentary.
    let announced = create_rw_signal(String::new());
    create_effect(move |prev: Option<Option<TimeoutHandle>>| {
        if let Some(Some(handle)) = prev {
            handle.clear();
        }
        let msg = outcome.get().message().unwrap_or_default();
        set_timeout_with_handle(move || announced.set(msg), ANNOUNCE_DELAY).ok()
    });

    // Focus falls back here when the last row is removed and there is no
    // sibling button left to step to.
    let add_btn = create_node_ref::<html::Button>();
    let add_row = move |_| {
        let row = new_row(counter, "", "", "annual", "", "");
        rows.update(|v| v.push(row));
    };
    let horizon_ref = bind_value(horizon_value);

    view! {
        <div class="wrap">
            <header class="site-head">
                <h1>"Investment Projection"</h1>
                <p class="tagline">
                    "Extrapolate the future value of a portfolio from a forward-looking \
                     return rate you supply for each holding. Every calculation runs in \
                     exact decimal arithmetic (Rust \u{2192} WebAssembly)."
                </p>
            </header>

            <div class="disclaimer" role="note">
                <strong>"Not financial advice."</strong>
                " This tool is for entertainment and curiosity, not planning. It performs a \
                 mathematical extrapolation from a return rate you supply \u{2014} it does not \
                 predict anything, and real returns vary. Nothing here is a recommendation to \
                 buy, sell, or hold any investment."
            </div>

            <main class="layout">
                <section class="panel panel-summary" aria-labelledby="projection-h">
                    <h2 id="projection-h">"Projection"</h2>
                    {move || {
                        let stale = outcome.get().error().is_some();
                        match displayed.get() {
                            Some(out) => view! {
                                <div class="results-body" class:stale=stale
                                     aria-busy=move || stale.then_some("true")>
                                    {summary_view(out)}
                                </div>
                            }.into_view(),
                            None => empty_summary_view().into_view(),
                        }
                    }}
                </section>

                <section class="panel">
                    <h2>"Your investments"</h2>
                    <div class="inv-editor">
                        <div class="inv-head" aria-hidden="true">
                            <span>"Name"</span>
                            <span>"Value today"</span>
                            <span>"Monthly top-up"</span>
                            <span>"Return figure"</span>
                            <span></span>
                        </div>
                        <For each=move || rows.get() key=|r| r.id children=move |r| {
                            // `node_ref` needs a plain `NodeRef` binding, so create
                            // the guarded refs as locals before the template.
                            let name_ref = bind_value(r.name);
                            let value_ref = bind_value(r.value);
                            let contribution_ref = bind_value(r.contribution);
                            let rate_ref = bind_value(r.rate);
                            // `node_ref` takes a plain binding, not a field access.
                            let remove_ref = r.remove_btn;
                            view! {
                            <div class="inv-row">
                                <label class="fld">
                                    <span class="fld-lbl">"Name"</span>
                                    <input
                                        type="text" placeholder="e.g. Equity Fund"
                                        node_ref=name_ref
                                        on:input=move |ev| r.name.set(event_target_value(&ev)) />
                                </label>
                                <label class="fld">
                                    <span class="fld-lbl">"Value today"</span>
                                    <span class="adorn adorn-money">
                                        <input
                                            type="text" inputmode="decimal"
                                            placeholder="10000"
                                            node_ref=value_ref
                                            aria-invalid=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Value)).0
                                            aria-describedby=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Value)).1
                                            class:field-invalid=move || outcome.get().flags(r.id, InvestmentField::Value)
                                            on:input=move |ev| r.value.set(event_target_value(&ev)) />
                                    </span>
                                </label>
                                <label class="fld">
                                    <span class="fld-lbl">"Monthly top-up"</span>
                                    <span class="adorn adorn-money">
                                        <input
                                            type="text" inputmode="decimal"
                                            placeholder="100"
                                            node_ref=contribution_ref
                                            aria-invalid=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Contribution)).0
                                            aria-describedby=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Contribution)).1
                                            class:field-invalid=move || outcome.get().flags(r.id, InvestmentField::Contribution)
                                            on:input=move |ev| r.contribution.set(event_target_value(&ev)) />
                                    </span>
                                </label>
                                <div class="fld fld-return">
                                    <span class="fld-lbl">"Return figure"</span>
                                    <div class="return-control">
                                        <span class="adorn adorn-pct">
                                            <input
                                                type="text" inputmode="decimal"
                                                placeholder="7"
                                                aria-label="Return percentage"
                                                node_ref=rate_ref
                                                aria-invalid=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Rate)).0
                                                aria-describedby=move || invalid_attrs(outcome.get().flags(r.id, InvestmentField::Rate)).1
                                                class:field-invalid=move || outcome.get().flags(r.id, InvestmentField::Rate)
                                                on:input=move |ev| r.rate.set(event_target_value(&ev)) />
                                        </span>
                                        <select
                                            aria-label="Return basis: per year or total over the whole period"
                                            on:change=move |ev| r.mode.set(event_target_value(&ev))>
                                            <option value="annual" selected=move || r.mode.get() == "annual">"a year"</option>
                                            <option value="total" selected=move || r.mode.get() == "total">"total"</option>
                                        </select>
                                    </div>
                                </div>
                                <button
                                    class="btn btn-remove"
                                    title=move || remove_label(r, rows)
                                    aria-label=move || remove_label(r, rows)
                                    node_ref=remove_ref
                                    on:click=move |_| remove_row(rows, r.id, add_btn)>
                                    <span class="rm-x" aria-hidden="true">"\u{00d7}"</span>
                                    <span class="rm-label">"Remove"</span>
                                </button>
                            </div>
                            }
                        } />

                    </div>

                    <button type="button" class="btn btn-ghost" node_ref=add_btn on:click=add_row>
                        "+ Add investment"
                    </button>

                    <div class="horizon">
                        <label for="horizon-value">"Project"</label>
                        <input
                            id="horizon-value" type="number" min="1" step="1" inputmode="numeric"
                            node_ref=horizon_ref
                            aria-invalid=move || invalid_attrs(outcome.get().flags_horizon()).0
                            aria-describedby=move || invalid_attrs(outcome.get().flags_horizon()).1
                            class:field-invalid=move || outcome.get().flags_horizon()
                            on:input=move |ev| horizon_value.set(event_target_value(&ev)) />
                        <select
                            aria-label="Projection unit"
                            on:change=move |ev| horizon_unit.set(event_target_value(&ev))>
                            <option value="years" selected=move || horizon_unit.get() == "years">"years"</option>
                            <option value="months" selected=move || horizon_unit.get() == "months">"months"</option>
                        </select>
                        <span>"into the future"</span>
                    </div>

                    // Visible immediately and not itself a live region: the
                    // invalid control points here via `aria-describedby`, so
                    // this text is read out with the field it belongs to.
                    {move || outcome.get().message().map(|msg| view! {
                        <p class="error-msg" id=ERROR_ID>{msg}</p>
                    })}

                    // The announcement, debounced, so a screen reader hears the
                    // settled message rather than one per keystroke.
                    <p class="sr-only" role="status" aria-live="polite">{move || announced.get()}</p>
                </section>

                <section class="panel results">
                    <h2>"Breakdown"</h2>
                    {move || {
                        let stale = outcome.get().error().is_some();
                        match displayed.get() {
                            // Results are held through a transient error rather
                            // than blanked; `.stale` marks them as not current.
                            Some(out) => view! {
                                <div class="results-body" class:stale=stale
                                     aria-busy=move || stale.then_some("true")>
                                    {results_view(out)}
                                </div>
                            }.into_view(),
                            None => empty_view().into_view(),
                        }
                    }}
                </section>
            </main>

            <footer class="site-foot">
                <p>
                    "Runs entirely in your browser \u{2014} no data leaves this page and nothing \
                     is stored. Reload to start again."
                </p>
            </footer>
        </div>
    }
}

/// The headline figures. Rendered in its own full-width panel above the two
/// columns: these are the answer the user came for, and hoisting them out also
/// closes most of the dead space that a short form column left beside a tall
/// results column.
fn summary_view(out: CalcOutput) -> impl IntoView {
    let horizon = out.horizon_months;
    let gain = !out.growth.is_sign_negative();
    let growth_color = format!("color: var({})", if gain { "--good" } else { "--bad" });

    // Only surface contributions when there actually are some, so a portfolio
    // without top-ups keeps the lean summary.
    let contributions_stat = (!out.contributed_total.is_zero()).then(|| {
        view! {
            <div class="stat">
                <span class="stat-label">{format!("Added over {}", horizon_label(horizon))}</span>
                <span class="stat-value">{fmt_money(out.contributed_total)}</span>
            </div>
        }
    });

    view! {
        <div class="summary">
            // The projection leads: it is the question the tool exists to answer,
            // and at the same size as its own inputs it did not read as one.
            <div class="stat stat-accent">
                <span class="stat-label">{format!("Value in {}", horizon_label(horizon))}</span>
                <span class="stat-value">{fmt_money(out.projected_total)}</span>
            </div>
            <div class="stat">
                <span class="stat-label">"Value today"</span>
                <span class="stat-value">{fmt_money(out.current_total)}</span>
            </div>
            {contributions_stat}
            <div class="stat">
                // The label carries the direction too, so gain vs loss does not
                // rest on green-vs-red alone.
                <span class="stat-label">
                    {if gain { "Projected growth" } else { "Projected loss" }}
                </span>
                <span class="stat-value" style=growth_color.clone()>
                    {fmt_signed_money(out.growth)}
                </span>
                <span class="stat-sub" style=growth_color>
                    {fmt_pct(out.growth_pct)}
                </span>
                // A bare percentage leaves the reader guessing the denominator.
                // It is measured against capital deployed, not today's value.
                <span class="stat-note">
                    {format!("of {} put in", fmt_money(out.deployed))}
                </span>
            </div>
        </div>
    }
}

fn results_view(out: CalcOutput) -> impl IntoView {
    let horizon = out.horizon_months;
    let svg = chart_svg(&out.series, &out.contributions_series);
    let has_contributions = !out.contributed_total.is_zero();

    // --- scrub state -------------------------------------------------------
    // The chart plotted 121 points but only ever reported two of them, so
    // "what is it worth in year four" had no answer. A slider laid over the
    // plot reads any month out; `active` keeps the marker hidden until the user
    // is actually pointing at or focused on the chart.
    let series = store_value(out.series.clone());
    let contribs = store_value(out.contributions_series.clone());
    let cursor = create_rw_signal(horizon as usize);
    let active = create_rw_signal(false);
    let scrub_ref = create_node_ref::<html::Div>();

    let readout = move || {
        let i = series.with_value(|s| cursor.get().min(s.len().saturating_sub(1)));
        let value = series.with_value(|s| fmt_money(s[i]));
        let when = month_label(i as u32);
        if has_contributions {
            let paid = contribs.with_value(|c| fmt_money(c[i]));
            format!("{when}: {value} \u{2014} {paid} of that paid in")
        } else {
            format!("{when}: {value}")
        }
    };

    // Pointer x within the scrub layer -> month. The plot does not start at the
    // element's left edge (the y-axis gutter comes first), hence the fractions
    // from `chart`, which are derived from the same viewBox the line is drawn in.
    let set_from_x = move |x: f64| {
        let Some(el) = scrub_ref.get_untracked() else {
            return;
        };
        let width = el.client_width() as f64;
        if width <= 0.0 {
            return;
        }
        let frac = ((x / width) - PLOT_LEFT_FRAC) / PLOT_WIDTH_FRAC;
        cursor.set((frac.clamp(0.0, 1.0) * horizon as f64).round() as usize);
    };

    let on_key = move |ev: ev::KeyboardEvent| {
        let step: i64 = match ev.key().as_str() {
            "ArrowLeft" | "ArrowDown" => -1,
            "ArrowRight" | "ArrowUp" => 1,
            // A year at a time is the useful coarse step for this data.
            "PageDown" => -12,
            "PageUp" => 12,
            "Home" => -(horizon as i64),
            "End" => horizon as i64,
            _ => return,
        };
        // Stop the arrow keys scrolling the page out from under the chart.
        ev.prevent_default();
        active.set(true);
        let next = (cursor.get_untracked() as i64 + step).clamp(0, horizon as i64);
        cursor.set(next as usize);
    };

    let marker_style = move || {
        let at = cursor.get() as f64 / horizon.max(1) as f64;
        format!(
            "left: {:.4}%; top: {:.4}%; bottom: {:.4}%",
            (PLOT_LEFT_FRAC + at * PLOT_WIDTH_FRAC) * 100.0,
            PLOT_TOP_FRAC * 100.0,
            PLOT_BOTTOM_FRAC * 100.0
        )
    };
    let chart_label = if has_contributions {
        format!(
            "Line chart of projected portfolio value, from {} today to {} in {}, \
             with a second line showing {} of cumulative contributions.",
            fmt_money(out.current_total),
            fmt_money(out.projected_total),
            horizon_label(horizon),
            fmt_money(out.contributed_total),
        )
    } else {
        format!(
            "Line chart of projected portfolio value, from {} today to {} in {}.",
            fmt_money(out.current_total),
            fmt_money(out.projected_total),
            horizon_label(horizon)
        )
    };

    let breakdown = out
        .investments
        .iter()
        .map(|r| {
            // Column order mirrors the arithmetic: what you hold plus what you
            // add, grown at this rate, lands on the projection. Without the
            // top-ups cell a row with contributions looks like today's value
            // grew at `annualised`, which it did not.
            let contributed = has_contributions.then(|| {
                // An em dash reads better than "£0.00" for a holding with no
                // top-ups, and keeps the eye on the rows that do have them.
                let cell = if r.contributed.is_zero() {
                    "\u{2014}".to_string()
                } else {
                    fmt_money(r.contributed)
                };
                view! { <td class="num">{cell}</td> }
            });
            view! {
                <tr>
                    <td>{r.name.clone()}</td>
                    <td class="num">{fmt_money(r.current_value)}</td>
                    {contributed}
                    <td class="num">{fmt_pct(r.annualised)}</td>
                    <td class="num">{fmt_money(r.projected_value)}</td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <figure class="chart-figure">
            <div class="chart-scroll">
                <div class="chart-stage">
                    <div class="chart" role="img"
                         aria-label=chart_label
                         inner_html=svg></div>
                    <div class="chart-marker" class:on=move || active.get()
                         style=marker_style aria-hidden="true"></div>
                    // A slider, not a bare mousemove target: the value is a
                    // point in time, `aria-valuetext` carries the readout, and
                    // it is reachable and steppable from the keyboard. A
                    // hover-only tooltip would have shut out everyone else.
                    <div class="chart-scrub"
                         node_ref=scrub_ref
                         tabindex="0"
                         role="slider"
                         aria-label="Read the projection at a point in time"
                         aria-valuemin="0"
                         aria-valuemax=horizon.to_string()
                         aria-valuenow=move || cursor.get().to_string()
                         aria-valuetext=readout
                         on:pointermove=move |ev| { active.set(true); set_from_x(ev.offset_x() as f64); }
                         on:pointerleave=move |_| active.set(false)
                         on:focus=move |_| active.set(true)
                         on:blur=move |_| active.set(false)
                         on:keydown=on_key></div>
                </div>
            </div>
            // Visual mirror of `aria-valuetext`. Hidden from the tree so the
            // slider announces the value once, not twice.
            <div class="chart-readout" aria-hidden="true">
                {move || if active.get() {
                    readout()
                } else {
                    "Point at the chart, or focus it and use the arrow keys, to read any month.".to_string()
                }}
            </div>
            <figcaption class="chart-caption">
                {if has_contributions {
                    format!("Portfolio value and cumulative contributions from today to {} ahead.", horizon_label(horizon))
                } else {
                    format!("Portfolio value from today to {} ahead.", horizon_label(horizon))
                }}
            </figcaption>
        </figure>

        <div class="table-scroll">
            <table class="breakdown">
                // A `<caption>` rather than a `title` tooltip: the note explains
                // a derived column ("80% total" shown as "+6.05%"), and a
                // tooltip would hide that from keyboard and touch users.
                <caption class="table-note">
                    "Per holding. \u{201c}Annualised\u{201d} is the equivalent yearly rate \u{2014} \
                     a return entered as a total over the whole period is converted to it."
                </caption>
                <thead>
                    <tr>
                        <th scope="col">"Investment"</th>
                        <th scope="col">"Value today"</th>
                        {has_contributions.then(|| view! {
                            <th scope="col">{format!("Top-ups over {}", horizon_label(horizon))}</th>
                        })}
                        <th scope="col">"Annualised"</th>
                        <th scope="col">"Projected"</th>
                    </tr>
                </thead>
                <tbody>{breakdown}</tbody>
            </table>
        </div>
    }
}

/// Placeholder stats, shown only when the form is genuinely empty. A transient
/// typo keeps the last good figures instead (see `displayed`), so reaching this
/// state really does mean there is nothing to project.
fn empty_summary_view() -> impl IntoView {
    let stat = |label: &'static str, accent: bool| {
        view! {
            <div class="stat" class:stat-accent=accent>
                <span class="stat-label">{label}</span>
                <span class="stat-value">"\u{2014}"</span>
            </div>
        }
    };
    view! {
        <div class="summary">
            {stat("Projected value", true)}
            {stat("Value today", false)}
            {stat("Projected growth", false)}
        </div>
    }
}

fn empty_view() -> impl IntoView {
    view! {
        <p class="chart-placeholder">
            "Enter an investment to see the projection."
        </p>
    }
}

// --- input helpers: map raw form strings to `calc` inputs --------------------
// (display formatting lives in `format`, the chart in `chart`)

fn blank_zero(s: String) -> String {
    if s.trim().is_empty() {
        "0".to_string()
    } else {
        s
    }
}

fn unit_from(s: &str) -> Unit {
    if s == "months" {
        Unit::Months
    } else {
        Unit::Years
    }
}

fn mode_from(s: &str) -> Mode {
    if s == "total" {
        Mode::Total
    } else {
        Mode::Annual
    }
}
