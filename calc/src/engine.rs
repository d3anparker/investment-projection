//! The projection engine: prepare holdings, advance the portfolio month by
//! month across both phases, and assemble the rounded [`CalcOutput`].
//!
//! The single source of numeric truth. Pure of input parsing (holdings arrive
//! validated as [`Prepared`]) and, in [`project`], of output rounding.

use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use taxkit::{PeriodPot, Pot, StopAt, TaxSession};

use crate::parse::{overflowed, parse_number, parse_or_zero, round2, to_months, too_large_msg};
use crate::strategy::{Limit, Order, Strategy};
use crate::tax::{draw_one, open_session, priority_of, tax_error, Ledger, Priority};
use crate::types::{
    CalcError, CalcInput, CalcOutput, Field, InvestmentField, InvestmentInput, InvestmentResult, Plan,
    TaxContext, Unit,
};
use crate::MAX_HORIZON_MONTHS;

/// The greedy cannot loop for ever: each pass either meets the requirement,
/// empties a holding, blocks one, or crosses one rate boundary. This bounds the
/// last of those generously — the never-hang contract outranks the last penny.
const MAX_GREEDY_PASSES: usize = 24;

/// One holding, parsed and validated up front, with its monthly growth factor
/// derived. Built once by [`prepare_holdings`]; the month loop in [`project`]
/// reads it and never re-parses.
///
/// Lifting it to module scope (out of `calculate`) is what lets the goal-seek
/// solvers prepare the portfolio once and vary only the withdrawal — or, for a
/// top-up, the deposit — across a bracket search, rather than re-parsing every
/// input string and recomputing every twelfth root on each probe.
pub(crate) struct Prepared {
    pub(crate) name: String,
    pub(crate) current_value: Decimal,
    pub(crate) contribution: Decimal,
    pub(crate) monthly: Decimal,
    pub(crate) annual: Decimal,
    /// The account kind this holding sits in, resolved against the active tax
    /// system's catalogue. `None` on an untaxed projection, where the notion
    /// does not apply.
    pub(crate) kind: Option<&'static taxkit::AccountKind>,
    /// The id as given, echoed back on the result so the UI can label the row
    /// without re-reading the form. Survives having no catalogue.
    pub(crate) kind_id: String,
    pub(crate) cost_basis: Decimal,
}

/// One month of a **static** order: work through the groups in turn, splitting
/// each group's share pro-rata by balance with the residue on its last member,
/// then mopping up anything the split could not deliver.
///
/// The residue rule is the existing pro-rata invariant, generalised: every
/// apportionment's rounding lands on the last member of the group it
/// apportions, so per-row withdrawals still sum exactly to the total.
fn static_month(
    session: &mut Option<Box<dyn TaxSession>>,
    led: &mut Ledger,
    groups: &[Vec<usize>],
    net_wanted: Decimal,
) -> Result<Decimal, CalcError> {
    let mut delivered = Decimal::ZERO;
    for group in groups {
        let remaining = net_wanted - delivered;
        if remaining <= Decimal::ZERO {
            break;
        }
        let mut group_bal = Decimal::ZERO;
        for j in group {
            group_bal = group_bal
                .checked_add(led.balances[*j])
                .ok_or_else(overflowed)?;
        }
        if group_bal <= Decimal::ZERO {
            continue;
        }

        // When nothing is being taxed (`gross == net`) and the requirement takes
        // the whole group, zero every member *exactly* rather than by a pro-rata
        // mul/div that could leave a sub-penny dust behind — the old pro-rata
        // full-drain special case, generalised to any all-untaxed group so
        // depletion still reads cleanly. A taxed group cannot use it: `remaining`
        // is net there and `group_bal` gross, so the two are not comparable.
        if session.is_none() && remaining >= group_bal {
            for j in group {
                delivered += led.balances[*j];
                led.withdrawn[*j] += led.balances[*j];
                led.balances[*j] = Decimal::ZERO;
            }
            continue;
        }

        let mut allocated = Decimal::ZERO;
        for (k, j) in group.iter().enumerate() {
            let share = if k + 1 == group.len() {
                remaining - allocated
            } else {
                remaining
                    .checked_mul(led.balances[*j])
                    .and_then(|x| x.checked_div(group_bal))
                    .ok_or_else(overflowed)?
            };
            allocated += share;
            delivered += draw_one(session, led, *j, share, StopAt::Requirement)?.net;
        }

        // A member may not have managed its share -- its balance ran out, or tax
        // took more of it than a split by balance assumed. Sweep the group once
        // before moving on, so the shortfall does not silently leak away.
        for j in group {
            let short = net_wanted - delivered;
            if short <= Decimal::ZERO {
                break;
            }
            delivered += draw_one(session, led, *j, short, StopAt::Requirement)?.net;
        }
    }
    Ok(delivered)
}

/// One month of a **dynamic** order: repeatedly take from whichever holding
/// keeps most of the next pound, stopping at each rate boundary so the choice is
/// reconsidered.
///
/// This is what makes it an optimiser rather than a fixed order — it will fill a
/// zero-rate allowance from a taxable account, switch to a tax-free one, and
/// switch back when that runs out — and it does so without knowing that any of
/// those things exist.
///
/// The holding to draw from next: cheapest first, by [`Priority`].
///
/// `blocked` is empty on the cap-breach pass, which reconsiders everything.
/// Extracted because the tie-breaking in `Priority::beats` is the subtle part of
/// the optimiser, and it had two call sites that had to stay in step by hand.
fn best_holding(
    session: &Option<Box<dyn TaxSession>>,
    led: &Ledger,
    blocked: &[bool],
) -> Option<usize> {
    let mut best: Option<Priority> = None;
    for j in 0..led.balances.len() {
        if blocked.get(j).copied().unwrap_or(false) || led.balances[j] <= Decimal::ZERO {
            continue;
        }
        let p = priority_of(session, led, j);
        if best.is_none_or(|b| p.beats(&b)) {
            best = Some(p);
        }
    }
    best.map(|p| p.index)
}

/// Returns the net delivered and whether a rate cap had to be breached.
fn greedy_month(
    session: &mut Option<Box<dyn TaxSession>>,
    led: &mut Ledger,
    net_wanted: Decimal,
    cap: Option<Decimal>,
    // Scratch, owned by the caller so the month loop reuses one allocation.
    blocked: &mut [bool],
) -> Result<(Decimal, bool), CalcError> {
    let n = led.balances.len();
    let stop = match cap {
        Some(c) => StopAt::RateAbove(c),
        None => StopAt::NextRung,
    };
    let mut delivered = Decimal::ZERO;
    blocked.fill(false);

    for _ in 0..(n * MAX_GREEDY_PASSES + 1) {
        let remaining = net_wanted - delivered;
        if remaining <= Decimal::ZERO {
            break;
        }
        let Some(j) = best_holding(session, led, blocked) else { break };

        let drawn = draw_one(session, led, j, remaining, stop)?;
        delivered += drawn.net;
        if drawn.gross <= Decimal::ZERO {
            // It gave nothing: either the cap shut it out or it is empty. Either
            // way it cannot help under this stop, so drop it from contention.
            blocked[j] = true;
        }
    }

    // A cap that cannot be honoured. Delivering the money and reporting the
    // breach beats silently handing over less than was asked for.
    let mut breached = false;
    if cap.is_some() {
        for _ in 0..(n + 1) {
            let short = net_wanted - delivered;
            if short <= Decimal::ZERO {
                break;
            }
            let Some(j) = best_holding(session, led, &[]) else { break };
            let drawn = draw_one(session, led, j, short, StopAt::Requirement)?;
            if drawn.gross <= Decimal::ZERO {
                break;
            }
            // An untaxed holding is never shut out by a rate cap, so anything
            // this pass manages to draw was drawn above the cap.
            breached = true;
            delivered += drawn.net;
        }
    }

    Ok((delivered, breached))
}

/// Group holdings by account kind, in the order the caller asked for.
///
/// Kinds present in the portfolio but missing from `order` are appended by
/// catalogue rank rather than rejected: a forgiving rule that adds no new way
/// for the projection to fail.
fn groups_by_kind(order: &[String], prepared: &[Prepared]) -> Vec<Vec<usize>> {
    let id_of = |j: usize| prepared[j].kind.map_or("", |k| k.id);

    let mut present: Vec<(&'static str, u8)> = Vec::new();
    for (j, p) in prepared.iter().enumerate() {
        let id = id_of(j);
        if !present.iter().any(|(q, _)| *q == id) {
            present.push((id, p.kind.map_or(0, |k| k.rank)));
        }
    }

    let mut sequence: Vec<&'static str> = Vec::with_capacity(present.len());
    for want in order {
        if let Some((id, _)) = present.iter().find(|(p, _)| *p == want.as_str()) {
            if !sequence.contains(id) {
                sequence.push(id);
            }
        }
    }
    let mut rest: Vec<(&'static str, u8)> = present
        .iter()
        .filter(|(p, _)| !sequence.contains(p))
        .copied()
        .collect();
    rest.sort_by_key(|(id, rank)| (*rank, *id));
    sequence.extend(rest.into_iter().map(|(id, _)| id));

    sequence
        .into_iter()
        .map(|id| (0..prepared.len()).filter(|j| id_of(*j) == id).collect())
        .collect()
}

/// Group holdings by annualised return, lowest first: drain the worst
/// compounder before touching the best. Equal-returning holdings share a group
/// and are drawn pro-rata, so the order never depends on how rows were typed.
fn groups_by_return(prepared: &[Prepared]) -> Vec<Vec<usize>> {
    let mut distinct: Vec<Decimal> = prepared.iter().map(|p| p.annual).collect();
    distinct.sort();
    distinct.dedup();
    distinct
        .into_iter()
        .map(|r| (0..prepared.len()).filter(|j| prepared[*j].annual == r).collect())
        .collect()
}

/// The accumulation (growth) period in whole months, validated against the
/// 1-month floor and the 100-year cap. Shared by [`calculate`] and the goal-seek
/// solvers so they agree on what a horizon is.
pub(crate) fn horizon_months_of(input: &CalcInput) -> Result<u32, CalcError> {
    let h = to_months(&input.horizon_value, input.horizon_unit, "The growth period")
        .map_err(|m| CalcError::new(m, Some(Field::Horizon)))?;
    if h < 1 {
        return Err(CalcError::new("Enter a growth period of at least 1 month.", Some(Field::Horizon)));
    }
    if h > MAX_HORIZON_MONTHS {
        return Err(CalcError::new(
            "The growth period is limited to 100 years (1200 months).",
            Some(Field::Horizon),
        ));
    }
    Ok(h)
}

/// The drawdown period in whole months, validated against the 1-month floor and
/// the combined 100-year cap.
pub(crate) fn drawdown_months_of(value: &str, unit: Unit, horizon_months: u32) -> Result<u32, CalcError> {
    let d = to_months(value, unit, "The drawdown period")
        .map_err(|m| CalcError::new(m, Some(Field::Drawdown)))?;
    if d < 1 {
        return Err(CalcError::new("Enter a drawdown period of at least 1 month.", Some(Field::Drawdown)));
    }
    if horizon_months as u64 + d as u64 > MAX_HORIZON_MONTHS as u64 {
        return Err(CalcError::new(
            "The growth and drawdown periods together are limited to 100 years (1200 months).",
            Some(Field::Drawdown),
        ));
    }
    Ok(d)
}

/// The portfolio-level monthly withdrawal, non-negative. A blank/zero is a legal
/// flat drawdown.
pub(crate) fn withdrawal_of(withdrawal: &str) -> Result<Decimal, CalcError> {
    let w = parse_number(withdrawal)
        .ok_or_else(|| CalcError::new("Enter a valid monthly withdrawal.", Some(Field::Withdrawal)))?;
    if w < Decimal::ZERO {
        return Err(CalcError::new("The monthly withdrawal cannot be negative.", Some(Field::Withdrawal)));
    }
    Ok(w)
}

/// The rate cap a [`Limit::RateCap`] stop carries, as a fraction, or `None` for
/// any other stop. Its errors point at the strategy control that owns it.
pub(crate) fn rate_cap_of(strategy: &Strategy) -> Result<Option<Decimal>, CalcError> {
    match &strategy.stop {
        Limit::RateCap(max_rate) => {
            let r = parse_or_zero(max_rate).ok_or_else(|| {
                CalcError::new("Enter a valid rate to cap withdrawals at.", Some(Field::Strategy))
            })? / Decimal::from(100u32);
            if r < Decimal::ZERO {
                return Err(CalcError::new("The rate cap cannot be negative.", Some(Field::Strategy)));
            }
            Ok(Some(r))
        }
        _ => Ok(None),
    }
}

/// The plan parameters a projection runs under, parsed and validated once. The
/// strategy is owned (cloned) so a deposits plan can carry the default [`Strategy`]
/// (pro-rata) without borrowing a local.
struct PlanParams {
    horizon_months: u32,
    drawdown_months: u32,
    withdrawal: Decimal,
    strategy: Strategy,
    rate_cap: Option<Decimal>,
}

fn plan_params(input: &CalcInput) -> Result<PlanParams, CalcError> {
    let horizon_months = horizon_months_of(input)?;
    let strategy = match &input.plan {
        Plan::Deposits => Strategy::default(),
        Plan::Drawdown { strategy, .. } => strategy.clone(),
    };
    let (drawdown_months, withdrawal) = match &input.plan {
        Plan::Deposits => (0u32, Decimal::ZERO),
        Plan::Drawdown { drawdown_value, drawdown_unit, withdrawal, .. } => (
            drawdown_months_of(drawdown_value, *drawdown_unit, horizon_months)?,
            withdrawal_of(withdrawal)?,
        ),
    };
    let rate_cap = rate_cap_of(&strategy)?;
    Ok(PlanParams { horizon_months, drawdown_months, withdrawal, strategy, rate_cap })
}

/// Parse and validate every holding, deriving its monthly growth factor and
/// resolving its account kind against `catalogue`. Reads only the investments;
/// the month loop in [`project`] then never touches an input string again, which
/// is what lets the solvers prepare once and re-project many times.
pub(crate) fn prepare_holdings(
    investments: &[InvestmentInput],
    catalogue: &'static [taxkit::AccountKind],
    default_kind: Option<&'static taxkit::AccountKind>,
) -> Result<Vec<Prepared>, CalcError> {
    let hundred = Decimal::from(100u32);
    let twelve = Decimal::from(12u32);
    let mut prepared: Vec<Prepared> = Vec::with_capacity(investments.len());

    for (index, inv) in investments.iter().enumerate() {
        use InvestmentField::{Contribution, Rate, Value};
        let too_large = |part| CalcError::at(too_large_msg(&inv.name), index, part);

        let current_value = parse_number(&inv.value)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid amount.", inv.name), index, Value))?;
        if current_value < Decimal::ZERO {
            return Err(CalcError::at(format!("'{}' has a negative amount.", inv.name), index, Value));
        }

        let contribution = parse_number(&inv.contribution).ok_or_else(|| {
            CalcError::at(format!("'{}' has an invalid monthly deposit.", inv.name), index, Contribution)
        })?;
        if contribution < Decimal::ZERO {
            return Err(CalcError::at(
                format!("'{}' has a negative monthly amount.", inv.name),
                index,
                Contribution,
            ));
        }

        let rate = parse_number(&inv.rate)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid rate.", inv.name), index, Rate))?
            / hundred;
        // 1 + rate must stay positive for real-valued compounding / roots.
        if rate <= Decimal::NEGATIVE_ONE {
            return Err(CalcError::at(
                format!("'{}' has a return of -100% or worse, which cannot be projected.", inv.name),
                index,
                Rate,
            ));
        }
        // The annualised input drives the projection directly; the monthly factor
        // is its geometric twelfth root, one `powd` per holding. Checked, because
        // an extreme-but-reachable input (100% annualised over 100 years) exceeds
        // the Decimal maximum and an unchecked `powd` would panic.
        let annual = rate;
        let monthly = (Decimal::ONE + annual)
            .checked_powd(Decimal::ONE / twelve)
            .ok_or_else(|| too_large(Rate))?;

        // Which account this holding sits in. Blank picks the system's default
        // kind — expected to be its untaxed one, so a portfolio that says nothing
        // about accounts keeps behaving as it always did.
        let wanted = inv.account_kind.trim();
        let kind = if catalogue.is_empty() {
            None
        } else if wanted.is_empty() {
            default_kind
        } else {
            Some(catalogue.iter().find(|k| k.id == wanted).ok_or_else(|| {
                CalcError::at(
                    format!("'{}' is in an account this calculator does not know.", inv.name),
                    index,
                    InvestmentField::AccountKind,
                )
            })?)
        };

        // What it originally cost, for kinds taxed on the gain. Blank means
        // "today's value" — nothing gained yet, and future growth is what
        // becomes taxable. Only validated where it is actually consulted, so a
        // leftover figure on a wrapper that ignores it is not an error.
        let cost_basis = if kind.is_some_and(|k| k.needs_cost_basis) && !inv.cost_basis.trim().is_empty() {
            let c = parse_number(&inv.cost_basis).ok_or_else(|| {
                CalcError::at(
                    format!("'{}' has an invalid original cost.", inv.name),
                    index,
                    InvestmentField::CostBasis,
                )
            })?;
            if c < Decimal::ZERO {
                return Err(CalcError::at(
                    format!("'{}' has a negative original cost.", inv.name),
                    index,
                    InvestmentField::CostBasis,
                ));
            }
            c
        } else {
            current_value
        };

        prepared.push(Prepared {
            name: inv.name.clone(),
            current_value,
            contribution,
            monthly,
            annual,
            kind,
            kind_id: kind.map_or_else(|| wanted.to_string(), |k| k.id.to_string()),
            cost_basis,
        });
    }
    Ok(prepared)
}

/// The tax session a projection needs, and whether that system charges for
/// merely *holding*.
///
/// The two travel together because they are decided together: `charging` is one
/// of the two reasons a session gets opened at all, and every reader of the flag
/// is about to use the session it justifies. Deriving it separately at each call
/// site let the pair drift — a caller could hand `project` a charging system with
/// `charging: false` and get a silently uncharged projection, with no compile
/// error and no panic, just a pot a few percent too big.
pub(crate) struct SessionPlan {
    pub(crate) session: Option<Box<dyn TaxSession>>,
    /// The system taxes holding as well as withdrawing, so a periodic charge has
    /// to be levied at every period boundary in *both* phases.
    pub(crate) charging: bool,
}

/// Open the tax session a projection needs, or `None` when it needs none.
///
/// Two reasons to open one. A tax-ordered withdrawal needs to price draws
/// (pro-rata never does — that is what keeps it byte-identical to the untaxed
/// model). And a system that charges tax for *holding* needs a session in both
/// phases, whatever the strategy: the charge is levied even while accumulating,
/// and even under pro-rata (whose *withdrawals* still go untaxed — see
/// `project`). `PreserveGrowth` uses a session if a context is present but does
/// not require one; the tax-aware orders require one and say so when it is
/// missing.
pub(crate) fn open_if_ordered(
    tax: &Option<TaxContext>,
    strategy: &Strategy,
    drawing: bool,
) -> Result<SessionPlan, CalcError> {
    let ordered = drawing && strategy.order != Order::ProRata;
    let charging = tax.as_ref().is_some_and(|t| t.system.has_periodic_charge());
    let plan = |session| SessionPlan { session, charging };
    if !(ordered || charging) {
        return Ok(plan(None));
    }
    match tax {
        Some(t) => Ok(plan(Some(open_session(t)?))),
        None if strategy.needs_tax() => Err(CalcError::new(
            "This withdrawal order needs to know how the accounts are taxed. \
             Fill in the tax details, or split the withdrawal pro-rata instead.",
            Some(Field::Strategy),
        )),
        None => Ok(plan(None)),
    }
}

/// The fixed month-by-month draw order a static order uses, worked out once.
/// Pro-rata is one group of every holding — the same generic split, no longer a
/// third mechanism of its own; empty only for the dynamic (greedy) order, which
/// picks a holding per pass rather than apportioning fixed groups.
pub(crate) fn groups_for(strategy: &Strategy, prepared: &[Prepared]) -> Vec<Vec<usize>> {
    match &strategy.order {
        Order::ProRata => vec![(0..prepared.len()).collect()],
        Order::ByKind(order) => groups_by_kind(order, prepared),
        Order::ByReturn => groups_by_return(prepared),
        Order::ByMarginalCost => Vec::new(),
    }
}

/// The raw output of one month-by-month run, before any rounding or per-row
/// assembly. [`calculate`] rounds it into a [`CalcOutput`]; the goal-seek solvers
/// read only `totals`/`depletion_month` and skip that assembly entirely.
pub(crate) struct Run {
    /// Raw (unrounded) portfolio total at each month `0..=total`.
    pub(crate) totals: Vec<Decimal>,
    /// Raw cumulative deposits / withdrawals / tax, parallel to `totals`.
    pub(crate) contribs: Vec<Decimal>,
    pub(crate) withdraws: Vec<Decimal>,
    pub(crate) taxed: Vec<Decimal>,
    /// Raw cumulative periodic charge, parallel to `totals`.
    pub(crate) charges_cum: Vec<Decimal>,
    /// Final per-holding balance and cumulative flows.
    pub(crate) balances: Vec<Decimal>,
    pub(crate) contributed: Vec<Decimal>,
    pub(crate) withdrawn: Vec<Decimal>,
    pub(crate) taxes: Vec<Decimal>,
    pub(crate) charged: Vec<Decimal>,
    pub(crate) handover: Vec<Option<Decimal>>,
    pub(crate) row_depletion: Vec<Option<u32>>,
    pub(crate) contributed_total: Decimal,
    pub(crate) withdrawn_total: Decimal,
    pub(crate) tax_total: Decimal,
    pub(crate) charged_total: Decimal,
    pub(crate) depletion_month: Option<u32>,
    pub(crate) accounts_touched: Vec<usize>,
    pub(crate) period_months: Option<u32>,
    pub(crate) rate_cap_breached: bool,
}

/// Advance the whole portfolio month by month across both phases.
///
/// Pure of input parsing (its holdings arrive already validated in `prepared`)
/// and of output rounding (it returns raw `Decimal`s). The month loop is
/// *month-major* — all holdings advance one month together — because the drawdown
/// split depends on every holding's current balance at once, so per-holding state
/// cannot run in isolation.
///
/// The `plan` is borrowed mutably so the caller retains its session afterwards
/// for the unused-allowance and rules-label figures the ledger holds.
pub(crate) fn project(
    prepared: &[Prepared],
    horizon_months: u32,
    drawdown_months: u32,
    withdrawal: Decimal,
    strategy: &Strategy,
    groups: &[Vec<usize>],
    rate_cap: Option<Decimal>,
    plan: &mut SessionPlan,
) -> Result<Run, CalcError> {
    let SessionPlan { session, charging } = plan;
    let charging = *charging;
    let n = prepared.len();
    let horizon = horizon_months as usize;
    let total = (horizon_months + drawdown_months) as usize;
    let drawing = drawdown_months > 0;
    // Pro-rata takes its withdrawal gross and opens no session; `ordered` gates
    // the tax-period bookkeeping that only the net orders have.
    let ordered = drawing && strategy.order != Order::ProRata;

    // Per-holding running balance and cumulative cash flow.
    let mut balances: Vec<Decimal> = prepared.iter().map(|p| p.current_value).collect();
    let mut basis: Vec<Decimal> = prepared.iter().map(|p| p.cost_basis).collect();
    let mut contributed: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut withdrawn: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut taxes: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut charged: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut handover: Vec<Option<Decimal>> = vec![None; n];
    let mut row_depletion: Vec<Option<u32>> = vec![None; n];
    // Whether a holding has ever actually held anything. Tracked rather than
    // read off `current_value`, because a holding can be built up entirely from
    // deposits: it starts at zero, and its value today says nothing about
    // whether it later had something to run out of.
    let mut ever_held: Vec<bool> = vec![false; n];

    // The tax period's length is the tax system's to state; an untaxed
    // projection has none, and manufactures no fiscal years to bucket by. The
    // loop constant below is only ever consulted while a session exists (the
    // period boundary is gated on that), so its fallback is never reached.
    let period_months = session.as_ref().map(|s| s.period_months().max(1));
    let period_len = period_months.unwrap_or(12) as usize;
    // Scratch reused by every drawdown month rather than reallocated per month:
    // `before` snapshots the balances to diff against, `blocked` is the greedy's
    // out-of-contention set. Both are overwritten in full before each use.
    let mut before: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut blocked: Vec<bool> = vec![false; n];
    // Periodic-charge scratch: `period_opening` snapshots each holding at the
    // start of the current tax period; the charge is measured against it. The
    // pots and charge buffers are reused each boundary rather than reallocated —
    // a goal-seek runs the whole projection tens of thousands of times. Left
    // empty unless a charge can actually be levied: every other projection —
    // untaxed, or any withdrawal-only system — would otherwise pay three
    // allocations per run for buffers nothing reads.
    let mut period_opening: Vec<Decimal> =
        if charging { balances.clone() } else { Vec::new() };
    let mut period_pots: Vec<PeriodPot> =
        Vec::with_capacity(if charging { n } else { 0 });
    let mut charge_buf: Vec<Decimal> =
        if charging { vec![Decimal::ZERO; n] } else { Vec::new() };
    // Pro-rata prices no withdrawal through the session even when one exists for
    // a holding charge: its splits stay gross. Routing its draws through this
    // always-empty session is what keeps pro-rata byte-identical to untaxed.
    let mut no_session: Option<Box<dyn TaxSession>> = None;
    // A charge is anchored to the start of the projection, because it accrues
    // while accumulating; a pricing-only session (no charge) has no accumulation
    // bookkeeping and keeps the handover as its period anchor, as before.
    let anchor = if charging { 0 } else { horizon };
    let mut accounts_touched: Vec<usize> = Vec::new();
    let mut period_kinds: Vec<&str> = Vec::new();
    let mut rate_cap_breached = false;

    // Portfolio series, one point per month inclusive of both endpoints.
    let mut totals: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut contribs: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut withdraws: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut taxed: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut charges_cum: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut contributed_total = Decimal::ZERO;
    let mut withdrawn_total = Decimal::ZERO;
    let mut tax_total = Decimal::ZERO;
    let mut charged_total = Decimal::ZERO;

    // Rate-attributed and contribution-attributed overflow helpers for the loop.
    let grow_too_large = |index: usize| CalcError::at(too_large_msg(&prepared[index].name), index, InvestmentField::Rate);
    let dep_too_large = |index: usize| CalcError::at(too_large_msg(&prepared[index].name), index, InvestmentField::Contribution);

    for i in 0..=total {
        // Record the start-of-month portfolio state before any flow this month.
        // Only the grown balances need re-summing; the cumulative cash flows are
        // already tracked exactly as running scalars (`Σ contributed[j]` and
        // `Σ withdrawn[j]` equal them by construction — see the residue rule below).
        let mut tsum = Decimal::ZERO;
        for j in 0..n {
            tsum = tsum.checked_add(balances[j]).ok_or_else(overflowed)?;
        }
        totals[i] = tsum;
        contribs[i] = contributed_total;
        withdraws[i] = withdrawn_total;
        taxed[i] = tax_total;
        charges_cum[i] = charged_total;

        // A holding is spent the first month it reaches zero having had
        // something in it. Under pro-rata they all land together; under an
        // ordered strategy they empty in turn, and this is the only way to see
        // which one is carrying the drawdown at any point.
        for j in 0..n {
            if balances[j] > Decimal::ZERO {
                ever_held[j] = true;
            } else if row_depletion[j].is_none() && ever_held[j] && balances[j].is_zero() {
                row_depletion[j] = Some(i as u32);
            }
        }

        // The handover pot is the start-of-month value at the accumulation
        // boundary, before the first withdrawal.
        if drawing && i == horizon {
            for j in 0..n {
                handover[j] = Some(balances[j]);
            }
        }

        if i == total {
            break; // endpoint: record only, apply no flow.
        }

        // Tax-period boundary and the periodic holding charge, spanning both
        // phases. Anchored at `anchor` — month zero when a system charges for
        // holding (the charge accrues while accumulating), the handover
        // otherwise. At `m == 0` snapshot the period's opening values; at each
        // whole-period boundary levy the charge for the period just ended, roll
        // the session into a fresh period, and re-snapshot. Only a tax system
        // defines a period, so an untaxed projection (no session) skips it all
        // and stays a single period. Placed before this month's growth so the
        // charge lands as part of the same month's movement in the series.
        //
        // Tax periods are anchored to a projection month, never to a real
        // calendar date: a date would make the projection depend on when it was
        // run, breaking shared links and failing the browser suite every year
        // boundary, and would manufacture a stub first period carrying a full
        // year's allowances (they are not pro-rated for part periods).
        if let Some(s) = session.as_mut().filter(|_| i >= anchor) {
            let m = i - anchor;
            if m == 0 {
                if charging {
                    period_opening.copy_from_slice(&balances);
                }
            } else if m % period_len == 0 {
                // The charge for the period just ended, measured on its opening
                // values. It posts to the session's own ledger (consuming any
                // allowance a later draw would have used); here we only deduct it
                // from the balance and book it as its own flow.
                if charging {
                    period_pots.clear();
                    for j in 0..n {
                        let kind = prepared[j].kind.map_or("", |k| k.id);
                        period_pots.push(PeriodPot {
                            pot: Pot { kind, available: balances[j], cost_basis: basis[j] },
                            opening: period_opening[j],
                        });
                    }
                    s.period_charge(&period_pots, &mut charge_buf)
                        .map_err(|e| tax_error(e, None))?;
                    for j in 0..n {
                        // Capped at what the holding still has: a charge levied
                        // on the period's *opening* value can outrun a balance
                        // that has since fallen, and booking more than actually
                        // left the pot would inflate `growth`, which adds the
                        // charge back.
                        let c = charge_buf[j].min(balances[j]).max(Decimal::ZERO);
                        if c > Decimal::ZERO {
                            balances[j] -= c;
                            charged[j] = charged[j].checked_add(c).ok_or_else(overflowed)?;
                            charged_total =
                                charged_total.checked_add(c).ok_or_else(overflowed)?;
                        }
                    }
                    period_opening.copy_from_slice(&balances);
                }
                s.start_period();
                // Per-period account-touch bookkeeping is a drawdown, net-order
                // concern only: pro-rata touches nothing tax-ordered, and an
                // accumulation period draws nothing at all. Strictly `>`: under a
                // charging system the anchor is month zero, so a boundary can land
                // *on* the handover, before a single withdrawal — closing a period
                // there would push an empty count and halve the reported average.
                if ordered && i > horizon {
                    accounts_touched.push(period_kinds.len());
                    period_kinds.clear();
                }
            }
        }

        // Grow every holding one month.
        for j in 0..n {
            balances[j] = balances[j].checked_mul(prepared[j].monthly).ok_or_else(|| grow_too_large(j))?;
        }

        if i < horizon {
            // Accumulation: add each holding's monthly deposit.
            for j in 0..n {
                let c = prepared[j].contribution;
                balances[j] = balances[j].checked_add(c).ok_or_else(|| dep_too_large(j))?;
                contributed[j] = contributed[j].checked_add(c).ok_or_else(|| dep_too_large(j))?;
                contributed_total = contributed_total.checked_add(c).ok_or_else(|| dep_too_large(j))?;
            }
        } else {
            // A drawdown month. Every order — pro-rata included — runs through the
            // one generic split or the one greedy pass; there is no third
            // mechanism. For a net order `withdrawal` is the net the holder wants
            // in pocket, so the gross leaving the investments is whatever delivers
            // it after tax. `ordered` gates the tax-period bookkeeping, meaningless
            // for pro-rata (no session, gross == net).

            before.copy_from_slice(&balances);
            let mut led = Ledger {
                prepared,
                balances: &mut balances,
                basis: &mut basis,
                withdrawn: &mut withdrawn,
                taxes: &mut taxes,
            };
            // The session prices withdrawals only for a net (ordered) strategy;
            // pro-rata draws through `no_session`, so a charge-only session never
            // touches them.
            let ws = if ordered { &mut *session } else { &mut no_session };
            match &strategy.order {
                // Dynamic: a greedy argmax, re-ranked at every rate boundary.
                Order::ByMarginalCost => {
                    let (_, breached) = greedy_month(ws, &mut led, withdrawal, rate_cap, &mut blocked)?;
                    rate_cap_breached |= breached;
                }
                // Static: a fixed apportionment across `groups` — pro-rata being
                // one group of every holding.
                _ => {
                    static_month(ws, &mut led, groups, withdrawal)?;
                }
            }

            // Roll the month's per-holding movement up into the portfolio totals.
            for j in 0..n {
                let took = before[j] - balances[j];
                if took > Decimal::ZERO {
                    withdrawn_total = withdrawn_total
                        .checked_add(took)
                        .ok_or_else(overflowed)?;
                    if ordered {
                        let id = prepared[j].kind_id.as_str();
                        if !period_kinds.contains(&id) {
                            period_kinds.push(id);
                        }
                    }
                }
            }
            tax_total = taxes.iter().sum();
        }
    }

    // The whole portfolio "runs out" only when its combined total actually hits
    // zero *having held something first*. Scan the **unrounded** totals from the
    // first month there was anything to spend, rather than gating on month zero:
    // a portfolio built entirely from deposits starts at nothing and would
    // otherwise never report running out. A portfolio that is never worth
    // anything still reports `None`, which is the degenerate case worth skipping.
    let depletion_month = totals
        .iter()
        .position(|v| *v > Decimal::ZERO)
        .and_then(|start| {
            totals[start..]
                .iter()
                .position(|v| v.is_zero())
                .map(|k| (start + k) as u32)
        });

    // The last (or, untaxed, the only) period never reaches a boundary, so
    // close it here.
    if ordered {
        accounts_touched.push(period_kinds.len());
    }

    Ok(Run {
        totals,
        contribs,
        withdraws,
        taxed,
        charges_cum,
        balances,
        contributed,
        withdrawn,
        taxes,
        charged,
        handover,
        row_depletion,
        contributed_total,
        withdrawn_total,
        tax_total,
        charged_total,
        depletion_month,
        accounts_touched,
        period_months,
        rate_cap_breached,
    })
}

/// Project a portfolio forward. Returns a user-facing message on any invalid
/// input rather than panicking.
pub fn calculate(input: &CalcInput) -> Result<CalcOutput, CalcError> {
    if input.investments.is_empty() {
        return Err(CalcError::new("Add at least one investment.", None));
    }

    let pp = plan_params(input)?;

    // The tax system's account catalogue, needed even in deposits mode because
    // account kinds are validated per row regardless. Blank picks the system's
    // own default kind.
    let catalogue: &'static [taxkit::AccountKind] =
        input.tax.as_ref().map_or(&[], |t| t.system.account_kinds());
    let default_kind = input.tax.as_ref().and_then(|t| t.system.default_account_kind());
    let prepared = prepare_holdings(&input.investments, catalogue, default_kind)?;

    let horizon_months = pp.horizon_months;
    let drawdown_months = pp.drawdown_months;
    let total_months = horizon_months + drawdown_months;
    let horizon = horizon_months as usize;
    let drawing = drawdown_months > 0;

    // Open the session (retained past the run for its allowance/rules figures)
    // and settle the static draw order, then run the month loop.
    let mut plan = open_if_ordered(&input.tax, &pp.strategy, drawing)?;
    let groups = groups_for(&pp.strategy, &prepared);
    let Run {
        totals,
        contribs,
        withdraws,
        taxed,
        charges_cum,
        balances,
        contributed,
        withdrawn,
        taxes,
        charged,
        handover,
        row_depletion,
        contributed_total,
        withdrawn_total,
        tax_total,
        charged_total,
        depletion_month,
        accounts_touched,
        period_months,
        rate_cap_breached,
    } = project(
        &prepared,
        horizon_months,
        drawdown_months,
        pp.withdrawal,
        &pp.strategy,
        &groups,
        pp.rate_cap,
        &mut plan,
    )?;

    let series: Vec<Decimal> = totals.iter().map(|v| round2(*v)).collect();
    let contributions_series: Vec<Decimal> = contribs.iter().map(|v| round2(*v)).collect();
    let withdrawals_series: Vec<Decimal> = withdraws.iter().map(|v| round2(*v)).collect();
    let current_total = round2(*totals.first().expect("horizon >= 1 guarantees a point"));
    let projected_total = round2(*totals.last().expect("horizon >= 1 guarantees a point"));
    let handover_total = if drawing { Some(round2(totals[horizon])) } else { None };
    let contributed_total = round2(contributed_total);
    let withdrawn_total = round2(withdrawn_total);
    let charged_total = round2(charged_total);

    // `depletion_month`, `accounts_touched` and `period_months` all come off the
    // `Run` above — the month loop that produces them lives in `project` now.
    //
    // Only reported when a session actually priced something. A pro-rata run
    // ignores tax entirely, so advertising which tax year it used would imply a
    // calculation it never did -- and it is what keeps pro-rata byte-identical
    // to the untaxed model.
    let (rules_label, rules_as_of) = match (&plan.session, &input.tax) {
        (Some(_), Some(t)) => (Some(t.system.rules_label()), Some(t.system.as_of())),
        _ => (None, None),
    };
    // Round-half-up in integers: no float enters a reported figure.
    let accounts_touched_typical = (!accounts_touched.is_empty()).then(|| {
        let total: usize = accounts_touched.iter().sum();
        let periods = accounts_touched.len();
        (total * 2 + periods) / (periods * 2)
    });
    let unused_allowance_total = plan
        .session
        .as_ref()
        .map_or(Decimal::ZERO, |s| round2(s.unused_allowance()));

    let results: Vec<InvestmentResult> = prepared
        .iter()
        .enumerate()
        .map(|(j, p)| {
            // Round gross and tax independently, then derive net from the
            // rounded pair, so the three reconcile exactly on screen rather than
            // to within a penny.
            let gross = round2(withdrawn[j]);
            let tax = round2(taxes[j]);
            InvestmentResult {
                name: p.name.clone(),
                current_value: round2(p.current_value),
                annualised: p.annual,
                contributed: round2(contributed[j]),
                withdrawn: gross,
                handover_value: handover[j].map(round2),
                projected_value: round2(balances[j]),
                tax_paid: tax,
                net_withdrawn: gross - tax,
                charged: round2(charged[j]),
                depletion_month: row_depletion[j],
                account_kind: p.kind_id.clone(),
            }
        })
        .collect();

    // Gain from returns only: strip out today's value and the *net* cash moved in
    // (deposits minus withdrawals), so money withdrawn is not booked as a loss.
    // Percentage is against the capital deployed (today's value plus deposits).
    let net_contributed = contributed_total
        .checked_sub(withdrawn_total)
        .ok_or_else(overflowed)?;
    // A periodic holding charge is added back too (like withdrawals): it shrinks
    // the pot but is tax on holding, not a negative return.
    let growth = projected_total
        .checked_sub(current_total)
        .and_then(|g| g.checked_sub(net_contributed))
        .and_then(|g| g.checked_add(charged_total))
        .ok_or_else(overflowed)?;
    let deployed = current_total
        .checked_add(contributed_total)
        .ok_or_else(overflowed)?;
    let growth_pct = growth.checked_div(deployed).unwrap_or(Decimal::ZERO);

    let tax_series: Vec<Decimal> = taxed.iter().map(|v| round2(*v)).collect();
    let charged_series: Vec<Decimal> = charges_cum.iter().map(|v| round2(*v)).collect();
    let tax_paid_total = round2(tax_total);
    let net_withdrawn_total = withdrawn_total - tax_paid_total;
    let effective_tax_rate = tax_paid_total
        .checked_div(withdrawn_total)
        .unwrap_or(Decimal::ZERO);

    Ok(CalcOutput {
        investments: results,
        series,
        contributions_series,
        withdrawals_series,
        horizon_months,
        drawdown_months,
        total_months,
        current_total,
        handover_total,
        contributed_total,
        withdrawn_total,
        depletion_month,
        projected_total,
        growth,
        growth_pct,
        deployed,
        tax_series,
        tax_paid_total,
        net_withdrawn_total,
        effective_tax_rate,
        charged_series,
        charged_total,
        unused_allowance_total,
        period_months,
        accounts_touched,
        accounts_touched_typical,
        rate_cap_breached,
        rules_label,
        rules_as_of,
    })
}
