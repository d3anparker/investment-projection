//! The full-width "Projection" summary panel: the four headline stat cards, and
//! the placeholder shown when the form is genuinely empty.

use crate::format::{fmt_money, fmt_pct, fmt_signed_money, horizon_label, month_label};
use crate::panel::stale_body;
use calc::CalcOutput;
use leptos::*;

/// The "Projection" panel: [`summary_view`]'s stat cards inside [`stale_body`]'s
/// shell, or [`empty_summary_view`] when there is nothing to project. Being a
/// component of its own is what leaves `App` as layout.
#[component]
pub fn SummaryPanel(
    #[prop(into)] displayed: Signal<Option<CalcOutput>>,
    #[prop(into)] stale: Signal<bool>,
) -> impl IntoView {
    stale_body(displayed, stale, summary_view, empty_summary_view)
}

/// The headline figures. Rendered in its own full-width panel above the two
/// columns: these are the answer the user came for, and hoisting them out also
/// closes most of the dead space that a short form column left beside a tall
/// results column.
fn summary_view(out: &CalcOutput) -> impl IntoView {
    let horizon = out.horizon_months;
    let span = out.total_months;
    let drawdown = out.drawdown_months;
    let gain = !out.growth.is_sign_negative();
    let growth_color = format!("color: var({})", if gain { "--good" } else { "--bad" });

    // In drawdown mode, the pot at the start of drawdown is the figure the whole
    // second phase draws from — shown so a falling final value still reconciles.
    let handover_stat = out.handover_total.map(|pot| {
        view! {
            <div class="stat">
                <span class="stat-label">{format!("After {} of growth", horizon_label(horizon))}</span>
                <span class="stat-value">{fmt_money(pot)}</span>
            </div>
        }
    });

    // Only surface deposits when there actually are some, so a portfolio without
    // top-ups keeps the lean summary. Deposits accrue over the growth period.
    let contributions_stat = (!out.contributed_total.is_zero()).then(|| {
        view! {
            <div class="stat">
                <span class="stat-label">{format!("Added over {}", horizon_label(horizon))}</span>
                <span class="stat-value">{fmt_money(out.contributed_total)}</span>
            </div>
        }
    });

    // Likewise, only show withdrawals when the portfolio is actually being drawn
    // down. They accrue over the drawdown period.
    let withdrawals_stat = (!out.withdrawn_total.is_zero()).then(|| {
        view! {
            <div class="stat">
                <span class="stat-label">{format!("Taken out over {}", horizon_label(drawdown))}</span>
                <span class="stat-value">{fmt_money(out.withdrawn_total)}</span>
            </div>
        }
    });

    // The headline of a drawdown: when the whole portfolio empties before the
    // drawdown period is up, say so plainly. It is the point of the feature, so it
    // sits above the stat cards rather than as one more number among them.
    let depletion_note = out.depletion_month.map(|m| {
        view! {
            <p class="depletion-note" role="note">
                <strong>{format!("Your money runs out in {}.", month_label(m))}</strong>
                " Drawing it down empties the portfolio before the drawdown period ends; \
                 after that point the value stays at \u{00a3}0."
            </p>
        }
    });

    view! {
        {depletion_note}
        <div class="summary">
            // The projection leads: it is the question the tool exists to answer,
            // and at the same size as its own inputs it did not read as one.
            <div class="stat stat-accent">
                <span class="stat-label">{format!("Value in {}", horizon_label(span))}</span>
                <span class="stat-value">{fmt_money(out.projected_total)}</span>
            </div>
            <div class="stat">
                <span class="stat-label">"Value today"</span>
                <span class="stat-value">{fmt_money(out.current_total)}</span>
            </div>
            {handover_stat}
            {contributions_stat}
            {withdrawals_stat}
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

/// Placeholder stats, shown only when the form is genuinely empty. A transient
/// typo keeps the last good figures instead (see `displayed` in `main`), so
/// reaching this state really does mean there is nothing to project.
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
