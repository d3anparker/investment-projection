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

/// Map a pointer x-offset (in element pixels) to a month index on the scrubber.
/// The plot does not start at the element's left edge — the y-axis gutter comes
/// first — so `x/width` is shifted by [`PLOT_LEFT_FRAC`] and scaled by
/// [`PLOT_WIDTH_FRAC`] before landing on `0..=span`. Pulled out of the view
/// closure so this arithmetic is unit-testable without a browser.
fn month_at_fraction(x: f64, width: f64, span: u32) -> usize {
    let frac = ((x / width) - PLOT_LEFT_FRAC) / PLOT_WIDTH_FRAC;
    (frac.clamp(0.0, 1.0) * span as f64).round() as usize
}

fn results_view(out: &CalcOutput) -> impl IntoView {
    let horizon = out.horizon_months; // the growth period (also the handover index)
    let drawdown = out.drawdown_months;
    let span = out.total_months; // the whole timeline the chart and scrubber cover
    let drawing = out.handover_total.is_some();
    let handover = drawing.then_some(horizon);
    let svg = chart_svg(&out.series, &out.contributions_series, handover);
    let has_contributions = !out.contributed_total.is_zero();
    let has_withdrawals = !out.withdrawn_total.is_zero();

    // --- scrub state -------------------------------------------------------
    // A slider laid over the plot reads any month out; `active` keeps the marker
    // hidden until the user is actually pointing at or focused on the chart. The
    // scrubber spans the *whole* timeline, accumulation and drawdown alike.
    let series = store_value(out.series.clone());
    let contribs = store_value(out.contributions_series.clone());
    let withdraws = store_value(out.withdrawals_series.clone());
    let cursor = create_rw_signal(span as usize);
    let active = create_rw_signal(false);
    let scrub_ref = create_node_ref::<html::Div>();

    let readout = move || {
        let i = series.with_value(|s| cursor.get().min(s.len().saturating_sub(1)));
        let value = series.with_value(|s| fmt_money(s[i]));
        let when = month_label(i as u32);
        // Name the cash flows to date at this point: paid in, and (past the
        // handover) taken out, so a drawdown month explains where the value went.
        let mut parts: Vec<String> = Vec::new();
        if has_contributions {
            let paid = contribs.with_value(|c| c[i]);
            if !paid.is_zero() {
                parts.push(format!("{} paid in", fmt_money(paid)));
            }
        }
        if has_withdrawals {
            let taken = withdraws.with_value(|w| w[i]);
            if !taken.is_zero() {
                parts.push(format!("{} taken out", fmt_money(taken)));
            }
        }
        if parts.is_empty() {
            format!("{when}: {value}")
        } else {
            format!("{when}: {value} \u{2014} {}", parts.join(", "))
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
        cursor.set(month_at_fraction(x, width, span));
    };

    let on_key = move |ev: ev::KeyboardEvent| {
        let step: i64 = match ev.key().as_str() {
            "ArrowLeft" | "ArrowDown" => -1,
            "ArrowRight" | "ArrowUp" => 1,
            // A year at a time is the useful coarse step for this data.
            "PageDown" => -12,
            "PageUp" => 12,
            "Home" => -(span as i64),
            "End" => span as i64,
            _ => return,
        };
        // Stop the arrow keys scrolling the page out from under the chart.
        ev.prevent_default();
        active.set(true);
        let next = (cursor.get_untracked() as i64 + step).clamp(0, span as i64);
        cursor.set(next as usize);
    };

    let marker_style = move || {
        let at = cursor.get() as f64 / span.max(1) as f64;
        format!(
            "left: {:.4}%; top: {:.4}%; bottom: {:.4}%",
            (PLOT_LEFT_FRAC + at * PLOT_WIDTH_FRAC) * 100.0,
            PLOT_TOP_FRAC * 100.0,
            PLOT_BOTTOM_FRAC * 100.0
        )
    };
    let chart_label = if drawing {
        format!(
            "Line chart of projected portfolio value over {}: growing to {} after {}, \
             then drawn down to {} over a further {}.",
            horizon_label(span),
            fmt_money(out.handover_total.expect("drawing implies a handover total")),
            horizon_label(horizon),
            fmt_money(out.projected_total),
            horizon_label(drawdown),
        )
    } else if has_contributions {
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
            // add, grown at this rate, reaches the handover pot, from which the
            // drawdown is taken to land on the projection.
            let contributed = has_contributions.then(|| {
                let cell = if r.contributed.is_zero() {
                    "\u{2014}".to_string()
                } else {
                    fmt_money(r.contributed)
                };
                view! { <td class="num">{cell}</td> }
            });
            let handover_cell = drawing.then(|| {
                let cell = r.handover_value.map_or("\u{2014}".to_string(), fmt_money);
                view! { <td class="num">{cell}</td> }
            });
            let withdrawn = has_withdrawals.then(|| {
                let cell = if r.withdrawn.is_zero() {
                    "\u{2014}".to_string()
                } else {
                    fmt_money(r.withdrawn)
                };
                view! { <td class="num">{cell}</td> }
            });
            view! {
                <tr>
                    <td>{r.name.clone()}</td>
                    <td class="num">{fmt_money(r.current_value)}</td>
                    {contributed}
                    {handover_cell}
                    {withdrawn}
                    <td class="num">{fmt_pct(r.annualised)}</td>
                    <td class="num">{fmt_money(r.projected_value)}</td>
                </tr>
            }
        })
        .collect_view();

    // The caption states the load-bearing assumption behind the drawdown split.
    let caption = if drawing {
        "Per holding. \u{201c}Annualised\u{201d} is the equivalent yearly rate. During drawdown the \
         monthly withdrawal is taken from the whole portfolio, split across holdings in proportion \
         to their value and rebalanced each month."
    } else {
        "Per holding. \u{201c}Annualised\u{201d} is the equivalent yearly rate, projected forward \
         from each holding\u{2019}s value today."
    };
    let caption_label = if drawing {
        format!("Portfolio value over {} \u{2014} {} of growth, then {} of drawdown.",
            horizon_label(span), horizon_label(horizon), horizon_label(drawdown))
    } else if has_contributions {
        format!("Portfolio value and cumulative contributions from today to {} ahead.", horizon_label(horizon))
    } else {
        format!("Portfolio value from today to {} ahead.", horizon_label(horizon))
    };

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
                         aria-valuemax=span.to_string()
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
            <figcaption class="chart-caption">{caption_label}</figcaption>
        </figure>

        <div class="table-scroll">
            <table class="breakdown">
                <caption class="table-note">{caption}</caption>
                <thead>
                    <tr>
                        <th scope="col">"Investment"</th>
                        <th scope="col">"Value today"</th>
                        {has_contributions.then(|| view! {
                            <th scope="col">{format!("Deposits over {}", horizon_label(horizon))}</th>
                        })}
                        {drawing.then(|| view! {
                            <th scope="col">"At start of drawdown"</th>
                        })}
                        {has_withdrawals.then(|| view! {
                            <th scope="col">{format!("Taken out over {}", horizon_label(drawdown))}</th>
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

#[cfg(test)]
mod tests {
    use super::month_at_fraction;
    use crate::chart::{PLOT_LEFT_FRAC, PLOT_WIDTH_FRAC};

    // A 640px-wide scrubber over a 120-month timeline. The plot occupies the
    // middle band; the y-axis gutter on the left is dead space.
    const W: f64 = 640.0;
    const SPAN: u32 = 120;

    #[test]
    fn the_plot_left_edge_is_month_zero() {
        // x at the start of the drawn plot (past the gutter) reads as month 0,
        // not a negative month.
        let x = PLOT_LEFT_FRAC * W;
        assert_eq!(month_at_fraction(x, W, SPAN), 0);
    }

    #[test]
    fn the_plot_right_edge_is_the_final_month() {
        let x = (PLOT_LEFT_FRAC + PLOT_WIDTH_FRAC) * W;
        assert_eq!(month_at_fraction(x, W, SPAN), SPAN as usize);
    }

    #[test]
    fn the_plot_midpoint_is_the_middle_month() {
        let x = (PLOT_LEFT_FRAC + PLOT_WIDTH_FRAC / 2.0) * W;
        assert_eq!(month_at_fraction(x, W, SPAN), 60);
    }

    #[test]
    fn positions_outside_the_plot_clamp_to_the_ends() {
        // Anywhere in the left gutter clamps to month 0; past the right edge to span.
        assert_eq!(month_at_fraction(0.0, W, SPAN), 0);
        assert_eq!(month_at_fraction(W, W, SPAN), SPAN as usize);
        assert_eq!(month_at_fraction(-50.0, W, SPAN), 0);
        assert_eq!(month_at_fraction(W * 2.0, W, SPAN), SPAN as usize);
    }
}
