//! Headless-browser tests for the `App` component and its children.
//!
//! These mount the *real* `<App/>` into a browser DOM, drive it with the same
//! `input`/`change`/`keydown` events a user would, and assert on rendered text
//! and ARIA attributes — covering both interactions and displayed values. They
//! are the coverage for the reactive/DOM layer (`app.rs`, `summary.rs`,
//! `results.rs`, `panel.rs`, `model.rs`) that the pure native `cargo test`
//! suite can't reach.
//!
//! Run: see `test.Dockerfile` and CLAUDE.md — needs a headless browser plus
//! `wasm-bindgen-test-runner`, whose version must match `wasm-bindgen` exactly.
//!
//! The whole file is gated to wasm32 so the native `cargo test --workspace`
//! skips it (there is no browser there); it compiles and runs only under
//! `cargo test -p app --target wasm32-unknown-unknown --test ui`.
#![cfg(target_arch = "wasm32")]

use app::convert::RowData;
use app::share::{self, ShareState};
use app::App;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

// =====================================================================
// Displayed values
// =====================================================================

#[wasm_bindgen_test]
async fn a_single_holding_renders_its_hand_checked_headline() {
    // £10,000 @ 7% over 10 years is the very projection calc's
    // `annualised_projection_matches_hand_calculation` pins by hand to
    // £19,671.51, so asserting the on-screen headline crosses the whole
    // CalcOutput → format → panel seam end to end.
    let root = harness::mount_with(&state_of(
        vec![row("Global Equity Fund", "10000", "7", "0")],
        "10",
        "years",
    ));

    assert_eq!(harness::text(&root, ".stat-accent .stat-value"), money(19671, 51));
    // "Value today" is the second card.
    let cards = harness::qa(&root, ".summary .stat");
    assert!(cards.len() >= 3, "expected the headline + supporting cards");
    assert!(harness::text_of(&cards[1]).contains("Value today"));
    assert!(harness::text_of(&cards[1]).contains(&money(10000, 0)));
    // Direction label (the non-colour cue).
    assert!(harness::any_text(&root, ".stat-label", "Projected growth"));
}

#[wasm_bindgen_test]
async fn deposits_add_a_card_and_a_table_column() {
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "100")], "10", "years"));

    assert!(
        harness::any_text(&root, ".summary .stat-label", "Added over 10 years"),
        "a monthly deposit should surface an 'Added over' card"
    );
    assert!(
        harness::any_text(&root, ".breakdown th", "Deposits over 10 years"),
        "and a deposits column in the breakdown table"
    );
}

#[wasm_bindgen_test]
async fn drawdown_shows_handover_and_withdrawal_figures() {
    let mut s = state_of(vec![row("Fund", "500000", "5", "0")], "10", "years");
    s.plan = "drawdown".into();
    s.drawdown_value = "30".into();
    s.drawdown_unit = "years".into();
    s.withdrawal = "2000".into();
    let root = harness::mount_with(&s);

    assert!(harness::any_text(&root, ".summary .stat-label", "After 10 years of growth"));
    assert!(harness::any_text(&root, ".breakdown th", "At start of drawdown"));
    assert!(harness::any_text(&root, ".summary .stat-label", "Taken out over 30 years"));
}

#[wasm_bindgen_test]
async fn a_loss_switches_the_label_not_just_the_colour() {
    let root = harness::mount_with(&state_of(vec![row("Sinker", "10000", "-5", "0")], "10", "years"));

    assert!(
        harness::any_text(&root, ".stat-label", "Projected loss"),
        "a negative return must say 'loss', not rely on red alone"
    );
    // The growth value carries an explicit minus sign.
    let loss = harness::qa(&root, ".summary .stat")
        .into_iter()
        .find(|c| harness::text_of(c).contains("Projected loss"))
        .expect("loss card");
    assert!(harness::text_of(&loss).contains('\u{2212}') || harness::text_of(&loss).contains('-'));
}

#[wasm_bindgen_test]
async fn growth_names_its_denominator() {
    let root = harness::mount_with(&ShareState::example());
    assert!(
        harness::any_text(&root, ".stat-note", "put in"),
        "the growth % must name the capital it is measured against"
    );
}

#[wasm_bindgen_test]
async fn depletion_note_appears_when_the_pot_empties() {
    // Small pot, big withdrawal: it runs out before the drawdown period ends.
    let mut s = state_of(vec![row("Fund", "10000", "0", "0")], "1", "years");
    s.plan = "drawdown".into();
    s.drawdown_value = "10".into();
    s.drawdown_unit = "years".into();
    s.withdrawal = "1000".into();
    let root = harness::mount_with(&s);

    assert!(
        harness::any_text(&root, ".depletion-note", "runs out"),
        "an emptying portfolio should announce when the money runs out"
    );
}

#[wasm_bindgen_test]
async fn breakdown_rows_match_the_holdings() {
    let root = harness::mount_with(&state_of(
        vec![row("Alpha", "10000", "7", "0"), row("Beta", "5000", "3", "0")],
        "10",
        "years",
    ));
    let body_rows = harness::qa(&root, ".breakdown tbody tr");
    assert_eq!(body_rows.len(), 2);
    assert!(harness::text_of(&body_rows[0]).contains("Alpha"));
    assert!(harness::text_of(&body_rows[0]).contains(&money(10000, 0)));
    assert!(harness::text_of(&body_rows[1]).contains("Beta"));
}

#[wasm_bindgen_test]
async fn chart_describes_itself_for_a_screen_reader() {
    let root = harness::mount_with(&ShareState::example());
    let chart = harness::q(&root, ".chart");
    assert_eq!(chart.get_attribute("role").as_deref(), Some("img"));
    let label = chart.get_attribute("aria-label").unwrap_or_default();
    assert!(
        label.contains("Line chart of projected portfolio value"),
        "chart aria-label was: {label}"
    );
}

#[wasm_bindgen_test]
async fn an_empty_form_shows_the_placeholder() {
    let root = harness::mount_with(&ShareState::example());
    // Remove every row.
    loop {
        let btns = harness::qa(&root, ".btn-remove");
        let Some(btn) = btns.into_iter().next() else { break };
        harness::click(&btn);
        harness::settle().await;
    }
    assert!(harness::q_opt(&root, ".chart-placeholder").is_some());
    // The three em-dash placeholder cards.
    let dashes = harness::qa(&root, ".summary .stat-value")
        .iter()
        .filter(|c| harness::text_of(c) == "\u{2014}")
        .count();
    assert_eq!(dashes, 3);
}

// =====================================================================
// Interactions
// =====================================================================

#[wasm_bindgen_test]
async fn typing_a_value_reprojects() {
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "0", "0")], "10", "years"));
    assert_eq!(harness::text(&root, ".stat-accent .stat-value"), money(10000, 0));

    let value = harness::row_input(&root, 0, 1);
    harness::type_into(&value, "20000").await;

    assert_eq!(harness::text(&root, ".stat-accent .stat-value"), money(20000, 0));
}

#[wasm_bindgen_test]
async fn switching_mode_reveals_and_hides_the_drawdown_controls() {
    let root = harness::mount_with(&ShareState::example());
    assert!(harness::q_opt(&root, "#drawdown-value").is_none());

    harness::click(&harness::q(&root, "#mode-drawdown"));
    harness::settle().await;
    assert!(harness::q_opt(&root, "#drawdown-value").is_some());
    assert!(harness::q_opt(&root, "#withdrawal").is_some());

    harness::click(&harness::q(&root, "#mode-deposits"));
    harness::settle().await;
    assert!(harness::q_opt(&root, "#drawdown-value").is_none());
}

#[wasm_bindgen_test]
async fn switching_mode_keeps_the_same_horizon_input_node() {
    // The growth-period input is shared by both modes: it must be the *same* DOM
    // node across a switch, or focus/caret would jump. (app.rs keeps it out of
    // the mode-switched closure precisely for this.)
    let root = harness::mount_with(&ShareState::example());
    let before = harness::q(&root, "#horizon-value");
    harness::click(&harness::q(&root, "#mode-drawdown"));
    harness::settle().await;
    let after = harness::q(&root, "#horizon-value");
    assert!(before == after, "the horizon input node was rebuilt across the mode switch");
}

#[wasm_bindgen_test]
async fn the_goal_select_swaps_option_sets_per_mode() {
    let root = harness::mount_with(&ShareState::example());
    // Deposits mode: the two deposits questions.
    let dep = harness::q(&root, ".goal select");
    let dep_text = harness::text_of(&dep);
    assert!(dep_text.contains("monthly top-up needed"));

    harness::click(&harness::q(&root, "#mode-drawdown"));
    harness::settle().await;
    let dd = harness::q(&root, ".goal select");
    let dd_text = harness::text_of(&dd);
    assert!(dd_text.contains("monthly withdrawal I can afford"));
    assert!(dd_text.contains("how long it lasts"));
}

#[wasm_bindgen_test]
async fn a_seeded_non_default_select_value_applies() {
    // The horizon-unit select defaults to "years"; a seeded "months" must win —
    // the `selected=` driving (not `prop:value`) is what makes that work.
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "18", "months"));
    let sel = harness::select_by_label(&root, "Growth period unit");
    assert_eq!(sel.value(), "months");
}

#[wasm_bindgen_test]
async fn a_seeded_drawdown_field_applies_when_it_mounts() {
    let mut s = state_of(vec![row("Fund", "500000", "5", "0")], "10", "years");
    s.plan = "drawdown".into();
    s.drawdown_value = "25".into();
    s.drawdown_unit = "years".into();
    s.withdrawal = "1500".into();
    let root = harness::mount_with(&s);
    // The drawdown fields are text inputs driven by `bind_value`'s mount-time
    // effect (a microtask), so let it apply the seeded values before reading.
    harness::settle().await;

    let dd = harness::input_by_id(&root, "drawdown-value");
    assert_eq!(dd.value(), "25");
    let w = harness::input_by_id(&root, "withdrawal");
    assert_eq!(w.value(), "1500");
}

#[wasm_bindgen_test]
async fn add_investment_appends_a_blank_row() {
    let root = harness::mount_with(&state_of(vec![row("Only", "10000", "7", "0")], "10", "years"));
    assert_eq!(harness::qa(&root, ".inv-row").len(), 1);

    let add = harness::find_button(&root, "+ Add investment");
    harness::click(&add);
    harness::settle().await;

    assert_eq!(harness::qa(&root, ".inv-row").len(), 2);
    // The new row is blank.
    assert_eq!(harness::row_input(&root, 1, 1).value(), "");
}

#[wasm_bindgen_test]
async fn removing_a_row_moves_focus_to_the_successor() {
    let root = harness::mount_with(&state_of(
        vec![row("Alpha", "10000", "7", "0"), row("Beta", "5000", "3", "0")],
        "10",
        "years",
    ));
    // Remove the first row; focus should land on the button of the row that slid
    // up into slot 0 (formerly Beta).
    let first_remove = harness::qa(&root, ".btn-remove").into_iter().next().unwrap();
    harness::click(&first_remove);
    harness::settle().await;

    let active = harness::active_element();
    let now_first = harness::qa(&root, ".btn-remove").into_iter().next().unwrap();
    assert!(active == Some(now_first), "focus did not move to the successor row's remove button");

    // Removing the last row focuses "+ Add investment".
    let last_remove = harness::qa(&root, ".btn-remove").into_iter().next().unwrap();
    harness::click(&last_remove);
    harness::settle().await;
    let add = harness::find_button(&root, "+ Add investment");
    assert!(harness::active_element() == Some(add.into()));
}

#[wasm_bindgen_test]
async fn remove_buttons_name_the_holding_they_remove() {
    let root = harness::mount_with(&state_of(
        vec![row("Global Equity Fund", "10000", "7", "0"), row("", "5000", "3", "0")],
        "10",
        "years",
    ));
    let btns = harness::qa(&root, ".btn-remove");
    assert_eq!(btns[0].get_attribute("aria-label").as_deref(), Some("Remove Global Equity Fund"));
    // The unnamed second row falls back to a 1-based position.
    assert_eq!(btns[1].get_attribute("aria-label").as_deref(), Some("Remove investment 2"));
}

#[wasm_bindgen_test]
async fn typing_mid_string_does_not_bounce_the_caret() {
    // bind_value writes the DOM value only when it differs from the signal, so a
    // mid-string edit keeps the caret where the user left it.
    let root = harness::mount_with(&state_of(vec![row("Fund", "12345", "7", "0")], "10", "years"));
    // Let `bind_value` write the seeded "12345" into the DOM before we place the
    // caret inside it (its mount-time effect is a microtask).
    harness::settle().await;
    let value = harness::row_input(&root, 0, 1);
    value.focus().ok();
    value.set_selection_range(2, 2).ok();
    // Fire input without moving the value (the DOM already matches the signal
    // after a no-op re-set). The caret must be untouched.
    harness::dispatch_input(&value).await;
    assert_eq!(value.selection_start().unwrap(), Some(2));
}

// =====================================================================
// Errors and the last-good panel contract
// =====================================================================

#[wasm_bindgen_test]
async fn an_invalid_value_flags_only_its_own_control() {
    let root = harness::mount_with(&state_of(
        vec![row("Fund", "10000", "7", "0"), row("Other", "5000", "3", "0")],
        "10",
        "years",
    ));
    let bad = harness::row_input(&root, 0, 1);
    harness::type_into(&bad, "abc").await;

    assert_eq!(bad.get_attribute("aria-invalid").as_deref(), Some("true"));
    assert_eq!(bad.get_attribute("aria-describedby").as_deref(), Some("calc-error"));
    assert!(bad.class_list().contains("field-invalid"));

    // A sibling control is untouched.
    let good = harness::row_input(&root, 1, 1);
    assert_ne!(good.get_attribute("aria-invalid").as_deref(), Some("true"));

    assert!(!harness::text(&root, "#calc-error").is_empty());
}

#[wasm_bindgen_test]
async fn an_invalid_horizon_flags_the_horizon_control() {
    let root = harness::mount_with(&ShareState::example());
    let horizon = harness::input_by_id(&root, "horizon-value");
    harness::type_into(&horizon, "0").await;
    assert_eq!(horizon.get_attribute("aria-invalid").as_deref(), Some("true"));
    assert_eq!(horizon.get_attribute("aria-describedby").as_deref(), Some("calc-error"));
}

#[wasm_bindgen_test]
async fn a_transient_error_dims_the_projection_instead_of_blanking_it() {
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    let good = harness::text(&root, ".stat-accent .stat-value");
    assert_eq!(good, money(19671, 51));

    let value = harness::row_input(&root, 0, 1);
    harness::type_into(&value, "abc").await;

    // Both panels' bodies are dimmed but still show the last-good figure.
    for body in harness::qa(&root, ".results-body") {
        assert!(body.class_list().contains("stale"), "results-body should be .stale during an error");
        assert_eq!(body.get_attribute("aria-busy").as_deref(), Some("true"));
    }
    assert_eq!(harness::text(&root, ".stat-accent .stat-value"), good, "figures must survive the error");

    // Fixing the input clears the dim.
    harness::type_into(&value, "10000").await;
    for body in harness::qa(&root, ".results-body") {
        assert!(!body.class_list().contains("stale"), "the dim should clear once valid");
    }
}

#[wasm_bindgen_test]
async fn the_error_announcement_is_debounced() {
    let root = harness::mount_with(&ShareState::example());
    let value = harness::row_input(&root, 0, 1);
    harness::type_into(&value, "abc").await;

    // Immediately after the keystroke the sr-only status is still empty.
    assert_eq!(harness::text(&root, ".sr-only[role=status]"), "");
    // After the debounce it carries the message.
    harness::sleep(900).await;
    assert!(!harness::text(&root, ".sr-only[role=status]").is_empty());
}

// =====================================================================
// Goal seek
// =====================================================================

#[wasm_bindgen_test]
async fn a_deposits_goal_reads_back_its_target_and_period() {
    let mut s = state_of(vec![row("Fund", "10000", "7", "500")], "10", "years");
    s.goal_kind = "topup".into();
    s.goal_target = "500000".into();
    let root = harness::mount_with(&s);

    let g = harness::text(&root, ".goal-result");
    assert!(!g.is_empty(), "a set target should produce an answer sentence");
    assert!(g.contains("10 years"), "answer names the growth period: {g}");
}

#[wasm_bindgen_test]
async fn an_unreachable_goal_does_not_dim_the_projection() {
    // Reaching an absurd target with a tiny pot and no time is impossible, but
    // that is not an *input* error — the projection must stay bright.
    let mut s = state_of(vec![row("Fund", "1000", "1", "0")], "1", "years");
    s.goal_kind = "topup".into();
    s.goal_target = "999999999".into();
    let root = harness::mount_with(&s);

    assert!(!harness::text(&root, ".goal-result").is_empty());
    for body in harness::qa(&root, ".results-body") {
        assert!(!body.class_list().contains("stale"), "a failed goal must not mark the projection stale");
    }
}

#[wasm_bindgen_test]
async fn drawdown_goals_use_the_withdrawal_box_not_a_target() {
    let mut s = state_of(vec![row("Fund", "500000", "5", "0")], "10", "years");
    s.plan = "drawdown".into();
    s.drawdown_value = "30".into();
    s.drawdown_unit = "years".into();
    s.withdrawal = "".into();
    s.goal_kind = "withdrawal".into();
    let root = harness::mount_with(&s);

    // No target box in drawdown mode.
    assert!(harness::q_opt(&root, "#goal-target").is_none());
    // MaxWithdrawal is live even with the withdrawal box blank.
    assert!(!harness::text(&root, ".goal-result").is_empty());
    assert!(harness::text(&root, ".goal-result").contains("take out"));
}

// =====================================================================
// Chart scrubber
// =====================================================================

#[wasm_bindgen_test]
async fn arrow_and_page_keys_step_the_scrubber() {
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    let scrub = harness::q(&root, ".chart-scrub");
    // Starts at the end of the timeline (month = total_months = 120).
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("120"));

    harness::press_key(&scrub, "ArrowLeft").await;
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("119"));
    harness::press_key(&scrub, "PageDown").await; // a year
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("107"));
    harness::press_key(&scrub, "Home").await;
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("0"));
    harness::press_key(&scrub, "End").await;
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("120"));

    // The readout describes the current month.
    let vt = scrub.get_attribute("aria-valuetext").unwrap_or_default();
    assert!(vt.contains("Year 10") || vt.contains("Month"), "valuetext was: {vt}");
}

#[wasm_bindgen_test]
async fn a_transient_error_does_not_reset_the_scrubber() {
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    let scrub = harness::q(&root, ".chart-scrub");
    harness::press_key(&scrub, "Home").await; // month 0
    assert_eq!(scrub.get_attribute("aria-valuenow").as_deref(), Some("0"));

    // A transient typo dims the panel via a reactive attribute, not a rebuild,
    // so the scrubber keeps its position and keyboard focus.
    let value = harness::row_input(&root, 0, 1);
    harness::type_into(&value, "abc").await;

    let scrub_after = harness::q(&root, ".chart-scrub");
    assert!(scrub == scrub_after, "the chart was rebuilt during the error");
    assert_eq!(scrub_after.get_attribute("aria-valuenow").as_deref(), Some("0"));
}

// =====================================================================
// Fixtures
// =====================================================================


// --- accounts and the tax controls -----------------------------------------

/// A drawdown state with a withdrawal order and tax details filled in.
fn taxed_state(rows: Vec<RowData>, strategy: &str) -> ShareState {
    let mut s = state_of(rows, "1", "months");
    s.plan = "drawdown".into();
    s.drawdown_value = "20".into();
    s.drawdown_unit = "years".into();
    s.withdrawal = "1500".into();
    s.strategy = strategy.into();
    s.age = "60".into();
    s
}

/// The account kinds the app is actually wired to, asked of the tax system so a
/// test never hard-codes a jurisdiction's wrappers.
fn kinds() -> &'static [taxkit::AccountKind] {
    app::convert::active_system().account_kinds()
}

#[wasm_bindgen_test]
async fn the_account_select_offers_exactly_what_the_tax_system_advertises() {
    // Asserted against the catalogue rather than a written-out list, so the
    // control cannot silently drift from the system behind it.
    let root = harness::mount_with(&taxed_state(vec![row("Fund", "10000", "7", "0")], "cheapest"));
    let options = harness::qa(&root, ".fld-account option");
    let values: Vec<String> = options
        .iter()
        .map(|o| o.get_attribute("value").unwrap_or_default())
        .collect();
    let expected: Vec<String> = kinds().iter().map(|k| k.id.to_string()).collect();
    assert_eq!(values, expected, "the select must be built from the catalogue");
}

#[wasm_bindgen_test]
async fn a_seeded_account_shows_the_seeded_value_not_the_first_option() {
    // The select trap: props are set before options exist, so a non-default
    // seeded value silently falls back to the first entry unless `selected=`
    // drives it. Pick the LAST kind so a fallback would be obvious.
    let last = kinds().last().expect("the catalogue is not empty").id;
    let mut r = row("Fund", "10000", "7", "0");
    r.account_kind = last.into();
    let root = harness::mount_with(&taxed_state(vec![r], "cheapest"));

    let select = harness::q(&root, ".fld-account select");
    let select: web_sys::HtmlSelectElement = select.dyn_into().unwrap();
    assert_eq!(select.value(), last);
}

#[wasm_bindgen_test]
async fn the_cost_box_follows_the_account_flag_not_a_named_wrapper() {
    // Shown for kinds whose `needs_cost_basis` is set, and only those — driven
    // by the catalogue flag, never by matching an account id in the markup.
    let wants = kinds()
        .iter()
        .find(|k| k.needs_cost_basis)
        .expect("some kind is taxed on the gain");
    let plain = kinds()
        .iter()
        .find(|k| !k.needs_cost_basis)
        .expect("some kind is not");

    let mut r = row("Fund", "10000", "7", "0");
    r.account_kind = wants.id.into();
    let root = harness::mount_with(&taxed_state(vec![r], "cheapest"));
    assert!(
        harness::q_opt(&root, ".fld-basis").is_some(),
        "a gains-taxed account asks what it cost"
    );

    let select = harness::q(&root, ".fld-account select");
    let sel: web_sys::HtmlSelectElement = select.clone().dyn_into().unwrap();
    sel.set_value(plain.id);
    harness::dispatch_change(&select).await;
    assert!(
        harness::q_opt(&root, ".fld-basis").is_none(),
        "and an account that ignores the figure must not ask for it"
    );
}

#[wasm_bindgen_test]
async fn the_account_picker_is_drawdown_only() {
    // In deposits mode the account changes nothing about the numbers, so an
    // inert control that looks like it matters would be worse than none — and
    // hiding it keeps the aligned four-field row exactly as it was before
    // accounts existed.
    let deposits =
        harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    assert!(harness::q_opt(&deposits, ".fld-account").is_none());
    assert!(
        !harness::any_text(&deposits, ".breakdown th", "Account"),
        "and the breakdown must not name accounts the reader cannot see or change"
    );
}

#[wasm_bindgen_test]
async fn the_tax_controls_appear_only_while_drawing_down() {
    let deposits =
        harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    assert!(harness::q_opt(&deposits, ".tax-settings").is_none());

    let drawing =
        harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest"));
    assert!(harness::q_opt(&drawing, ".tax-settings").is_some());
}

#[wasm_bindgen_test]
async fn the_tax_details_are_inert_while_splitting_pro_rata() {
    // Pro-rata ignores tax entirely, so asking for other income and an age would
    // imply they changed something. The order picker is still there.
    let root = harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], ""));
    assert!(harness::q_opt(&root, "#strategy").is_some(), "the picker is always offered");
    assert!(
        harness::q_opt(&root, "#other-income").is_none(),
        "but the tax details it does not use are not"
    );
}

#[wasm_bindgen_test]
async fn seeded_tax_details_apply_when_they_mount() {
    let mut s = taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest");
    s.other_income = "12000".into();
    let root = harness::mount_with(&s);
    // bind_value's mount-time effect is a microtask; let it apply before reading.
    harness::settle().await;

    assert_eq!(harness::input_by_id(&root, "other-income").value(), "12000");
    assert_eq!(harness::input_by_id(&root, "age").value(), "60");
}

#[wasm_bindgen_test]
async fn an_invalid_other_income_marks_that_control_and_no_other() {
    let root =
        harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest"));
    harness::settle().await;

    let income = harness::input_by_id(&root, "other-income");
    harness::type_into(&income, "not a number").await;

    assert_eq!(income.get_attribute("aria-invalid").as_deref(), Some("true"));
    assert_eq!(
        income.get_attribute("aria-describedby").as_deref(),
        Some("calc-error"),
        "the message must be read out with the field it belongs to"
    );
    let withdrawal = harness::input_by_id(&root, "withdrawal");
    assert!(
        withdrawal.get_attribute("aria-invalid").is_none(),
        "only the offending control is flagged"
    );
}

// --- the strategy comparison ------------------------------------------------

#[wasm_bindgen_test]
async fn the_comparison_lists_every_strategy_and_ranks_none_of_them() {
    let root =
        harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest"));
    let rows = harness::qa(&root, ".strategy-compare tbody tr");
    assert_eq!(rows.len(), app::strategy::candidates().len(), "every candidate gets a row");

    // No ranking, no winner, no delta. Reporting several axes without ordering
    // them is the design, so a "best" marker appearing here is a regression.
    let html = harness::q(&root, ".strategy-compare").inner_html().to_lowercase();
    for banned in ["recommend", "saves you", "winner"] {
        assert!(!html.contains(banned), "the table must not rank: found '{banned}'");
    }
}

#[wasm_bindgen_test]
async fn the_comparison_lives_in_its_own_panel_with_explained_headings() {
    let root =
        harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest"));
    // Its own panel, not tucked below the breakdown table.
    let panel = harness::q_opt(&root, ".panel-strategy").expect("the comparison has its own panel");
    assert!(
        panel.query_selector(".strategy-compare").unwrap().is_some(),
        "the comparison table lives inside that panel"
    );
    // Every column heading and every strategy row explains itself on hover.
    for th in harness::qa(&root, ".strategy-compare thead th") {
        assert!(
            !th.get_attribute("title").unwrap_or_default().is_empty(),
            "each column heading carries a tooltip"
        );
    }
    for th in harness::qa(&root, ".strategy-compare tbody th[scope='row']") {
        assert!(
            !th.get_attribute("title").unwrap_or_default().is_empty(),
            "each strategy row says what it does"
        );
    }
}

#[wasm_bindgen_test]
async fn the_comparison_does_not_dim_the_projection() {
    // It is a separate memo for exactly this reason: a strategy that empties the
    // pot early is not an input error and must leave the panels alone.
    let root = harness::mount_with(&taxed_state(vec![row("Fund", "10000", "0", "0")], "cheapest"));
    for body in harness::qa(&root, ".results-body") {
        assert!(
            !body.class_list().contains("stale"),
            "a strategy that runs out must not dim the projection"
        );
    }
}

// --- the freshness line -----------------------------------------------------

#[wasm_bindgen_test]
async fn tax_figures_always_carry_the_rules_they_used() {
    let root =
        harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], "cheapest"));
    let line = harness::text(&root, ".tax-asof");
    assert!(
        line.contains(app::convert::active_system().rules_label()),
        "a tax figure without a date is one you cannot judge: {line}"
    );

    // A stale warning, if shown, must be a note rather than an alert: the page
    // recomputes on every keystroke and an alert would interrupt a screen reader.
    if let Some(note) = harness::q_opt(&root, ".tax-stale") {
        assert_eq!(note.get_attribute("role").as_deref(), Some("note"));
    }
}

#[wasm_bindgen_test]
async fn a_pro_rata_projection_advertises_no_tax_rules() {
    // It never consulted them, so claiming a tax year would be a lie.
    let root = harness::mount_with(&taxed_state(vec![row("Fund", "300000", "5", "0")], ""));
    assert!(harness::q_opt(&root, ".tax-asof").is_none());
}
fn row(name: &str, value: &str, rate: &str, contribution: &str) -> RowData {
    RowData {
        id: 0,
        name: name.into(),
        value: value.into(),
        rate: rate.into(),
        contribution: contribution.into(),
        ..Default::default()
    }
}

/// A deposits-mode `ShareState` from a set of rows and a growth horizon; the
/// drawdown/goal fields default empty. Mirrors calc's `deposits`/`holding`
/// builders so a test reads as one line of intent.
fn state_of(rows: Vec<RowData>, horizon_value: &str, horizon_unit: &str) -> ShareState {
    ShareState {
        rows,
        horizon_value: horizon_value.into(),
        horizon_unit: horizon_unit.into(),
        plan: "deposits".into(),
        drawdown_value: String::new(),
        drawdown_unit: "years".into(),
        withdrawal: String::new(),
        goal_target: String::new(),
        goal_kind: "topup".into(),
        strategy: String::new(),
        rate_cap: String::new(),
        region: String::new(),
        other_income: String::new(),
        age: String::new(),
        uprate: String::new(),
        options: std::collections::BTreeMap::new(),
        jurisdiction: String::new(),
    }
}

/// `£{pounds}.{pence:02}` with grouping, matching `format::fmt_money`, so an
/// assertion reads as the on-screen string rather than a raw number.
fn money(pounds: u64, pence: u64) -> String {
    let digits: Vec<char> = pounds.to_string().chars().collect();
    let mut grouped = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*c);
    }
    format!("\u{00a3}{grouped}.{pence:02}")
}

// =====================================================================
// The jurisdiction picker (Phase C)
// =====================================================================

/// A drawdown state seeded under Germany.
fn de_state(rows: Vec<RowData>, strategy: &str) -> ShareState {
    let mut s = taxed_state(rows, strategy);
    s.jurisdiction = "de".into();
    s
}

#[wasm_bindgen_test]
async fn a_german_link_uses_euro_and_the_german_catalogue() {
    let root = harness::mount_with(&de_state(vec![row("Depot", "100000", "5", "0")], "cheapest"));
    harness::settle().await;
    let summary = harness::text(&root, ".panel-summary");
    assert!(summary.contains('\u{20ac}'), "figures show the euro: {summary}");
    assert!(!summary.contains('\u{00a3}'), "no pound under Germany: {summary}");

    let values: Vec<String> = harness::qa(&root, ".fld-account option")
        .iter()
        .map(|o| o.get_attribute("value").unwrap_or_default())
        .collect();
    let expected: Vec<String> = app::jurisdiction::system_from("de")
        .account_kinds()
        .iter()
        .map(|k| k.id.to_string())
        .collect();
    assert_eq!(values, expected, "the account select is the German catalogue");
}

#[wasm_bindgen_test]
async fn switching_the_picker_changes_the_currency_even_in_deposits_mode() {
    // Deposits mode is the exacting case: the projection numbers are identical
    // between jurisdictions (no tax), so only the currency symbol changes — the
    // panel must still re-render.
    let root = harness::mount_with(&state_of(vec![row("Fund", "10000", "7", "0")], "10", "years"));
    assert!(harness::text(&root, ".panel-summary").contains('\u{00a3}'), "starts in pounds");

    let pick = harness::q(&root, ".jurisdiction-pick select");
    let sel: web_sys::HtmlSelectElement = pick.clone().dyn_into().unwrap();
    sel.set_value("de");
    harness::dispatch_change(&pick).await;
    harness::settle().await;

    let summary = harness::text(&root, ".panel-summary");
    assert!(summary.contains('\u{20ac}'), "switches to euro: {summary}");
    assert!(!summary.contains('\u{00a3}'), "and drops the pound: {summary}");
}

#[wasm_bindgen_test]
async fn the_german_settings_panel_mounts_only_under_germany() {
    let uk = harness::mount_with(&taxed_state(vec![row("Fund", "10000", "7", "0")], "cheapest"));
    assert!(
        harness::q_opt(&uk, ".system-options").is_none(),
        "the UK declares no bespoke controls, so nothing mounts"
    );
    let de = harness::mount_with(&de_state(vec![row("Depot", "100000", "5", "0")], "cheapest"));
    assert!(
        harness::q_opt(&de, ".system-options").is_some(),
        "Germany mounts its assessment/base-year panel"
    );
    // Switching mode is unaffected by the jurisdiction: still drawing down.
    assert!(
        harness::q_opt(&de, ".tax-settings").is_some(),
        "the drawdown tax fieldset is present under Germany too"
    );
}

// =====================================================================
// Harness
// =====================================================================

mod harness {
    use super::*;
    use leptos::*;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        Element, Event, EventInit, HtmlButtonElement, HtmlElement, HtmlInputElement,
        HtmlSelectElement, KeyboardEvent, KeyboardEventInit,
    };

    fn document() -> web_sys::Document {
        window().document().unwrap()
    }

    /// Mount a fresh `<App/>` seeded from `state` (through the real share codec,
    /// so the `#v=` load path is exercised too) into its own appended `<div>`,
    /// and return that root for scoped querying. CSS is injected once so
    /// layout-dependent behaviour (the scrubber width) works.
    pub fn mount_with(state: &ShareState) -> Element {
        inject_styles();
        let hash = format!("#{}", share::encode(state));
        window().location().set_hash(&hash).unwrap();

        let doc = document();
        let host = doc.create_element("div").unwrap();
        doc.body().unwrap().append_child(&host).unwrap();
        let mount: HtmlElement = host.clone().dyn_into().unwrap();
        mount_to(mount, || view! { <App/> });
        host
    }

    /// Give the test document the app stylesheet once. Without a real width the
    /// scrubber's pixel maths (client_width) can't run.
    fn inject_styles() {
        let doc = document();
        if doc.get_element_by_id("ui-test-styles").is_some() {
            return;
        }
        let style = doc.create_element("style").unwrap();
        style.set_id("ui-test-styles");
        style.set_text_content(Some(include_str!("../styles.css")));
        doc.head().unwrap().append_child(&style).unwrap();
    }

    /// Flush Leptos's microtask-queued effects. Leptos 0.6 has no `tick()`; a
    /// resolved-promise await drains the microtask queue, and two passes cover an
    /// effect that queues another effect.
    pub async fn settle() {
        for _ in 0..2 {
            let p = js_sys::Promise::resolve(&JsValue::UNDEFINED);
            let _ = JsFuture::from(p).await;
        }
    }

    /// Wait `ms` real milliseconds (for the debounced announcement).
    pub async fn sleep(ms: i32) {
        let p = js_sys::Promise::new(&mut |resolve, _| {
            window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        let _ = JsFuture::from(p).await;
    }

    pub fn q(root: &Element, sel: &str) -> Element {
        q_opt(root, sel).unwrap_or_else(|| panic!("no element matched {sel}"))
    }

    pub fn q_opt(root: &Element, sel: &str) -> Option<Element> {
        root.query_selector(sel).unwrap()
    }

    pub fn qa(root: &Element, sel: &str) -> Vec<Element> {
        let list = root.query_selector_all(sel).unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i))
            .filter_map(|n| n.dyn_into::<Element>().ok())
            .collect()
    }

    pub fn text(root: &Element, sel: &str) -> String {
        text_of(&q(root, sel))
    }

    pub fn text_of(el: &Element) -> String {
        el.text_content().unwrap_or_default().trim().to_string()
    }

    /// True if any element matching `sel` under `root` contains `needle`.
    pub fn any_text(root: &Element, sel: &str, needle: &str) -> bool {
        qa(root, sel).iter().any(|e| text_of(e).contains(needle))
    }

    pub fn input_by_id(root: &Element, id: &str) -> HtmlInputElement {
        q(root, &format!("#{id}")).dyn_into().unwrap()
    }

    /// The `input` in `.inv-row[row]` at DOM position `field` (0 name, 1 value,
    /// 2 deposit, 3 rate — the order they render in).
    pub fn row_input(root: &Element, row: usize, field: usize) -> HtmlInputElement {
        let rows = qa(root, ".inv-row");
        let inputs = qa(&rows[row], "input");
        inputs[field].clone().dyn_into().unwrap()
    }

    pub fn select_by_label(root: &Element, label: &str) -> HtmlSelectElement {
        qa(root, "select")
            .into_iter()
            .find(|s| s.get_attribute("aria-label").as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no select labelled {label}"))
            .dyn_into()
            .unwrap()
    }

    pub fn find_button(root: &Element, label: &str) -> HtmlButtonElement {
        qa(root, "button")
            .into_iter()
            .find(|b| text_of(b).contains(label))
            .unwrap_or_else(|| panic!("no button reading {label}"))
            .dyn_into()
            .unwrap()
    }

    pub fn click(el: &Element) {
        el.unchecked_ref::<HtmlElement>().click();
    }

    pub fn active_element() -> Option<Element> {
        document().active_element()
    }

    fn bubbling(kind: &str) -> Event {
        let init = EventInit::new();
        init.set_bubbles(true);
        Event::new_with_event_init_dict(kind, &init).unwrap()
    }

    /// Set an input's value and fire a bubbling `input` (Leptos delegates events
    /// at the root, so it must bubble), then settle.
    pub async fn type_into(el: &HtmlInputElement, value: &str) {
        el.set_value(value);
        el.dispatch_event(&bubbling("input")).unwrap();
        settle().await;
    }

    /// Fire a bubbling `input` without changing the value (caret test).
    pub async fn dispatch_input(el: &HtmlInputElement) {
        el.dispatch_event(&bubbling("input")).unwrap();
        settle().await;
    }

    /// Fire a bubbling `change` — what a `<select>` handler listens for, where
    /// an `<input>` listens for `input`.
    pub async fn dispatch_change(el: &Element) {
        el.dispatch_event(&bubbling("change")).unwrap();
        settle().await;
    }

    pub async fn press_key(el: &Element, key: &str) {
        let init = KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        let ev = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        el.dispatch_event(&ev).unwrap();
        settle().await;
    }
}
