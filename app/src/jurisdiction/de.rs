//! Germany's bespoke controls and figure notes.
//!
//! Layout only: the option ids, labels and note text come from `de-tax`'s
//! exported consts, so a rename there is a compile error here rather than a
//! silently ignored option. The values live in the shared option map, which the
//! projection threads to the tax system.

use leptos::*;

use de_tax::engine::options as de_opts;

use std::collections::BTreeMap;

use super::{NotesSlot, SettingsSlot};

type Options = RwSignal<BTreeMap<String, String>>;

/// Whether option `id` currently equals `want` (treating an unset value as
/// `default`).
fn is(options: Options, id: &'static str, want: &'static str, default: &'static str) -> bool {
    options.with(|m| m.get(id).map(String::as_str).unwrap_or(default) == want)
}

fn set(options: Options, id: &'static str, ev: &web_sys::Event) {
    let v = event_target_value(ev);
    options.update(|m| {
        m.insert(id.to_string(), v);
    });
}

/// The portfolio-level German controls: assessment (individual/joint) and the
/// year drawing starts (which fixes a Rürup pension's cohort taxable share).
pub fn settings(slot: SettingsSlot) -> View {
    let options = slot.options;
    let default_year = slot.today_year.to_string();
    let year_value = move || {
        options.with(|m| {
            m.get(de_opts::BASE_YEAR)
                .cloned()
                .unwrap_or_else(|| default_year.clone())
        })
    };

    view! {
        <div class="system-options">
            <label class="fld">
                <span class="fld-lbl">{de_opts::FILING_LABEL}</span>
                <select on:change=move |ev| set(options, de_opts::FILING, &ev)>
                    <option
                        value=de_opts::FILING_INDIVIDUAL
                        selected=move || is(options, de_opts::FILING, de_opts::FILING_INDIVIDUAL, de_opts::FILING_INDIVIDUAL)
                    >
                        "Individual"
                    </option>
                    <option
                        value=de_opts::FILING_JOINT
                        selected=move || is(options, de_opts::FILING, de_opts::FILING_JOINT, de_opts::FILING_INDIVIDUAL)
                    >
                        "Joint (Splitting)"
                    </option>
                </select>
            </label>
            <label class="fld">
                <span class="fld-lbl">{de_opts::BASE_YEAR_LABEL}</span>
                <input
                    type="number"
                    inputmode="numeric"
                    prop:value=year_value
                    on:change=move |ev| set(options, de_opts::BASE_YEAR, &ev)
                />
            </label>
            <p class="system-options-note">{de_opts::BASE_YEAR_NOTE}</p>
            <p class="system-options-note">{de_opts::FILING_NOTE}</p>
        </div>
    }
    .into_view()
}

/// A plain-language note on what Germany's figures include, shown under the
/// output panels. Static text — descriptive, never advisory.
pub fn notes(_slot: NotesSlot) -> View {
    view! {
        <div class="system-notes" role="note">
            <p>
                "German figures model the \u{00a7}32a income-tax tariff with the \
                 Solidarit\u{00e4}tszuschlag and, where set, Kirchensteuer; the flat \
                 Abgeltungsteuer on capital gains after the Sparer-Pauschbetrag, with fund \
                 Teilfreistellung; and the Vorabpauschale as a yearly charge on fund holdings."
            </p>
            <p>
                "Not modelled: social contributions on pension income, capital losses, and the \
                 realised-gain cap on the Vorabpauschale during drawdown. Not tax advice."
            </p>
        </div>
    }
    .into_view()
}
