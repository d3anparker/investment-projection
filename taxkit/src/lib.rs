//! The abstraction between a projection engine and the tax system it is
//! working with.
//!
//! Nothing in this crate names a country, a currency, an account type or an
//! allowance. If a reviewer can tell which jurisdiction it is for, it is wrong.
//! A tax system is a [`TaxSystem`] implementation living in its own crate (see
//! the `uk-tax` crate); the projection engine (`calc`) and the UI (`app`) are
//! written against these traits and never against a concrete one.
//!
//! # The shape of the contract
//!
//! A tax system advertises a catalogue of [`AccountKind`]s and a set of
//! [`Region`]s, and opens a [`TaxSession`] for one projection. The session
//! carries a ledger -- amounts accumulated within the current tax period, and
//! amounts accumulated for life -- and answers the two questions a caller
//! actually needs:
//!
//! * [`TaxSession::marginal_keep`] -- what fraction of the next unit withdrawn
//!   from this account would survive tax, *right now*. A sort key.
//! * [`TaxSession::draw`] -- take money out, and tell me what it cost.
//!
//! That is deliberately the whole interface. A caller can order withdrawals by
//! cost, stop at a rate boundary, or cap the rate it is willing to pay, without
//! ever learning what a band, an allowance or a taper is.
//!
//! # Why `draw` commits
//!
//! There is no separate quote/apply pair. Splitting them would force [`Draw`]
//! to carry enough ledger bookkeeping for a later `apply` to post it -- which is
//! precisely the jurisdiction-specific detail this crate exists to hide.
//! Callers sort by `marginal_keep`, draw from the best account, and re-sort;
//! they never need to speculatively price several accounts and then pick one.

#![forbid(unsafe_code)]

use rust_decimal::Decimal;
use std::fmt;

// --- dates ------------------------------------------------------------------

/// A plain calendar date.
///
/// Deliberately not `chrono`: this crate needs ordering and display and nothing
/// else, and derived `Ord` over `(year, month, day)` is exactly calendar order.
/// Staying dependency-light is also what lets a whole tax system be a `const`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimpleDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl SimpleDate {
    pub const fn new(year: u16, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    /// Whole months from `self` to `other`, negative if `other` is earlier.
    /// Used for "these figures are more than N months old" judgements.
    pub fn months_until(self, other: SimpleDate) -> i32 {
        let months = (i32::from(other.year) - i32::from(self.year)) * 12
            + (i32::from(other.month) - i32::from(self.month));
        // A partial month does not count until the day-of-month is reached.
        if other.day < self.day {
            months - 1
        } else {
            months
        }
    }
}

impl fmt::Display for SimpleDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let name = MONTHS
            .get(usize::from(self.month.saturating_sub(1)))
            .copied()
            .unwrap_or("?");
        write!(f, "{} {} {}", self.day, name, self.year)
    }
}

// --- catalogue --------------------------------------------------------------

/// A kind of account a holding can sit in, as advertised by a tax system.
///
/// `id` is opaque to every consumer: the projection engine carries it, the UI
/// renders `label` / `short_label`, and share links persist `id` as a string
/// precisely so an old link stays decodable when the catalogue changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountKind {
    /// Stable, opaque identifier. Never parsed or matched on outside the tax
    /// system that defines it.
    pub id: &'static str,
    /// Full name, for tables and legends.
    pub label: &'static str,
    /// Abbreviated name, for a cramped `<select>`.
    pub short_label: &'static str,
    /// Whether an acquisition cost is meaningful for this kind. Drives a UI's
    /// conditional cost-basis input, so no consumer needs to know *which* kinds
    /// are taxed on gains.
    pub needs_cost_basis: bool,
    /// Whether access to this kind is restricted by age.
    pub age_gated: bool,
    /// Whether the tax system actually models this kind's taxation. A `false`
    /// kind is still selectable -- it is a real thing a portfolio can hold --
    /// but callers should exclude it from tax-ordered strategies and say so.
    pub modelled: bool,
    /// Canonical order, for deterministic tie-breaks and for appending kinds
    /// that a caller-supplied ordering forgot to mention. Lower is drawn first.
    pub rank: u8,
    /// One-line caveat for a UI legend. May be empty.
    pub note: &'static str,
}

/// A sub-jurisdiction with its own rates.
///
/// A system with a single nationwide schedule returns one `Region`, and a UI is
/// expected to hide the control entirely in that case rather than render a
/// pointless one-option `<select>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub id: &'static str,
    pub label: &'static str,
}

// --- drawing ----------------------------------------------------------------

/// A holding, as the tax system sees it.
///
/// `cost_basis` is only consulted for kinds whose
/// [`AccountKind::needs_cost_basis`] is set; other kinds ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pot {
    pub kind: &'static str,
    pub available: Decimal,
    pub cost_basis: Decimal,
}

impl Pot {
    /// The taxable fraction of each unit withdrawn under proportional disposal:
    /// selling a slice realises the same fraction of the gain, so the taxable
    /// fraction of each unit is the fraction of the holding that is profit.
    ///
    /// Lives here rather than in each tax system because it is a property of
    /// `Pot` itself, not of any jurisdiction. A holding at a loss yields zero
    /// rather than a negative -- carrying losses is a jurisdiction's business.
    pub fn proportional_leak(&self) -> Decimal {
        if self.available > Decimal::ZERO {
            (Decimal::ONE - self.cost_basis / self.available).max(Decimal::ZERO)
        } else {
            Decimal::ZERO
        }
    }
}

/// The cost of one withdrawal.
///
/// `gross - tax == net` exactly; no rounding happens inside a draw (see the
/// note on rounding in [`TaxSession::draw`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Draw {
    /// Taken out of the account.
    pub gross: Decimal,
    /// Charged on it.
    pub tax: Decimal,
    /// Delivered to the holder. May be less than requested if the account ran
    /// dry, or if `stop` cut the draw short.
    pub net: Decimal,
    /// The draw stopped because the marginal rate was about to step up, rather
    /// than because the requirement was met or the account emptied.
    ///
    /// This is what lets a caller implement a cheapest-first strategy without
    /// knowing what a rate band is: draw, and if `rung_limited`, re-sort and
    /// try again.
    pub rung_limited: bool,
}

/// When to stop drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopAt {
    /// Take as much as it takes to meet the requirement, or empty the account.
    Requirement,
    /// Stop as soon as the marginal rate would step up, so the caller can
    /// reconsider which account is now cheapest.
    NextRung,
    /// Never draw at a marginal rate above this, expressed as a fraction -- so
    /// `0.20` for twenty percent.
    ///
    /// The comparison is against the *statutory* rate applied to the taxable
    /// slice, not the blended rate on the withdrawal as a whole: "keep me out of
    /// the higher-rate band" is a statement about the band, not about the
    /// effective cost of a partly tax-free withdrawal.
    RateAbove(Decimal),
}

/// Everything a tax system needs in order to open a session.
///
/// `Default`-derived and deliberately **not** `#[non_exhaustive]`: new fields
/// are added here as tax systems need them, and every construction site spreads
/// `..Default::default()` so a new field is one line at the definition rather
/// than a break at each caller. A system ignores any field it does not use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSpec {
    /// An id from [`TaxSystem::regions`].
    pub region: String,
    /// Other taxable income the holder receives each tax period, independent of
    /// the portfolio. Withdrawals are marginal on top of it.
    pub other_income: Decimal,
    /// Age in whole years at the point the drawdown begins, where known.
    pub age: Option<u32>,
    /// Annual uprating applied to thresholds, as a fraction. Zero means frozen.
    pub uprate: Decimal,
}

/// Whether a tax system's figures still look current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Staleness {
    Fresh,
    /// The figures are out of date. `current_period` names the period that
    /// appears to be current now, for a message like "these are 2026/27 rates
    /// but the current tax year is 2027/28".
    Stale { current_period: String },
}

// --- errors -----------------------------------------------------------------

/// What went wrong.
///
/// `message` is written by the tax system, so a caller can surface a
/// jurisdiction-appropriate sentence without composing one itself -- and
/// without hard-coding an age limit or an account name it should not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxError {
    pub message: String,
    pub kind: TaxErrorKind,
}

impl TaxError {
    pub fn new(kind: TaxErrorKind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind,
        }
    }

    /// An amount outgrew what `Decimal` can represent. A named constructor
    /// because both the walker and every tax system raise this same condition,
    /// and the holder should not meet two different sentences for it depending
    /// on which one noticed first.
    pub fn overflow() -> Self {
        Self::new(
            TaxErrorKind::Overflow,
            "This portfolio is too large to project.",
        )
    }

    /// A schedule charges 100% or more at the margin, so a gross-up would
    /// diverge. Cannot arise from a real schedule; it means a table is wrong.
    pub fn confiscatory() -> Self {
        Self::new(
            TaxErrorKind::BadRules,
            "This tax schedule charges 100% or more at the margin, which cannot be grossed up.",
        )
    }
}

impl fmt::Display for TaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The category of a [`TaxError`], so a caller can attribute it to the right
/// input control without parsing the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxErrorKind {
    /// An amount exceeded what the arithmetic can represent.
    Overflow,
    /// The tax tables themselves are inconsistent. Should be unreachable; it
    /// exists so a bad table update fails loudly instead of quietly.
    BadRules,
    /// The region id is not one this system advertises.
    BadRegion,
    /// Other income is negative or otherwise unusable.
    BadOtherIncome,
    /// The holder is too young to access one of the accounts in the portfolio.
    AgeGated,
    /// The account kind is not one this system advertises.
    UnknownAccount,
}

// --- the traits -------------------------------------------------------------

/// A tax system: a catalogue of accounts, a set of regions, and a set of rules
/// stamped with the date they were checked.
///
/// Implementations are expected to be zero-sized `const` values, so a consumer
/// can hold a `&'static dyn TaxSystem` and swap the whole tax model on one line.
pub trait TaxSystem: Sync {
    /// Human name of the jurisdiction, e.g. "United Kingdom".
    fn label(&self) -> &'static str;

    /// Currency symbol for display. The projection engine never formats money;
    /// this exists for the UI.
    fn currency_symbol(&self) -> &'static str;

    /// Every account kind a holding may sit in. Order is presentation order.
    fn account_kinds(&self) -> &'static [AccountKind];

    /// Sub-jurisdictions with distinct rates. Always at least one entry.
    fn regions(&self) -> &'static [Region];

    /// The conventional order in which accounts are spent, as account-kind ids.
    ///
    /// Which accounts a holder is conventionally advised to spend first is a
    /// judgement about a particular tax system -- it follows from how each
    /// account is taxed on the way out and on death -- so it belongs here
    /// rather than in a projection engine.
    fn conventional_order(&self) -> &'static [&'static str];

    /// The tax period these rules describe, e.g. "2026/27".
    fn rules_label(&self) -> &'static str;

    /// When the figures were last checked against source.
    fn as_of(&self) -> SimpleDate;

    /// Where the figures came from. Displayed, and read by the update skill.
    fn source_note(&self) -> &'static str;

    /// Whether these rules look out of date as at `today`.
    ///
    /// The judgement belongs to the system, not to the UI: what counts as stale
    /// depends on the jurisdiction's own cycle.
    fn staleness(&self, today: SimpleDate) -> Staleness;

    /// The account kind a holding falls into when it names none — what a blank
    /// picker resolves to.
    ///
    /// Every system is expected to make this its *untaxed* kind, so a portfolio
    /// that says nothing about accounts is projected untaxed, exactly as it was
    /// before the tax model existed. The default is the first advertised kind
    /// (presentation order), which encodes that expectation as one overridable
    /// method instead of three independent `account_kinds().first()` calls in
    /// the consumers. A system whose catalogue does not lead with its untaxed
    /// kind must override this.
    fn default_account_kind(&self) -> Option<&'static AccountKind> {
        self.account_kinds().first()
    }

    /// Look up an account kind by id.
    fn account_kind(&self, id: &str) -> Option<&'static AccountKind> {
        self.account_kinds().iter().find(|k| k.id == id)
    }

    /// Look up a region by id.
    fn region(&self, id: &str) -> Option<&'static Region> {
        self.regions().iter().find(|r| r.id == id)
    }

    /// Begin a projection.
    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn TaxSession>, TaxError>;
}

/// One projection's tax state: allowances consumed so far this period, plus
/// anything the jurisdiction tracks for life.
pub trait TaxSession {
    /// Months in a tax period. The caller decides where a period *starts*; this
    /// is only its length.
    fn period_months(&self) -> u32;

    /// Roll into a new tax period: reset periodic allowances, retain lifetime
    /// ones, and bank whatever allowance the finished period left unused.
    fn start_period(&mut self);

    /// The fraction of the next unit withdrawn from `pot` that would survive
    /// tax, right now. A pure sort key in `0.0..=1.0`; higher is cheaper.
    fn marginal_keep(&self, pot: &Pot) -> Decimal;

    /// How much could come out of `pot` at that rate before it steps up, or
    /// `None` if it never does.
    ///
    /// This exists to break ties between accounts that are equally cheap *right
    /// now*. One that is cheap because an allowance has not been spent is
    /// use-it-or-lose-it and should go first; one that is cheap indefinitely can
    /// wait. Without this a caller cannot tell them apart and will let
    /// allowances expire unclaimed — which is the single behaviour a tax-aware
    /// withdrawal order exists to get right.
    ///
    /// Returning `None` is always safe: it makes a caller's tie-break less
    /// clever, never wrong. Hence the default.
    fn marginal_headroom(&self, _pot: &Pot) -> Option<Decimal> {
        None
    }

    /// Both marginal figures at once: what the next unit keeps, and the headroom
    /// at that rate before it steps up.
    ///
    /// Exists so an implementation that has to build a rate ladder can build it
    /// **once** and read both figures off it, rather than paying for the same
    /// build twice when a caller (a cheapest-first sort) needs both. Defaulted in
    /// terms of [`marginal_keep`](Self::marginal_keep) and
    /// [`marginal_headroom`](Self::marginal_headroom), so it is a purely additive
    /// change: overriding it buys speed, and is never a requirement.
    fn marginal(&self, pot: &Pot) -> (Decimal, Option<Decimal>) {
        (self.marginal_keep(pot), self.marginal_headroom(pot))
    }

    /// Take money out of `pot`, aiming to deliver `net_wanted`, and commit the
    /// result to the ledger.
    ///
    /// Delivers less than asked only if the account ran dry or `stop` cut the
    /// draw short. Implementations must **not** round: the caller carries full
    /// precision through its own loop and rounds once at its output boundary,
    /// and rounding per draw would accumulate a systematic drift over hundreds
    /// of months.
    fn draw(&mut self, pot: &Pot, net_wanted: Decimal, stop: StopAt) -> Result<Draw, TaxError>;

    /// Tax charged so far in the current period.
    fn period_tax(&self) -> Decimal;

    /// Tax-free headroom that has gone unclaimed: across completed periods, plus
    /// whatever is still unclaimed in the current one.
    ///
    /// This is the "show your working" figure -- it is what explains why one
    /// withdrawal order beats another. A system with no allowances returns zero.
    fn unused_allowance(&self) -> Decimal;
}

pub mod ladder;

#[cfg(feature = "mock")]
pub mod mock;

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: u16, month: u8, day: u8) -> SimpleDate {
        SimpleDate::new(year, month, day)
    }

    #[test]
    fn dates_order_by_calendar_not_by_field_width() {
        assert!(d(2026, 4, 5) < d(2026, 4, 6));
        assert!(d(2025, 12, 31) < d(2026, 1, 1));
        assert!(d(2026, 10, 1) > d(2026, 9, 1));
    }

    #[test]
    fn months_until_counts_whole_months_only() {
        assert_eq!(d(2026, 4, 6).months_until(d(2027, 4, 6)), 12);
        // One day short of a year is eleven whole months, not twelve.
        assert_eq!(d(2026, 4, 6).months_until(d(2027, 4, 5)), 11);
        assert_eq!(d(2026, 4, 6).months_until(d(2026, 4, 6)), 0);
        assert_eq!(d(2026, 4, 6).months_until(d(2025, 4, 6)), -12);
    }

    #[test]
    fn dates_display_for_a_human() {
        assert_eq!(d(2026, 4, 6).to_string(), "6 April 2026");
    }
}
