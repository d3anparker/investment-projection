//! Germany, implementing the `taxkit` traits.
//!
//! Mechanism, not data: how the [`tables`](crate::tables) figures become a
//! schedule, and how a withdrawal is priced against it. Two schedules meet here
//! — a flat-rate [`taxkit::Ladder`] for capital income (Abgeltungsteuer) and the
//! progressive [`Tarif`](crate::tarif::Tarif) for pension income — dispatched by
//! the account's [`WithdrawalTax`]. A rate change never touches this file; only
//! a change to the *kind* of taxation does.

use rust_decimal::Decimal;

use taxkit::ladder::{Ladder, Rung};
use taxkit::{
    AccountKind, Draw, PeriodPot, Pot, Region, SessionSpec, SimpleDate, Staleness, StopAt,
    TaxError, TaxErrorKind, TaxSession, TaxSystem,
};

use crate::tables::{self, bp, de_tax_year_of, tax_year_label, TaxYear, Treatment, WithdrawalTax};
use crate::tarif::Tarif;

/// Months after `as_of` beyond which the figures are called stale even if the
/// calendar year has not turned over.
const STALE_AFTER_MONTHS: i32 = 18;

/// The one geographic rate variation Germany has is the church-tax rate, so it
/// *is* the region axis. This conflates church membership with a Land, which the
/// labels say plainly.
const REGIONS: &[Region] = &[
    Region { id: "de_none", label: "No church tax" },
    Region { id: "de_ks8", label: "Church tax 8% (Bayern, Baden-Württemberg)" },
    Region { id: "de_ks9", label: "Church tax 9% (other Länder)" },
];

/// Option ids this system reads off [`SessionSpec::options`], with the labels and
/// notes its bespoke app panel renders. Exported so the app owns layout only, and
/// a rename here is a compile error there rather than a silently ignored option.
pub mod options {
    pub const FILING: &str = "filing";
    pub const FILING_INDIVIDUAL: &str = "individual";
    pub const FILING_JOINT: &str = "joint";
    pub const FILING_LABEL: &str = "Assessment";
    pub const FILING_NOTE: &str =
        "Joint assessment (Ehegattensplitting) halves the progression and doubles the allowances.";

    pub const BASE_YEAR: &str = "base_year";
    pub const BASE_YEAR_LABEL: &str = "Year drawing starts";
    pub const BASE_YEAR_NOTE: &str =
        "A Rürup pension's taxable share is fixed for life by the year you start drawing it.";
}

fn eur(v: i64) -> Decimal {
    Decimal::from(v)
}

/// The effective flat capital rate, §32d(1): 0.25 grossed up for Soli and church
/// tax, church tax being deductible against its own base.
///
/// A function of the rules and the church-tax setting only, so a session works it
/// out once at `open` rather than on every schedule build.
fn flat_rate(rules: &TaxYear, kirche_bp: u32) -> Decimal {
    let k = bp(kirche_bp);
    let kapest = bp(rules.kapest_bp);
    let soli = bp(rules.soli_bp);
    kapest * (Decimal::ONE + soli + k) / (Decimal::ONE + kapest * k)
}

/// Germany.
#[derive(Clone, Copy, Debug, Default)]
pub struct GermanTaxSystem;

/// The instance to wire in: `&de_tax::DE`.
pub static DE: GermanTaxSystem = GermanTaxSystem;

impl GermanTaxSystem {
    /// The Kirchensteuer rate a region id selects, read from the tables rather
    /// than written out here: the rates are data, and a rate update is only
    /// allowed to touch `tables.rs`. Spelling them here too would mean an update
    /// that edits the table and changes nothing.
    fn kirche_bp(region: &str) -> u32 {
        let rates = tables::LATEST.kirchensteuer_bp;
        match region {
            "de_ks8" => rates[0],
            "de_ks9" => rates[1],
            _ => 0,
        }
    }
}

impl TaxSystem for GermanTaxSystem {
    fn label(&self) -> &'static str {
        "Germany"
    }

    fn currency_symbol(&self) -> &'static str {
        "\u{20ac}" // €, as an escape, never a literal
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        tables::DE_ACCOUNTS
    }

    fn regions(&self) -> &'static [Region] {
        REGIONS
    }

    fn conventional_order(&self) -> &'static [&'static str] {
        tables::DE_CONVENTIONAL_ORDER
    }

    fn rules_label(&self) -> &'static str {
        tables::LATEST.label
    }

    fn as_of(&self) -> SimpleDate {
        tables::LATEST.as_of
    }

    fn source_note(&self) -> &'static str {
        tables::LATEST.source_note
    }

    fn staleness(&self, today: SimpleDate) -> Staleness {
        let checked = tables::LATEST.as_of;
        let now = de_tax_year_of(today);
        let then = de_tax_year_of(checked);
        let aged = checked.months_until(today) > STALE_AFTER_MONTHS;
        if now > then || aged {
            Staleness::Stale { current_period: tax_year_label(now) }
        } else {
            Staleness::Fresh
        }
    }

    fn has_periodic_charge(&self) -> bool {
        true
    }

    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn TaxSession>, TaxError> {
        if self.region(&spec.region).is_none() {
            return Err(TaxError::new(
                TaxErrorKind::BadRegion,
                format!("'{}' is not a church-tax setting this calculator knows.", spec.region),
            ));
        }
        if spec.other_income < Decimal::ZERO {
            return Err(TaxError::new(
                TaxErrorKind::BadOtherIncome,
                "Other taxable income cannot be negative.",
            ));
        }
        if spec.uprate <= Decimal::NEGATIVE_ONE {
            return Err(TaxError::new(
                TaxErrorKind::BadRules,
                "Thresholds cannot shrink by 100% or more each year.",
            ));
        }

        let splitting = spec.option(options::FILING) == Some(options::FILING_JOINT);
        let start_year = spec.option(options::BASE_YEAR)
            .and_then(|v| v.trim().parse::<u16>().ok())
            .unwrap_or_else(|| de_tax_year_of(tables::LATEST.starts));

        let rules = tables::LATEST;
        let kirche_bp = Self::kirche_bp(&spec.region);
        let tarif = Tarif::build(rules, kirche_bp, splitting, Decimal::ONE)?;

        Ok(Box::new(GermanSession {
            rules,
            kirche_bp,
            flat_rate: flat_rate(rules, kirche_bp),
            splitting,
            start_year,
            uprate: spec.uprate,
            scale: Decimal::ONE,
            tarif,
            age: spec.age,
            other_income: spec.other_income,
            income: spec.other_income,
            kapital: Decimal::ZERO,
            period_tax: Decimal::ZERO,
            banked_unused: Decimal::ZERO,
        }))
    }
}

struct GermanSession {
    rules: &'static TaxYear,
    kirche_bp: u32,
    /// The effective flat capital rate, fixed for the session by the rules and
    /// the church-tax setting. Cached because it is read on every capital
    /// schedule build: per holding, per greedy pass, per month.
    flat_rate: Decimal,
    splitting: bool,
    /// Year the drawdown starts, fixing the cohort Besteuerungsanteil for life.
    start_year: u16,
    uprate: Decimal,
    /// Compounded threshold uprating for the current period; `1` when frozen.
    scale: Decimal,
    tarif: Tarif,
    age: Option<u32>,
    /// Other taxable income, which recurs every period. Retained so
    /// `start_period` can re-seed `income` to it.
    other_income: Decimal,
    /// §32a income booked this period, before the Grundfreibetrag.
    income: Decimal,
    /// Capital income booked this period, before the Sparer-Pauschbetrag.
    kapital: Decimal,
    period_tax: Decimal,
    banked_unused: Decimal,
}

/// Which schedule prices a given holding.
enum Sched {
    /// A flat-rate ladder (capital income), or an untaxed pass-through.
    Ladder(Ladder),
    /// The progressive tariff, on `leak` of each gross unit.
    Tarif { leak: Decimal },
}

impl GermanSession {
    fn split(&self) -> Decimal {
        if self.splitting {
            Decimal::TWO
        } else {
            Decimal::ONE
        }
    }

    /// Günstigerprüfung, simplified to a monotone lesser-of: capital is charged
    /// at the flat rate or the personal marginal rate, whichever is lower.
    fn capital_rate(&self) -> Decimal {
        self.flat_rate.min(self.tarif.marginal_rate_at(self.income))
    }

    fn sparer_remaining(&self) -> Decimal {
        (eur(self.rules.sparer_pauschbetrag_eur) * self.split() - self.kapital).max(Decimal::ZERO)
    }

    fn grundfreibetrag_remaining(&self) -> Decimal {
        (eur(self.rules.grundfreibetrag_eur) * self.scale * self.split() - self.income)
            .max(Decimal::ZERO)
    }

    fn check_age(&self, min_age: u8) -> Result<(), TaxError> {
        if min_age > 0 {
            if let Some(age) = self.age {
                if age < u32::from(min_age) {
                    return Err(TaxError::new(
                        TaxErrorKind::AgeGated,
                        format!(
                            "This account cannot normally be accessed before {min_age}, and this \
                             drawdown starts at {age}.",
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The withdrawal treatment for a holding's account kind.
    ///
    /// Looked up in the treatment table alone on the happy path. The catalogue is
    /// only consulted when that misses, and then only to say *which* kind of
    /// wrong it is: an id nobody advertises, versus an advertised one the tables
    /// forgot. That distinction is worth a scan on an error path; it is not worth
    /// one per holding per draw on the month loop, which is what checking both up
    /// front cost.
    fn resolve(&self, pot: &Pot) -> Result<Treatment, TaxError> {
        let t = tables::treatment_of(pot.kind).ok_or_else(|| {
            if DE.account_kind(pot.kind).is_none() {
                return TaxError::new(
                    TaxErrorKind::UnknownAccount,
                    format!("'{}' is not an account this calculator knows.", pot.kind),
                );
            }
            TaxError::new(
                TaxErrorKind::BadRules,
                format!("'{}' has no withdrawal treatment in the tax tables.", pot.kind),
            )
        })?;
        self.check_age(t.min_age)?;
        Ok(t)
    }

    /// The taxable share (fraction) of a pension payment, cohort-resolved when the
    /// table says `0`.
    fn pension_share(&self, t: &Treatment) -> Decimal {
        let share_bp = if t.taxable_share_bp == 0 {
            self.rules.besteuerungsanteil_for(self.start_year)
        } else {
            t.taxable_share_bp
        };
        bp(share_bp)
    }

    fn schedule_for(&self, pot: &Pot) -> Result<Sched, TaxError> {
        let t = self.resolve(pot)?;
        Ok(match t.tax {
            WithdrawalTax::None | WithdrawalTax::NotModelled => Sched::Ladder(Ladder::untaxed()),
            WithdrawalTax::FlatCapital => {
                let leak = bp(t.taxable_share_bp) * pot.proportional_leak();
                let mut l = Ladder::new();
                l.push(Rung { headroom: Some(self.sparer_remaining()), rate: Decimal::ZERO, leak })?;
                l.push(Rung { headroom: None, rate: self.capital_rate(), leak })?;
                Sched::Ladder(l)
            }
            WithdrawalTax::ProgressiveIncome => Sched::Tarif { leak: self.pension_share(&t) },
            WithdrawalTax::ProgressiveGain => {
                Sched::Tarif { leak: self.pension_share(&t) * pot.proportional_leak() }
            }
        })
    }
}

impl TaxSession for GermanSession {
    fn period_months(&self) -> u32 {
        12
    }

    fn start_period(&mut self) {
        self.banked_unused += self.grundfreibetrag_remaining() + self.sparer_remaining();
        if !self.uprate.is_zero() {
            self.scale *= Decimal::ONE + self.uprate;
            // The tariff is stretched by the new scale; a failed rebuild leaves
            // the old one in place, which is the safe direction.
            if let Ok(t) = Tarif::build(self.rules, self.kirche_bp, self.splitting, self.scale) {
                self.tarif = t;
            }
        }
        // Other income arrives whether or not the portfolio is touched, so the
        // first unit withdrawn is marginal on top of it.
        self.income = self.other_income;
        self.kapital = Decimal::ZERO;
        self.period_tax = Decimal::ZERO;
    }

    fn marginal_keep(&self, pot: &Pot) -> Decimal {
        match self.schedule_for(pot) {
            Ok(Sched::Ladder(l)) => l.marginal_keep(),
            Ok(Sched::Tarif { leak }) => self.tarif.marginal_keep(self.income, leak),
            Err(_) => Decimal::ZERO,
        }
    }

    fn marginal_headroom(&self, pot: &Pot) -> Option<Decimal> {
        match self.schedule_for(pot) {
            // The flat ladder has a real step at the Sparer-Pauschbetrag.
            Ok(Sched::Ladder(l)) => l.marginal_headroom(),
            // The progressive rate never holds over an interval, so there is no
            // honest headroom figure — `None` is always safe (see the trait doc).
            Ok(Sched::Tarif { .. }) => None,
            Err(_) => None,
        }
    }

    /// Both figures off a single `schedule_for`, the same reason `uk-tax`
    /// overrides this: building a schedule means scanning the treatment table
    /// and, for capital income, constructing a `Ladder`, and a cheapest-first
    /// ranking asks for the keep *and* the headroom of every holding on every
    /// greedy pass. Taking the default would do that work twice per query.
    fn marginal(&self, pot: &Pot) -> (Decimal, Option<Decimal>) {
        match self.schedule_for(pot) {
            Ok(Sched::Ladder(l)) => (l.marginal_keep(), l.marginal_headroom()),
            Ok(Sched::Tarif { leak }) => (self.tarif.marginal_keep(self.income, leak), None),
            Err(_) => (Decimal::ZERO, None),
        }
    }

    fn draw(&mut self, pot: &Pot, net_wanted: Decimal, stop: StopAt) -> Result<Draw, TaxError> {
        match self.schedule_for(pot)? {
            Sched::Ladder(l) => {
                let walk = l.walk(pot.available, net_wanted, stop)?;
                self.kapital += walk.taxable;
                self.period_tax += walk.draw.tax;
                Ok(walk.draw)
            }
            Sched::Tarif { leak } => {
                let walk = self.tarif.walk(self.income, leak, pot.available, net_wanted, stop)?;
                self.income += walk.taxable;
                self.period_tax += walk.draw.tax;
                Ok(walk.draw)
            }
        }
    }

    fn period_charge(
        &mut self,
        pots: &[PeriodPot],
        charges: &mut [Decimal],
    ) -> Result<(), TaxError> {
        let basiszins = bp(self.rules.basiszins_bp);
        let faktor = bp(self.rules.vorab_faktor_bp);
        let rate = self.flat_rate;
        for (idx, p) in pots.iter().enumerate() {
            charges[idx] = Decimal::ZERO;
            let Some(t) = tables::treatment_of(p.pot.kind) else { continue };
            if !t.vorabpauschale {
                continue;
            }
            // Basisertrag: 70% of (opening × Basiszins), capped at the period's
            // actual gain (zero if the fund fell). During drawdown `available` is
            // post-withdrawal, so this under- rather than over-states the cap — a
            // documented simplification.
            let basisertrag = (p.opening * basiszins * faktor).max(Decimal::ZERO);
            let gain = (p.pot.available - p.opening).max(Decimal::ZERO);
            let vorab = basisertrag.min(gain);
            if vorab <= Decimal::ZERO {
                continue;
            }
            let taxable = bp(t.taxable_share_bp) * vorab;
            let free = self.sparer_remaining().min(taxable);
            let taxed = (taxable - free).max(Decimal::ZERO);
            let charge = taxed * rate;
            self.kapital += taxable;
            self.period_tax += charge;
            charges[idx] = charge;
        }
        Ok(())
    }

    fn period_tax(&self) -> Decimal {
        self.period_tax
    }

    fn unused_allowance(&self) -> Decimal {
        self.banked_unused + self.grundfreibetrag_remaining() + self.sparer_remaining()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::ids;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn open(region: &str, other_income: &str, age: Option<u32>, options: Vec<(String, String)>) -> Box<dyn TaxSession> {
        DE.open(&SessionSpec {
            region: region.into(),
            other_income: d(other_income),
            age,
            options,
            ..Default::default()
        })
        .unwrap()
    }

    /// No church tax, single, age 65 (past every gate).
    fn plain(other_income: &str) -> Box<dyn TaxSession> {
        open("de_none", other_income, Some(65), vec![])
    }

    fn pot(kind: &'static str, available: &str, basis: &str) -> Pot {
        Pot { kind, available: d(available), cost_basis: d(basis) }
    }

    fn period_pot(kind: &'static str, available: &str, opening: &str) -> PeriodPot {
        PeriodPot { pot: pot(kind, available, "0"), opening: d(opening) }
    }

    // --- capital income (Abgeltungsteuer) ----------------------------------

    #[test]
    fn a_depot_gain_pays_the_effective_flat_rate_after_the_allowance() {
        // A €200,000 depot half of which is gain (basis 100,000). The first
        // €1,000 of gain is covered by the Sparer-Pauschbetrag; beyond that the
        // effective rate is 26.375% on the gain fraction (leak 0.5). Draw enough
        // that the personal marginal rate is not the binding one.
        let mut s = plain("100000"); // high other income → personal marginal > flat
        // Marginal keep on the next unit: leak 0.5, rate 26.375% → keep
        // 1 − 0.5·0.26375 = 0.868125, once the allowance is spent.
        // Spend the allowance first with a small draw.
        let _ = s.draw(&pot(ids::DEPOT_AKTIEN, "200000", "100000"), d("2000"), StopAt::Requirement).unwrap();
        let keep = s.marginal_keep(&pot(ids::DEPOT_AKTIEN, "198000", "99000"));
        assert_eq!(keep.round_dp(4), d("0.8681"));
    }

    #[test]
    fn teilfreistellung_makes_an_equity_fund_cheaper_than_a_plain_depot() {
        // Same pot, same gain fraction: the 30%-exempt equity fund keeps more of
        // each euro than the fully-taxable depot, because only 70% of its gain
        // is taxable. Spend the Sparer-Pauschbetrag first, or both keep 1.0 on
        // their allowance-covered first unit.
        let mut s = plain("100000");
        let _ = s.draw(&pot(ids::DEPOT_AKTIEN, "200000", "100000"), d("3000"), StopAt::Requirement).unwrap();
        let depot = s.marginal_keep(&pot(ids::DEPOT_AKTIEN, "197000", "98500"));
        let fund = s.marginal_keep(&pot(ids::FONDS_AKTIEN, "197000", "98500"));
        assert!(fund > depot, "the part-exempt fund must keep more: {fund} vs {depot}");
    }

    #[test]
    fn a_holding_with_no_gain_costs_nothing_to_sell() {
        let mut s = plain("100000");
        let drawn = s.draw(&pot(ids::DEPOT_AKTIEN, "50000", "50000"), d("10000"), StopAt::Requirement).unwrap();
        assert_eq!(drawn.tax.round_dp(2), d("0.00"));
        assert_eq!(drawn.gross.round_dp(2), d("10000.00"));
    }

    #[test]
    fn cash_is_never_taxed() {
        let mut s = plain("500000");
        let drawn = s.draw(&pot(ids::GIRO, "100000", "0"), d("100000"), StopAt::Requirement).unwrap();
        assert_eq!(drawn.tax, Decimal::ZERO);
    }

    // --- progressive pension income ----------------------------------------

    #[test]
    fn a_bav_pension_is_grossed_up_across_the_progression() {
        // Fully-taxable occupational pension, no other income. €20,000 net.
        // Below the Grundfreibetrag (€12,348) nothing is due, then the 14%→ zone
        // bites — so some tax is paid and gross exceeds net.
        let mut s = plain("0");
        let drawn = s.draw(&pot(ids::BAV, "1000000", "0"), d("20000"), StopAt::Requirement).unwrap();
        assert_eq!(drawn.net.round_dp(2), d("20000.00"));
        assert!(drawn.tax > d("0.00"), "some tax is due above the Grundfreibetrag");
        assert_eq!((drawn.gross - drawn.tax).round_dp(2), drawn.net.round_dp(2));
    }

    #[test]
    fn other_income_pushes_a_pension_up_the_tariff() {
        // The same pension draw costs more when it stacks on other income,
        // because it starts higher up the progression.
        let mut low = plain("0");
        let mut high = plain("40000");
        let a = low.draw(&pot(ids::BAV, "1000000", "0"), d("10000"), StopAt::Requirement).unwrap();
        let b = high.draw(&pot(ids::BAV, "1000000", "0"), d("10000"), StopAt::Requirement).unwrap();
        assert!(b.tax > a.tax, "marginal on top of other income costs more: {} vs {}", b.tax, a.tax);
    }

    #[test]
    fn a_ruerup_pension_uses_the_cohort_taxable_share() {
        // A Rürup started in 2026 is 84% taxable; one started earlier is less.
        let mut early = open("de_none", "0", Some(65), vec![("base_year".into(), "2023".into())]);
        let mut late = open("de_none", "0", Some(65), vec![("base_year".into(), "2030".into())]);
        let e = early.draw(&pot(ids::RUERUP, "1000000", "0"), d("30000"), StopAt::Requirement).unwrap();
        let l = late.draw(&pot(ids::RUERUP, "1000000", "0"), d("30000"), StopAt::Requirement).unwrap();
        assert!(l.tax > e.tax, "the later, higher-share cohort pays more: {} vs {}", l.tax, e.tax);
    }

    // --- Vorabpauschale (the periodic charge) ------------------------------

    #[test]
    fn a_rising_fund_is_charged_vorabpauschale_and_a_falling_one_is_not() {
        let mut s = plain("100000"); // allowance already needed elsewhere? no — fresh
        let mut charges = [Decimal::ZERO; 2];
        // Opening 100,000; one rose to 110,000, one fell to 95,000.
        let pots = [
            period_pot(ids::FONDS_AKTIEN, "110000", "100000"),
            period_pot(ids::FONDS_AKTIEN, "95000", "100000"),
        ];
        s.period_charge(&pots, &mut charges).unwrap();
        assert!(charges[0] > Decimal::ZERO, "the risen fund is charged");
        assert_eq!(charges[1], Decimal::ZERO, "the fallen fund is not");
    }

    #[test]
    fn the_vorabpauschale_is_capped_at_the_actual_gain() {
        // Basisertrag = 100,000 · 3.20% · 70% = 2,240 taxable base (before
        // Teilfreistellung). A fund that rose only €500 is charged on €500, not
        // 2,240 — the min bites.
        let mut small = plain("100000");
        let mut big = plain("100000");
        let mut cs = [Decimal::ZERO];
        let mut cb = [Decimal::ZERO];
        small.period_charge(&[period_pot(ids::FONDS_AKTIEN, "100500", "100000")], &mut cs).unwrap();
        big.period_charge(&[period_pot(ids::FONDS_AKTIEN, "150000", "100000")], &mut cb).unwrap();
        assert!(cs[0] < cb[0], "a small gain caps the charge below a large one");
    }

    #[test]
    fn cash_and_pensions_are_never_charged_a_holding_levy() {
        let mut s = plain("0");
        let mut charges = [Decimal::ZERO; 2];
        let pots = [
            period_pot(ids::GIRO, "110000", "100000"),
            period_pot(ids::BAV, "110000", "100000"),
        ];
        s.period_charge(&pots, &mut charges).unwrap();
        assert_eq!(charges, [Decimal::ZERO; 2]);
    }

    // --- allowances, periods, joint ----------------------------------------

    #[test]
    fn allowances_reset_at_the_boundary_and_bank_what_was_left() {
        let mut s = plain("0");
        // A pension draw inside the Grundfreibetrag leaves most of it unclaimed.
        let before = s.unused_allowance();
        assert!(before > d("12000"), "a fresh period has the Grundfreibetrag to spend");
        s.start_period();
        let after = s.unused_allowance();
        assert!(after > before, "an unused period banks its allowance: {after} vs {before}");
    }

    #[test]
    fn joint_assessment_costs_less_than_single_on_the_same_pension() {
        let mut single = open("de_none", "0", Some(65), vec![]);
        let mut joint = open("de_none", "0", Some(65), vec![("filing".into(), "joint".into())]);
        let s = single.draw(&pot(ids::BAV, "1000000", "0"), d("40000"), StopAt::Requirement).unwrap();
        let j = joint.draw(&pot(ids::BAV, "1000000", "0"), d("40000"), StopAt::Requirement).unwrap();
        assert!(j.tax < s.tax, "splitting lowers the tax: {} vs {}", j.tax, s.tax);
    }

    #[test]
    fn church_tax_raises_the_bill_and_de_none_does_not() {
        let mut none = open("de_none", "0", Some(65), vec![]);
        let mut ks9 = open("de_ks9", "0", Some(65), vec![]);
        let a = none.draw(&pot(ids::BAV, "1000000", "0"), d("40000"), StopAt::Requirement).unwrap();
        let b = ks9.draw(&pot(ids::BAV, "1000000", "0"), d("40000"), StopAt::Requirement).unwrap();
        assert!(b.tax > a.tax, "church tax adds to the bill: {} vs {}", b.tax, a.tax);
    }

    // --- stops -------------------------------------------------------------

    #[test]
    fn a_rate_cap_keeps_a_pension_draw_out_of_the_42_percent_zone() {
        let mut s = plain("0");
        let drawn = s
            .draw(&pot(ids::BAV, "5000000", "0"), d("5000000"), StopAt::RateAbove(d("0.42")))
            .unwrap();
        assert!(drawn.rung_limited, "the cap must bite before the whole pot is taken");
    }

    // --- errors ------------------------------------------------------------

    #[test]
    fn a_pension_below_the_access_age_is_reported_not_silently_taken() {
        let mut s = open("de_none", "0", Some(55), vec![]); // under 62
        let err = s
            .draw(&pot(ids::RUERUP, "100000", "0"), d("1000"), StopAt::Requirement)
            .unwrap_err();
        assert_eq!(err.kind, TaxErrorKind::AgeGated);
        assert!(err.message.contains("62"), "the message names the access age");
        // A depot is still fine at 55.
        assert!(s.draw(&pot(ids::DEPOT_AKTIEN, "100000", "50000"), d("1000"), StopAt::Requirement).is_ok());
    }

    #[test]
    fn an_unknown_region_or_account_is_refused() {
        assert_eq!(
            DE.open(&SessionSpec { region: "narnia".into(), ..Default::default() })
                .err()
                .unwrap()
                .kind,
            TaxErrorKind::BadRegion
        );
        let mut s = plain("0");
        assert_eq!(
            s.draw(&pot("isa", "1000", "0"), d("100"), StopAt::Requirement).unwrap_err().kind,
            TaxErrorKind::UnknownAccount
        );
    }

    // --- catalogue & freshness ---------------------------------------------

    #[test]
    fn the_system_advertises_a_coherent_catalogue() {
        let ids: Vec<_> = DE.account_kinds().iter().map(|k| k.id).collect();
        for id in DE.conventional_order() {
            assert!(ids.contains(id), "'{id}' is ordered but not advertised");
        }
        assert_eq!(ids.len(), DE.conventional_order().len());
        assert!(!DE.regions().is_empty());
        // The default (blank-picker) kind is the untaxed one.
        assert_eq!(DE.default_account_kind().map(|k| k.id), Some(ids::GIRO));
    }

    #[test]
    fn figures_are_fresh_in_their_year_and_stale_after_it() {
        assert_eq!(DE.staleness(SimpleDate::new(2026, 12, 31)), Staleness::Fresh);
        match DE.staleness(SimpleDate::new(2028, 1, 1)) {
            Staleness::Stale { current_period } => assert_eq!(current_period, "2028"),
            Staleness::Fresh => panic!("a later year must read stale"),
        }
    }
}
