//! A closed-form walker for piecewise-rate tax schedules.
//!
//! This is a **helper, not part of the contract**. A [`TaxSystem`] is free to
//! ignore it entirely. It is here because most tax schedules share one shape --
//! a monotone ladder of rate bands applied to some taxable fraction of a
//! withdrawal -- and because deriving a gross withdrawal from a wanted net
//! amount across such a ladder is fiddly enough that two independent
//! implementations would drift apart.
//!
//! # The two quantities
//!
//! Each [`Rung`] carries a **rate** and a **leak**. The leak is the taxable
//! fraction of each unit withdrawn: `0` for an untouched wrapper, `1` for fully
//! taxable income, `0.75` where a quarter comes out tax free, `1 - basis/value`
//! where only the gain is chargeable. Net kept per unit of gross is therefore
//! `1 - leak * rate`, constant within a rung -- which is what makes the walk
//! closed-form, with **one division per rung crossed** and usually one in total.
//!
//! # Grossing up is a walk, not a search
//!
//! To deliver `N` net you must withdraw more than `N` gross, and how much more
//! depends on which bands the withdrawal lands in -- which depends on how much
//! you withdraw. That circularity tempts an iterative solver. It is not needed:
//! the marginal rate is a step function of cumulative taxable amount, so walking
//! the steps and dividing once inside the final one is exact.
//!
//! # Flatten policy into rungs
//!
//! Anything that behaves like a marginal rate should be *expressed* as one
//! rather than computed as an adjustment. A withdrawn personal allowance, for
//! instance, is an extra rung at a higher rate, not a subtraction applied to a
//! total you do not know yet. Keeping the ladder monotone is what keeps the walk
//! correct.
//!
//! # Rounding
//!
//! Nothing here rounds. Callers carry full precision and round once at their own
//! output boundary; rounding per draw would accumulate a systematic drift over
//! hundreds of months. One consequence: where a rung's `keep` is not exactly
//! representable (0.6, say), the delivered net can fall short of the requested
//! net by a fraction of an attopenny. Compare rounded figures, not raw ones.

use rust_decimal::Decimal;

use crate::{Draw, StopAt, TaxError, TaxErrorKind};

/// Maximum rungs in one ladder.
///
/// Sized for the worst realistic case with headroom to spare: a six-band income
/// schedule, plus a withdrawn-allowance rung, plus a zero-rate allowance at the
/// foot, plus a breakpoint splitting one rung where a tax-free fraction runs
/// out -- nine, for the widest schedule currently modelled. A fixed array keeps
/// the walk allocation-free, which matters because a goal-seek runs it tens of
/// thousands of times; keep it *tight* as well as fixed, since a `Ladder` is
/// `Copy` and every build zeroes and every return memcpies the whole array.
/// Overflowing it is a loud `BadRules` from [`Ladder::push`], not a silent
/// truncation, so there is no reason to over-allocate against a schedule that
/// does not exist yet.
pub const MAX_RUNGS: usize = 16;

/// One step of a schedule: everything up to `headroom` more taxable units is
/// charged at `rate`, and `leak` of each gross unit withdrawn is taxable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rung {
    /// Taxable amount this rung can absorb before the next one applies.
    /// `None` is open-ended, and only valid on the final rung.
    pub headroom: Option<Decimal>,
    /// Marginal rate on the taxable slice, as a fraction.
    pub rate: Decimal,
    /// Taxable fraction of each gross unit withdrawn, as a fraction.
    pub leak: Decimal,
}

/// The result of a walk: what it cost, and how much taxable amount it consumed.
///
/// `taxable` is what the caller posts to its own ledger. It is returned rather
/// than recomputed because the leak can change part-way through a single draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Walk {
    pub draw: Draw,
    pub taxable: Decimal,
}

/// A fixed-capacity ladder, lowest rung first.
#[derive(Clone, Copy, Debug)]
pub struct Ladder {
    rungs: [Rung; MAX_RUNGS],
    len: usize,
}

impl Default for Ladder {
    fn default() -> Self {
        Self::new()
    }
}

impl Ladder {
    pub const fn new() -> Self {
        Self {
            rungs: [Rung {
                headroom: None,
                rate: Decimal::ZERO,
                leak: Decimal::ZERO,
            }; MAX_RUNGS],
            len: 0,
        }
    }

    /// A ladder for a wholly untaxed account: one open rung at zero rate.
    ///
    /// Built directly rather than through `push` so it can be `const`: this is
    /// the commonest ladder in any portfolio (every tax-free account hits it),
    /// and there is nothing to validate in a single zero-rate rung.
    pub const fn untaxed() -> Self {
        let mut l = Self::new();
        l.rungs[0] = Rung {
            headroom: None,
            rate: Decimal::ZERO,
            leak: Decimal::ZERO,
        };
        l.len = 1;
        l
    }

    /// Append a rung.
    ///
    /// Rejects a rung whose `keep` is not positive -- a rate at or above 100% on
    /// a fully taxable slice would make the gross-up diverge. That cannot arise
    /// from any real schedule, so it means a table is wrong, and failing here
    /// makes a bad table update loud rather than quiet.
    pub fn push(&mut self, rung: Rung) -> Result<(), TaxError> {
        if self.len >= MAX_RUNGS {
            return Err(TaxError::new(
                TaxErrorKind::BadRules,
                "This tax schedule has more rate bands than the calculator can hold.",
            ));
        }
        if Decimal::ONE - rung.leak * rung.rate <= Decimal::ZERO {
            return Err(TaxError::confiscatory());
        }
        debug_assert!(
            rung.headroom.is_none_or(|h| h >= Decimal::ZERO),
            "a rung cannot have negative headroom",
        );
        self.rungs[self.len] = rung;
        self.len += 1;
        Ok(())
    }

    pub fn rungs(&self) -> &[Rung] {
        &self.rungs[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Net kept per unit of gross on the *first* unit drawn: the sort key a
    /// cheapest-first caller orders accounts by. Rungs with no room left are
    /// skipped, since they cannot charge anything.
    pub fn marginal_keep(&self) -> Decimal {
        for r in self.rungs() {
            if r.headroom.is_some_and(|h| h <= Decimal::ZERO) {
                continue;
            }
            return Decimal::ONE - r.leak * r.rate;
        }
        Decimal::ONE
    }

    /// Withdraw enough to deliver `net_wanted`, from an account holding
    /// `available`.
    ///
    /// Stops early if the account empties or `stop` says to. The returned
    /// `Draw` satisfies `gross - tax == net` exactly, because `net` is derived
    /// from the other two rather than accumulated alongside them.
    pub fn walk(
        &self,
        available: Decimal,
        net_wanted: Decimal,
        stop: StopAt,
    ) -> Result<Walk, TaxError> {
        let zero = Decimal::ZERO;
        let mut gross = zero;
        let mut tax = zero;
        let mut taxable = zero;
        let mut rung_limited = false;

        if available <= zero || net_wanted <= zero {
            return Ok(Walk::default());
        }

        for (i, r) in self.rungs().iter().enumerate() {
            let net_so_far = gross - tax;
            if net_so_far >= net_wanted || gross >= available {
                break;
            }

            // A rate cap bites only where something is actually taxable; a
            // tax-free slice is never "too expensive".
            if let StopAt::RateAbove(cap) = stop {
                if !r.leak.is_zero() && r.rate > cap {
                    rung_limited = true;
                    break;
                }
            }

            let room = available - gross;
            let keep = Decimal::ONE - r.leak * r.rate;
            if keep <= zero {
                return Err(TaxError::confiscatory());
            }

            // Gross that fits inside this rung's taxable headroom. An untaxed
            // or open-ended rung absorbs whatever is left in the account.
            let by_band = match r.headroom {
                None => room,
                Some(_) if r.leak.is_zero() => room,
                Some(h) => h.checked_div(r.leak).ok_or_else(TaxError::overflow)?,
            };
            let take = by_band.min(room);

            if take <= zero {
                // An allowance already spent. Costs nothing, absorbs nothing.
                continue;
            }

            let net_here = take.checked_mul(keep).ok_or_else(TaxError::overflow)?;
            if net_so_far + net_here >= net_wanted {
                // The requirement lands inside this rung: one division, done.
                let g = (net_wanted - net_so_far)
                    .checked_div(keep)
                    .ok_or_else(TaxError::overflow)?
                    .min(take);
                let t = g.checked_mul(r.leak).ok_or_else(TaxError::overflow)?;
                gross = gross.checked_add(g).ok_or_else(TaxError::overflow)?;
                taxable = taxable.checked_add(t).ok_or_else(TaxError::overflow)?;
                tax = tax
                    .checked_add(t.checked_mul(r.rate).ok_or_else(TaxError::overflow)?)
                    .ok_or_else(TaxError::overflow)?;
                break;
            }

            let t = take.checked_mul(r.leak).ok_or_else(TaxError::overflow)?;
            gross = gross.checked_add(take).ok_or_else(TaxError::overflow)?;
            taxable = taxable.checked_add(t).ok_or_else(TaxError::overflow)?;
            tax = tax
                .checked_add(t.checked_mul(r.rate).ok_or_else(TaxError::overflow)?)
                .ok_or_else(TaxError::overflow)?;

            if take == room {
                break; // account emptied, not a rate boundary
            }
            if i + 1 < self.len {
                // This rung is spent and the next one costs more.
                rung_limited = true;
                if matches!(stop, StopAt::NextRung) {
                    break;
                }
            }
        }

        Ok(Walk {
            draw: Draw {
                gross,
                tax,
                net: gross - tax,
                rung_limited,
            },
            taxable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// A fully taxable source: nothing up to `free`, then `rate` for ever.
    fn income(free: &str, rate: &str) -> Ladder {
        let mut l = Ladder::new();
        l.push(Rung {
            headroom: Some(d(free)),
            rate: Decimal::ZERO,
            leak: Decimal::ONE,
        })
        .unwrap();
        l.push(Rung {
            headroom: None,
            rate: d(rate),
            leak: Decimal::ONE,
        })
        .unwrap();
        l
    }

    #[test]
    fn an_untaxed_account_hands_over_exactly_what_was_asked() {
        let w = Ladder::untaxed()
            .walk(d("10000"), d("2500"), StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw.gross, d("2500"));
        assert_eq!(w.draw.tax, Decimal::ZERO);
        assert_eq!(w.draw.net, d("2500"));
        assert_eq!(w.taxable, Decimal::ZERO);
        assert!(!w.draw.rung_limited);
    }

    #[test]
    fn a_draw_inside_the_free_band_is_untaxed_but_still_taxable_amount() {
        let w = income("12570", "0.2")
            .walk(d("100000"), d("10000"), StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw.gross, d("10000"));
        assert_eq!(w.draw.tax, Decimal::ZERO);
        // It consumed allowance even though it cost nothing -- that is exactly
        // what the caller must post to its ledger.
        assert_eq!(w.taxable, d("10000"));
    }

    #[test]
    fn grossing_up_across_a_band_is_hand_checkable() {
        // 12,570 free, then 20%. To net 40,000: take the free slice whole, then
        // gross the remaining 27,430 up by 1/0.8.
        let w = income("12570", "0.2")
            .walk(d("1000000"), d("40000"), StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw.gross, d("46857.50"));
        assert_eq!(w.draw.tax, d("6857.50"));
        assert_eq!(w.draw.net, d("40000"));
    }

    #[test]
    fn gross_always_equals_net_plus_tax_exactly() {
        // 0.6 keep is not exactly representable, so this is the case where a
        // naively accumulated `net` would drift off the identity.
        let mut l = Ladder::new();
        l.push(Rung {
            headroom: None,
            rate: d("0.4"),
            leak: Decimal::ONE,
        })
        .unwrap();
        let w = l.walk(d("100000"), d("1000"), StopAt::Requirement).unwrap();
        assert_eq!(w.draw.gross - w.draw.tax, w.draw.net, "the identity is by construction");
        assert_eq!(w.draw.net.round_dp(2), d("1000.00"), "and it still delivers");
    }

    #[test]
    fn an_empty_account_delivers_what_it_can_without_erroring() {
        let w = income("0", "0.2")
            .walk(d("1000"), d("5000"), StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw.gross, d("1000"), "took the lot");
        assert_eq!(w.draw.tax, d("200"));
        assert_eq!(w.draw.net, d("800"), "less than asked, and that is fine");
        assert!(!w.draw.rung_limited, "it ran dry, it did not hit a rate step");
    }

    #[test]
    fn next_rung_stops_at_the_step_and_says_so() {
        let w = income("12570", "0.2")
            .walk(d("1000000"), d("40000"), StopAt::NextRung)
            .unwrap();
        assert_eq!(w.draw.gross, d("12570"), "stopped at the top of the free band");
        assert_eq!(w.draw.net, d("12570"));
        assert!(w.draw.rung_limited, "the caller needs to know why it stopped");
    }

    #[test]
    fn a_rate_cap_refuses_the_expensive_rung_entirely() {
        let mut l = Ladder::new();
        l.push(Rung { headroom: Some(d("1000")), rate: d("0.2"), leak: Decimal::ONE }).unwrap();
        l.push(Rung { headroom: None, rate: d("0.4"), leak: Decimal::ONE }).unwrap();

        let w = l.walk(d("100000"), d("50000"), StopAt::RateAbove(d("0.2"))).unwrap();
        assert_eq!(w.draw.gross, d("1000"), "took the 20% band and stopped");
        assert_eq!(w.draw.tax, d("200"));
        assert!(w.draw.rung_limited);

        // A cap below the very first taxable rung yields nothing at all, so the
        // caller can move on to another account without special-casing.
        let none = l.walk(d("100000"), d("50000"), StopAt::RateAbove(d("0.1"))).unwrap();
        assert_eq!(none.draw.gross, Decimal::ZERO);
        assert!(none.draw.rung_limited);
    }

    #[test]
    fn a_partial_leak_taxes_only_the_taxable_fraction() {
        // Three quarters taxable at 20% -> 15% effective, so 1,000 net needs
        // 1,000 / 0.85 gross.
        let mut l = Ladder::new();
        l.push(Rung { headroom: None, rate: d("0.2"), leak: d("0.75") }).unwrap();
        let w = l.walk(d("100000"), d("850"), StopAt::Requirement).unwrap();
        assert_eq!(w.draw.gross, d("1000"));
        assert_eq!(w.taxable, d("750"));
        assert_eq!(w.draw.tax, d("150"));
        assert_eq!(w.draw.net, d("850"));
    }

    #[test]
    fn marginal_keep_reports_the_first_unit_and_skips_spent_allowances() {
        assert_eq!(Ladder::untaxed().marginal_keep(), Decimal::ONE);
        assert_eq!(income("12570", "0.2").marginal_keep(), Decimal::ONE);
        // Allowance exhausted: the first live rung is the 20% one.
        assert_eq!(income("0", "0.2").marginal_keep(), d("0.8"));
    }

    #[test]
    fn a_confiscatory_schedule_is_rejected_rather_than_diverging() {
        let mut l = Ladder::new();
        let err = l
            .push(Rung { headroom: None, rate: Decimal::ONE, leak: Decimal::ONE })
            .unwrap_err();
        assert_eq!(err.kind, TaxErrorKind::BadRules);
    }

    #[test]
    fn asking_for_nothing_costs_nothing() {
        let w = income("12570", "0.2")
            .walk(d("10000"), Decimal::ZERO, StopAt::Requirement)
            .unwrap();
        assert_eq!(w.draw, Draw::default());
    }
}
