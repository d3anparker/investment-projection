//! Germany's bespoke controls and figure notes.
//!
//! Layout only: the option ids, labels and note text come from `de-tax`'s
//! exported consts, so a rename there is a compile error here rather than a
//! silently ignored option. The values live in the shared option map, which the
//! projection threads to the tax system.

use leptos::*;

use de_tax::engine::options as de_opts;

use std::collections::BTreeMap;

use super::{GlossarySlot, NotesSlot, SettingsSlot};

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

/// A worked Vorabpauschale, projected into the glossary modal.
///
/// The term list defines the word; this shows the arithmetic, which is the part
/// a reader actually needs — the charge is the one figure in a German
/// projection that appears while nothing is being withdrawn, and "advance
/// lump-sum charge" does not explain where it came from.
///
/// Every figure here is a **written-in illustration**, not a projection value
/// and not a table lookup: a slot may not compute, derive or round anything,
/// and quoting the real Basiszins would put a yearly-changing rate in a second
/// place to update. The round numbers are chosen so the steps can be followed.
pub fn glossary(_slot: GlossarySlot) -> View {
    view! {
        <section class="gloss-worked">
            <h4>"A worked example: the Vorabpauschale"</h4>
            <p>
                "Round illustrative figures, not this year\u{2019}s rates and not your \
                 projection \u{2014} the point is the order of the steps."
            </p>
            <ol>
                <li>
                    "An equity fund is worth \u{20ac}50,000 on 1 January, and say the \
                     year\u{2019}s Basiszins is 2%."
                </li>
                <li>
                    "The Basisertrag is 70% of that rate on the opening value: \
                     \u{20ac}50,000 \u{d7} 2% \u{d7} 70% = \u{20ac}700."
                </li>
                <li>
                    "It is capped at what the fund actually gained. A fund that rose \
                     \u{20ac}400 is charged on \u{20ac}400; a fund that fell is charged \
                     nothing at all."
                </li>
                <li>
                    "The Teilfreistellung exempts part of it \u{2014} for an equity fund, \
                     30% \u{2014} leaving \u{20ac}490 of the \u{20ac}700 taxable."
                </li>
                <li>
                    "That is set against the Sparer-Pauschbetrag first. Whatever is left \
                     is charged at the flat Abgeltungsteuer rate, with Soli and any \
                     Kirchensteuer on top."
                </li>
            </ol>
            <p>
                "Two consequences worth knowing. The charge is paid out of the holding \
                 itself, so a German accumulation grows a little more slowly than the \
                 return alone suggests. And because it uses the same yearly allowance a \
                 withdrawal would, it can leave less of that allowance for later in the \
                 same year."
            </p>
        </section>
    }
    .into_view()
}
