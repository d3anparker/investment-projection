//! The top-level [`App`] component and its browser-bound glue.
//!
//! This is the reactive form state plus the layout that wires the focused
//! modules together. It owns the two genuinely `web_sys`-bound helpers
//! (`read_hash` reads the shareable-link fragment on load; `write_hash_and_copy`
//! is the "Copy link" clipboard/history glue); everything numeric it merely
//! *formats* — see the crate-root docs and `calc`.

use calc::{calculate, solve, CalcOutput, Field, InvestmentField, Solution};
use crate::convert::{build_input, FormInput, RowData};
use crate::goal::{build_goal, describe, GoalKind};
use crate::model::{bind_value, new_row, remove_label, remove_row};
use crate::outcome::{invalid_attrs, Outcome, ANNOUNCE_DELAY, ERROR_ID};
use crate::results::ResultsPanel;
use crate::share::ShareState;
use crate::summary::SummaryPanel;
use crate::{convert, format, freshness, model, share, strategy};
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
            account_kind: r.account_kind.get(),
            cost_basis: r.cost_basis.get(),
        })
        .collect()
}

/// A text `<input>` wired to `sig`, carrying the invalid-state a11y contract —
/// `aria-invalid` + `aria-describedby` + `.field-invalid`, all driven by `bad` —
/// in one place rather than repeated per control. The single spelling of that
/// contract is what stops one control quietly drifting out of step with the rest.
fn aria_text_input(
    id: &'static str,
    inputmode: &'static str,
    placeholder: &'static str,
    node_ref: NodeRef<html::Input>,
    sig: RwSignal<String>,
    bad: Memo<bool>,
) -> impl IntoView {
    view! {
        <input id=id type="text" inputmode=inputmode placeholder=placeholder node_ref=node_ref
            aria-invalid=move || invalid_attrs(bad.get()).0
            aria-describedby=move || invalid_attrs(bad.get()).1
            class:field-invalid=move || bad.get()
            on:input=move |ev| sig.set(event_target_value(&ev)) />
    }
}

/// One `label → adorned decimal input → trailing words` period-row, the shape the
/// tax fieldset repeats. `adorn` is the adornment suffix (`"money"` / `"pct"`);
/// the a11y contract comes from [`aria_text_input`].
fn adorned_field(
    id: &'static str,
    label: &'static str,
    adorn: &'static str,
    placeholder: &'static str,
    suffix: &'static str,
    node_ref: NodeRef<html::Input>,
    sig: RwSignal<String>,
    bad: Memo<bool>,
) -> impl IntoView {
    view! {
        <div class="period-row">
            <label for=id>{label}</label>
            <span class=format!("adorn adorn-{adorn}")>
                {aria_text_input(id, "decimal", placeholder, node_ref, sig, bad)}
            </span>
            <span>{suffix}</span>
        </div>
    }
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

    // Resolve the jurisdiction first and point `convert` at its tax system,
    // *before* the rows and region below are seeded — they resolve account and
    // region ids against the active catalogue. `jurisdiction` is a catalogue id;
    // `active_sys` derives the live system reactively so every catalogue-driven
    // control rebuilds when it changes.
    let jurisdiction = create_rw_signal(
        crate::jurisdiction::from_id(&state.jurisdiction).id.to_string(),
    );
    convert::set_active_system(crate::jurisdiction::system_from(&jurisdiction.get_untracked()));
    let active_sys = move || crate::jurisdiction::system_from(&jurisdiction.get());
    // The currency symbol, as a reactive context every figure reads through
    // `format::currency`, so a jurisdiction switch re-renders them all.
    provide_context(crate::format::ActiveCurrency(Signal::derive(move || {
        active_sys().currency_symbol()
    })));

    let rows = create_rw_signal(
        state
            .rows
            .iter()
            .map(|r| new_row(counter, r))
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

    // The tax controls. All drawdown-only, and all inert while the withdrawal is
    // split pro-rata -- that strategy ignores tax entirely, so `convert::tax_from`
    // hands `calc` no context at all and the projection stays exactly what it was
    // before any of this existed.
    let strategy = create_rw_signal(state.strategy);
    let rate_cap = create_rw_signal(state.rate_cap);
    let region = create_rw_signal(convert::region_from(&state.region));
    let other_income = create_rw_signal(state.other_income);
    let age = create_rw_signal(state.age);
    let uprate = create_rw_signal(state.uprate);
    let options = create_rw_signal(state.options);
    // Asked of the resolver rather than tested against the raw string:
    // `strategy_from` maps anything it does not recognise to pro-rata (so an old
    // link still opens), and a second predicate spelling that rule out by hand
    // drifts from it -- an unknown id used to render the whole tax fieldset for
    // a projection that was quietly running pro-rata and ignoring every field
    // in it.
    let tax_aware = move || {
        is_drawdown() && convert::StrategyChoice::from_id(&strategy.get()) != convert::StrategyChoice::ProRata
    };

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
    // The form, as plain strings. Split out from `build_current` so the
    // comparison memo can reach the raw fields as well as the built input.
    let form_snapshot = move || FormInput {
        rows: snapshot(rows),
        horizon_value: horizon_value.get(),
        horizon_unit: horizon_unit.get(),
        plan: plan_kind.get(),
        drawdown_value: drawdown_value.get(),
        drawdown_unit: drawdown_unit.get(),
        withdrawal: withdrawal.get(),
        strategy: strategy.get(),
        rate_cap: rate_cap.get(),
        region: region.get(),
        other_income: other_income.get(),
        age: age.get(),
        uprate: uprate.get(),
        jurisdiction: jurisdiction.get(),
        options: options.with(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    };
    let build_current = move || build_input(&form_snapshot());


    // The strategy comparison, a separate memo for the same reason the goal
    // answer is: a strategy that empties the pot early is not an input error, so
    // it must never mark the form stale or dim the projection panels.
    //
    // It is also *debounced*. `compare` runs four full projections, so on top of
    // `outcome`'s own it is the heaviest thing on the typing path; recomputing it
    // per keystroke is what made the comparison the largest cost in the diff.
    // Instead the input it reads is refreshed only once typing settles, so the
    // table follows a short beat behind rather than thrashing on every character.
    // The signal is seeded synchronously (a plain read, no subscription) so the
    // first render already has the table — the browser suite reads it at mount.
    let comparison_of = move |f: &FormInput| {
        let (mut input, _) = convert::build_input(f);
        // The comparison needs tax details whatever order is currently selected —
        // otherwise every tax-aware row reads "fill in the tax details" for a
        // reader who has not yet worked out that those details are the thing
        // that unlocks them. See `convert::tax_context`.
        input.tax = convert::tax_context(f);
        input
    };
    let comparison_input = create_rw_signal(comparison_of(&form_snapshot()));
    // Re-run on any form edit (through `form_snapshot`), but defer the refresh
    // behind one shared timeout, cancelling the pending one each keystroke — the
    // same settle-then-fire shape the error announcement below uses. The mount
    // run only subscribes; the seed above already covers the first render.
    create_effect(move |prev: Option<Option<TimeoutHandle>>| -> Option<TimeoutHandle> {
        let f = form_snapshot();
        if let Some(Some(handle)) = prev {
            handle.clear();
        }
        if prev.is_none() {
            return None;
        }
        set_timeout_with_handle(move || comparison_input.set(comparison_of(&f)), ANNOUNCE_DELAY).ok()
    });
    let comparison = create_memo(move |_| comparison_input.with(strategy::compare));

    // Read once at mount, not in a memo: the clock is not reactive, and letting
    // a date into the projection would make the same shared link produce
    // different figures depending on when it was opened.
    let today = store_value(freshness::today());
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
        let row = new_row(counter, &RowData::default());
        rows.update(|v| v.push(row));
    };
    let horizon_ref = bind_value(horizon_value);

    // The current goal kind, resolved within the active mode so a kind left over
    // from the other mode falls back to that mode's default (see `GoalKind::parse`).
    let current_kind = move || GoalKind::parse(&goal_kind.get(), &plan_kind.get());

    // The form-level controls whose error state comes from a `Field` rather than a
    // row: one borrowed read of `outcome` each, shared by their three attributes.
    let horizon_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Horizon)));
    let drawdown_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Drawdown)));
    let withdrawal_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Withdrawal)));
    let income_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::OtherIncome)));
    let age_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Age)));
    let strategy_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Strategy)));
    let uprate_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Uprate)));
    let region_bad = create_memo(move |_| outcome.with(|o| o.flags_field(Field::Region)));

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
            strategy: strategy.get(),
            rate_cap: rate_cap.get(),
            region: region.get(),
            other_income: other_income.get(),
            age: age.get(),
            uprate: uprate.get(),
            options: options.get(),
            jurisdiction: jurisdiction.get(),
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
                    // The jurisdiction picker: a static option set (the fixed
                    // catalogue), so it never falls foul of the swap-reset trap.
                    // Hidden entirely when only one jurisdiction is compiled in,
                    // the same courtesy the region select gets.
                    {(crate::jurisdiction::JURISDICTIONS.len() > 1).then(|| view! {
                        <label class="jurisdiction-pick">
                            <span>"Taxed in"</span>
                            <select on:change=move |ev| jurisdiction.set(event_target_value(&ev))>
                                {crate::jurisdiction::JURISDICTIONS.iter().map(|j| view! {
                                    <option value=j.id selected=move || jurisdiction.get() == j.id>
                                        {j.label}
                                    </option>
                                }).collect_view()}
                            </select>
                        </label>
                    })}
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
                    <div class="inv-editor" class:with-accounts=is_drawdown>
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
                            let basis_bad = flagged(InvestmentField::CostBasis);
                            let account_bad = flagged(InvestmentField::AccountKind);
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
                                // Drawdown-only, like the cost box below it. In
                                // deposits mode the account changes nothing about
                                // the numbers, and an inert control that looks
                                // like it matters is worse than no control — it
                                // also keeps the aligned four-field row exactly
                                // as it was before accounts existed.
                                {move || (is_drawdown()).then(|| {
                                    // Reading `jurisdiction` here rebuilds the whole
                                    // `<select>` node when the jurisdiction changes, so its
                                    // option set never *swaps* under a persistent control
                                    // (which the browser would reset). Options come from the
                                    // active catalogue, and `selected=` (not `prop:value`)
                                    // drives it, since a select's props are set before its
                                    // options exist.
                                    let kinds = active_sys().account_kinds();
                                    view! {
                                    <label class="fld fld-account">
                                        <span class="fld-lbl">"Account"</span>
                                        <select
                                            aria-invalid=move || invalid_attrs(account_bad.get()).0
                                            aria-describedby=move || invalid_attrs(account_bad.get()).1
                                            class:field-invalid=move || account_bad.get()
                                            on:change=move |ev| r.account_kind.set(event_target_value(&ev))>
                                            {kinds.iter().map(|k| view! {
                                                <option
                                                    value=k.id
                                                    title=k.note
                                                    selected=move || r.account_kind.get() == k.id>
                                                    {k.short_label}
                                                </option>
                                            }).collect_view()}
                                        </select>
                                    </label>
                                    }
                                })}
                                // Only for kinds that are taxed on the gain, and asked
                                // of the catalogue rather than matched against a named
                                // wrapper — so no jurisdiction leaks into the markup.
                                {move || {
                                    let needs = active_sys()
                                        .account_kind(&r.account_kind.get())
                                        .is_some_and(|k| k.needs_cost_basis);
                                    (needs && is_drawdown()).then(|| {
                                        let basis_ref = bind_value(r.cost_basis);
                                        view! {
                                            <label class="fld fld-basis">
                                                <span class="fld-lbl">"Cost"</span>
                                                <span class="adorn adorn-money">
                                                    <input
                                                        type="text" inputmode="decimal"
                                                        placeholder="what it cost"
                                                        node_ref=basis_ref
                                                        aria-invalid=move || invalid_attrs(basis_bad.get()).0
                                                        aria-describedby=move || invalid_attrs(basis_bad.get()).1
                                                        class:field-invalid=move || basis_bad.get()
                                                        on:input=move |ev| r.cost_basis.set(event_target_value(&ev)) />
                                                </span>
                                            </label>
                                        }
                                    })
                                }}
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


                    // The withdrawal order, and the tax details it needs. Both
                    // drawdown-only. Every label and option here is asked of the
                    // tax system rather than written out, so nothing in this
                    // markup names a jurisdiction.
                    {move || is_drawdown().then(|| {
                        // Refs are created inside this block so a fresh binding
                        // effect applies the seeded value when it mounts.
                        let cap_ref = bind_value(rate_cap);
                        let income_ref = bind_value(other_income);
                        let age_ref = bind_value(age);
                        let uprate_ref = bind_value(uprate);
                        // Reading the active system here rebuilds the whole tax
                        // fieldset (regions, and the jurisdiction's own panel
                        // below) when the jurisdiction changes.
                        let regions = active_sys().regions();
                        view! {
                        <fieldset class="tax-settings">
                            <legend>"How to take it"</legend>

                            <div class="period-row">
                                <label for="strategy">"Take it"</label>
                                <select id="strategy"
                                    on:change=move |ev| strategy.set(event_target_value(&ev))>
                                    // One option per catalogue entry, so a new
                                    // strategy appears here without touching the
                                    // markup. `from_id` maps blank/unknown to
                                    // pro-rata, which is why that option lights up
                                    // by default.
                                    {convert::StrategyChoice::ALL.into_iter().map(|choice| {
                                        let id = choice.id();
                                        view! {
                                            <option value=id
                                                selected=move || convert::StrategyChoice::from_id(&strategy.get()) == choice>
                                                {format!("\u{2014} {}", choice.picker_label())}
                                            </option>
                                        }
                                    }).collect_view()}
                                </select>
                            </div>

                            {move || (convert::StrategyChoice::from_id(&strategy.get()) == convert::StrategyChoice::Capped).then(|| {
                                adorned_field("rate-cap", "never paying more than", "pct", "20",
                                    "at the margin", cap_ref, rate_cap, strategy_bad)
                            })}

                            // Inert while splitting pro-rata: that ignores tax
                            // entirely, so asking for these would imply they
                            // changed something.
                            {move || tax_aware().then(|| view! {
                                {adorned_field("other-income", "Other taxable income", "money", "0",
                                    "a year", income_ref, other_income, income_bad)}
                                <div class="period-row">
                                    <label for="age">"Age when it starts"</label>
                                    {aria_text_input("age", "numeric", "60", age_ref, age, age_bad)}
                                    // A single-region tax system needs no control
                                    // at all, so it gets none rather than a
                                    // pointless one-option select.
                                    {(regions.len() > 1).then(|| view! {
                                        <label for="region">"living in"</label>
                                        <select id="region"
                                            aria-invalid=move || invalid_attrs(region_bad.get()).0
                                            aria-describedby=move || invalid_attrs(region_bad.get()).1
                                            class:field-invalid=move || region_bad.get()
                                            on:change=move |ev| region.set(event_target_value(&ev))>
                                            {regions.iter().map(|rg| view! {
                                                <option value=rg.id
                                                    selected=move || region.get() == rg.id>
                                                    {rg.label}
                                                </option>
                                            }).collect_view()}
                                        </select>
                                    })}
                                </div>
                                {adorned_field("uprate", "Tax thresholds rise", "pct", "0",
                                    "a year", uprate_ref, uprate, uprate_bad)}
                                // The active jurisdiction's own bespoke controls,
                                // if it has any. Rendered here inside the rebuilt
                                // fieldset (so it follows the jurisdiction), and
                                // nothing at all for a jurisdiction with no panel.
                                {crate::jurisdiction::from_id(&jurisdiction.get())
                                    .settings_panel
                                    .map(|panel| panel(crate::jurisdiction::SettingsSlot {
                                        options,
                                        today_year: today.get_value().year,
                                    }))}
                            })}
                        </fieldset>
                        }
                    })}
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

                // How the money could be taken, and what each way costs, in its
                // own full-width panel below the projection: it is the answer to
                // a question the reader only asks once they believe the figures
                // above, and its seven-column table wants the whole page width.
                // Gated on there being something to compare, so it is absent —
                // not an empty card — outside drawdown mode.
                {move || (!comparison.get().is_empty()).then(|| view! {
                    <section class="panel panel-strategy" aria-labelledby="strategy-h">
                        <h2 id="strategy-h">"Ways to draw it down"</h2>
                        <strategy::StrategyPanel rows=comparison/>

                        // Which rules produced the tax figures, and when they were
                        // checked. Always shown alongside them, never as a tooltip:
                        // a tax figure without a date is a figure you cannot judge.
                        {move || outcome.with(|o| {
                            o.result.as_ref().ok().and_then(|out| {
                                let (label, checked) = (out.rules_label?, out.rules_as_of?);
                                let line = freshness::as_of_line(convert::active_system(), label, checked);
                                let stale = freshness::stale_note(convert::active_system(), today.get_value());
                                Some(view! {
                                    <p class="tax-asof">{line}</p>
                                    // `role="note"`, never `role="alert"`: this is
                                    // standing context, not something that just
                                    // happened, and an alert here would interrupt a
                                    // screen reader on every recomputation.
                                    {stale.map(|msg| view! {
                                        <p class="tax-stale" role="note">{msg}</p>
                                    })}
                                })
                            })
                        })}
                        // The active jurisdiction's own figure notes, if any.
                        {move || crate::jurisdiction::from_id(&jurisdiction.get())
                            .notes_panel
                            .map(|panel| panel(crate::jurisdiction::NotesSlot))}
                    </section>
                })}
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
