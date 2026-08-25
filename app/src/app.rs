//! The top-level [`App`] component and its browser-bound glue.
//!
//! This is the reactive form state plus the layout that wires the focused
//! modules together. It owns the two genuinely `web_sys`-bound helpers
//! (`read_hash` reads the shareable-link fragment on load; `write_hash_and_copy`
//! is the "Copy link" clipboard/history glue); everything numeric it merely
//! *formats* — see the crate-root docs and `calc`.

use calc::{calculate, solve, CalcOutput, InvestmentField, Solution};
use crate::convert::{build_input, FormInput, RowData};
use crate::goal::{build_goal, describe, GoalKind};
use crate::model::{bind_value, new_row, remove_label, remove_row};
use crate::outcome::{invalid_attrs, Outcome, ANNOUNCE_DELAY, ERROR_ID};
use crate::results::ResultsPanel;
use crate::share::ShareState;
use crate::summary::SummaryPanel;
use crate::{format, model, share};
use leptos::leptos_dom::helpers::TimeoutHandle;
use leptos::*;
use std::time::Duration;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Snapshot the reactive rows down to plain-string [`RowData`], the form the
/// pure `convert`/`share` layers work in. Shared by the `outcome`/`solution`
/// memos and the copy-link handler so the field reads live in one place.
fn snapshot(rows: RwSignal<Vec<model::Row>>) -> Vec<RowData> {
    rows.get()
        .iter()
        .map(|r| RowData {
            id: r.id,
            name: r.name.get(),
            value: r.value.get(),
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
pub fn App() -> impl IntoView {
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
            .map(|r| new_row(counter, &r.name, &r.value, &r.rate, &r.contribution))
            .collect::<Vec<_>>(),
    );
    let horizon_value = create_rw_signal(state.horizon_value);
    let horizon_unit = create_rw_signal(state.horizon_unit);

    // The top-level mode (`"deposits"` / `"drawdown"`) and the drawdown-only
    // controls it reveals. The growth period above is shared by both modes;
    // `drawdown_value`/`drawdown_unit`/`withdrawal` only matter while drawing down.
    let plan_kind = create_rw_signal(state.plan);
    let drawdown_value = create_rw_signal(state.drawdown_value);
    let drawdown_unit = create_rw_signal(state.drawdown_unit);
    let withdrawal = create_rw_signal(state.withdrawal);
    let is_drawdown = move || plan_kind.get() == "drawdown";

    // Goal-seek state. The target is blank in the example, which keeps the
    // feature inert (`build_goal` returns `None`) until the user asks a question.
    let goal_target = create_rw_signal(state.goal_target);
    let goal_kind = create_rw_signal(state.goal_kind);

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
    // Single source of computed truth: one snapshot of the whole form through the
    // pure `build_input`. Reading the signals here is what makes the projection
    // recompute on any edit; the memo caches so `calculate` runs once. A `Copy`
    // closure over `Copy` signal handles, so it is reusable across the memos.
    let build_current = move || {
        build_input(&FormInput {
            rows: snapshot(rows),
            horizon_value: horizon_value.get(),
            horizon_unit: horizon_unit.get(),
            plan: plan_kind.get(),
            drawdown_value: drawdown_value.get(),
            drawdown_unit: drawdown_unit.get(),
            withdrawal: withdrawal.get(),
        })
    };

    let outcome = create_memo(move |_| {
        let (input, row_ids) = build_current();
        Outcome { result: calculate(&input), row_ids }
    });

    // The goal answer, separate from `outcome` on purpose: a goal that can't be
    // met is not an input error, so it must never mark the form `stale` or dim
    // the projection panels. `None` when the goal is inert; else the solved
    // sentence or the reason it failed, both as plain text.
    let solution = create_memo(move |_| {
        let plan = plan_kind.get();
        let kind = goal_kind.get();
        let target = goal_target.get();
        let draw = withdrawal.get();
        let g = build_goal(&kind, &plan, &target, &draw)?;
        let (input, _ids) = build_current();
        let result: Result<Solution, String> = solve(&input, &g).map_err(|e| e.message);
        // The amount box the answer echoes (the target in deposits mode, the
        // monthly withdrawal in drawdown mode) plus the two period labels. The
        // labels come from a projection with the withdrawal neutralised, *not*
        // `outcome`: a `MaxWithdrawal` answer solves *for* the withdrawal, so it
        // must still name the drawdown period even when the withdrawal box holds
        // invalid text — which would error `outcome` and blank the label
        // ("...to zero over  of drawdown."). Whenever `solve` succeeds the periods
        // and rows are valid, so this probe is too. Numbers stay in `calc`.
        let (horizon_lbl, drawdown_lbl) = {
            let mut probe = input.clone();
            if let calc::Plan::Drawdown { withdrawal, .. } = &mut probe.plan {
                *withdrawal = "0".to_string();
            }
            calculate(&probe)
                .ok()
                .map(|out| {
                    (format::horizon_label(out.horizon_months), format::horizon_label(out.drawdown_months))
                })
                .unwrap_or_default()
        };
        let amount = if plan == "drawdown" { draw } else { target };
        Some(describe(&result, &amount, &horizon_lbl, &drawdown_lbl))
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
        let row = new_row(counter, "", "", "", "");
        rows.update(|v| v.push(row));
    };
    let horizon_ref = bind_value(horizon_value);

    // The current goal kind, resolved within the active mode so a kind left over
    // from the other mode falls back to that mode's default (see `GoalKind::parse`).
    let current_kind = move || GoalKind::parse(&goal_kind.get(), &plan_kind.get());

    // The form-level controls whose error state comes from a `Field` rather than a
    // row: one borrowed read of `outcome` each, shared by their three attributes.
    let horizon_bad = create_memo(move |_| outcome.with(|o| o.flags_horizon()));
    let drawdown_bad = create_memo(move |_| outcome.with(|o| o.flags_drawdown()));
    let withdrawal_bad = create_memo(move |_| outcome.with(|o| o.flags_withdrawal()));

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
            plan: plan_kind.get(),
            drawdown_value: drawdown_value.get(),
            drawdown_unit: drawdown_unit.get(),
            withdrawal: withdrawal.get(),
            goal_target: goal_target.get(),
            goal_kind: goal_kind.get(),
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
                // The top-level mode switch, full-width above the panels. A radio
                // group (styled as a segmented control), not a tablist: the
                // holdings editor is shared between modes and stays put, so there
                // is no tabpanel to control — a radio group is the honest "setting
                // that reconfigures the form" and gives arrow-key navigation, one
                // tab stop and "2 of 2, selected" for free. `prop:checked` (not the
                // `checked` attribute) so it re-drives after user interaction.
                <fieldset class="mode-switch">
                    <legend>"What are you planning?"</legend>
                    <div class="segmented">
                        <input type="radio" id="mode-deposits" name="mode" value="deposits"
                               prop:checked=move || !is_drawdown()
                               on:change=move |_| plan_kind.set("deposits".to_string()) />
                        <label for="mode-deposits">"Building it up"</label>
                        <input type="radio" id="mode-drawdown" name="mode" value="drawdown"
                               prop:checked=is_drawdown
                               on:change=move |_| plan_kind.set("drawdown".to_string()) />
                        <label for="mode-drawdown">"Drawing it down"</label>
                    </div>
                </fieldset>

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
                            <span>"Monthly deposit"</span>
                            <span>"Annual return"</span>
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
                                    // The label wraps the input, so its visible
                                    // text *is* the accessible name (WCAG 2.5.3) —
                                    // no `aria-label` to override the on-screen
                                    // "Monthly deposit".
                                    <span class="fld-lbl">"Monthly deposit"</span>
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
                                <label class="fld">
                                    <span class="fld-lbl">"Annual return"</span>
                                    <span class="adorn adorn-pct">
                                        <input
                                            type="text" inputmode="decimal"
                                            placeholder="7"
                                            node_ref=rate_ref
                                            aria-invalid=move || invalid_attrs(rate_bad.get()).0
                                            aria-describedby=move || invalid_attrs(rate_bad.get()).1
                                            class:field-invalid=move || rate_bad.get()
                                            on:input=move |ev| r.rate.set(event_target_value(&ev)) />
                                    </span>
                                </label>
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

                    // The periods. Row one is shared: the *same* horizon input
                    // node in both modes (only the surrounding words change), so
                    // switching mode never rebuilds it and takes focus/caret with
                    // it. Rows two and three appear only while drawing down.
                    <div class="periods">
                        <div class="period-row">
                            <label for="horizon-value">
                                {move || if is_drawdown() { "Grow for" } else { "Project" }}
                            </label>
                            <input
                                id="horizon-value" type="number" min="1" step="1" inputmode="numeric"
                                node_ref=horizon_ref
                                aria-invalid=move || invalid_attrs(horizon_bad.get()).0
                                aria-describedby=move || invalid_attrs(horizon_bad.get()).1
                                class:field-invalid=move || horizon_bad.get()
                                on:input=move |ev| horizon_value.set(event_target_value(&ev)) />
                            <select
                                aria-label="Growth period unit"
                                on:change=move |ev| horizon_unit.set(event_target_value(&ev))>
                                <option value="years" selected=move || horizon_unit.get() == "years">"years"</option>
                                <option value="months" selected=move || horizon_unit.get() == "months">"months"</option>
                            </select>
                            <span>{move || if is_drawdown() { "," } else { "into the future" }}</span>
                        </div>

                        {move || is_drawdown().then(|| {
                            // Refs created inside the block so a fresh binding
                            // effect applies the seeded value when it mounts.
                            let drawdown_ref = bind_value(drawdown_value);
                            let withdrawal_ref = bind_value(withdrawal);
                            view! {
                                <div class="period-row">
                                    <label for="drawdown-value">"then draw down for"</label>
                                    <input
                                        id="drawdown-value" type="number" min="1" step="1" inputmode="numeric"
                                        node_ref=drawdown_ref
                                        aria-invalid=move || invalid_attrs(drawdown_bad.get()).0
                                        aria-describedby=move || invalid_attrs(drawdown_bad.get()).1
                                        class:field-invalid=move || drawdown_bad.get()
                                        on:input=move |ev| drawdown_value.set(event_target_value(&ev)) />
                                    <select
                                        aria-label="Drawdown period unit"
                                        on:change=move |ev| drawdown_unit.set(event_target_value(&ev))>
                                        <option value="years" selected=move || drawdown_unit.get() == "years">"years"</option>
                                        <option value="months" selected=move || drawdown_unit.get() == "months">"months"</option>
                                    </select>
                                </div>
                                <div class="period-row">
                                    <label for="withdrawal">"Withdraw"</label>
                                    <span class="adorn adorn-money">
                                        <input
                                            id="withdrawal" type="text" inputmode="decimal"
                                            placeholder="2000"
                                            node_ref=withdrawal_ref
                                            aria-invalid=move || invalid_attrs(withdrawal_bad.get()).0
                                            aria-describedby=move || invalid_attrs(withdrawal_bad.get()).1
                                            class:field-invalid=move || withdrawal_bad.get()
                                            on:input=move |ev| withdrawal.set(event_target_value(&ev)) />
                                    </span>
                                    <span>"a month from the whole portfolio"</span>
                                </div>
                            }
                        })}
                    </div>

                    // The goal, mode-aware. Two separate `<select>`s with static
                    // option sets — never one select whose options swap, which the
                    // browser resets to its first entry when the selected option is
                    // destroyed. Only the current mode's select is rendered; both
                    // write the one `goal_kind` signal.
                    <div class="goal">
                        // The target box belongs to the deposits questions only;
                        // the drawdown questions read the withdrawal box above.
                        {move || (!is_drawdown()).then(|| {
                            let goal_ref = bind_value(goal_target);
                            view! {
                                <label for="goal-target">"Reach"</label>
                                <span class="adorn adorn-money">
                                    <input
                                        id="goal-target" type="text" inputmode="decimal"
                                        placeholder="500,000"
                                        node_ref=goal_ref
                                        on:input=move |ev| goal_target.set(event_target_value(&ev)) />
                                </span>
                            }
                        })}
                        {move || if is_drawdown() {
                            view! {
                                <select
                                    aria-label="What to work out"
                                    on:change=move |ev| goal_kind.set(event_target_value(&ev))>
                                    <option value="withdrawal" selected=move || current_kind() == GoalKind::Withdrawal>
                                        "\u{2014} monthly withdrawal I can afford"
                                    </option>
                                    <option value="lasts" selected=move || current_kind() == GoalKind::Lasts>
                                        "\u{2014} how long it lasts"
                                    </option>
                                </select>
                            }.into_view()
                        } else {
                            view! {
                                <select
                                    aria-label="What to work out"
                                    on:change=move |ev| goal_kind.set(event_target_value(&ev))>
                                    <option value="topup" selected=move || current_kind() == GoalKind::TopUp>
                                        "\u{2014} monthly top-up needed"
                                    </option>
                                    <option value="time" selected=move || current_kind() == GoalKind::Time>
                                        "\u{2014} time needed"
                                    </option>
                                </select>
                            }.into_view()
                        }}
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
