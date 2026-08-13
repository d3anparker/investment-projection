//! The "Breakdown" panel: the interactive value chart (with keyboard-reachable
//! scrubber) and the per-holding table.

use crate::chart::{
    chart_svg, PLOT_BOTTOM_FRAC, PLOT_LEFT_FRAC, PLOT_TOP_FRAC, PLOT_WIDTH_FRAC,
};
use crate::format::{fmt_money, fmt_pct, horizon_label, month_label};
use crate::panel::stale_body;
use calc::CalcOutput;
use leptos::*;

/// The "Breakdown" panel: [`results_view`]'s chart and table inside
/// [`stale_body`]'s shell, or [`empty_view`] when there is nothing to project.
#[component]
pub fn ResultsPanel(
    #[prop(into)] displayed: Signal<Option<CalcOutput>>,
    #[prop(into)] stale: Signal<bool>,
) -> impl IntoView {
    stale_body(displayed, stale, results_view, empty_view)
}

fn results_view(out: &CalcOutput) -> impl IntoView {
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

fn empty_view() -> impl IntoView {
    view! {
        <p class="chart-placeholder">
            "Enter an investment to see the projection."
        </p>
    }
}
