//! The portfolio value chart: a hand-built inline SVG string. Kept dependency
//! free (no charting library) and self-contained. Colours reference the site's
//! CSS custom properties, which resolve because the SVG is injected into the
//! live DOM.

use crate::format::{currency, group_thousands};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// Axis and legend type size. The viewBox is sized to the chart's *measured*
/// rendered width ([`Layout::for_width`]), so one viewBox unit is one CSS pixel
/// and this is literally the on-screen size at every width — a phone and a
/// desktop column get the same 13px type. (It used to be 22 units on a fixed
/// 640-unit viewBox, which made the type a function of the rendered width and
/// forced a `min-width` on the chart that overflowed every phone; sideways
/// scrolling then swallowed touch drags before the scrubber saw them.) The
/// gutters in [`Layout`] are sized against it.
const AXIS_FONT: f64 = 13.0;

/// The viewBox width used before the stage has been measured (first paint,
/// and the native tests, which have no layout). Also the widest the chart is
/// allowed to render, via `.chart-stage { max-width }` in `styles.css`.
pub const DEFAULT_WIDTH: f64 = 640.0;

/// Narrower than this and the gutters would eat the plot; the stage is never
/// this narrow in practice (a 320px phone still leaves ~230px inside the panel).
const MIN_WIDTH: f64 = 240.0;

/// viewBox geometry for one rendered width. Shared with the interactive overlay
/// (as fractions, below) so the scrub marker lines up with the plotted line
/// instead of re-deriving these numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// viewBox width — equal to the rendered width, so units are pixels.
    pub w: f64,
    pub h: f64,
    /// Left gutter: holds the widest right-anchored y-label plus an 8-unit gap.
    pub pl: f64,
    /// Right gutter: the right half of the final x-label ("120m").
    pub pr: f64,
    pub pt: f64,
    pub pb: f64,
}

impl Layout {
    /// The layout for a chart rendered `width_px` wide. A non-positive width
    /// (unmeasured) falls back to [`DEFAULT_WIDTH`]; the height follows the
    /// width so a phone chart is shorter, capped at the desktop proportion.
    pub fn for_width(width_px: f64) -> Layout {
        let w = if width_px > 0.0 && width_px.is_finite() {
            width_px.max(MIN_WIDTH)
        } else {
            DEFAULT_WIDTH
        };
        // Gutters in units of `AXIS_FONT`: `£999,999` is ~4.4em wide, plus the
        // 8-unit gap before the axis; the right gutter holds half of "120m".
        let pl = (AXIS_FONT * 4.5 + 8.0).round();
        let pr = (AXIS_FONT * 1.2).round();
        let pt = (AXIS_FONT * 0.8).round();
        let pb = (AXIS_FONT * 1.7).round();
        let h = (w * 300.0 / 640.0).clamp(180.0, 300.0).round();
        Layout { w, h, pl, pr, pt, pb }
    }

    /// Where the plot starts, as a fraction of the chart box's width.
    pub fn left_frac(&self) -> f64 {
        self.pl / self.w
    }

    /// How wide the plot runs, as a fraction of the chart box's width.
    pub fn width_frac(&self) -> f64 {
        (self.w - self.pl - self.pr) / self.w
    }

    /// The top gutter as a fraction of the chart box's height.
    pub fn top_frac(&self) -> f64 {
        self.pt / self.h
    }

    /// The bottom gutter as a fraction of the chart box's height.
    pub fn bottom_frac(&self) -> f64 {
        self.pb / self.h
    }
}

/// Compact currency label for the y-axis (no decimals): `£12,000`, abbreviated
/// past a million to `£1.5M`. The abbreviation is what bounds the label's
/// width: these are right-anchored into a fixed left gutter, so an unabbreviated
/// `£12,345,678` would run off the left edge of the viewBox.
fn fmt_axis(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    // Round before the threshold test so 999,999.6 abbreviates to "£1M" rather
    // than widening to "£1,000,000".
    let a = v.abs().round();
    for &(unit, suffix) in &[(1e12, "T"), (1e9, "B"), (1e6, "M")] {
        if a >= unit {
            let mut s = format!("{:.1}", (a / unit * 10.0).round() / 10.0);
            if let Some(trimmed) = s.strip_suffix(".0") {
                s = trimmed.to_string();
            }
            return format!("{sign}{}{s}{suffix}", currency());
        }
    }
    format!("{sign}{}{}", currency(), group_thousands(&(a as i64).to_string()))
}

/// A "nice" step (1, 2, 3, 5, …) giving roughly six or fewer intervals over `total`.
fn nice_step(total: u32) -> u32 {
    let raw = (total as f64 / 6.0).max(1.0);
    for &s in &[1u32, 2, 3, 5, 10, 15, 20, 25, 50, 100] {
        if s as f64 >= raw {
            return s;
        }
    }
    ((raw / 100.0).ceil() as u32) * 100
}

/// Evenly spaced month tick positions from 0 to `total` (inclusive).
fn month_ticks(total: u32) -> Vec<u32> {
    let step = nice_step(total.max(1));
    let mut v = Vec::new();
    let mut m = 0;
    while m < total {
        v.push(m);
        m += step;
    }
    v.push(total);
    v
}

/// Evenly spaced year tick positions (expressed in months) from 0 to `years` (inclusive).
fn year_ticks(years: u32) -> Vec<u32> {
    let step = nice_step(years);
    let mut v = Vec::new();
    let mut y = 0;
    while y < years {
        v.push(y * 12);
        y += step;
    }
    v.push(years * 12);
    v
}

/// Build a `"x,y x,y …"` polyline point string for `vals` under the given
/// scale closures.
fn polyline_points(vals: &[f64], x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64) -> String {
    vals.iter()
        .enumerate()
        .map(|(i, &v)| format!("{:.1},{:.1}", x(i as f64), y(v)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render the projection as an SVG string: `series` is the portfolio value at
/// month 0..=total, `contributions` the cumulative amount deposited by each of
/// those months (parallel to `series`). The contributions line is drawn only when
/// there are actual top-ups. `handover`, when set, is the month the drawdown phase
/// begins (an index into `series`); a dashed divider marks it and its month joins
/// the axis ticks. `layout` is the viewBox geometry for the width the chart is
/// rendered at (see [`Layout::for_width`]). Returns an empty string for an
/// empty series.
pub fn chart_svg(
    series: &[Decimal],
    contributions: &[Decimal],
    handover: Option<u32>,
    layout: &Layout,
) -> String {
    let vals: Vec<f64> = series.iter().map(|d| d.to_f64().unwrap_or(0.0)).collect();
    if vals.is_empty() {
        return String::new();
    }
    let contrib: Vec<f64> = contributions.iter().map(|d| d.to_f64().unwrap_or(0.0)).collect();
    // Only plot contributions when they line up with the value series and any of
    // them are non-zero.
    let show_contrib = contrib.len() == vals.len() && contrib.iter().any(|&c| c > 0.0);

    let Layout { w, h, pl, pr, pt, pb } = *layout;
    let plot_w = w - pl - pr;
    let plot_h = h - pt - pb;

    let max_m = series.len().saturating_sub(1).max(1) as f64;
    // Scale to whichever line reaches furthest; contributions can top the value
    // line if returns are negative, so fold both into the max.
    let max_v = vals
        .iter()
        .chain(if show_contrib { contrib.iter() } else { [].iter() })
        .cloned()
        .fold(f64::MIN, f64::max);
    // The baseline is always £0: every series is a sum of non-negative balances
    // (values non-negative, rates > -100%, withdrawals capped at the pot), so the
    // minimum is never below zero — and anchoring at £0 is what makes a drawdown
    // to nothing read as reaching the floor.
    let min_v = 0.0_f64;
    let span = if (max_v - min_v).abs() < 1e-9 { 1.0 } else { max_v - min_v };

    let x = |m: f64| pl + (m / max_m) * plot_w;
    let y = |v: f64| pt + plot_h - ((v - min_v) / span) * plot_h;

    let line = polyline_points(&vals, x, y);
    let area = format!("{:.1},{:.1} {} {:.1},{:.1}", pl, pt + plot_h, line, pl + plot_w, pt + plot_h);
    let contrib_line = if show_contrib {
        polyline_points(&contrib, x, y)
    } else {
        String::new()
    };

    let mut grid = String::new();
    let y_ticks = 4;
    for i in 0..=y_ticks {
        let v = min_v + span * (i as f64) / (y_ticks as f64);
        let yy = y(v);
        grid.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"grid\"/>",
            pl, yy, pl + plot_w, yy
        ));
        grid.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"y-lbl\" text-anchor=\"end\" dominant-baseline=\"middle\">{}</text>",
            pl - 8.0, yy, fmt_axis(v)
        ));
    }

    // X-axis ticks: a clean, evenly spaced set. When the horizon is a whole number
    // of years (>= 2) step in whole years so every label reads "Ny"; otherwise step
    // in months. This avoids the ugly mixed "20m / 5y / 80m" scale.
    let total_m = max_m.round() as u32;
    // A handover only draws when it falls strictly inside the plotted span; pin the
    // boundary rule once so the tick and divider sites can't drift apart.
    let handover = handover.filter(|&h| h > 0 && h < total_m);
    let mut ticks = if total_m >= 24 && total_m % 12 == 0 {
        year_ticks(total_m / 12)
    } else {
        month_ticks(total_m)
    };
    // Make sure the handover month is readable off the axis: replace the *nearest*
    // regular tick with it rather than adding one, so labels can't collide.
    if let Some(h) = handover {
        // Only interior ticks are candidates: overwriting the first or last would
        // drop the "0" origin or the final-month label, which the handover — being
        // strictly inside the span — can never stand in for.
        let last = ticks.len() - 1;
        if let Some((idx, _)) = ticks
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != 0 && i != last)
            .min_by_key(|(_, &t)| (t as i64 - h as i64).abs())
        {
            ticks[idx] = h;
        }
        ticks.sort_unstable();
        ticks.dedup();
    }
    let mut x_labels = String::new();
    for &mo in &ticks {
        // Per-tick unit so a whole-year run reads "Ny" while a stray handover tick
        // that is not a round year still labels cleanly in months.
        let label = if mo == 0 {
            "0".to_string()
        } else if mo % 12 == 0 {
            format!("{}y", mo / 12)
        } else {
            format!("{}m", mo)
        };
        x_labels.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"x-lbl\" text-anchor=\"middle\">{}</text>",
            x(mo as f64), h - 8.0, label
        ));
    }

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {} {}\" xmlns=\"http://www.w3.org/2000/svg\" preserveAspectRatio=\"xMidYMid meet\">",
        w, h
    ));
    svg.push_str(
        "<defs><linearGradient id=\"fill\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop class=\"g0\" offset=\"0%\"/><stop class=\"g1\" offset=\"100%\"/></linearGradient></defs>",
    );
    svg.push_str(&format!(
        "<style>\
         .g0{{stop-color:var(--accent);stop-opacity:0.35}}\
         .g1{{stop-color:var(--accent);stop-opacity:0}}\
         .grid{{stroke:var(--line);stroke-width:1}}\
         .y-lbl,.x-lbl,.lgnd{{fill:var(--muted-strong);font-size:{AXIS_FONT}px;font-family:system-ui,sans-serif}}\
         .line{{fill:none;stroke:var(--accent);stroke-width:2.5;stroke-linejoin:round}}\
         .cline{{fill:none;stroke:var(--good);stroke-width:2;stroke-dasharray:5 4;stroke-linejoin:round}}\
         .phase{{fill:none;stroke:var(--muted-strong);stroke-width:1.5;stroke-dasharray:4 4;opacity:0.7}}\
         .phase-lbl{{fill:var(--muted-strong);font-size:{AXIS_FONT}px;font-family:system-ui,sans-serif}}\
         </style>"
    ));
    svg.push_str(&grid);
    svg.push_str(&format!("<polygon points=\"{}\" fill=\"url(#fill)\"/>", area));
    svg.push_str(&format!("<polyline points=\"{}\" class=\"line\"/>", line));
    if show_contrib {
        svg.push_str(&format!("<polyline points=\"{}\" class=\"cline\"/>", contrib_line));
    }
    svg.push_str(&x_labels);
    // The handover divider: a dashed vertical rule where drawdown begins, with a
    // short label near the axis (kept to one word so it can't outgrow the plot).
    if let Some(h) = handover {
        let hx = x(h as f64);
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"phase\"/>",
            hx, pt, hx, pt + plot_h
        ));
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"phase-lbl\" text-anchor=\"start\">drawdown</text>",
            hx + 5.0, pt + plot_h - 6.0
        ));
    }
    // Legend, top-left of the plot, only when both lines are present.
    if show_contrib {
        let lx = pl + 6.0;
        let ly = pt + AXIS_FONT / 2.0;
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"line\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" class=\"lgnd\" dominant-baseline=\"middle\">Projected value</text>",
            lx, ly, lx + 16.0, ly, lx + 20.0, ly
        ));
        // Clear the type with a little air; rows one em apart nearly touched.
        let ly2 = ly + (AXIS_FONT * 1.2).round();
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"cline\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" class=\"lgnd\" dominant-baseline=\"middle\">Contributions</text>",
            lx, ly2, lx + 16.0, ly2, lx + 20.0, ly2
        ));
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn series(values: &[&str]) -> Vec<Decimal> {
        values.iter().map(|s| Decimal::from_str(s).unwrap()).collect()
    }

    /// The unmeasured (desktop-default) layout most tests draw with.
    fn l() -> Layout {
        Layout::for_width(DEFAULT_WIDTH)
    }

    #[test]
    fn the_viewbox_is_the_rendered_width_so_units_are_pixels() {
        let s = series(&["100", "150", "225"]);
        let z = series(&["0", "0", "0"]);
        let phone = chart_svg(&s, &z, None, &Layout::for_width(300.0));
        assert!(phone.contains("viewBox=\"0 0 300 "), "{phone}");
        let desk = chart_svg(&s, &z, None, &Layout::for_width(640.0));
        assert!(desk.contains("viewBox=\"0 0 640 300\""), "{desk}");
        // The type is the same size at both — it no longer scales with width.
        let font = format!("font-size:{AXIS_FONT}px");
        assert!(phone.contains(&font) && desk.contains(&font));
        // A phone chart is shorter, but never squashed below the floor.
        assert!(Layout::for_width(300.0).h < 300.0);
        assert!(Layout::for_width(240.0).h >= 180.0);
    }

    #[test]
    fn an_unmeasured_width_falls_back_to_the_default() {
        for w in [0.0, -5.0, f64::NAN] {
            assert_eq!(Layout::for_width(w), Layout::for_width(DEFAULT_WIDTH), "width {w}");
        }
        // Below the floor the layout stops shrinking rather than losing the plot.
        assert_eq!(Layout::for_width(10.0).w, 240.0);
        let l = Layout::for_width(240.0);
        assert!(l.w - l.pl - l.pr > 100.0, "plot too narrow at the floor: {l:?}");
    }

    #[test]
    fn the_plot_fractions_describe_the_gutters() {
        let l = Layout::for_width(500.0);
        assert!((l.left_frac() * 500.0 - l.pl).abs() < 1e-9);
        assert!((l.width_frac() * 500.0 - (500.0 - l.pl - l.pr)).abs() < 1e-9);
        assert!((l.top_frac() * l.h - l.pt).abs() < 1e-9);
        assert!((l.bottom_frac() * l.h - l.pb).abs() < 1e-9);
    }

    #[test]
    fn empty_series_yields_empty_string() {
        assert_eq!(chart_svg(&[], &[], None, &l()), "");
    }

    #[test]
    fn renders_well_formed_svg() {
        let svg = chart_svg(&series(&["100", "150", "225"]), &series(&["0", "0", "0"]), None, &l());
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("class=\"line\""));
        // Colours are referenced via CSS custom properties, not hard-coded hex
        // (the only `#` is the gradient's `url(#fill)` reference).
        assert!(svg.contains("var(--accent)"));
        assert!(!svg.contains(":#"));
    }

    #[test]
    fn flat_series_does_not_divide_by_zero() {
        // Equal values give a zero span; must not produce NaN coordinates.
        let svg = chart_svg(&series(&["500", "500", "500"]), &series(&["0", "0", "0"]), None, &l());
        assert!(!svg.contains("NaN"));
    }

    #[test]
    fn contributions_line_drawn_only_when_non_zero() {
        // No top-ups: single value line, no contributions line or legend.
        let none = chart_svg(&series(&["100", "150", "225"]), &series(&["0", "0", "0"]), None, &l());
        assert!(!none.contains("class=\"cline\""));
        assert!(!none.contains("Contributions"));

        // With top-ups: the dashed contributions line and legend appear.
        let with = chart_svg(&series(&["100", "150", "225"]), &series(&["0", "50", "100"]), None, &l());
        assert!(with.contains("class=\"cline\""));
        assert!(with.contains("var(--good)"));
        assert!(with.contains("Contributions"));
        assert!(with.contains("Projected value"));
    }

    #[test]
    fn mismatched_contributions_length_is_ignored() {
        // A contributions slice that doesn't parallel the value series is skipped
        // rather than mis-plotted.
        let svg = chart_svg(&series(&["100", "150", "225"]), &series(&["0", "50"]), None, &l());
        assert!(!svg.contains("class=\"cline\""));
    }

    #[test]
    fn handover_divider_drawn_only_in_drawdown() {
        let flat = series(&["100", "110", "120", "115", "108"]);
        let zeros = series(&["0", "0", "0", "0", "0"]);
        // No handover: no phase divider.
        assert!(!chart_svg(&flat, &zeros, None, &l()).contains("class=\"phase\""));
        // A handover mid-series draws the dashed divider and its label.
        let dd = chart_svg(&flat, &zeros, Some(2), &l());
        assert!(dd.contains("class=\"phase\""));
        assert!(dd.contains("drawdown"));
        assert!(dd.contains("var(--muted-strong)"));
    }

    #[test]
    fn handover_at_the_series_ends_is_not_drawn() {
        let flat = series(&["100", "110", "120"]);
        let zeros = series(&["0", "0", "0"]);
        // 0 and the final index are degenerate: no divider, no NaN, still valid.
        for h in [0u32, 2] {
            let svg = chart_svg(&flat, &zeros, Some(h), &l());
            assert!(!svg.contains("class=\"phase\""), "handover {h} should not draw a divider");
            assert!(!svg.contains("NaN"));
            assert!(svg.starts_with("<svg"));
        }
    }

    #[test]
    fn a_drawdown_to_zero_renders_without_nan() {
        // A pot drawn all the way to £0 must still plot (baseline is £0).
        let svg = chart_svg(&series(&["1000", "1200", "600", "0"]), &series(&["0", "0", "0", "0"]), Some(1), &l());
        assert!(!svg.contains("NaN"));
        assert!(svg.contains("class=\"phase\""));
    }

    #[test]
    fn axis_label_is_compact_currency() {
        assert_eq!(fmt_axis(12000.0), "\u{00a3}12,000");
        assert_eq!(fmt_axis(0.0), "\u{00a3}0");
        assert_eq!(fmt_axis(-2500.0), "-\u{00a3}2,500");
    }

    #[test]
    fn axis_label_abbreviates_past_a_million() {
        // Unabbreviated these would overrun the left gutter they're anchored in.
        assert_eq!(fmt_axis(1_500_000.0), "\u{00a3}1.5M");
        assert_eq!(fmt_axis(2_000_000.0), "\u{00a3}2M");
        assert_eq!(fmt_axis(12_340_000.0), "\u{00a3}12.3M");
        assert_eq!(fmt_axis(4_200_000_000.0), "\u{00a3}4.2B");
        assert_eq!(fmt_axis(3_000_000_000_000.0), "\u{00a3}3T");
        // Just under the threshold stays grouped, and rounding up to a million
        // crosses into the abbreviation rather than widening to "£1,000,000".
        assert_eq!(fmt_axis(999_499.0), "\u{00a3}999,499");
        assert_eq!(fmt_axis(999_999.6), "\u{00a3}1M");
    }

    #[test]
    fn axis_labels_fit_inside_the_left_gutter() {
        // The labels are right-anchored at x = pl - 8, so a label wider than
        // that runs off the viewBox. Approximate advance widths for system-ui:
        // digits/£ ~0.55em, separators ~0.28em, M/B/T ~0.85em.
        let width = |s: &str| -> f64 {
            s.chars()
                .map(|c| match c {
                    ',' | '.' | '-' => 0.28,
                    'M' | 'B' | 'T' => 0.85,
                    _ => 0.55,
                })
                .sum::<f64>()
                * AXIS_FONT
        };
        // The gutter is the same at every width — it is sized to the type, not
        // the chart — so the narrowest layout is as good a check as the default.
        for w in [240.0, DEFAULT_WIDTH] {
            let gutter = Layout::for_width(w).pl - 8.0;
            for v in [0.0, 999_999.0, 62_882.0, 12_345_678.0, 4.2e9, 3.0e12] {
                let label = fmt_axis(v);
                assert!(
                    width(&label) <= gutter,
                    "{label} is ~{:.0} units wide, gutter is {gutter}",
                    width(&label)
                );
            }
        }
    }

    #[test]
    fn year_ticks_are_whole_years_no_month_mix() {
        // 10 years must land on clean 2-year boundaries, not "20m / 5y / 80m".
        assert_eq!(year_ticks(10), vec![0, 24, 48, 72, 96, 120]);
        // Short horizons still start at 0 and reach the end exactly.
        assert_eq!(*year_ticks(3).first().unwrap(), 0);
        assert_eq!(*year_ticks(3).last().unwrap(), 36);
    }

    #[test]
    fn month_ticks_span_zero_to_total_inclusive() {
        let t = month_ticks(30);
        assert_eq!(*t.first().unwrap(), 0);
        assert_eq!(*t.last().unwrap(), 30);
        // Strictly increasing (no duplicate final tick).
        assert!(t.windows(2).all(|w| w[0] < w[1]));
    }
}
