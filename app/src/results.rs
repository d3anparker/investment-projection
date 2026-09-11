//! The "Breakdown" panel: the interactive value chart (with keyboard-reachable
//! scrubber) and the per-holding table.

use crate::chart::{chart_svg, Layout};
use crate::format::{fmt_money, fmt_pct, horizon_label, month_label};
use crate::panel::stale_body;
use calc::CalcOutput;
use leptos::*;
use wasm_bindgen::prelude::*;

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
/// first — so `x/width` is shifted by the layout's [`Layout::left_frac`] and
/// scaled by its [`Layout::width_frac`] before landing on `0..=span`. Pulled
/// out of the view closure so this arithmetic is unit-testable without a
/// browser.
fn month_at_fraction(x: f64, width: f64, span: u32, layout: &Layout) -> usize {
    let frac = ((x / width) - layout.left_frac()) / layout.width_frac();
    (frac.clamp(0.0, 1.0) * span as f64).round() as usize
}

/// Watch `stage`'s rendered width so the chart can be drawn at exactly that
/// width (one viewBox unit per pixel — see [`Layout::for_width`]). Seeds
/// `width` from the current box at once, then a `ResizeObserver` keeps it
/// current across column changes and rotation. Returns the observer so the
/// caller can keep it (and its callback) alive and disconnect it on cleanup.
fn observe_width(
    stage: &web_sys::Element,
    width: RwSignal<f64>,
) -> Option<(web_sys::ResizeObserver, Closure<dyn FnMut(js_sys::Array)>)> {
    width.set(stage.client_width() as f64);
    let cb = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
        let Ok(entry) = entries.get(0).dyn_into::<web_sys::ResizeObserverEntry>() else {
            return;
        };
        let w = entry.content_rect().width();
        // Only a real change re-renders the SVG; the observer's first delivery
        // repeats the seeded width.
        if w > 0.0 && (w - width.get_untracked()).abs() > 0.5 {
            width.set(w);
        }
    });
    let observer = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()).ok()?;
    observer.observe(stage);
    Some((observer, cb))
}

fn results_view(out: &CalcOutput) -> impl IntoView {
    let horizon = out.horizon_months; // the growth period (also the handover index)
    let drawdown = out.drawdown_months;
    let span = out.total_months; // the whole timeline the chart and scrubber cover
    let drawing = out.handover_total.is_some();
    // A drawdown with no growth phase hands over at month zero: there is no
    // divider to draw, no "at start of drawdown" column worth its width (it would
    // repeat "value today"), and the growth clause drops out of the wording. Keyed
    // on the figure, not the mode, so a zero typed into the growth box reads right.
    let has_handover = drawing && horizon > 0;
    let from_today = drawing && horizon == 0;
    let handover = has_handover.then_some(horizon);
    let has_contributions = !out.contributed_total.is_zero();
    let has_withdrawals = !out.withdrawn_total.is_zero();
    // Tax columns only when tax was actually charged, so an untaxed or pro-rata
    // projection keeps the table it always had.
    let has_tax = !out.tax_paid_total.is_zero();
    // A periodic holding charge (Germany's Vorabpauschale) gets its own column
    // only when one was levied, so a UK or untaxed run keeps its existing table.
    let has_charge = !out.charged_total.is_zero();
    // The account column is worth its width only once accounts differ; a
    // portfolio all in one wrapper says nothing by repeating it on every row.
    // Only while drawing down, matching the editor: in deposits mode the account
    // picker is hidden because it changes nothing, so a column naming accounts
    // the reader cannot see or change would be a puzzle rather than information.
    let has_accounts = drawing
        && out
            .investments
            .windows(2)
            .any(|w| w[0].account_kind != w[1].account_kind);
    // Resolved through the catalogue, never matched against a named wrapper.
    let account_label = |id: &str| {
        crate::convert::active_system()
            .account_kind(id)
            .map_or_else(String::new, |k| k.label.to_string())
    };

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

    // --- chart geometry ----------------------------------------------------
    // The SVG is drawn at the stage's measured width (0 = not yet measured, so
    // the default layout paints first), which is what keeps the axis type one
    // fixed size on a phone and a desktop alike and lets the chart fit its
    // column without a sideways scroll — a scroll that swallowed touch drags.
    let width = create_rw_signal(0.0_f64);
    let layout = create_memo(move |_| Layout::for_width(width.get()));
    let stage_ref = create_node_ref::<html::Div>();
    let watcher = store_value(None);
    stage_ref.on_load(move |el| {
        watcher.set_value(observe_width(&el, width));
    });
    on_cleanup(move || {
        watcher.update_value(|w| {
            if let Some((observer, _)) = w.take() {
                observer.disconnect();
            }
        });
    });
    // Rebuilt only when the width changes: the scrub overlay is a sibling
    // node, so a resize never disturbs the reader's month or focus.
    let svg = move || {
        let l = layout.get();
        series.with_value(|s| contribs.with_value(|c| chart_svg(s, c, handover, &l)))
    };

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
        cursor.set(month_at_fraction(x, width, span, &layout.get_untracked()));
    };
    let read_at = move |ev: &ev::PointerEvent| {
        active.set(true);
        set_from_x(ev.offset_x() as f64);
    };
    // A finger covers the marker while it is down, so the reading has to
    // outlive the lift: `pointerleave` fires on lift for touch, and clearing
    // there would wipe the answer at the moment it became visible. Touch
    // readings clear on blur instead — the tap focuses the slider so that a
    // later tap elsewhere produces one. Mouse and pen still clear on leave.
    let on_pointer_down = move |ev: ev::PointerEvent| {
        if ev.pointer_type() == "touch" {
            if let Some(el) = scrub_ref.get_untracked() {
                let _ = el.focus();
            }
        }
        read_at(&ev);
    };
    let on_pointer_leave = move |ev: ev::PointerEvent| {
        if ev.pointer_type() != "touch" {
            active.set(false);
        }
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
        let l = layout.get();
        format!(
            "left: {:.4}%; top: {:.4}%; bottom: {:.4}%",
            (l.left_frac() + at * l.width_frac()) * 100.0,
            l.top_frac() * 100.0,
            l.bottom_frac() * 100.0
        )
    };
    let chart_label = if from_today {
        format!(
            "Line chart of projected portfolio value, drawn down from {} today to {} over {}.",
            fmt_money(out.current_total),
            fmt_money(out.projected_total),
            horizon_label(drawdown),
        )
    } else if drawing {
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
            let handover_cell = has_handover.then(|| {
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
            // Tax and what was kept sit next to the gross figure they split, so
            // the three read across as one sum rather than as three statistics.
            // A row that took nothing out has no split to show, so both cells
            // read as absent rather than as two zeroes.
            let split = |v| {
                if r.withdrawn.is_zero() {
                    "\u{2014}".to_string()
                } else {
                    fmt_money(v)
                }
            };
            let tax_cell = has_tax.then(|| view! { <td class="num">{split(r.tax_paid)}</td> });
            let net_cell = has_tax.then(|| view! { <td class="num">{split(r.net_withdrawn)}</td> });
            let charged_cell = has_charge.then(|| {
                let cell = if r.charged.is_zero() {
                    "\u{2014}".to_string()
                } else {
                    fmt_money(r.charged)
                };
                view! { <td class="num">{cell}</td> }
            });
            let account_cell =
                has_accounts.then(|| view! { <td>{account_label(&r.account_kind)}</td> });
            view! {
                <tr>
                    <td>{r.name.clone()}</td>
                    {account_cell}
                    <td class="num">{fmt_money(r.current_value)}</td>
                    {contributed}
                    {handover_cell}
                    {withdrawn}
                    {tax_cell}
                    {net_cell}
                    {charged_cell}
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
    let caption_label = if from_today {
        format!("Portfolio value over {} of drawdown, starting today.", horizon_label(drawdown))
    } else if drawing {
        format!("Portfolio value over {} \u{2014} {} of growth, then {} of drawdown.",
            horizon_label(span), horizon_label(horizon), horizon_label(drawdown))
    } else if has_contributions {
        format!("Portfolio value and cumulative contributions from today to {} ahead.", horizon_label(horizon))
    } else {
        format!("Portfolio value from today to {} ahead.", horizon_label(horizon))
    };

    view! {
        <figure class="chart-figure">
            <div class="chart-stage" node_ref=stage_ref>
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
                     on:pointerdown=on_pointer_down
                     on:pointermove=move |ev| read_at(&ev)
                     on:pointerleave=on_pointer_leave
                     on:focus=move |_| active.set(true)
                     on:blur=move |_| active.set(false)
                     on:keydown=on_key></div>
            </div>
            // Visual mirror of `aria-valuetext`. Hidden from the tree so the
            // slider announces the value once, not twice.
            <div class="chart-readout" aria-hidden="true">
                {move || if active.get() {
                    readout()
                } else {
                    "Point at or tap the chart, or focus it and use the arrow keys, to read any month.".to_string()
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
                        {has_accounts.then(|| view! { <th scope="col">"Account"</th> })}
                        <th scope="col">"Value today"</th>
                        {has_contributions.then(|| view! {
                            <th scope="col">{format!("Deposits over {}", horizon_label(horizon))}</th>
                        })}
                        {has_handover.then(|| view! {
                            <th scope="col">"At start of drawdown"</th>
                        })}
                        {has_withdrawals.then(|| view! {
                            <th scope="col">{format!("Taken out over {}", horizon_label(drawdown))}</th>
                        })}
                        {has_tax.then(|| view! { <th scope="col">"Tax"</th> })}
                        {has_tax.then(|| view! { <th scope="col">"Kept"</th> })}
                        {has_charge.then(|| view! { <th scope="col">"Charged while invested"</th> })}
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
    use crate::chart::Layout;

    // A 120-month timeline. The plot occupies the middle band; the y-axis
    // gutter on the left is dead space. Checked at the desktop default and at
    // a phone width, since the gutter is a fixed number of pixels and so a
    // larger *fraction* of a narrow chart.
    const WIDTHS: [f64; 2] = [640.0, 300.0];
    const SPAN: u32 = 120;

    #[test]
    fn the_plot_left_edge_is_month_zero() {
        // x at the start of the drawn plot (past the gutter) reads as month 0,
        // not a negative month.
        for w in WIDTHS {
            let l = Layout::for_width(w);
            let x = l.left_frac() * w;
            assert_eq!(month_at_fraction(x, w, SPAN, &l), 0, "width {w}");
        }
    }

    #[test]
    fn the_plot_right_edge_is_the_final_month() {
        for w in WIDTHS {
            let l = Layout::for_width(w);
            let x = (l.left_frac() + l.width_frac()) * w;
            assert_eq!(month_at_fraction(x, w, SPAN, &l), SPAN as usize, "width {w}");
        }
    }

    #[test]
    fn the_plot_midpoint_is_the_middle_month() {
        for w in WIDTHS {
            let l = Layout::for_width(w);
            let x = (l.left_frac() + l.width_frac() / 2.0) * w;
            assert_eq!(month_at_fraction(x, w, SPAN, &l), 60, "width {w}");
        }
    }

    #[test]
    fn positions_outside_the_plot_clamp_to_the_ends() {
        // Anywhere in the left gutter clamps to month 0; past the right edge to span.
        let w = 640.0;
        let l = Layout::for_width(w);
        assert_eq!(month_at_fraction(0.0, w, SPAN, &l), 0);
        assert_eq!(month_at_fraction(w, w, SPAN, &l), SPAN as usize);
        assert_eq!(month_at_fraction(-50.0, w, SPAN, &l), 0);
        assert_eq!(month_at_fraction(w * 2.0, w, SPAN, &l), SPAN as usize);
    }
}
