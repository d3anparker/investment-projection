//! A walker for the §32a income-tax tariff — the one thing `taxkit::ladder`
//! cannot do.
//!
//! `taxkit::ladder` walks a *piecewise-constant* marginal rate. §32a is
//! *piecewise-quadratic*: in its two middle zones the tax is a quadratic
//! function of income, so the marginal rate rises **continuously** rather than
//! in steps. Slicing that into constant rungs would need far more than
//! `MAX_RUNGS`, and give up exactness. So this crate carries its own walker.
//!
//! # The whole charge, as one curve
//!
//! [`Tarif`] models the *total* charge on income — §32a **plus** the
//! Solidaritätszuschlag (with its Freigrenze and Milderungszone) **plus**
//! Kirchensteuer — as one piecewise-quadratic function `C(i)` of taxable income
//! `i`. The surcharges fold in because each is, within its own stretch, a
//! constant multiple of the income tax plus a constant, and a constant multiple
//! of a quadratic is still a quadratic. The Soli Freigrenze and the top of its
//! Milderungszone become two extra segment boundaries. This is the same
//! discipline `uk-tax` applies to the withdrawn personal allowance: a
//! marginal-rate effect is *expressed* as part of the schedule, never computed
//! as a correction afterwards.
//!
//! # Grossing up is a walk, not a search
//!
//! To deliver `N` net from a pension pot you withdraw `g` gross, of which
//! `leak·g` is taxable income stacked on what is already booked. Within one
//! segment `C` is quadratic in income, so the net delivered is quadratic in the
//! income increment — met by **one quadratic root**, not an iterative solver,
//! exactly as `ladder`'s linear rungs are met by one division.
//!
//! # Precision
//!
//! Everything is base-10 `Decimal`; no floating point. The one place exactness
//! is not attainable is the quadratic root (`Decimal::sqrt`), which is accurate
//! to ~28 significant digits — orders of magnitude finer than a cent, and
//! strictly better than approximating the progression with constant rungs.
//! Compare rounded figures, never raw ones (the same caveat `ladder` carries).
//! Nothing here rounds; the caller rounds once at its own boundary.
//!
//! The statute rounds the zvE and the tax down to whole euros; we implement the
//! unrounded continuous formula deliberately, because per-assessment rounding
//! would inject step artefacts into a month-by-month projection.

use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;

use taxkit::ladder::Walk;
use taxkit::{Draw, StopAt, TaxError};

use crate::tables::{bp, TaxYear};

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// One segment of the charge curve. On `[lower, upper)`,
/// `C(i) = a·(i − lower)² + b·(i − lower) + c`.
#[derive(Clone, Copy, Debug)]
struct Seg {
    lower: Decimal,
    upper: Option<Decimal>,
    a: Decimal,
    b: Decimal,
    c: Decimal,
}

impl Seg {
    fn charge(&self, i: Decimal) -> Decimal {
        let d = i - self.lower;
        self.a * d * d + self.b * d + self.c
    }
    /// Marginal charge rate dC/di at income `i`.
    fn rate(&self, i: Decimal) -> Decimal {
        let d = i - self.lower;
        Decimal::TWO * self.a * d + self.b
    }
}

/// The segment holding income `i`: the first whose `upper` it falls below.
///
/// One function rather than the same loop written at each use, and total by
/// construction — the last segment is always open-topped (`upper: None`), so the
/// `unwrap_or` arm is unreachable rather than a guess. Used over both the §32a
/// zones while the curve is being built and the finished total-charge segments.
fn seg_at(segs: &[Seg], i: Decimal) -> &Seg {
    segs.iter()
        .find(|s| s.upper.is_none_or(|u| i < u))
        .unwrap_or(&segs[segs.len() - 1])
}

/// The total charge curve for one set of rules, region (church rate) and filing
/// status. Rebuilt once per tax period, then read per draw.
#[derive(Clone, Debug)]
pub struct Tarif {
    segs: Vec<Seg>,
}

impl Tarif {
    /// Build the curve.
    ///
    /// `kirche_bp` is the Kirchensteuer rate (0 for none). `splitting` doubles
    /// the tariff (Ehegattensplitting: `2·T(zvE/2)`) and the Soli Freigrenze.
    /// `scale` is threshold uprating ("Tarif auf Rädern": `s·T(x/s)`), `1` when
    /// frozen. The two compose into one axis factor `f`.
    pub fn build(
        rules: &TaxYear,
        kirche_bp: u32,
        splitting: bool,
        scale: Decimal,
    ) -> Result<Self, TaxError> {
        let split = if splitting { Decimal::TWO } else { Decimal::ONE };
        let f = scale * split;
        if f <= Decimal::ZERO {
            return Err(TaxError::confiscatory());
        }

        // The §32a income-tax zones, in income space, scaled by `f`. Under the
        // axis factor a zone's quadratic keeps its `b`, has `a` divided by `f`,
        // and has its thresholds and additive tax constants multiplied by `f`.
        let e0 = Decimal::from(rules.grundfreibetrag_eur) * f;
        let e1 = Decimal::from(rules.zone2_top_eur) * f;
        let e2 = Decimal::from(rules.zone3_top_eur) * f;
        let e3 = Decimal::from(rules.zone4_top_eur) * f;
        let e4 = Decimal::from(10_000i64);
        let e8 = Decimal::from(100_000_000i64);
        let r42 = bp(rules.upper_rate_bp);
        let r45 = bp(rules.top_rate_bp);

        // T-only zones (income tax before surcharges).
        let zones = [
            Seg { lower: Decimal::ZERO, upper: Some(e0), a: Decimal::ZERO, b: Decimal::ZERO, c: Decimal::ZERO },
            Seg {
                lower: e0,
                upper: Some(e1),
                a: cents(rules.zone2_a_cents) / (e8 * f),
                b: cents(rules.zone2_b_cents) / e4,
                c: Decimal::ZERO,
            },
            Seg {
                lower: e1,
                upper: Some(e2),
                a: cents(rules.zone3_a_cents) / (e8 * f),
                b: cents(rules.zone3_b_cents) / e4,
                c: cents(rules.zone3_c_cents) * f,
            },
            Seg { lower: e2, upper: Some(e3), a: Decimal::ZERO, b: r42, c: r42 * e2 - cents(rules.zone4_sub_cents) * f },
            Seg { lower: e3, upper: None, a: Decimal::ZERO, b: r45, c: r45 * e3 - cents(rules.zone5_sub_cents) * f },
        ];

        let t_eval = |i: Decimal| -> Decimal { seg_at(&zones, i).charge(i) };
        // The zone containing `l`, as a quadratic re-based to `l`: (a, b at l).
        let coeffs_at = |l: Decimal| -> (Decimal, Decimal) {
            let z = seg_at(&zones, l);
            (z.a, Decimal::TWO * z.a * (l - z.lower) + z.b)
        };

        // Solve T(i) = target for the smallest i ≥ 0 (income at a given income
        // tax), used to place the Soli breakpoints in income space.
        let income_at_tax = |target: Decimal| -> Option<Decimal> {
            for z in &zones {
                let lo_ok = z.charge(z.lower) <= target;
                let hi_ok = z.upper.map_or(true, |u| z.charge(u) >= target);
                if lo_ok && hi_ok {
                    if z.a.is_zero() {
                        if z.b.is_zero() {
                            continue;
                        }
                        return Some(z.lower + (target - z.c) / z.b);
                    }
                    // a·d² + b·d + (c − target) = 0, positive root.
                    let disc = z.b * z.b - Decimal::from(4i64) * z.a * (z.c - target);
                    let root = disc.sqrt()?;
                    return Some(z.lower + (-z.b + root) / (Decimal::TWO * z.a));
                }
            }
            None
        };

        // Soli thresholds, on the *tax*. The Freigrenze doubles for joint filing.
        let freigrenze = Decimal::from(rules.soli_freigrenze_eur) * split;
        let soli = bp(rules.soli_bp);
        let mild = bp(rules.soli_milderung_bp);
        let k = bp(kirche_bp);
        // Where the 11.9% ramp meets the full 5.5%: 0.119(T−F) = 0.055·T.
        let denom = mild - soli;
        let t_switch = if denom > Decimal::ZERO {
            Some(freigrenze * mild / denom)
        } else {
            None
        };

        // Extra income-space cut points from the Soli regime changes.
        let mut cuts: Vec<Decimal> = zones.iter().filter_map(|z| z.upper).collect();
        if freigrenze > Decimal::ZERO {
            if let Some(x) = income_at_tax(freigrenze) {
                cuts.push(x);
            }
        }
        if let Some(ts) = t_switch {
            if let Some(x) = income_at_tax(ts) {
                cuts.push(x);
            }
        }
        cuts.retain(|c| *c > Decimal::ZERO);
        cuts.sort();
        cuts.dedup();

        // Build the total-charge segments. Each is the §32a quadratic re-based to
        // the segment's lower and scaled by the segment's marginal multiplier
        // (1 for zone 1, (1+k) below the Soli Freigrenze, (1+k+11.9%) in the
        // Milderungszone, (1+k+5.5%) above it). The constant term is *carried*
        // from the previous segment's end value rather than taken from each
        // zone's own rounded constant: the published coefficients are rounded to
        // the cent and so leave sub-cent kinks between zones, which would break
        // the value-telescoping a gross-up relies on. Carrying the value makes
        // the curve continuous by construction, which also expresses the Soli
        // Milderungszone exactly — its extra 11.9% accrues only on income above
        // the Freigrenze, which is precisely `mild·(T − F)`.
        let regime_mult = |t_mid: Decimal| -> Decimal {
            let base = Decimal::ONE + k;
            match t_switch {
                _ if t_mid <= freigrenze => base,
                Some(ts) if t_mid <= ts => base + mild,
                _ => base + soli,
            }
        };
        let mut segs: Vec<Seg> = Vec::with_capacity(cuts.len() + 1);
        let mut lower = Decimal::ZERO;
        let mut carry = Decimal::ZERO; // C evaluated at `lower`
        let make = |lower: Decimal, upper: Option<Decimal>, carry: Decimal| -> (Seg, Decimal) {
            let (a, b_l) = coeffs_at(lower);
            let probe = match upper {
                Some(u) => (lower + u) / Decimal::TWO,
                None => lower + Decimal::ONE,
            };
            let m = regime_mult(t_eval(probe));
            let seg = Seg { lower, upper, a: a * m, b: b_l * m, c: carry };
            let next = match upper {
                Some(u) => {
                    let d = u - lower;
                    seg.a * d * d + seg.b * d + carry
                }
                None => carry,
            };
            (seg, next)
        };
        for cut in &cuts {
            if *cut > lower {
                let (seg, next) = make(lower, Some(*cut), carry);
                segs.push(seg);
                carry = next;
                lower = *cut;
            }
        }
        let (seg, _) = make(lower, None, carry);
        segs.push(seg);

        Ok(Tarif { segs })
    }

    /// Total charge on taxable income `i`.
    pub fn charge_at(&self, i: Decimal) -> Decimal {
        if i <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        seg_at(&self.segs, i).charge(i)
    }

    /// Marginal charge rate at income `i` (dC/di), the total including surcharges.
    pub fn marginal_rate_at(&self, i: Decimal) -> Decimal {
        let i = i.max(Decimal::ZERO);
        seg_at(&self.segs, i).rate(i)
    }

    /// Net kept on the next unit of gross withdrawn from a pot whose taxable
    /// fraction is `leak`, given `booked` income already stacked below it.
    pub fn marginal_keep(&self, booked: Decimal, leak: Decimal) -> Decimal {
        (Decimal::ONE - leak * self.marginal_rate_at(booked)).max(Decimal::ZERO)
    }

    /// Withdraw enough gross to deliver `net_wanted`, given `booked` income
    /// already present and a taxable fraction `leak` of each gross unit. Same
    /// contract as [`taxkit::ladder::Ladder::walk`]: `gross − tax == net`
    /// exactly, and it stops early on an emptied pot or a `stop`.
    pub fn walk(
        &self,
        booked: Decimal,
        leak: Decimal,
        available: Decimal,
        net_wanted: Decimal,
        stop: StopAt,
    ) -> Result<Walk, TaxError> {
        let zero = Decimal::ZERO;
        if available <= zero || net_wanted <= zero {
            return Ok(Walk::default());
        }
        // A wholly tax-free withdrawal (leak 0): gross is net, nothing taxable.
        if leak <= zero {
            let g = net_wanted.min(available);
            return Ok(Walk {
                draw: Draw { gross: g, tax: zero, net: g, rung_limited: false },
                taxable: zero,
            });
        }

        let charge0 = self.charge_at(booked);
        let i_max = booked + leak * available; // income ceiling from the pot size
        let mut i = booked;
        let mut net = zero;
        let mut rung_limited = false;

        // Iterate segments from the one holding `booked` upward.
        let start = self.segs.iter().position(|s| s.upper.map_or(true, |u| booked < u)).unwrap_or(0);
        for s in &self.segs[start..] {
            if net >= net_wanted || i >= i_max {
                break;
            }
            let mut seg_hi = s.upper.map_or(i_max, |u| u.min(i_max));

            // RateAbove: never cross into a marginal rate above the cap. The rate
            // rises within a quadratic segment, so cap `seg_hi` at the income
            // where the rate hits the cap, or bail if we are already above it.
            if let StopAt::RateAbove(cap) = stop {
                let rate_here = s.rate(i);
                if rate_here > cap {
                    rung_limited = true;
                    break;
                }
                if !s.a.is_zero() {
                    // 2a(x−lower) + b = cap  →  x = lower + (cap − b) / 2a
                    let x_cap = s.lower + (cap - s.b) / (Decimal::TWO * s.a);
                    if x_cap < seg_hi {
                        seg_hi = x_cap;
                        rung_limited = true;
                    }
                }
            }

            if seg_hi <= i {
                continue;
            }

            // net(Δ) = Δ/leak − [C(i+Δ) − C(i)], quadratic in the income
            // increment Δ = i' − i within this segment:
            //   net(Δ) = −a·Δ² + (1/leak − 2a(i−lower) − b)·Δ
            let d0 = i - s.lower;
            let lin = Decimal::ONE / leak - Decimal::TWO * s.a * d0 - s.b; // net rate at i, per gross
            if lin <= zero {
                // The marginal keep is zero or negative here: a 100%+ effective
                // rate, which a real schedule cannot reach.
                return Err(TaxError::confiscatory());
            }
            let remaining = net_wanted - net;
            let dmax = seg_hi - i;

            // Net delivered consuming the whole way to seg_hi.
            let net_full = if s.a.is_zero() {
                lin * dmax
            } else {
                -s.a * dmax * dmax + lin * dmax
            };

            if net_full >= remaining {
                // The requirement is met inside this segment: solve net(Δ)=remaining.
                let delta = if s.a.is_zero() {
                    remaining / lin
                } else {
                    // a·Δ² − lin·Δ + remaining = 0, smallest positive root.
                    let disc = lin * lin - Decimal::from(4i64) * s.a * remaining;
                    let root = disc.max(zero).sqrt().ok_or_else(TaxError::overflow)?;
                    (lin - root) / (Decimal::TWO * s.a)
                };
                i += delta;
                break;
            }

            // Consume the whole segment and carry on.
            net += net_full;
            i = seg_hi;
            if s.upper.is_some() && seg_hi < i_max {
                if matches!(stop, StopAt::NextRung) {
                    rung_limited = true;
                    break;
                }
            } else {
                // Reached the pot ceiling, not a rate boundary.
                break;
            }
        }

        let gross = (i - booked) / leak;
        let taxable = i - booked;
        let tax = self.charge_at(i) - charge0;
        Ok(Walk {
            draw: Draw { gross, tax, net: gross - tax, rung_limited },
            taxable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::LATEST;

    fn t() -> Tarif {
        // No church tax, single, frozen thresholds: the bare §32a + Soli curve.
        Tarif::build(LATEST, 0, false, Decimal::ONE).unwrap()
    }

    fn approx(a: Decimal, b: &str) {
        let b: Decimal = b.parse().unwrap();
        let diff = (a - b).abs();
        assert!(diff < Decimal::new(5, 2), "expected ~{b}, got {a}");
    }

    // --- the four transcription-guard invariants ---------------------------

    #[test]
    fn the_tariff_is_continuous_in_value_at_every_zone_boundary() {
        let ty = LATEST;
        let tt = t();
        for e in [ty.grundfreibetrag_eur, ty.zone2_top_eur, ty.zone3_top_eur] {
            let x = Decimal::from(e);
            let below = tt.charge_at(x - Decimal::ONE);
            let above = tt.charge_at(x + Decimal::ONE);
            // No jump: the value just below, at, and just above agree closely.
            assert!((above - below).abs() < Decimal::new(200, 2), "value jump at {e}");
        }
    }

    #[test]
    fn the_marginal_rate_is_continuous_at_the_zone_2_3_boundary() {
        // Both sides must read 23.97% at the top of zone 2 — the specific
        // continuity that a mismatched coefficient set (the trap in the plan)
        // breaks.
        let ty = LATEST;
        let tt = t();
        let e1 = Decimal::from(ty.zone2_top_eur);
        approx(tt.marginal_rate_at(e1 - Decimal::ONE), "0.2397");
        approx(tt.marginal_rate_at(e1 + Decimal::ONE), "0.2397");
    }

    #[test]
    fn the_entry_and_top_marginal_rates_are_14_42_and_45_percent() {
        let ty = LATEST;
        let tt = t();
        approx(tt.marginal_rate_at(Decimal::from(ty.grundfreibetrag_eur) + Decimal::ONE), "0.14");
        // Just above the top of zone 3: 42%.
        approx(tt.marginal_rate_at(Decimal::from(ty.zone3_top_eur) + Decimal::ONE), "0.42");
        // Deep in zone 5 the *income-tax* marginal is 45% (plus soli/church).
        approx(tt.charge_at(Decimal::from(400_000i64)) - tt.charge_at(Decimal::from(399_999i64)), "0.4747");
    }

    #[test]
    fn the_income_tax_marginal_rate_never_falls() {
        // The §32a marginal rate is monotone non-decreasing — the transcription
        // property. Tested below the Soli Freigrenze income (~€75k), where the
        // total charge equals the income tax: above it the *total* marginal
        // genuinely dips after the Soli Milderungszone, which is correct German
        // behaviour and not what this invariant is about.
        let tt = t();
        let mut prev = Decimal::ZERO;
        let mut x = Decimal::from(1000i64);
        while x < Decimal::from(70_000i64) {
            let r = tt.marginal_rate_at(x);
            assert!(r + Decimal::new(1, 6) >= prev, "marginal fell at {x}: {r} < {prev}");
            prev = r;
            x += Decimal::from(500i64);
        }
    }

    // --- grossing up -------------------------------------------------------

    #[test]
    fn a_zero_income_holder_pays_nothing_inside_the_grundfreibetrag() {
        let tt = t();
        // Draw £10,000 net of fully-taxable pension income with no other income:
        // it fits inside the Grundfreibetrag, so gross == net and no tax.
        let w = tt
            .walk(Decimal::ZERO, Decimal::ONE, Decimal::from(1_000_000i64), Decimal::from(10_000i64), StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw.tax.round_dp(2), Decimal::ZERO);
        assert_eq!(w.draw.gross.round_dp(2), Decimal::from(10_000i64));
    }

    #[test]
    fn gross_minus_tax_equals_net_exactly() {
        let tt = t();
        for (booked, leak, net) in [
            ("0", "1", "40000"),
            ("30000", "1", "20000"),
            ("60000", "1", "50000"),
            ("0", "0.5", "30000"),
        ] {
            let w = tt
                .walk(
                    booked.parse().unwrap(),
                    leak.parse().unwrap(),
                    Decimal::from(2_000_000i64),
                    net.parse().unwrap(),
                    StopAt::Requirement,
                )
                .unwrap();
            assert_eq!(
                w.draw.net.round_dp(2),
                (w.draw.gross - w.draw.tax).round_dp(2),
                "gross-tax must equal net"
            );
            // The requested net is delivered (to the penny).
            assert_eq!(w.draw.net.round_dp(2), net.parse::<Decimal>().unwrap());
        }
    }

    #[test]
    fn a_dry_pot_delivers_what_it_can() {
        let tt = t();
        let w = tt
            .walk(Decimal::ZERO, Decimal::ONE, Decimal::from(5_000i64), Decimal::from(100_000i64), StopAt::Requirement)
            .unwrap();
        // Only £5,000 available, inside the Grundfreibetrag: all of it, untaxed.
        assert_eq!(w.draw.gross.round_dp(2), Decimal::from(5_000i64));
        assert_eq!(w.draw.tax.round_dp(2), Decimal::ZERO);
    }

    #[test]
    fn a_rate_cap_keeps_the_draw_below_the_capped_marginal_rate() {
        let tt = t();
        // Cap the marginal at 30%: the draw must stop where dC/di reaches 0.30.
        let w = tt
            .walk(Decimal::ZERO, Decimal::ONE, Decimal::from(2_000_000i64), Decimal::from(1_000_000i64), StopAt::RateAbove(Decimal::new(30, 2)))
            .unwrap();
        assert!(w.draw.rung_limited, "the cap must bite");
        // At the stopping income the marginal rate is essentially the cap.
        let reached = w.taxable; // booked 0, leak 1 → income == taxable
        assert!(tt.marginal_rate_at(reached) <= Decimal::new(3001, 4));
    }

    // --- splitting and uprating -------------------------------------------

    #[test]
    fn joint_assessment_costs_twice_the_tariff_at_half_the_income() {
        let single = t();
        let joint = Tarif::build(LATEST, 0, true, Decimal::ONE).unwrap();
        // C_joint(x) == 2 · C_single(x/2).
        for x in ["40000", "120000", "300000"] {
            let x: Decimal = x.parse().unwrap();
            let lhs = joint.charge_at(x);
            let rhs = Decimal::TWO * single.charge_at(x / Decimal::TWO);
            assert!((lhs - rhs).abs() < Decimal::new(2, 2), "splitting at {x}: {lhs} vs {rhs}");
        }
    }

    #[test]
    fn uprating_stretches_the_tariff_onto_wheels() {
        // "Tarif auf Rädern": at a 10% uprate, an income 10% higher pays exactly
        // 10% more tax.
        // Below the Soli Freigrenze income, where the charge is pure §32a: the
        // Soli Freigrenze is a fixed policy amount that Tarif-auf-Rädern does not
        // uprate, so the invariance is about the tariff proper.
        let frozen = t();
        let up = Tarif::build(LATEST, 0, false, Decimal::new(11, 1)).unwrap(); // 1.1
        for base in ["20000", "50000"] {
            let base: Decimal = base.parse().unwrap();
            let lhs = up.charge_at(base * Decimal::new(11, 1));
            let rhs = frozen.charge_at(base) * Decimal::new(11, 1);
            assert!((lhs - rhs).abs() < Decimal::new(5, 2), "uprating at {base}: {lhs} vs {rhs}");
        }
    }

    #[test]
    fn church_tax_adds_a_constant_fraction_of_the_income_tax() {
        let plain = t();
        let ks9 = Tarif::build(LATEST, 900, false, Decimal::ONE).unwrap();
        // Below the Soli Freigrenze there is no soli, so ks9 == plain·1.09.
        let x = Decimal::from(40_000i64);
        let lhs = ks9.charge_at(x);
        let rhs = plain.charge_at(x) * Decimal::new(109, 2);
        assert!((lhs - rhs).abs() < Decimal::new(2, 2), "church: {lhs} vs {rhs}");
    }
}
