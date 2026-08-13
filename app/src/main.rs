//! Leptos (CSR) front end for the investment projection tool.
//!
//! This layer owns the reactive form state and *formats* the `Decimal`s that
//! the `calc` crate returns. It performs no financial arithmetic itself — every
//! number is produced by `calc::calculate`. Its responsibilities are split into
//! focused modules:
//!
//! - [`convert`] — form strings → `calc::CalcInput` (blank-row filtering, the
//!   `<select>`→enum maps); pure and natively tested.
//! - [`outcome`] — a recomputation's result plus the error→control mapping;
//!   pure and natively tested.
//! - [`model`] — the reactive [`Row`] and its DOM helpers (signals, focus).
//! - [`summary`] / [`results`] — the two output panels' views.
//! - [`format`] / [`chart`] — `Decimal`→string formatting and the SVG chart.
//!
//! `main.rs` keeps only the mount and the top-level [`App`] that wires these
//! together.

mod chart;
mod convert;
mod format;
mod model;
mod outcome;
mod results;
mod summary;

use calc::{calculate, CalcOutput, InvestmentField};
use convert::{build_input, RowData};
use leptos::leptos_dom::helpers::TimeoutHandle;
use leptos::*;
use model::{bind_value, new_row, remove_label, remove_row};
use outcome::{invalid_attrs, Outcome, ANNOUNCE_DELAY, ERROR_ID};
use results::ResultsPanel;
use summary::SummaryPanel;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });
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
    // (error line + results panel). Blank-row filtering and the `row_ids`
    // mapping that survives it live in `convert::build_input`.
    let outcome = create_memo(move |_| {
        let row_data: Vec<RowData> = rows
            .get()
            .iter()
            .map(|r| RowData {
                id: r.id,
                name: r.name.get(),
                value: r.value.get(),
                mode: r.mode.get(),
                rate: r.rate.get(),
                contribution: r.contribution.get(),
            })
            .collect();
        let (input, row_ids) = build_input(&row_data, &horizon_value.get(), &horizon_unit.get());
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

    // True while the current input is mid-error, so `displayed` is holding the
    // last good projection rather than a current one. Both output panels dim
    // themselves (`.stale`/`aria-busy`) on it.
    let stale = Signal::derive(move || outcome.get().error().is_some());

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
                    <SummaryPanel displayed=displayed stale=stale/>
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
                    <ResultsPanel displayed=displayed stale=stale/>
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
