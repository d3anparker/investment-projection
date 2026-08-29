//! Turning the figures in [`super::tables`] into a working tax system.
//!
//! This is where UK policy becomes arithmetic. It is the half of the crate that
//! changes when the tax *system* changes rather than when its rates do -- a new
//! band structure, a different taper mechanic, a new kind of account. Rate
//! updates should never need to come here.
//!
//! # Three schedules
//!
//! Every account resolves to a [`taxkit::ladder::Ladder`]: a monotone series of
//! rungs, each with a rate and a taxable fraction. Building them is the whole
//! job.
//!
//! * **Untaxed** -- one open rung at zero. ISAs, Premium Bonds, VCT/EIS.
//! * **Income** -- the jurisdiction's bands, with the personal allowance
//!   *flattened into the ladder* (see below). Pensions add a tax-free fraction.
//! * **Capital gains** -- the annual exempt amount, then 18% for whatever is
//!   left of the basic-rate band, then 24%.
//!
//! # The taper is data, not an adjustment
//!
//! Above £100,000 the personal allowance is withdrawn at £1 per £2, which makes
//! the true marginal rate 60% in that stretch. Computing that as a correction
//! is circular with grossing up -- you would need the total before you could
//! work out the allowance that determines the total. Expressing it as an extra
//! 60% rung instead is algebraically identical, keeps the ladder monotone, and
//! means the walker needs no special case. The same trick handles Scotland's
//! six bands with no code change: it is simply a longer slice.
//!
//! # Pensions
//!
//! Tax-free cash is modelled as *phased*: each withdrawal is a quarter tax free
//! and three quarters income, up to the lifetime lump sum allowance, after which
//! it is wholly income. So the tax-free quarter is the ladder's `leak`, and the
//! allowance running out is just another rung boundary.
//!
//! Taking 25% of the whole pot up front is deliberately **not** offered. It
//! would be a single enormous outflow leaving the portfolio, and this model has
//! no concept of cash held outside the portfolio -- so it would render as a
//! cliff-edge crash rather than as money in a bank account.

use rust_decimal::Decimal;
use taxkit::ladder::{Ladder, Rung};
use taxkit::{
    AccountKind, Draw, Pot, Region, SessionSpec, SimpleDate, Staleness, StopAt, TaxError,
    TaxErrorKind, TaxSession, TaxSystem,
};

use crate::tables::{
    self, tax_year_label, uk_tax_year_of, Band, TaxJurisdiction, TaxYear, WithdrawalTax,
};

/// Most rate changes the schedule can produce: one segment per band, plus the
/// personal allowance at the foot and the stretch where it is withdrawn.
const MAX_SEGMENTS: usize = 12;

/// Months after `as_of` beyond which the figures are called stale even if the
/// tax year has not turned over. Guards against a table that was written a year
/// late as well as one that has simply aged.
const STALE_AFTER_MONTHS: i32 = 18;

/// The United Kingdom.
#[derive(Clone, Copy, Debug, Default)]
pub struct UkTaxSystem;

/// The instance to wire in: `&uktax::UK`.
pub static UK: UkTaxSystem = UkTaxSystem;

const REGIONS: &[Region] = &[
    Region { id: TaxJurisdiction::ID_ENGLAND, label: "England" },
    Region { id: TaxJurisdiction::ID_WALES, label: "Wales" },
    Region { id: TaxJurisdiction::ID_SCOTLAND, label: "Scotland" },
    Region { id: TaxJurisdiction::ID_NORTHERN_IRELAND, label: "Northern Ireland" },
];

fn gbp(pounds: i64) -> Decimal {
    Decimal::from(pounds)
}

fn rate(basis_points: u32) -> Decimal {
    Decimal::new(i64::from(basis_points), 4)
}

impl TaxSystem for UkTaxSystem {
    fn label(&self) -> &'static str {
        "United Kingdom"
    }

    fn currency_symbol(&self) -> &'static str {
        "\u{a3}"
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        tables::UK_ACCOUNTS
    }

    fn regions(&self) -> &'static [Region] {
        REGIONS
    }

    fn conventional_order(&self) -> &'static [&'static str] {
        tables::UK_CONVENTIONAL_ORDER
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
        let now = uk_tax_year_of(today);
        let then = uk_tax_year_of(checked);
        let aged = checked.months_until(today) > STALE_AFTER_MONTHS;
        if now > then || aged {
            Staleness::Stale {
                current_period: tax_year_label(now),
            }
        } else {
            Staleness::Fresh
        }
    }

    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn TaxSession>, TaxError> {
        let jurisdiction = TaxJurisdiction::from_id(&spec.region).ok_or_else(|| {
            TaxError::new(
                TaxErrorKind::BadRegion,
                format!("'{}' is not a part of the UK this calculator knows.", spec.region),
            )
        })?;
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
        let mut session = UkSession {
            rules: tables::LATEST,
            jurisdiction,
            other_income: spec.other_income,
            age: spec.age,
            uprate: spec.uprate,
            scale: Decimal::ONE,
            segments: [(None, Decimal::ZERO); MAX_SEGMENTS],
            segment_count: 0,
            income: spec.other_income,
            gains: Decimal::ZERO,
            pcls_taken: Decimal::ZERO,
            period_tax: Decimal::ZERO,
            banked_unused: Decimal::ZERO,
        };
        session.rebuild_segments();
        Ok(Box::new(session))
    }
}

/// One projection's UK tax state.
struct UkSession {
    rules: &'static TaxYear,
    jurisdiction: TaxJurisdiction,
    other_income: Decimal,
    age: Option<u32>,
    uprate: Decimal,

    /// Compounded threshold uprating for the current period. `1` when frozen.
    scale: Decimal,
    /// The flattened income schedule for the current thresholds, rebuilt only
    /// when those change. See `rebuild_segments`.
    segments: [(Option<Decimal>, Decimal); MAX_SEGMENTS],
    segment_count: usize,
    /// Taxable income booked this period, before any allowance.
    income: Decimal,
    /// Chargeable gains booked this period, before the exempt amount.
    gains: Decimal,
    /// Tax-free pension cash taken **for life**: the lump sum allowance is not
    /// an annual allowance, and resetting it would hand the holder a fresh
    /// £268,275 every year.
    pcls_taken: Decimal,
    period_tax: Decimal,
    banked_unused: Decimal,
}

impl UkSession {
    /// A threshold, uprated for the current period.
    fn t(&self, pounds: i64) -> Decimal {
        if self.scale == Decimal::ONE {
            gbp(pounds)
        } else {
            gbp(pounds) * self.scale
        }
    }

    /// Pounds of income that withdraw one pound of personal allowance. Guarded
    /// against a zero in the tables, which would be a division by zero.
    fn taper_divisor(&self) -> Decimal {
        Decimal::from(self.rules.pa_taper_divisor.max(1))
    }

    /// The personal allowance actually available at a given income, after the
    /// taper. Used for reporting and for the capital gains pivot; the income
    /// schedule itself expresses the taper as a rung instead.
    fn allowance_at(&self, income: Decimal) -> Decimal {
        let pa = self.t(self.rules.personal_allowance_gbp);
        let threshold = self.t(self.rules.pa_taper_threshold_gbp);
        if income <= threshold {
            return pa;
        }
        let lost = (income - threshold) / self.taper_divisor();
        (pa - lost).max(Decimal::ZERO)
    }

    /// Rebuild the flattened income schedule for the current thresholds.
    ///
    /// This is the heart of the crate: the personal allowance becomes a 0%
    /// segment at the foot, and its withdrawal becomes a higher-rate segment in
    /// the middle, so the marginal rate is a genuine step function of income.
    ///
    /// Depends only on the jurisdiction and the uprating scale, so it is
    /// computed once per tax period rather than per withdrawal -- a goal-seek
    /// runs tens of thousands of draws, and rebuilding this each time was the
    /// one place in the crate that would allocate in a hot loop.
    fn rebuild_segments(&mut self) {
        let (segments, count) = self.compute_segments();
        self.segments = segments;
        self.segment_count = count;
    }

    /// Split out from `rebuild_segments` so the whole computation can borrow
    /// `self` immutably -- the rate probe below closes over `self`, and it
    /// cannot stay alive across writes to `self.segments`.
    fn compute_segments(&self) -> ([(Option<Decimal>, Decimal); MAX_SEGMENTS], usize) {
        let bands: &[Band] = self.rules.bands_for(self.jurisdiction);
        let pa = self.t(self.rules.personal_allowance_gbp);
        let taper_from = self.t(self.rules.pa_taper_threshold_gbp);
        let divisor = self.taper_divisor();
        let taper_to = taper_from + pa * divisor;
        // Each extra pound of income in the taper stretch also exposes
        // `1/divisor` of a pound that the allowance used to cover.
        let taper_mult = Decimal::ONE + Decimal::ONE / divisor;

        // Every point at which the rate can change, in gross income space.
        let mut cuts = [Decimal::ZERO; MAX_SEGMENTS];
        let mut n = 0usize;
        let add = |v: Decimal, cuts: &mut [Decimal; MAX_SEGMENTS], n: &mut usize| {
            // Dropping a cut point would not error, it would produce a
            // plausible but wrong schedule -- exactly the quiet failure
            // `Ladder::push` refuses to allow. Today's widest schedule
            // (Scotland) needs eight, so this only fires on a table update
            // that outgrows the buffer, and it should fire loudly.
            debug_assert!(
                *n < MAX_SEGMENTS,
                "more rate changes than MAX_SEGMENTS holds; raise it to match the tables",
            );
            if v > Decimal::ZERO && *n < MAX_SEGMENTS {
                cuts[*n] = v;
                *n += 1;
            }
        };
        add(pa, &mut cuts, &mut n);
        for b in bands.iter().skip(1) {
            add(pa + gbp(b.from_gbp) * self.scale, &mut cuts, &mut n);
        }
        if pa > Decimal::ZERO {
            add(taper_from, &mut cuts, &mut n);
            add(taper_to, &mut cuts, &mut n);
        }
        let cuts = &mut cuts[..n];
        cuts.sort();

        // Statutory rate at a given gross income, before the taper is folded in.
        let base_rate = |gross: Decimal| -> Decimal {
            let taxable = (gross - self.allowance_at(gross)).max(Decimal::ZERO);
            let mut r = rate(bands[0].rate_bp);
            for b in bands {
                if taxable >= gbp(b.from_gbp) * self.scale {
                    r = rate(b.rate_bp);
                }
            }
            r
        };

        let mut segments = [(None, Decimal::ZERO); MAX_SEGMENTS];
        let mut count = 0usize;
        let mut lower = Decimal::ZERO;
        for cut in cuts.iter() {
            // Always leave room for the open-ended final segment.
            debug_assert!(
                *cut <= lower || count + 1 < MAX_SEGMENTS,
                "more segments than MAX_SEGMENTS holds; raise it to match the tables",
            );
            if *cut > lower && count + 1 < MAX_SEGMENTS {
                let r = self.rate_in(lower, *cut, taper_from, taper_to, taper_mult, &base_rate);
                segments[count] = (Some(*cut), r);
                count += 1;
                lower = *cut;
            }
        }
        let r = self.rate_in(
            lower,
            lower + Decimal::ONE,
            taper_from,
            taper_to,
            taper_mult,
            &base_rate,
        );
        segments[count] = (None, r);
        count += 1;
        (segments, count)
    }

    #[allow(clippy::too_many_arguments)]
    fn rate_in(
        &self,
        lower: Decimal,
        upper: Decimal,
        taper_from: Decimal,
        taper_to: Decimal,
        taper_mult: Decimal,
        base_rate: &dyn Fn(Decimal) -> Decimal,
    ) -> Decimal {
        // Probe just inside the segment; the schedule is constant across it.
        let probe = lower + (upper - lower) / Decimal::TWO;
        let r = if lower < self.t(self.rules.personal_allowance_gbp) {
            Decimal::ZERO
        } else {
            base_rate(probe)
        };
        if lower >= taper_from && upper <= taper_to {
            r * taper_mult
        } else {
            r
        }
    }

    /// Rungs for fully taxable income, positioned at the income booked so far.
    ///
    /// Segments already consumed come through with zero headroom rather than
    /// being filtered out; the walker skips them, and keeping the shape fixed
    /// keeps this loop free of special cases.
    fn income_ladder(&self) -> Result<Ladder, TaxError> {
        // Fully taxable. A tax-free fraction is applied afterwards by
        // `with_tax_free_fraction`, never through this ladder.
        let leak = Decimal::ONE;
        let mut l = Ladder::new();
        let mut lower = Decimal::ZERO;
        for (upper, r) in &self.segments[..self.segment_count] {
            match upper {
                Some(u) => {
                    // Headroom left in this segment, given what is already booked.
                    let head = (*u - self.income.max(lower)).max(Decimal::ZERO);
                    l.push(Rung { headroom: Some(head), rate: *r, leak })?;
                    lower = *u;
                }
                None => {
                    l.push(Rung { headroom: None, rate: *r, leak })?;
                }
            }
        }
        Ok(l)
    }

    /// Split an income ladder where the lifetime tax-free allowance runs out:
    /// below the breakpoint a fraction comes out free, above it nothing does.
    fn with_tax_free_fraction(&self, base: &Ladder) -> Result<Ladder, TaxError> {
        let free_fraction = rate(self.rules.pcls_bp);
        let leak = Decimal::ONE - free_fraction;
        let remaining = (self.t(self.rules.lump_sum_allowance_gbp) - self.pcls_taken).max(Decimal::ZERO);
        // Gross withdrawal that would exhaust the remaining allowance.
        let breakpoint = if free_fraction.is_zero() {
            None
        } else {
            Some(remaining.checked_div(free_fraction).ok_or_else(TaxError::overflow)?)
        };

        let mut out = Ladder::new();
        let mut gross_so_far = Decimal::ZERO;
        let mut exhausted = breakpoint.is_some_and(|b| b <= Decimal::ZERO);

        for r in base.rungs() {
            let this_leak = if exhausted { Decimal::ONE } else { leak };
            match r.headroom {
                None => {
                    out.push(Rung { headroom: None, rate: r.rate, leak: this_leak })?;
                }
                Some(h) => {
                    // Gross needed to fill this rung's taxable headroom.
                    let gross_needed = h.checked_div(this_leak).ok_or_else(TaxError::overflow)?;
                    match breakpoint {
                        Some(b) if !exhausted && gross_so_far + gross_needed > b => {
                            // The allowance runs out inside this rung: split it.
                            let first_gross = b - gross_so_far;
                            let first_taxable = first_gross * leak;
                            out.push(Rung { headroom: Some(first_taxable), rate: r.rate, leak })?;
                            out.push(Rung {
                                headroom: Some(h - first_taxable),
                                rate: r.rate,
                                leak: Decimal::ONE,
                            })?;
                            exhausted = true;
                            gross_so_far = b + (h - first_taxable);
                        }
                        _ => {
                            out.push(Rung { headroom: Some(h), rate: r.rate, leak: this_leak })?;
                            gross_so_far += gross_needed;
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Rungs for a chargeable gain: the exempt amount, then the two rates,
    /// pivoting on whatever is left of the basic-rate band.
    fn gains_ladder(&self, pot: &Pot) -> Result<Ladder, TaxError> {
        // Proportional disposal, floored at zero for a holding at a loss --
        // losses are not carried here. See `Pot::proportional_leak`.
        let leak = pot.proportional_leak();

        let exempt = self.exempt_remaining();
        let taxable_income = (self.income - self.allowance_at(self.income)).max(Decimal::ZERO);
        let basic_room =
            (self.t(self.rules.basic_rate_limit_gbp) - taxable_income).max(Decimal::ZERO);

        let mut l = Ladder::new();
        l.push(Rung { headroom: Some(exempt), rate: Decimal::ZERO, leak })?;
        l.push(Rung {
            headroom: Some(basic_room),
            rate: rate(self.rules.cgt_rate_basic_bp),
            leak,
        })?;
        l.push(Rung {
            headroom: None,
            rate: rate(self.rules.cgt_rate_higher_bp),
            leak,
        })?;
        Ok(l)
    }

    fn check_age(&self, kind: &AccountKind) -> Result<(), TaxError> {
        let min = u32::from(self.rules.normal_minimum_pension_age);
        if kind.age_gated {
            if let Some(age) = self.age {
                if age < min {
                    return Err(TaxError::new(
                        TaxErrorKind::AgeGated,
                        format!(
                            "A {} cannot normally be accessed before {min}, and this drawdown \
                             starts at {age}.",
                            kind.label.to_lowercase()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Look the pot's kind up in the catalogue and in the treatment table, and
    /// check the holder is old enough to touch it.
    ///
    /// One place, because `draw` needs the treatment too: resolving it twice
    /// per draw meant two linear scans of the tables on the per-month path, and
    /// two `WithdrawalTax` matches that had to be kept in step by hand.
    fn resolve(&self, pot: &Pot) -> Result<WithdrawalTax, TaxError> {
        let kind = UK.account_kind(pot.kind).ok_or_else(|| {
            TaxError::new(
                TaxErrorKind::UnknownAccount,
                format!("'{}' is not an account this calculator knows.", pot.kind),
            )
        })?;
        self.check_age(kind)?;
        tables::treatment_of(pot.kind).ok_or_else(|| {
            TaxError::new(
                TaxErrorKind::BadRules,
                format!("'{}' has no withdrawal treatment in the tax tables.", pot.kind),
            )
        })
    }

    fn ladder_for(&self, pot: &Pot) -> Result<(Ladder, WithdrawalTax), TaxError> {
        let treatment = self.resolve(pot)?;
        let ladder = match treatment {
            WithdrawalTax::None => Ladder::untaxed(),
            WithdrawalTax::IncomeOnly => self.income_ladder()?,
            WithdrawalTax::IncomeWithTaxFreeFraction => {
                let base = self.income_ladder()?;
                self.with_tax_free_fraction(&base)?
            }
            WithdrawalTax::CapitalGains => self.gains_ladder(pot)?,
            // An unmodelled wrapper is not an error -- it is a real thing to
            // hold. Treating it as untaxed would flatter it, so callers are
            // expected to exclude it from tax-ordered strategies and say so;
            // here it simply costs nothing extra to take money out of.
            WithdrawalTax::NotModelled => Ladder::untaxed(),
        };
        Ok((ladder, treatment))
    }

    /// Capital gains exempt amount still available this period.
    fn exempt_remaining(&self) -> Decimal {
        (self.t(self.rules.cgt_annual_exempt_gbp) - self.gains).max(Decimal::ZERO)
    }

    fn unused_now(&self) -> Decimal {
        let pa = (self.allowance_at(self.income) - self.income).max(Decimal::ZERO);
        pa + self.exempt_remaining()
    }
}

impl TaxSession for UkSession {
    fn period_months(&self) -> u32 {
        12
    }

    fn start_period(&mut self) {
        self.banked_unused += self.unused_now();
        if !self.uprate.is_zero() {
            self.scale *= Decimal::ONE + self.uprate;
            self.rebuild_segments();
        }
        // Other income arrives whether or not the portfolio is touched, and is
        // not pro-rated, so the first pound withdrawn in month one is marginal
        // on top of the whole year's worth.
        self.income = self.other_income;
        self.gains = Decimal::ZERO;
        self.period_tax = Decimal::ZERO;
    }

    fn marginal_keep(&self, pot: &Pot) -> Decimal {
        self.ladder_for(pot)
            .map(|(l, _)| l.marginal_keep())
            .unwrap_or(Decimal::ZERO)
    }

    fn marginal_headroom(&self, pot: &Pot) -> Option<Decimal> {
        self.ladder_for(pot).ok().and_then(|(l, _)| l.marginal_headroom())
    }

    /// Both figures off a single ladder build. `ladder_for` is the crate's one
    /// hot allocation (a linear scan of the tables plus, for a pension, the
    /// tax-free-fraction split), and a cheapest-first ranking asks for both the
    /// keep and the headroom of every holding every pass — so building the
    /// ladder once here rather than once per figure roughly halves that cost.
    fn marginal(&self, pot: &Pot) -> (Decimal, Option<Decimal>) {
        match self.ladder_for(pot) {
            Ok((l, _)) => (l.marginal_keep(), l.marginal_headroom()),
            Err(_) => (Decimal::ZERO, None),
        }
    }

    fn draw(&mut self, pot: &Pot, net_wanted: Decimal, stop: StopAt) -> Result<Draw, TaxError> {
        let (ladder, treatment) = self.ladder_for(pot)?;
        let walk = ladder.walk(pot.available, net_wanted, stop)?;

        match treatment {
            WithdrawalTax::IncomeOnly => self.income += walk.taxable,
            WithdrawalTax::IncomeWithTaxFreeFraction => {
                self.income += walk.taxable;
                // Whatever came out that was not taxable was tax-free cash, and
                // it counts against the lifetime allowance. Deriving it this way
                // stays exact even when a single draw straddles the point at
                // which the allowance runs out.
                self.pcls_taken += walk.draw.gross - walk.taxable;
            }
            WithdrawalTax::CapitalGains => self.gains += walk.taxable,
            WithdrawalTax::None | WithdrawalTax::NotModelled => {}
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
    use crate::tables::ids;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn open(region: &str, other_income: &str, age: Option<u32>) -> Box<dyn TaxSession> {
        UK.open(&SessionSpec {
            region: region.into(),
            other_income: d(other_income),
            age,
            uprate: Decimal::ZERO,
        })
        .unwrap()
    }

    fn england(other_income: &str) -> Box<dyn TaxSession> {
        open(TaxJurisdiction::ID_ENGLAND, other_income, Some(60))
    }

    fn pot(kind: &'static str, available: &str, basis: &str) -> Pot {
        Pot { kind, available: d(available), cost_basis: d(basis) }
    }

    // --- income -------------------------------------------------------------

    #[test]
    fn a_pension_draw_is_grossed_up_across_the_basic_rate_band() {
        // The worked example: with the lump sum allowance already spent a
        // pension is fully taxable, so £40,000 net needs the free 12,570 plus
        // 27,430 grossed up by 1/0.8.
        let mut s = england("0");
        // Spend the lump sum allowance so the tax-free quarter is out of play.
        s.draw(&pot(ids::SIPP, "10000000", "0"), d("1073100"), StopAt::Requirement)
            .unwrap();
        s.start_period();

        let dr = s
            .draw(&pot(ids::SIPP, "10000000", "0"), d("40000"), StopAt::Requirement)
            .unwrap();
        assert_eq!(dr.gross.round_dp(2), d("46857.50"));
        assert_eq!(dr.tax.round_dp(2), d("6857.50"));
    }

    #[test]
    fn a_pension_gives_a_quarter_tax_free_while_the_allowance_lasts() {
        let mut s = england("0");
        // 1,000 gross: 250 tax free, 750 within the personal allowance.
        let dr = s
            .draw(&pot(ids::SIPP, "500000", "0"), d("1000"), StopAt::Requirement)
            .unwrap();
        assert_eq!(dr.tax, Decimal::ZERO);
        assert_eq!(dr.gross, d("1000"));
        // Only three quarters counted as income, so the allowance has 12,570 -
        // 750 left. Drawing the rest of it should still cost nothing.
        let dr2 = s
            .draw(&pot(ids::SIPP, "500000", "0"), d("15760"), StopAt::Requirement)
            .unwrap();
        assert_eq!(dr2.tax, Decimal::ZERO, "12,570 of allowance covers 16,760 gross");
    }

    #[test]
    fn other_income_is_charged_in_full_from_the_first_month() {
        // Other income is not pro-rated across the year: the whole year's worth
        // is already on the ledger when month one is drawn, so a withdrawal is
        // marginal on top of all of it. A defined benefit pension is used here
        // because it is fully taxable, with no tax-free fraction to muddy it.
        let p = pot(ids::DEFINED_BENEFIT, "100000", "0");
        assert_eq!(
            england("0").marginal_keep(&p),
            Decimal::ONE,
            "the allowance is untouched"
        );
        assert_eq!(
            england("12570").marginal_keep(&p),
            d("0.8"),
            "other income has already spent the whole allowance"
        );
    }

    #[test]
    fn the_withdrawn_allowance_shows_up_as_a_sixty_percent_rung() {
        // Someone already on 100,000 of other income: the next pound of pension
        // costs 40% plus the allowance being clawed back, i.e. 60%.
        let s = england("100000");
        assert_eq!(
            s.marginal_keep(&pot(ids::DEFINED_BENEFIT, "100000", "0")),
            d("0.4"),
            "the 60% trap must appear as a real marginal rate"
        );
    }

    #[test]
    fn above_the_taper_the_additional_rate_applies_not_sixty_percent() {
        let s = england("130000");
        assert_eq!(s.marginal_keep(&pot(ids::DEFINED_BENEFIT, "10000", "0")), d("0.55"));
    }

    #[test]
    fn scotland_has_its_own_bands_and_england_does_not_change() {
        // 30,000 of other income: Scotland charges 21% at the margin here,
        // England 20%.
        let scot = open(TaxJurisdiction::ID_SCOTLAND, "30000", Some(60));
        let eng = open(TaxJurisdiction::ID_ENGLAND, "30000", Some(60));
        let p = pot(ids::DEFINED_BENEFIT, "10000", "0");
        assert_eq!(scot.marginal_keep(&p), d("0.79"));
        assert_eq!(eng.marginal_keep(&p), d("0.80"));
    }

    #[test]
    fn wales_and_northern_ireland_track_england_for_now() {
        let p = pot(ids::DEFINED_BENEFIT, "10000", "0");
        let eng = open(TaxJurisdiction::ID_ENGLAND, "30000", Some(60)).marginal_keep(&p);
        for id in [TaxJurisdiction::ID_WALES, TaxJurisdiction::ID_NORTHERN_IRELAND] {
            assert_eq!(open(id, "30000", Some(60)).marginal_keep(&p), eng, "{id}");
        }
    }

    // --- capital gains ------------------------------------------------------

    #[test]
    fn only_the_gain_is_taxed_and_the_exempt_amount_comes_first() {
        let mut s = england("0");
        // Half the holding is profit.
        let p = pot(ids::GIA, "20000", "10000");
        assert_eq!(s.marginal_keep(&p), Decimal::ONE, "the exempt amount is free");
        // 6,000 gross realises 3,000 of gain, exactly the exempt amount.
        let dr = s.draw(&p, d("6000"), StopAt::Requirement).unwrap();
        assert_eq!(dr.tax, Decimal::ZERO);
        // Now 18% on half of each pound: 9% effective.
        assert_eq!(s.marginal_keep(&p), d("0.91"));
    }

    #[test]
    fn gains_above_the_basic_band_pay_the_higher_rate() {
        // Other income already past the basic-rate limit, so gains are at 24%.
        let mut s = england("60000");
        let p = pot(ids::GIA, "100000", "0"); // wholly gain
        s.draw(&p, d("3000"), StopAt::Requirement).unwrap(); // spend the exempt amount
        assert_eq!(s.marginal_keep(&p), d("0.76"));
    }

    #[test]
    fn a_holding_with_no_gain_costs_nothing_to_sell() {
        let s = england("60000");
        assert_eq!(s.marginal_keep(&pot(ids::GIA, "10000", "10000")), Decimal::ONE);
        // A loss is not a negative tax; it is simply nothing to pay.
        assert_eq!(s.marginal_keep(&pot(ids::GIA, "10000", "18000")), Decimal::ONE);
    }

    // --- wrappers -----------------------------------------------------------

    #[test]
    fn isas_are_free_and_stay_free_however_much_is_taken() {
        let mut s = england("200000");
        for id in [ids::STOCKS_ISA, ids::CASH_ISA, ids::LIFETIME_ISA, ids::PREMIUM_BONDS] {
            let dr = s.draw(&pot(id, "500000", "0"), d("100000"), StopAt::Requirement).unwrap();
            assert_eq!(dr.tax, Decimal::ZERO, "{id} should never be taxed on the way out");
            assert_eq!(dr.net, d("100000"), "{id}");
        }
    }

    // --- the lifetime allowance --------------------------------------------

    #[test]
    fn tax_free_cash_stops_at_the_lifetime_allowance() {
        let sipp_pot = pot(ids::SIPP, "5000000", "0");
        let db_pot = pot(ids::DEFINED_BENEFIT, "5000000", "0");

        let mut spent = england("0");
        // Far more than the lifetime allowance, so the tax-free quarter is gone.
        let cap = Decimal::from(tables::LATEST.lump_sum_allowance_gbp);
        spent
            .draw(&sipp_pot, cap * Decimal::from(10), StopAt::Requirement)
            .unwrap();
        spent.start_period();

        // A defined benefit pension is fully taxable by definition. Once the
        // lifetime allowance is spent, a SIPP must behave *identically* -- that
        // is what "no more tax-free cash" means, and asserting it this way beats
        // hand-computing a withdrawal that crosses four bands.
        let mut db = england("0");
        let after = spent.draw(&sipp_pot, d("100000"), StopAt::Requirement).unwrap();
        let baseline = db.draw(&db_pot, d("100000"), StopAt::Requirement).unwrap();
        assert_eq!(after.gross, baseline.gross, "no quarter is coming out free any more");
        assert_eq!(after.tax, baseline.tax);

        // And the allowance is worth something while it lasts.
        let mut fresh = england("0");
        let intact = fresh.draw(&sipp_pot, d("100000"), StopAt::Requirement).unwrap();
        assert!(intact.tax < baseline.tax, "the tax-free quarter must reduce the bill");
    }

    #[test]
    fn the_annual_allowance_resets_but_the_lifetime_one_does_not() {
        let p = pot(ids::SIPP, "5000000", "0");
        let mut s = england("0");
        let before = s.draw(&p, d("40000"), StopAt::Requirement).unwrap();
        s.start_period();
        let after = s.draw(&p, d("40000"), StopAt::Requirement).unwrap();
        assert_eq!(before.gross, after.gross, "a new year restores the personal allowance");

        // Meanwhile the lifetime allowance is being consumed and never restored,
        // so a session that has already spent it pays more for the same money.
        let mut heavy = england("0");
        let cap = Decimal::from(tables::LATEST.lump_sum_allowance_gbp);
        heavy.draw(&p, cap * Decimal::from(10), StopAt::Requirement).unwrap();
        heavy.start_period();
        let later = heavy.draw(&p, d("40000"), StopAt::Requirement).unwrap();
        assert!(
            later.tax > after.tax,
            "the lump sum allowance must not come back with the tax year"
        );
    }

    // --- periods ------------------------------------------------------------

    #[test]
    fn allowances_reset_at_the_boundary_and_only_there() {
        let mut s = england("0");
        let p = pot(ids::DEFINED_BENEFIT, "1000000", "0");
        s.draw(&p, d("12570"), StopAt::Requirement).unwrap();
        assert_eq!(s.marginal_keep(&p), d("0.8"), "allowance spent");
        s.start_period();
        assert_eq!(s.marginal_keep(&p), Decimal::ONE, "and restored");
        assert_eq!(s.period_tax(), Decimal::ZERO);
    }

    #[test]
    fn twelve_monthly_draws_cost_the_same_as_one_annual_draw() {
        // The telescoping property: tax is a difference of a cumulative
        // function, so slicing a year's withdrawal into months cannot change
        // the total. This is what makes a month-by-month engine agree with an
        // annual tax calculation.
        let mut monthly = england("0");
        let p = pot(ids::DEFINED_BENEFIT, "1000000", "0");
        let mut total = Decimal::ZERO;
        for _ in 0..12 {
            total += monthly.draw(&p, d("2000"), StopAt::Requirement).unwrap().tax;
        }
        let mut annual = england("0");
        let once = annual.draw(&p, d("24000"), StopAt::Requirement).unwrap();
        assert_eq!(total.round_dp(2), once.tax.round_dp(2));
    }

    #[test]
    fn unused_allowance_banks_what_was_left_on_the_table() {
        let mut s = england("0");
        let full = Decimal::from(
            tables::LATEST.personal_allowance_gbp + tables::LATEST.cgt_annual_exempt_gbp,
        );
        assert_eq!(s.unused_allowance(), full);
        s.start_period();
        assert_eq!(s.unused_allowance(), full * Decimal::TWO, "two wasted years");

        // Using the allowance means there is less of it left unused.
        let mut used = england("0");
        used.draw(&pot(ids::DEFINED_BENEFIT, "100000", "0"), d("12570"), StopAt::Requirement)
            .unwrap();
        assert_eq!(
            used.unused_allowance(),
            Decimal::from(tables::LATEST.cgt_annual_exempt_gbp),
            "the personal allowance was fully claimed"
        );
    }

    #[test]
    fn uprating_lifts_the_thresholds_each_period() {
        let mut s = UK
            .open(&SessionSpec {
                region: TaxJurisdiction::ID_ENGLAND.into(),
                other_income: Decimal::ZERO,
                age: Some(60),
                uprate: d("0.1"),
                })
            .unwrap();
        let p = pot(ids::DEFINED_BENEFIT, "1000000", "0");
        // First period: the ordinary allowance.
        let first = s.draw(&p, d("12570"), StopAt::Requirement).unwrap();
        assert_eq!(first.tax, Decimal::ZERO);
        s.start_period();
        // Second period: 10% more allowance, so 13,827 is still free.
        let second = s.draw(&p, d("13827"), StopAt::Requirement).unwrap();
        assert_eq!(second.tax, Decimal::ZERO, "thresholds should have risen");
    }

    // --- stops --------------------------------------------------------------

    #[test]
    fn next_rung_stops_at_the_top_of_the_allowance() {
        let mut s = england("0");
        let dr = s
            .draw(&pot(ids::DEFINED_BENEFIT, "1000000", "0"), d("100000"), StopAt::NextRung)
            .unwrap();
        assert_eq!(dr.gross, d("12570"));
        assert!(dr.rung_limited);
    }

    #[test]
    fn a_rate_cap_keeps_a_draw_out_of_the_higher_band() {
        let mut s = england("0");
        let dr = s
            .draw(
                &pot(ids::DEFINED_BENEFIT, "1000000", "0"),
                d("100000"),
                StopAt::RateAbove(d("0.2")),
            )
            .unwrap();
        // Allowance plus the whole basic-rate band, and no further.
        assert_eq!(dr.gross, d("50270"));
        assert!(dr.rung_limited, "it stopped because of the cap, and says so");
    }

    // --- errors -------------------------------------------------------------

    #[test]
    fn a_pension_below_the_access_age_is_reported_not_silently_taken() {
        let mut s = open(TaxJurisdiction::ID_ENGLAND, "0", Some(50));
        let err = s
            .draw(&pot(ids::SIPP, "100000", "0"), d("100"), StopAt::Requirement)
            .unwrap_err();
        assert_eq!(err.kind, TaxErrorKind::AgeGated);
        assert!(err.message.contains("55"), "the message names the age: {}", err.message);
        // An ISA is fine at any age.
        assert!(s.draw(&pot(ids::STOCKS_ISA, "100", "0"), d("50"), StopAt::Requirement).is_ok());
    }

    #[test]
    fn an_unknown_region_or_account_is_refused() {
        assert_eq!(
            UK.open(&SessionSpec {
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
        assert_eq!(
            UK.open(&SessionSpec {
                region: TaxJurisdiction::ID_ENGLAND.into(),
                other_income: d("-1"),
                age: None,
                uprate: Decimal::ZERO,
            })
            .err()
            .expect("this should be refused")
            .kind,
            TaxErrorKind::BadOtherIncome
        );
        let mut s = england("0");
        assert_eq!(
            s.draw(&pot("nonsense", "1", "0"), d("1"), StopAt::Requirement)
                .unwrap_err()
                .kind,
            TaxErrorKind::UnknownAccount
        );
    }

    // --- freshness ----------------------------------------------------------

    #[test]
    fn figures_are_fresh_within_their_own_tax_year() {
        let checked = tables::LATEST.as_of;
        assert_eq!(UK.staleness(checked), Staleness::Fresh);
        assert_eq!(UK.staleness(SimpleDate::new(2027, 4, 5)), Staleness::Fresh);
    }

    #[test]
    fn a_new_tax_year_makes_the_figures_stale_and_names_the_current_one() {
        match UK.staleness(SimpleDate::new(2027, 4, 6)) {
            Staleness::Stale { current_period } => assert_eq!(current_period, "2027/28"),
            Staleness::Fresh => panic!("6 April starts a new tax year, so these are stale"),
        }
    }

    #[test]
    fn the_system_advertises_a_coherent_catalogue() {
        assert!(!UK.regions().is_empty());
        for id in UK.conventional_order() {
            assert!(UK.account_kind(id).is_some(), "'{id}' is ordered but not advertised");
        }
        for k in UK.account_kinds() {
            assert!(!k.label.is_empty() && !k.short_label.is_empty(), "'{}'", k.id);
        }
        for r in UK.regions() {
            assert!(TaxJurisdiction::from_id(r.id).is_some(), "'{}' must resolve", r.id);
        }
    }
}
