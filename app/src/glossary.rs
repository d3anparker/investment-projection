//! The glossary: what the words on the page mean, in a modal.
//!
//! Two sources, one renderer. The active tax system supplies its own terms
//! through `taxkit::TaxSystem::glossary()` — it owns its vocabulary, and this
//! module never learns which jurisdiction it is showing. What a tax system
//! cannot explain is *this app's* model: growth period, handover, deployed,
//! pro-rata. Those are [`APP_GLOSSARY`], written here in the same
//! [`taxkit::GlossaryEntry`] shape so both render through the same code.
//!
//! The grouping helpers ([`sections`], [`term_of`]) are pure and natively
//! tested. The browser-bound surface is two calls, `show_modal` and `close`.

use leptos::*;
use taxkit::GlossaryEntry;

/// Topic labels for the app's own terms.
mod topics {
    pub const SHAPE: &str = "The projection";
    pub const DRAWDOWN: &str = "Drawing it down";
    pub const FIGURES: &str = "Reading the figures";
}

/// Terms belonging to this app's model rather than to any tax system.
///
/// A jurisdiction explains its taxes; nothing else explains what "handover" or
/// "deployed" mean, and those are the words a reader stumbles on first. Held in
/// the same type as a system's own entries so there is one renderer, not two.
///
/// Names no jurisdiction and no currency, which the CI greps keep honest.
pub const APP_GLOSSARY: &[GlossaryEntry] = &[
    // --- the projection -----------------------------------------------------
    GlossaryEntry {
        id: "value_today",
        term: "Value today",
        also: "",
        definition: "What a holding is worth now, including any growth it has \
                     already had. There is no history in this tool: every \
                     projection starts from the figure you enter and goes \
                     forward from there.",
        seen_in: "The first box on each holding.",
        topic: topics::SHAPE,
        see_also: &["annualised_return"],
    },
    GlossaryEntry {
        id: "annualised_return",
        term: "Annualised return",
        also: "Growth rate",
        definition: "The rate a holding is assumed to grow at, every year, for \
                     the whole projection. Real returns are nothing like this \
                     steady — the figure is a single smooth assumption, which is \
                     what makes this an illustration rather than a forecast.",
        seen_in: "The rate box on each holding.",
        topic: topics::SHAPE,
        see_also: &["value_today", "not_advice"],
    },
    GlossaryEntry {
        id: "monthly_deposit",
        term: "Monthly deposit",
        also: "Contribution",
        definition: "An amount added to that holding at the end of every month \
                     of the growth period. Deposits stop at the handover: once a \
                     drawdown begins, nothing more goes in.",
        seen_in: "The deposit box on each holding.",
        topic: topics::SHAPE,
        see_also: &["handover", "contributed"],
    },
    GlossaryEntry {
        id: "growth_period",
        term: "Growth period",
        also: "Horizon",
        definition: "How long the holdings grow before anything is taken out. \
                     It is the whole projection when you are building up, and \
                     the first of two phases when you are drawing down.",
        seen_in: "The period control under the holdings, and the x-axis of the \
                  chart.",
        topic: topics::SHAPE,
        see_also: &["handover", "drawdown_period"],
    },
    GlossaryEntry {
        id: "handover",
        term: "Handover",
        also: "",
        definition: "The moment the growth period ends and the drawdown begins. \
                     The pot that gets drawn down is the projected value at that \
                     point, not today's value — which is why a drawdown figure \
                     can look generous against what you hold now.",
        seen_in: "The divider on the chart, marked where the drawdown starts.",
        topic: topics::SHAPE,
        see_also: &["growth_period", "drawdown_period"],
    },
    // --- drawing it down ----------------------------------------------------
    GlossaryEntry {
        id: "drawdown_period",
        term: "Drawdown period",
        also: "",
        definition: "How long you go on taking money out after the handover. \
                     The holdings keep growing throughout it; the withdrawal is \
                     simply larger than the growth, most of the time.",
        seen_in: "The second period control, in drawdown mode.",
        topic: topics::DRAWDOWN,
        see_also: &["handover", "depletion"],
    },
    GlossaryEntry {
        id: "withdrawal",
        term: "Monthly withdrawal",
        also: "",
        definition: "One amount for the whole portfolio, not one per holding. \
                     How it is split across the holdings is what the withdrawal \
                     order decides.",
        seen_in: "The withdrawal box, and the withdrawn column in the breakdown.",
        topic: topics::DRAWDOWN,
        see_also: &["withdrawal_order", "gross_and_net"],
    },
    GlossaryEntry {
        id: "withdrawal_order",
        term: "Withdrawal order",
        also: "Strategy",
        definition: "Which holdings the monthly withdrawal comes out of, and in \
                     what sequence. It changes the tax, how long the pot lasts \
                     and what is left at the end, without changing the amount \
                     you receive.",
        seen_in: "The strategy picker, and each row of the comparison table.",
        topic: topics::DRAWDOWN,
        see_also: &["pro_rata", "nothing_is_ranked", "gross_and_net"],
    },
    GlossaryEntry {
        id: "pro_rata",
        term: "Pro-rata",
        also: "In proportion, rebalanced",
        definition: "Taking from every holding in proportion to what it is \
                     currently worth, recalculated each month. Because the split \
                     follows the balances, every holding runs out in the same \
                     month rather than one at a time.",
        seen_in: "The strategy picker, and the withdrawn column spread across \
                  every row.",
        topic: topics::DRAWDOWN,
        see_also: &["withdrawal_order", "depletion"],
    },
    GlossaryEntry {
        id: "depletion",
        term: "Depletion",
        also: "Running out",
        definition: "The month the portfolio reaches nothing and the withdrawal \
                     can no longer be paid in full. A drawdown that never gets \
                     there simply ends with money still in it.",
        seen_in: "The note under the summary, and where the chart line reaches \
                  the floor.",
        topic: topics::DRAWDOWN,
        see_also: &["drawdown_period", "pro_rata"],
    },
    GlossaryEntry {
        id: "nothing_is_ranked",
        term: "Why nothing is ranked",
        also: "",
        definition: "The comparison shows several consequences of each \
                     withdrawal order — tax, income delivered, how long it \
                     lasts, what is left, allowance unused — and picks no \
                     winner. The order that pays least tax is usually worse on \
                     something else, and a single 'best' figure would read as a \
                     recommendation whatever it was called.",
        seen_in: "The comparison table, which has no highlighted row.",
        topic: topics::DRAWDOWN,
        see_also: &["withdrawal_order", "not_advice"],
    },
    // --- reading the figures ------------------------------------------------
    GlossaryEntry {
        id: "contributed",
        term: "Contributed",
        also: "Deposits",
        definition: "The total of the monthly deposits over the growth period. \
                     Shown alongside the projected value because a holding that \
                     grew from one figure to another makes no sense without the \
                     deposits that bridge them.",
        seen_in: "The contributed column, and the deposits card.",
        topic: topics::FIGURES,
        see_also: &["monthly_deposit", "deployed", "projected_growth"],
    },
    GlossaryEntry {
        id: "deployed",
        term: "Deployed",
        also: "Capital put in",
        definition: "What you actually put in: today's value plus every deposit. \
                     It is the figure the growth percentage is measured against, \
                     stated openly so a percentage is never left to be guessed \
                     at.",
        seen_in: "Under the growth percentage.",
        topic: topics::FIGURES,
        see_also: &["projected_growth", "contributed"],
    },
    GlossaryEntry {
        id: "projected_growth",
        term: "Projected growth",
        also: "Returns only",
        definition: "Investment return alone. Money you deposit is not counted \
                     as growth, and money you withdraw is added back before the \
                     figure is worked out, so neither your own cash movements nor \
                     the tax charged can flatter it.",
        seen_in: "The growth card, which becomes a loss card when negative.",
        topic: topics::FIGURES,
        see_also: &["deployed", "tax_charged"],
    },
    GlossaryEntry {
        id: "gross_and_net",
        term: "Gross and net",
        also: "Before and after tax",
        definition: "Gross is what leaves the holdings; net is what reaches you \
                     after tax. Most withdrawal orders work backwards from the \
                     net you asked for and take whatever gross that needs, so \
                     the amount leaving the pot varies while your income does \
                     not.",
        seen_in: "The withdrawn and tax columns, which add up to the gross.",
        topic: topics::FIGURES,
        see_also: &["withdrawal_order", "withdrawal"],
    },
    GlossaryEntry {
        id: "tax_charged",
        term: "Tax charged while invested",
        also: "Periodic charge",
        definition: "Tax some systems charge on a holding you have not touched, \
                     year by year. Where it applies, the pot grows more slowly \
                     than the return alone suggests — and where it does not, the \
                     figure is simply absent.",
        seen_in: "The charge card and column, which appear only when there is a \
                  charge to show.",
        topic: topics::FIGURES,
        see_also: &["projected_growth"],
    },
    GlossaryEntry {
        id: "not_advice",
        term: "Not advice",
        also: "",
        definition: "This extrapolates the figures you type at a rate you \
                     choose, under one reading of the tax rules, with no \
                     inflation and no allowance for markets going wrong. It is \
                     for curiosity. It is not a forecast, and it is not \
                     financial or tax advice.",
        seen_in: "Everywhere, and worth remembering at the headline figure.",
        topic: topics::FIGURES,
        see_also: &["annualised_return", "nothing_is_ranked"],
    },
];

/// Group entries into their topic sections, preserving order.
///
/// Entries are already contiguous by topic (each glossary's own tests pin
/// that), so this is a single pass and never reorders anything: a system
/// decides how its terms read, not the renderer.
pub fn sections(entries: &'static [GlossaryEntry]) -> Vec<(&'static str, Vec<&'static GlossaryEntry>)> {
    let mut out: Vec<(&'static str, Vec<&'static GlossaryEntry>)> = Vec::new();
    for e in entries {
        match out.last_mut() {
            Some((topic, items)) if *topic == e.topic => items.push(e),
            _ => out.push((e.topic, vec![e])),
        }
    }
    out
}

/// The display term for a cross-reference id, within the same glossary.
///
/// Cross-references render as **text, not links**. An `<a href="#...">` would
/// overwrite the location fragment, and the fragment is where a shared link
/// keeps the whole projection — one click on "see also" and reloading the page
/// would lose the portfolio.
pub fn term_of(entries: &'static [GlossaryEntry], id: &str) -> Option<&'static str> {
    entries.iter().find(|e| e.id == id).map(|e| e.term)
}

/// The "see also" line for an entry, or `None` when it has no live references.
pub fn see_also_line(entries: &'static [GlossaryEntry], entry: &GlossaryEntry) -> Option<String> {
    let terms: Vec<&str> = entry
        .see_also
        .iter()
        .filter_map(|id| term_of(entries, id))
        .collect();
    (!terms.is_empty()).then(|| terms.join(", "))
}

/// One glossary's sections, as a run of headings and definition lists.
fn glossary_view(heading: String, entries: &'static [GlossaryEntry]) -> View {
    if entries.is_empty() {
        return ().into_view();
    }
    view! {
        <section class="gloss-group">
            <h3>{heading}</h3>
            {sections(entries)
                .into_iter()
                .map(|(topic, items)| {
                    view! {
                        <h4 class="gloss-topic">{topic}</h4>
                        <dl class="gloss">
                            {items
                                .into_iter()
                                .map(|e| {
                                    let also = (!e.also.is_empty())
                                        .then(|| view! { <span class="gloss-also">{e.also}</span> });
                                    let seen = (!e.seen_in.is_empty())
                                        .then(|| {
                                            view! {
                                                <p class="gloss-seen">
                                                    <span class="gloss-seen-lbl">"Where you see it: "</span>
                                                    {e.seen_in}
                                                </p>
                                            }
                                        });
                                    let more = see_also_line(entries, e)
                                        .map(|line| {
                                            view! {
                                                <p class="gloss-more">
                                                    <span class="gloss-seen-lbl">"See also: "</span>
                                                    {line}
                                                </p>
                                            }
                                        });
                                    view! {
                                        <div class="gloss-item">
                                            <dt>{e.term}{also}</dt>
                                            <dd>
                                                <p>{e.definition}</p>
                                                {seen}
                                                {more}
                                            </dd>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </dl>
                    }
                })
                .collect_view()}
        </section>
    }
    .into_view()
}

/// The glossary button and the modal it opens.
///
/// A native `<dialog>` shown with `show_modal()`, rather than a hand-rolled
/// overlay: the focus trap, `Esc`, the inert background and the top layer are
/// exactly the parts a hand-rolled modal gets wrong, and the platform has them.
/// Closing is a single path — the close button calls `close()` too, so the
/// `close` event is the only place `open` goes back to `false`.
///
/// The body is built only while open. It is a few hundred nodes, and there is
/// no reason to carry them on every keystroke of a form the reader is actually
/// using.
#[component]
pub fn Glossary(#[prop(into)] jurisdiction: Signal<String>) -> impl IntoView {
    let dialog: NodeRef<html::Dialog> = create_node_ref();
    let open = create_rw_signal(false);

    let show = move |_| {
        open.set(true);
        if let Some(d) = dialog.get_untracked() {
            let _ = d.show_modal();
        }
    };
    let hide = move |_| {
        if let Some(d) = dialog.get_untracked() {
            d.close();
        }
    };

    let system = move || crate::jurisdiction::system_from(&jurisdiction.get());

    view! {
        // The accessible name *extends* the visible text rather than replacing
        // it, so voice control can still say "Glossary" (WCAG 2.5.3).
        <button
            type="button"
            class="btn btn-ghost glossary-open"
            aria-label="Glossary \u{2014} what these terms mean"
            aria-haspopup="dialog"
            on:click=show
        >
            "Glossary"
        </button>
        <dialog
            class="glossary-dialog"
            node_ref=dialog
            aria-labelledby="glossary-h"
            on:close=move |_| open.set(false)
        >
            <div class="glossary-head">
                <h2 id="glossary-h">"Glossary"</h2>
                <button type="button" class="btn btn-ghost glossary-close" on:click=hide>
                    "Close"
                </button>
            </div>
            <div class="glossary-body">
                {move || {
                    open.get()
                        .then(|| {
                            let sys = system();
                            // Content projection: after the generic term list,
                            // the jurisdiction may insert whatever a flat list
                            // cannot say. A jurisdiction with none renders no
                            // wrapper element at all.
                            let projected = crate::jurisdiction::from_id(&jurisdiction.get())
                                .glossary_panel
                                .map(|panel| panel(crate::jurisdiction::GlossarySlot));
                            view! {
                                <p class="glossary-intro">
                                    "What the words on this page mean, and how to read the \
                                     figures. Descriptions only \u{2014} nothing here is \
                                     financial or tax advice."
                                </p>
                                {glossary_view("How this projection works".to_string(), APP_GLOSSARY)}
                                {glossary_view(format!("{} tax terms", sys.label()), sys.glossary())}
                                {projected}
                            }
                        })
                }}
            </div>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sections_group_without_reordering() {
        let grouped = sections(APP_GLOSSARY);
        let flat: Vec<&str> = grouped
            .iter()
            .flat_map(|(_, items)| items.iter().map(|e| e.id))
            .collect();
        let original: Vec<&str> = APP_GLOSSARY.iter().map(|e| e.id).collect();
        assert_eq!(flat, original, "grouping reordered the entries");
        assert!(grouped.len() > 1, "expected more than one topic");
    }

    #[test]
    fn a_topic_never_starts_twice() {
        let grouped = sections(APP_GLOSSARY);
        let mut seen = HashSet::new();
        for (topic, items) in &grouped {
            assert!(seen.insert(*topic), "topic {topic:?} resumes after another");
            assert!(!items.is_empty());
        }
    }

    #[test]
    fn sections_of_nothing_is_nothing() {
        assert!(sections(&[]).is_empty());
    }

    #[test]
    fn app_entries_are_unique_and_say_something() {
        let mut seen = HashSet::new();
        for e in APP_GLOSSARY {
            assert!(seen.insert(e.id), "duplicate id {}", e.id);
            assert!(!e.term.is_empty() && !e.definition.is_empty(), "{}", e.id);
        }
    }

    #[test]
    fn every_cross_reference_resolves() {
        for e in APP_GLOSSARY {
            for r in e.see_also {
                assert!(
                    term_of(APP_GLOSSARY, r).is_some(),
                    "{} points at missing entry {}",
                    e.id,
                    r
                );
            }
        }
    }

    /// A dangling id is dropped rather than printed raw, so a cross-reference
    /// into another glossary cannot leak an internal id onto the page.
    #[test]
    fn see_also_drops_unresolvable_ids() {
        let entry = GlossaryEntry {
            id: "x",
            term: "X",
            also: "",
            definition: "d",
            seen_in: "",
            topic: "t",
            see_also: &["deployed", "no_such_entry"],
        };
        assert_eq!(
            see_also_line(APP_GLOSSARY, &entry).as_deref(),
            Some("Deployed")
        );
    }

    #[test]
    fn see_also_line_is_none_when_nothing_resolves() {
        let entry = GlossaryEntry {
            id: "x",
            term: "X",
            also: "",
            definition: "d",
            seen_in: "",
            topic: "t",
            see_also: &["no_such_entry"],
        };
        assert!(see_also_line(APP_GLOSSARY, &entry).is_none());
    }

    /// The app's own terms describe the projection, not any jurisdiction, and
    /// must never print money — the symbol is reactive and would go stale.
    #[test]
    fn the_app_glossary_names_no_currency() {
        for e in APP_GLOSSARY {
            for text in [e.term, e.also, e.definition, e.seen_in, e.topic] {
                assert!(
                    !text.contains('\u{a3}') && !text.contains('\u{20ac}'),
                    "{} prints a currency symbol",
                    e.id
                );
            }
        }
    }
}
