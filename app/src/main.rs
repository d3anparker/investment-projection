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
//! - [`summary`] / [`results`] — the two output panels' views, both wrapped by
//!   [`panel`]'s shared last-good/`.stale` shell.
//! - [`format`] / [`chart`] — `Decimal`→string formatting and the SVG chart.
//!
//! `main.rs` keeps only the mount and the top-level [`App`] that wires these
//! together.

mod chart;
mod convert;
mod format;
mod goal;
mod model;
mod outcome;
mod panel;
mod results;
mod share;
mod summary;

use calc::{calculate, solve, CalcOutput, InvestmentField, Solution};
use convert::{build_input, RowData};
use goal::{build_goal, describe, subject_label, PORTFOLIO};
use leptos::leptos_dom::helpers::TimeoutHandle;
use leptos::*;
use model::{bind_value, new_row, remove_label, remove_row};
use outcome::{invalid_attrs, Outcome, ANNOUNCE_DELAY, ERROR_ID};
use results::ResultsPanel;
use share::ShareState;
use std::time::Duration;
use summary::SummaryPanel;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });
}

/// Snapshot the reactive rows down to plain-string [`RowData`], the form the
/// pure `convert`/`share` layers work in. Shared by the `outcome`/`holdings`/
/// `solution` memos and the copy-link handler so the field reads live in one
/// place.
fn snapshot(rows: RwSignal<Vec<model::Row>>) -> Vec<RowData> {
    rows.get()
        .iter()
        .map(|r| RowData {
            id: r.id,
            name: r.name.get(),
            value: r.value.get(),
            mode: r.mode.get(),
            rate: r.rate.get(),
            contribution: r.contribution.get(),
        })
        .collect()
}

/// The current location fragment (`#v=…`), or `None` when there isn't one
/// (absent or empty). `decode` does the rest — this only reads the string.
fn read_hash() -> Option<String> {
    window().location().hash().ok().filter(|h| !h.is_empty())
}

/// Write `url` (a `#v=…` fragment) into the address bar via `replaceState` — so
/// it doesn't push a Back-button entry — then copy the resulting absolute URL to
/// the clipboard, reporting the result into `status`.
///
/// The fragment is set regardless, so the baseline message is always the
/// truthful "it's in the address bar"; it is upgraded to "copied" only when the
/// asynchronous `write_text` actually resolves, so a denied/rejected write never
/// claims a copy that didn't happen.
fn write_hash_and_copy(url: &str, status: RwSignal<String>) {
    let win = window();
    if let Ok(history) = win.history() {
        // replaceState(null, "", "#v=…") swaps only the fragment in place.
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(url));
    }
    let href = win.location().href().unwrap_or_else(|_| url.to_string());
    let in_bar = "Link is in the address bar \u{2014} copy it from there.".to_string();

    // `navigator.clipboard` is undefined on insecure origins and older engines,
    // so probe it by reflection before casting — a bare cast would hand back a
    // value whose `write_text` throws.
    let nav = win.navigator();
    let clipboard = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard"))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| v.dyn_into::<web_sys::Clipboard>().ok());

    match clipboard {
        Some(clip) => {
            let promise = clip.write_text(&href);
            // Baseline truth now; upgrade to "copied" only if the write resolves.
            status.set(in_bar);
            spawn_local(async move {
                if JsFuture::from(promise).await.is_ok() {
                    status.set(
                        "Link copied \u{2014} it's in your clipboard and the address bar."
                            .to_string(),
                    );
                }
            });
        }
        None => status.set(in_bar),
    }
}

#[component]
fn App() -> impl IntoView {
    let counter = store_value(0usize);

    // Seed from a shared link if the fragment holds one; otherwise the built-in
    // illustrative example. `decode` is total, so a mangled (or absent) hash
    // simply yields `None` and the example loads, exactly as a bare page load
    // does. Either branch hands back one `ShareState`, so every signal below is
    // built from a single source instead of a decoded/inline split.
    let state = read_hash()
        .and_then(|h| share::decode(&h))
        .unwrap_or_else(ShareState::example);

    let rows = create_rw_signal(
        state
            .rows
            .iter()
            .map(|r| new_row(counter, &r.name, &r.value, &r.mode, &r.rate, &r.contribution))
            .collect::<Vec<_>>(),
    );
    let horizon_value = create_rw_signal(state.horizon_value);
    let horizon_unit = create_rw_signal(state.horizon_unit);

    // Goal-seek state. The target is blank in the example, which keeps the
    // feature inert (`build_goal` returns `None`) until the user asks a question.
    // `goal_scope` is the picker's raw string — the `"portfolio"` sentinel or an
    // index into the *filtered* investments — the same in both goal kinds, so a
    // shared goal reopens on the holding (or whole portfolio) it was solved for.
    let goal_target = create_rw_signal(state.goal_target);
    let goal_kind = create_rw_signal(state.goal_kind);
    let goal_scope = create_rw_signal(state.goal_scope);

    // Single source of computed truth. Reading every field's signal here means
    // the projection recomputes whenever any input changes; the memo caches the
    // result so `calculate` runs once even though several readers want it (the
    // error line, each control's invalid flag, and the `displayed`/`stale`
    // memos). Blank-row filtering and the `row_ids` mapping that survives it
    // live in `convert::build_input`.
    // Build the live `calc` input from the current form: one snapshot of the row
    // signals through the pure `build_input`. Shared by the three memos below —
    // each still subscribes to whatever signals it reads *through* this closure,
    // so the "form strings -> CalcInput" step lives in exactly one place. (It's a
    // `Copy` closure over `Copy` signal handles, hence reusable across memos.)
    let build_current =
        move || build_input(&snapshot(rows), &horizon_value.get(), &horizon_unit.get());

    let outcome = create_memo(move |_| {
        let (input, row_ids) = build_current();
        Outcome { result: calculate(&input), row_ids }
    });

    // The holdings a goal can target: the *filtered* rows the projection sees,
    // paired with their name for the picker. Index here is the index `calc::solve`
    // expects, so a blank row dropped upstream never shifts it. `Investment`
    // stands in for an unnamed holding, matching `build_input`.
    let holdings = create_memo(move |_| {
        let (input, _ids) = build_current();
        input
            .investments
            .iter()
            .enumerate()
            .map(|(i, inv)| (i, inv.name.clone()))
            .collect::<Vec<_>>()
    });

    // Keep a holding-scoped goal inside the (filtered) holdings list. When a row
    // is blanked or removed the list shrinks; without this, `goal_scope` could
    // name an index past the end while the `<select>` visually shows a valid
    // holding, and `solve` would report a spurious "pick a holding". A
    // `"portfolio"` scope is always valid and left alone. Tracks `holdings` only
    // (reads `goal_scope` untracked), so it can't loop on its own write.
    create_effect(move |_| {
        let n = holdings.with(|h| h.len());
        if let Ok(i) = goal_scope.get_untracked().parse::<usize>() {
            if n > 0 && i >= n {
                goal_scope.set((n - 1).to_string());
            }
        }
    });

    // Drive the scope `<select>`'s selection from `goal_scope` through a node-ref
    // effect, the same way text inputs use `bind_value`. Removing the selected
    // holding's `<option>` (a row deletion) makes the browser silently reset the
    // control to its first entry, and a reactive `selected=` attribute on the
    // options does not re-assert it; the option set also changes underneath a
    // holding-scoped shared link at mount, the classic `<select>` binding trap.
    // Tracking `holdings` re-runs this after the option list is rebuilt, and the
    // `!=` guard means it writes only when the DOM has actually drifted.
    let goal_scope_ref = create_node_ref::<html::Select>();
    create_effect(move |_| {
        holdings.with(|_| ());
        let v = goal_scope.get();
        if let Some(sel) = goal_scope_ref.get() {
            if sel.value() != v {
                sel.set_value(&v);
            }
        }
    });

    // The goal answer, separate from `outcome` on purpose: a goal that can't be
    // met is not an input error, so it must never mark the form `stale` or dim
    // the projection panels. `None` when the goal is inert (blank target); else
    // the solved sentence or the reason it failed, both as plain text.
    let solution = create_memo(move |_| {
        let target = goal_target.get();
        if target.trim().is_empty() {
            return None;
        }
        let kind = goal_kind.get();
        // Both goal kinds are now scoped by the same picker, so both subscribe to
        // `goal_scope`: changing the scope is meant to re-solve either question.
        let scope = goal_scope.get();
        let g = build_goal(&kind, &target, &scope)?;
        let (input, _ids) = build_current();
        let result: Result<Solution, String> = solve(&input, &g).map_err(|e| e.message);
        // Name what was solved for, so the answer states its own scope.
        let subject = holdings.with(|h| subject_label(&scope, h));
        // A top-up answer is solved *for the projection horizon*, so name that
        // period. Reuse the months `calc` already derived (via `outcome`) rather
        // than re-parsing the horizon field here — numbers stay in `calc`. When a
        // top-up succeeds the same input was valid, so `outcome` is `Ok` too.
        let horizon = outcome
            .with(|o| o.result.as_ref().ok().map(|out| format::horizon_label(out.horizon_months)))
            .unwrap_or_default();
        Some(describe(&result, &target, &subject, &horizon))
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
    // last good projection rather than a current one. Both output panels bind
    // it to `.stale`/`aria-busy` (see `panel::stale_body`). A memo rather than
    // `Signal::derive`, because a derived signal re-runs at every read: those
    // bindings would then be subscribed to `outcome` itself and rewrite the
    // class on every keystroke rather than when the flag actually flips.
    // `with` reads the error without cloning the whole `Outcome`, both
    // per-month series included.
    let stale = create_memo(move |_| outcome.with(|o| o.error().is_some()));

    // The visible message updates immediately; the announcement waits for a
    // pause. Each keystroke cancels the previous pending announcement, so a
    // screen reader hears one settled message instead of a running commentary.
    let announced = create_rw_signal(String::new());
    create_effect(move |prev: Option<Option<TimeoutHandle>>| {
        if let Some(Some(handle)) = prev {
            handle.clear();
        }
        let msg = outcome.with(|o| o.message()).unwrap_or_default();
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
    let goal_ref = bind_value(goal_target);
    // As with the row controls: one read of `outcome`, borrowed, shared by the
    // three attributes the horizon input's error state drives.
    let horizon_bad = create_memo(move |_| outcome.with(|o| o.flags_horizon()));

    // "Copy link" confirmation. A discrete click, not a per-keystroke rewrite,
    // so a live region here is safe (it can't talk over typing). Cleared after a
    // few seconds so the message doesn't linger.
    let copy_status = create_rw_signal(String::new());
    let copy_clear = store_value::<Option<TimeoutHandle>>(None);
    let copy_link = move |_| {
        let state = ShareState {
            rows: snapshot(rows),
            horizon_value: horizon_value.get(),
            horizon_unit: horizon_unit.get(),
            goal_target: goal_target.get(),
            goal_kind: goal_kind.get(),
            goal_scope: goal_scope.get(),
        };
        // Write the fragment with replace_state so the shared link doesn't pile
        // up Back-button history entries. The status is set inside (address-bar
        // baseline now, "copied" only once the async clipboard write actually
        // resolves) so it never over-claims a copy.
        let url = format!("#{}", share::encode(&state));
        write_hash_and_copy(&url, copy_status);
        copy_clear.update_value(|h| {
            if let Some(handle) = h.take() {
                handle.clear();
            }
            *h = set_timeout_with_handle(
                move || copy_status.set(String::new()),
                Duration::from_secs(5),
            )
            .ok();
        });
    };

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
                    // The goal-seek answer, when a target is set. Its own line so
                    // it never disturbs the four headline cards; it holds the
                    // sentence *or* the reason the target can't be met. Not a
                    // live region (it rewrites per keystroke) — the field it
                    // answers is right there in the form.
                    {move || solution.get().map(|text| view! {
                        <p class="goal-result">{text}</p>
                    })}
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
                            // One memo per control. The three attributes each
                            // drives (`aria-invalid`, `aria-describedby` and
                            // `.field-invalid`) are separate closures, so they
                            // would otherwise read `outcome` three times over —
                            // and `get` clones the whole projection, both
                            // per-month series included, to test one bool.
                            let flagged = |part: InvestmentField| {
                                create_memo(move |_| outcome.with(|o| o.flags(r.id, part)))
                            };
                            let value_bad = flagged(InvestmentField::Value);
                            let contribution_bad = flagged(InvestmentField::Contribution);
                            let rate_bad = flagged(InvestmentField::Rate);
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
                                            aria-invalid=move || invalid_attrs(value_bad.get()).0
                                            aria-describedby=move || invalid_attrs(value_bad.get()).1
                                            class:field-invalid=move || value_bad.get()
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
                                            aria-invalid=move || invalid_attrs(contribution_bad.get()).0
                                            aria-describedby=move || invalid_attrs(contribution_bad.get()).1
                                            class:field-invalid=move || contribution_bad.get()
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
                                                aria-invalid=move || invalid_attrs(rate_bad.get()).0
                                                aria-describedby=move || invalid_attrs(rate_bad.get()).1
                                                class:field-invalid=move || rate_bad.get()
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

                    <div class="form-actions">
                        <button type="button" class="btn btn-ghost" node_ref=add_btn on:click=add_row>
                            "+ Add investment"
                        </button>
                        <button type="button" class="btn btn-ghost" on:click=copy_link>
                            "Copy link"
                        </button>
                    </div>
                    // Copy confirmation. A discrete click, so a live region is
                    // safe here — it announces once and clears itself.
                    <p class="copy-status" role="status" aria-live="polite">
                        {move || copy_status.get()}
                    </p>

                    <div class="horizon">
                        <label for="horizon-value">"Project"</label>
                        <input
                            id="horizon-value" type="number" min="1" step="1" inputmode="numeric"
                            node_ref=horizon_ref
                            aria-invalid=move || invalid_attrs(horizon_bad.get()).0
                            aria-describedby=move || invalid_attrs(horizon_bad.get()).1
                            class:field-invalid=move || horizon_bad.get()
                            on:input=move |ev| horizon_value.set(event_target_value(&ev)) />
                        <select
                            aria-label="Projection unit"
                            on:change=move |ev| horizon_unit.set(event_target_value(&ev))>
                            <option value="years" selected=move || horizon_unit.get() == "years">"years"</option>
                            <option value="months" selected=move || horizon_unit.get() == "months">"months"</option>
                        </select>
                        <span>"into the future"</span>
                    </div>

                    <div class="goal">
                        <label for="goal-target">"Reach"</label>
                        <span class="adorn adorn-money">
                            <input
                                id="goal-target" type="text" inputmode="decimal"
                                placeholder="500,000"
                                node_ref=goal_ref
                                on:input=move |ev| goal_target.set(event_target_value(&ev)) />
                        </span>
                        <select
                            aria-label="What to work out"
                            on:change=move |ev| goal_kind.set(event_target_value(&ev))>
                            <option value="topup" selected=move || goal_kind.get() != "time">
                                "\u{2014} monthly top-up needed"
                            </option>
                            <option value="time" selected=move || goal_kind.get() == "time">
                                "\u{2014} time needed"
                            </option>
                        </select>
                        <span>"for"</span>
                        // What the goal is about, shown in *both* modes so the
                        // scope is a deliberate choice, not inferred from whether a
                        // picker happens to be visible. "Whole portfolio" is the
                        // combined total; a holding is tracked on its own. Holding
                        // options come from the *filtered* investments, so the value
                        // is the index `solve` expects. The current selection is
                        // driven by the `goal_scope_ref` effect above, not a
                        // per-option `selected=` (which the browser drops when the
                        // option list changes).
                        <select
                            node_ref=goal_scope_ref
                            aria-label="What the goal is about"
                            on:change=move |ev| goal_scope.set(event_target_value(&ev))>
                            <option value=PORTFOLIO>"your whole portfolio"</option>
                            <For
                                each=move || holdings.get()
                                key=|(i, name)| (*i, name.clone())
                                children=move |(i, name)| view! {
                                    <option value=i.to_string()>{name}</option>
                                } />
                        </select>
                    </div>

                    // Visible immediately and not itself a live region: the
                    // invalid control points here via `aria-describedby`, so
                    // this text is read out with the field it belongs to.
                    {move || outcome.with(|o| o.message()).map(|msg| view! {
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
                    "Runs entirely in your browser \u{2014} nothing is sent to a server. \
                     Use \u{201c}Copy link\u{201d} to save or share a projection: the figures \
                     travel inside the link itself, so anyone you send it to can see them."
                </p>
            </footer>
        </div>
    }
}
