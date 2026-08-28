//! A deliberately fake tax system, for testing consumers of this crate.
//!
//! It exists for two reasons. First, a projection engine needs *some* tax
//! system to test against, and testing it against a real jurisdiction couples
//! the engine's tests to figures that change every April -- a rate update would
//! then break two crates instead of one. Second, and more important, an
//! abstraction designed against a single implementation quietly grows that
//! implementation's shape. This is the second implementation that stops it.
//!
//! The numbers are round on purpose: every figure a test needs is arithmetic a
//! reader can do in their head.
//!
//! | Account   | Taxation                                                   |
//! |-----------|------------------------------------------------------------|
//! | `free`    | none at all                                                 |
//! | `income`  | fully taxable: 12,000 free per period, then 20%, then 40% over 50,000 |
//! | `gains`   | only the gain is taxable: 1,000 free per period, then 20%   |
//!
//! One period is twelve months. `income` is age-gated at 55, so consumers can
//! exercise that path without a real pension.

use rust_decimal::Decimal;

use crate::ladder::{Ladder, Rung};
use crate::{
    AccountKind, Draw, Pot, Region, SessionSpec, SimpleDate, Staleness, StopAt, TaxError,
    TaxErrorKind, TaxSession, TaxSystem,
};

/// The mock tax system. Use `&MOCK` where a `&'static dyn TaxSystem` is wanted.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockTaxSystem;

/// A ready-made instance.
pub static MOCK: MockTaxSystem = MockTaxSystem;

pub const FREE: &str = "free";
pub const INCOME: &str = "income";
pub const GAINS: &str = "gains";

/// Free income band per period.
pub const INCOME_ALLOWANCE: i64 = 12_000;
/// Taxable income at which the higher rate starts (measured after the
/// allowance, as a real schedule would).
pub const HIGHER_BAND: i64 = 50_000;
/// Free gains per period.
pub const GAINS_ALLOWANCE: i64 = 1_000;
/// Age below which `income` cannot be touched.
pub const ACCESS_AGE: u32 = 55;

const ACCOUNTS: &[AccountKind] = &[
    AccountKind {
        id: FREE,
        label: "Untaxed account",
        short_label: "Free",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 0,
        note: "",
    },
    AccountKind {
        id: GAINS,
        label: "Gains-taxed account",
        short_label: "Gains",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 1,
        note: "Only the gain is taxable.",
    },
    AccountKind {
        id: INCOME,
        label: "Income-taxed account",
        short_label: "Income",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 2,
        note: "Taxed as income when withdrawn.",
    },
];

const REGIONS: &[Region] = &[Region {
    id: "all",
    label: "Everywhere",
}];

const ORDER: &[&str] = &[GAINS, FREE, INCOME];

impl TaxSystem for MockTaxSystem {
    fn label(&self) -> &'static str {
        "Testland"
    }

    fn currency_symbol(&self) -> &'static str {
        "\u{00a4}" // the generic currency sign: not any real money
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        ACCOUNTS
    }

    fn regions(&self) -> &'static [Region] {
        REGIONS
    }

    fn conventional_order(&self) -> &'static [&'static str] {
        ORDER
    }

    fn rules_label(&self) -> &'static str {
        "test"
    }

    fn as_of(&self) -> SimpleDate {
        SimpleDate::new(2000, 1, 1)
    }

    fn source_note(&self) -> &'static str {
        "Invented for testing. Not a real tax system."
    }

    fn staleness(&self, _today: SimpleDate) -> Staleness {
        // Always fresh, so a consumer's tests never depend on the clock.
        Staleness::Fresh
    }

    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn TaxSession>, TaxError> {
        if self.region(&spec.region).is_none() {
            return Err(TaxError::new(
                TaxErrorKind::BadRegion,
                format!("'{}' is not a region of Testland.", spec.region),
            ));
        }
        if spec.other_income < Decimal::ZERO {
            return Err(TaxError::new(
                TaxErrorKind::BadOtherIncome,
                "Other income cannot be negative.",
            ));
        }
        Ok(Box::new(MockSession {
            other_income: spec.other_income,
            age: spec.age,
            // Seeded directly rather than via `start_period`, which would bank a
            // full period's allowance as unused before the projection had begun.
            income: spec.other_income,
            gains: Decimal::ZERO,
            period_tax: Decimal::ZERO,
            banked_unused: Decimal::ZERO,
        }))
    }
}

struct MockSession {
    other_income: Decimal,
    age: Option<u32>,
    /// Taxable income booked so far this period, before the allowance.
    income: Decimal,
    /// Chargeable gains booked so far this period, before the allowance.
    gains: Decimal,
    period_tax: Decimal,
    banked_unused: Decimal,
}

impl MockSession {
    fn unused_now(&self) -> Decimal {
        let inc = (Decimal::from(INCOME_ALLOWANCE) - self.income).max(Decimal::ZERO);
        let gns = (Decimal::from(GAINS_ALLOWANCE) - self.gains).max(Decimal::ZERO);
        inc + gns
    }

    fn ladder_for(&self, pot: &Pot) -> Result<Ladder, TaxError> {
        match pot.kind {
            FREE => Ok(Ladder::untaxed()),
            INCOME => {
                if self.age.is_some_and(|a| a < ACCESS_AGE) {
                    return Err(TaxError::new(
                        TaxErrorKind::AgeGated,
                        format!("An income account cannot be touched before {ACCESS_AGE}."),
                    ));
                }
                let mut l = Ladder::new();
                let free = (Decimal::from(INCOME_ALLOWANCE) - self.income).max(Decimal::ZERO);
                let used_above = (self.income - Decimal::from(INCOME_ALLOWANCE)).max(Decimal::ZERO);
                let basic = (Decimal::from(HIGHER_BAND) - used_above).max(Decimal::ZERO);
                l.push(Rung { headroom: Some(free), rate: Decimal::ZERO, leak: Decimal::ONE })?;
                l.push(Rung { headroom: Some(basic), rate: Decimal::new(2, 1), leak: Decimal::ONE })?;
                l.push(Rung { headroom: None, rate: Decimal::new(4, 1), leak: Decimal::ONE })?;
                Ok(l)
            }
            GAINS => {
                let leak = pot.proportional_leak();
                let mut l = Ladder::new();
                let free = (Decimal::from(GAINS_ALLOWANCE) - self.gains).max(Decimal::ZERO);
                l.push(Rung { headroom: Some(free), rate: Decimal::ZERO, leak })?;
                l.push(Rung { headroom: None, rate: Decimal::new(2, 1), leak })?;
                Ok(l)
            }
            other => Err(TaxError::new(
                TaxErrorKind::UnknownAccount,
                format!("'{other}' is not an account kind in Testland."),
            )),
        }
    }
}

impl TaxSession for MockSession {
    fn period_months(&self) -> u32 {
        12
    }

    fn start_period(&mut self) {
        self.banked_unused += self.unused_now();
        // Other income arrives whether or not the portfolio is touched, so the
        // first unit withdrawn is marginal on top of all of it.
        self.income = self.other_income;
        self.gains = Decimal::ZERO;
        self.period_tax = Decimal::ZERO;
    }

    fn marginal_keep(&self, pot: &Pot) -> Decimal {
        self.ladder_for(pot)
            .map(|l| l.marginal_keep())
            .unwrap_or(Decimal::ZERO)
    }

    fn draw(&mut self, pot: &Pot, net_wanted: Decimal, stop: StopAt) -> Result<Draw, TaxError> {
        let ladder = self.ladder_for(pot)?;
        let walk = ladder.walk(pot.available, net_wanted, stop)?;
        match pot.kind {
            INCOME => self.income += walk.taxable,
            GAINS => self.gains += walk.taxable,
            _ => {}
        }
        self.period_tax += walk.draw.tax;
        Ok(walk.draw)
    }

    fn period_tax(&self) -> Decimal {
        self.period_tax
    }

    fn unused_allowance(&self) -> Decimal {
        self.banked_unused + self.unused_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn session(other_income: &str) -> Box<dyn TaxSession> {
        MOCK.open(&SessionSpec {
            region: "all".into(),
            other_income: d(other_income),
            age: Some(60),
            uprate: Decimal::ZERO,
        })
        .unwrap()
    }

    fn pot(kind: &'static str, available: &str, basis: &str) -> Pot {
        Pot { kind, available: d(available), cost_basis: d(basis) }
    }

    #[test]
    fn the_untaxed_account_is_free_and_consumes_no_allowance() {
        let mut s = session("0");
        let before = s.unused_allowance();
        let dr = s.draw(&pot(FREE, "50000", "0"), d("1000"), StopAt::Requirement).unwrap();
        assert_eq!(dr.tax, Decimal::ZERO);
        assert_eq!(dr.net, d("1000"));
        assert_eq!(s.unused_allowance(), before, "free money spends no allowance");
    }

    #[test]
    fn other_income_is_charged_in_full_at_the_start_of_the_period() {
        // 10,000 of other income leaves 2,000 of allowance, so a 3,000 draw
        // pays 20% on the last 1,000 -- and must be grossed up to deliver it.
        let mut s = session("10000");
        let dr = s.draw(&pot(INCOME, "100000", "0"), d("2800"), StopAt::Requirement).unwrap();
        assert_eq!(dr.gross, d("3000"));
        assert_eq!(dr.tax, d("200"));
        assert_eq!(dr.net, d("2800"));
    }

    #[test]
    fn allowances_reset_at_the_period_boundary_and_not_before() {
        let mut s = session("0");
        let p = pot(INCOME, "1000000", "0");
        s.draw(&p, d("12000"), StopAt::Requirement).unwrap();
        // Allowance now spent: the next unit costs 20%.
        assert_eq!(s.marginal_keep(&p), d("0.8"));

        s.start_period();
        assert_eq!(s.marginal_keep(&p), Decimal::ONE, "a new period restores it");
        assert_eq!(s.period_tax(), Decimal::ZERO, "and the period's tax with it");
    }

    #[test]
    fn unused_allowance_accumulates_across_periods() {
        let mut s = session("0");
        // Touch nothing for two periods: 13,000 of headroom wasted each time.
        let per_period = Decimal::from(INCOME_ALLOWANCE + GAINS_ALLOWANCE);
        assert_eq!(s.unused_allowance(), per_period);
        s.start_period();
        assert_eq!(s.unused_allowance(), per_period * Decimal::TWO);
    }

    #[test]
    fn a_gains_account_taxes_only_the_profit_fraction() {
        // Half the holding is profit, so half of each unit withdrawn is taxable.
        let mut s = session("0");
        let p = pot(GAINS, "20000", "10000");
        // First 1,000 of gain is free -- that is 2,000 gross.
        let dr = s.draw(&p, d("2000"), StopAt::Requirement).unwrap();
        assert_eq!(dr.tax, Decimal::ZERO);
        assert_eq!(dr.gross, d("2000"));
        // Now the allowance is gone: 10% effective (half taxable at 20%).
        assert_eq!(s.marginal_keep(&p), d("0.9"));
    }

    #[test]
    fn a_holding_with_no_gain_is_never_taxed() {
        let s = session("0");
        assert_eq!(s.marginal_keep(&pot(GAINS, "10000", "10000")), Decimal::ONE);
        // A holding at a loss is not a taxable gain, and not an error either.
        assert_eq!(s.marginal_keep(&pot(GAINS, "10000", "15000")), Decimal::ONE);
    }

    #[test]
    fn age_gating_reports_rather_than_silently_skipping() {
        let mut s = MOCK
            .open(&SessionSpec {
                region: "all".into(),
                other_income: Decimal::ZERO,
                age: Some(40),
                uprate: Decimal::ZERO,
            })
            .unwrap();
        let err = s
            .draw(&pot(INCOME, "10000", "0"), d("100"), StopAt::Requirement)
            .unwrap_err();
        assert_eq!(err.kind, TaxErrorKind::AgeGated);
    }

    #[test]
    fn an_unknown_region_or_account_is_an_error_not_a_guess() {
        assert_eq!(
            MOCK.open(&SessionSpec {
                region: "narnia".into(),
                other_income: Decimal::ZERO,
                age: None,
                uprate: Decimal::ZERO,
            })
            .err()
            .expect("this should be refused")
            .kind,
            TaxErrorKind::BadRegion
        );
        let mut s = session("0");
        assert_eq!(
            s.draw(&pot("nonsense", "1", "0"), d("1"), StopAt::Requirement)
                .unwrap_err()
                .kind,
            TaxErrorKind::UnknownAccount
        );
    }

    #[test]
    fn the_catalogue_is_internally_consistent() {
        let ids: Vec<_> = MOCK.account_kinds().iter().map(|k| k.id).collect();
        for id in MOCK.conventional_order() {
            assert!(ids.contains(id), "'{id}' is ordered but not in the catalogue");
        }
        assert_eq!(ids.len(), MOCK.conventional_order().len());
        assert!(!MOCK.regions().is_empty(), "there is always at least one region");
    }
}
