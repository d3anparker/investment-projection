//! The tax-session pricing layer: how a withdrawal is priced against a
//! [`TaxSession`], and the per-month ledger the drawdown advances.
//!
//! Knows nothing of tax bands: it asks the session what a draw costs
//! ([`TaxSession::marginal`]) and when to stop ([`Draw::rung_limited`]), which is
//! what lets the jurisdiction be swapped without touching this file.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use taxkit::{Draw, Pot, SessionSpec, StopAt, TaxError, TaxErrorKind, TaxSession};

use crate::engine::Prepared;
use crate::parse::{overflowed, parse_number, parse_or_zero};
use crate::types::{CalcError, Field, InvestmentField, TaxContext};

/// Mutable per-holding state one month of drawdown advances, alongside a
/// read-only view of the static per-holding data ([`Prepared`]).
///
/// Carrying `prepared` here is what lets the drawdown helpers read a holding's
/// account kind and rank straight off the ledger, rather than being handed a
/// parallel `kinds` array kept in lockstep with the mutable slices by hand.
pub(crate) struct Ledger<'a> {
    pub(crate) prepared: &'a [Prepared],
    pub(crate) balances: &'a mut [Decimal],
    /// What each holding originally cost, shrinking proportionally as it is
    /// sold. Only consulted for account kinds taxed on the gain.
    pub(crate) basis: &'a mut [Decimal],
    pub(crate) withdrawn: &'a mut [Decimal],
    pub(crate) taxes: &'a mut [Decimal],
}

impl Ledger<'_> {
    /// The account kind of holding `j`, read off the static data.
    fn kind(&self, j: usize) -> Option<&'static taxkit::AccountKind> {
        self.prepared[j].kind
    }
}

/// Map a tax system's failure onto the control the user can go and fix.
///
/// The *message* is always the tax system's own, so `calc` never hard-codes an
/// age limit, an allowance or an account name; its job is only to pick a field.
///
/// `holding` is the index of the holding whose draw raised the error, where a
/// caller knows it. It lets the one error that is unambiguously about a single
/// row — an account kind the system does not recognise — name that row's
/// account picker, rather than stranding its message at the foot of the form.
/// Portfolio-level failures (a bad region, negative other income) ignore it.
pub(crate) fn tax_error(e: TaxError, holding: Option<usize>) -> CalcError {
    let field = match e.kind {
        TaxErrorKind::BadRegion => Some(Field::Region),
        TaxErrorKind::BadOtherIncome => Some(Field::OtherIncome),
        TaxErrorKind::AgeGated => Some(Field::Age),
        TaxErrorKind::UnknownAccount => holding
            .map(|index| Field::Investment { index, part: InvestmentField::AccountKind }),
        TaxErrorKind::Overflow | TaxErrorKind::BadRules => None,
    };
    CalcError::new(e.message, field)
}

/// Open the tax session for a projection, validating the portfolio-level tax
/// inputs on the way.
pub(crate) fn open_session(tax: &TaxContext) -> Result<Box<dyn TaxSession>, CalcError> {
    let other_income = parse_or_zero(&tax.other_income).ok_or_else(|| {
        CalcError::new("Enter a valid amount of other taxable income.", Some(Field::OtherIncome))
    })?;
    if other_income < Decimal::ZERO {
        return Err(CalcError::new(
            "Other taxable income cannot be negative.",
            Some(Field::OtherIncome),
        ));
    }

    let age = if tax.age.trim().is_empty() {
        None
    } else {
        let a = parse_number(&tax.age)
            .ok_or_else(|| CalcError::new("Enter a valid age.", Some(Field::Age)))?;
        if a < Decimal::ZERO || a > Decimal::from(120u32) {
            return Err(CalcError::new("Enter an age between 0 and 120.", Some(Field::Age)));
        }
        a.to_u32()
    };

    let uprate = parse_or_zero(&tax.uprate)
        .ok_or_else(|| CalcError::new("Enter a valid uprating percentage.", Some(Field::Uprate)))?
        / Decimal::from(100u32);
    if uprate <= Decimal::NEGATIVE_ONE {
        return Err(CalcError::new(
            "Tax thresholds cannot shrink by 100% or more a year.",
            Some(Field::Uprate),
        ));
    }

    tax.system
        .open(&SessionSpec { region: tax.region.clone(), other_income, age, uprate, ..Default::default() })
        .map_err(|e| tax_error(e, None))
}

/// How the greedy ranks one holding against another, cheapest first.
///
/// `keep` is what survives tax on the next pound out. `expiring` breaks ties
/// between equally cheap holdings, and is the difference between an optimiser
/// and a fixed order: an account that is cheap because an allowance has not been
/// spent is use-it-or-lose-it, so it must be drawn *before* one that is cheap
/// indefinitely. Rank and input order then settle the rest, so the answer never
/// depends on sort stability.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Priority {
    keep: Decimal,
    expiring: bool,
    rank: u8,
    pub(crate) index: usize,
}

impl Priority {
    pub(crate) fn beats(&self, other: &Priority) -> bool {
        (self.keep, self.expiring, std::cmp::Reverse(self.rank), std::cmp::Reverse(self.index))
            > (other.keep, other.expiring, std::cmp::Reverse(other.rank), std::cmp::Reverse(other.index))
    }
}

fn pot_of(led: &Ledger, j: usize) -> Option<Pot> {
    led.kind(j).map(|k| Pot {
        kind: k.id,
        available: led.balances[j],
        cost_basis: led.basis[j],
    })
}

pub(crate) fn priority_of(session: &Option<Box<dyn TaxSession>>, led: &Ledger, j: usize) -> Priority {
    let (keep, expiring) = match (session, pot_of(led, j)) {
        (Some(s), Some(pot)) => {
            // One ladder build for both figures: `marginal` returns the keep and
            // the headroom together, so a cheapest-first pass does not price the
            // same holding twice. Finite headroom means the cheapness runs out.
            let (keep, headroom) = s.marginal(&pot);
            (keep, headroom.is_some())
        }
        _ => (Decimal::ONE, false),
    };
    Priority { keep, expiring, rank: led.kind(j).map_or(0, |k| k.rank), index: j }
}

/// Take up to `net_wanted` out of one holding and post it to the ledger.
pub(crate) fn draw_one(
    session: &mut Option<Box<dyn TaxSession>>,
    led: &mut Ledger,
    j: usize,
    net_wanted: Decimal,
    stop: StopAt,
) -> Result<Draw, CalcError> {
    let available = led.balances[j];
    if available <= Decimal::ZERO || net_wanted <= Decimal::ZERO {
        return Ok(Draw::default());
    }

    let drawn = match (session.as_mut(), led.prepared[j].kind) {
        (Some(s), Some(k)) => {
            let pot = Pot { kind: k.id, available, cost_basis: led.basis[j] };
            s.draw(&pot, net_wanted, stop).map_err(|e| tax_error(e, Some(j)))?
        }
        // No tax system: every account is free, so gross is net.
        _ => {
            let g = net_wanted.min(available);
            Draw { gross: g, tax: Decimal::ZERO, net: g, rung_limited: false }
        }
    };

    if drawn.gross > Decimal::ZERO {
        // Proportional disposal: a slice carries the same fraction of the
        // original cost away with it, so what remains keeps its profit ratio.
        // That also makes the taxable fraction path-independent, which is what
        // keeps the goal-seek's feasibility search well behaved.
        //
        // This rescales the *absolute* remaining basis and is deliberately not
        // `Pot::proportional_leak`, which returns the taxable *fraction*
        // (1 - basis/available). They share a term but are different quantities;
        // don't fold one into the other.
        let left = (available - drawn.gross).max(Decimal::ZERO);
        led.basis[j] = led.basis[j]
            .checked_mul(left)
            .and_then(|x| x.checked_div(available))
            .ok_or_else(overflowed)?;
        led.balances[j] = left;
        led.withdrawn[j] += drawn.gross;
        led.taxes[j] += drawn.tax;
    }
    Ok(drawn)
}
