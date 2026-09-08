# Still to fix

Five things came out of the code review. **Four are now fixed** (1, 2, 3 and 4);
one is still open (5).

The original numbering is kept so earlier discussion still lines up. Each open
item now carries what was actually measured and a recommendation, so the
decision left to you is a genuine choice rather than an open question.

| # | Item | State | Next step |
|---|------|-------|-----------|
| 1 | No tax on German capital with no other income | **Fixed** `05ea147` | Optional: model it faithfully |
| 2 | Deposits counted as growth for the yearly fund charge | **Fixed** (Opus 4.8) | Optional: model the per-share cap |
| 3 | Input boxes showed £ under Germany | **Fixed** `05ea147` | Optional: decide £5 vs 5 € |
| 4 | Unused allowance counted during the saving-up years | **Fixed** (Opus 5) | None — both halves done together |
| 5 | Pension start-year box shows one year, uses another | Open | Seed it synchronously |

---

# Who wrote what, and whose opinions these are

This file has two authors, and the **Assessment** sections are opinions, not
findings. If you are picking this up in a fresh session, read this first — the
first person in those sections is *not* you.

**The five findings** (the "Where / what happens now / why I did not fix it"
descriptions) came from a **code-review session**, committed in `f0f669c`. That
session read the Germany work with fresh eyes and deliberately implemented none
of it, on the grounds that each item needed a decision from the repository owner.

**The fixes to 1 and 3, all the measured figures, and every `Assessment`
section** came from the **implementation session** that built the Germany work
in the first place — the `taxkit` contract changes, the `de-tax` crate and the
runtime jurisdiction picker, commits `0ca3c5c` through `05ea147`. So "I
recommend" in an Assessment means *that* session, writing with knowledge of why
the code is shaped as it is.

That provenance cuts both ways and you should weigh it accordingly:

- **In favour:** those assessments are backed by figures actually measured
  against the running code (reproduction recipes below), not by reading alone,
  and they draw on the design reasoning behind `taxkit` and `de-tax`.
- **Against:** it is the author of the code marking its own homework. Where it
  says a finding was "smaller than first assessed" (item 2) or offers "a third
  option" (item 4), treat that as an interested party's argument, not a neutral
  ruling. The review session's caution about widening a shared contract is a
  legitimate position that the assessment argues against rather than refutes.
- **Declared interest:** the thread-local weakness under *Also worth knowing* is
  the implementation session's own doing, not a review finding. It is recorded
  there as such rather than being quietly folded into item 5.

**The fix to 4** came from a **third session** (Opus 5), working from the
repository owner's decision rather than from its own recommendation: it laid out
the options, the owner chose, and it implemented that choice. Its section is
appended under item 4 and attributed. It is not the author of the Germany work,
so it has no homework of its own to mark here — but it did correct one detail of
the implementation session's Assessment, which is called out in place.

**If you add to this file,** append your own attributed section rather than
editing an existing Assessment in place. The value here is the record of who
concluded what, on what evidence — rewriting it in a later voice destroys that.

## Reproducing the measured figures

The numbers quoted below came from throwaway tests that were deleted after use.
To re-derive them, open a `de-tax` session directly (`DE.open(&SessionSpec{ .. }`
with `region: "de_none"`) and:

- **Item 2 (€149.81):** call `period_charge` with a single
  `PeriodPot { pot: fonds_aktien available 110000, opening: 100000 }` — a fund
  that fell to 90,000 but received 20,000 of deposits.
- **Item 4 (€280,308):** call `start_period()` twenty times without drawing
  anything, then read `unused_allowance()`. Still reproduces, and always will —
  it is the *session's* figure, and a session is right to bank a year it was
  offered. What changed is that `calc` no longer reports those years; that is
  pinned by `calc`'s `an_accumulation_only_projection_claims_no_allowance` and
  `unused_allowance_counts_only_the_drawdown_years`.
- **Item 1 (now fixed):** the €500,000-at-€0 case is pinned permanently by
  `de-tax/src/engine.rs`'s `capital_is_taxed_even_when_there_is_no_other_income`.

---

# Fixed

## 1. Germany charged no tax at all on capital with no other income — fixed

**Was:** `de-tax/src/engine.rs`, `capital_rate`.

Germany taxes capital gains at a flat ~26%, *or* at your normal income tax rate
if that is cheaper. The code picked the cheaper of the two, which is right in
principle. It worked out your normal rate from the income you already had,
ignoring the capital you were about to take out — and capital withdrawals are
booked separately, so that figure never grew.

With no other income the answer was always "your normal rate is 0%", and 0% wins
every comparison. **Measured: €500,000 of pure gain came back taxed at €0.** The
same draw with €60,000 of other income was taxed €75,650, so it was the whole
bill missing, not a rounding error. Because the "other income" box starts blank,
this was the *default* first experience of Germany, not a corner case.

It was also broader than "no other income": the comparison rate was read at the
wrong point for everyone whose personal marginal rate sits below 26.375% — which
is any income below about **€24,750** (where the §32a marginal crosses the flat
rate). Zero income was the catastrophic end of that range, not the whole of it.

**Now:** capital is always charged the flat Abgeltungsteuer. The
Günstigerprüfung is not modelled at all, and says so in `de-tax/src/lib.rs`.

**The trade, stated plainly.** This over-taxes someone whose genuine personal
rate is lower, by a bounded amount. That is the safe direction for a tool that
disclaims advice; understating a bill without limit is not. A regression test
(`capital_is_taxed_even_when_there_is_no_other_income`) pins it, and the reason
is written on `capital_rate` itself so the lesser-of is not reintroduced by
someone fixing the over-taxation in good faith.

**Still open, if you want it.** Modelling the Günstigerprüfung properly means
pricing capital through the progressive `Tarif` walker instead of a flat rung.
The original note called this "new machinery" — it is not: `de-tax/src/tarif.rs`
already exists and does exactly this for pensions. It is a real but
self-contained piece of work. **Decide it together with item 4** — see the
coupling noted there.

## 3. The input boxes showed £ when Germany was picked — fixed

**Was:** `app/styles.css`, `.adorn-money::before`.

Picking Germany switched the answers to euros but left a pound sign inside each
of the six money boxes you type into, so the screen showed £ and € at once for
the same money.

The sign was not in the program at all. It was in the stylesheet, as
`content: "\00a3"` — a pound spelled in a way that does not look like one.
Everything else that prints money asks the tax system which sign to use; a
stylesheet cannot ask anything.

**Now:** the program tells it. `--currency` is set on `.layout` from the active
tax system, and the rule reads `content: var(--currency)`. Verified in the real
app: the boxes read "€ 10000" alongside euro figures.

**Two deliberate choices worth knowing.**

There is **no literal fallback** in the CSS. An unset variable renders no
adornment rather than a wrong one — a safe degradation — and it means the
stylesheet can be grepped for currency literals, which is now a CI step. That
grep gap is why this survived Phase C in the first place: the existing boundary
checks only read `.rs` files, so nothing was ever looking at the stylesheet.

**Sign placement is untouched and still yours to decide.** Britain writes £5,
Germany normally writes 5 €. Everything in the app — boxes and answers alike —
still puts the sign first. That is at least *consistent*, which the old state
was not, so it is no longer a bug, just a convention that does not match German
habit. Fixing it properly means the adornment has to be able to sit on either
side, and the output figures should move with it, so it is one decision covering
both.

---

# Still outstanding

## 2. Germany's yearly fund charge treats your deposits as growth — fixed

> **Fixed** by the implementation session (Opus 4.8). The finding and the
> original Assessment are kept intact below; the resolution is appended after
> them. The bug recipe (€149.81) is now pinned permanently the other way by
> `de-tax`'s `deposits_in_the_period_are_not_a_gain_the_charge_can_bite`.

**Where:** `de-tax/src/engine.rs`, the `gain` line in `period_charge`.

Germany charges a small tax each year on funds you are merely *holding*. A fair
rule limits it: the charge can never exceed how much the fund actually rose that
year, and if the fund fell you pay nothing.

The code measures "how much it rose" as end value minus start value. That is
right if you left the fund alone — but if you were paying in monthly, your own
deposits are sitting in the end value, so the sum reads them as growth.

**Verified.** A fund that **fell 10%** over the year but received €20,000 of
deposits ended higher than it started, and was charged **€149.81**. The correct
charge is zero.

### Assessment — implementation session

**Real, and worth fixing — but smaller than it was first assessed.**

Two things make it less alarming than item 1 was. The charge is capped at the
Basisertrag (about 2.24% of the opening value), so this is a *bounded*
over-charge, not an unbounded one. And it only bites during accumulation, when
deposits are flowing; a pure drawdown is unaffected.

I also think the original assessment over-stated the cost. It reads as widening
"the shared agreement between the maths engine and every country's tax rules",
which sounds like a change with wide blast radius. In practice `PeriodPot` is
new, and has **exactly one producer (`calc`) and one consumer (`de-tax`)**.
Adding a `contributed` field to it touches two files and breaks nothing else.
The instinct to treat the shared contract carefully is right in general; this
particular corner of it is barely used yet.

**Recommended:** add `contributed: Decimal` to `PeriodPot`, have `calc` fill it
(it already knows), and compute the cap as
`available − opening − contributed`. Pin it with a test using the numbers above.

**Do this regardless, it costs nothing:** the comment beside that line currently
admits only the *opposite* error — that the cap under-states during drawdown.
It should name both directions. Right now the code documents one of its two
known inaccuracies, which is worse than documenting neither, because it reads as
though the case has been thought through.

### Resolution — implementation session (Opus 4.8)

Done as recommended, `contributed` only. The `withdrawn` question (raised when
this was planned) was decided the way the plan argued: **left alone on purpose.**

What changed, and where:

- **`taxkit/src/lib.rs`** — `PeriodPot` gains `pub contributed: Decimal`, "paid
  into this holding during the period now ending". The doc says *why* it exists
  in jurisdiction-neutral terms (a growth-capped charge must tell a rise from
  the holder's own cash), so the neutral contract stays unwarped; the first
  boundary grep is still clean.
- **`calc/src/engine.rs`** — one `charging`-gated scratch vector
  (`period_contrib`) beside `period_opening`, snapshotting each row's cumulative
  `contributed` at every period open. The period's contribution is
  `contributed[j] − period_contrib[j]`, written into the pot. Zero cost to an
  untaxed or UK projection — the vec stays empty, exactly like `period_opening`.
- **`de-tax/src/engine.rs`** — `gain = available − opening − contributed`. The
  comment now names **both** inaccuracies, and records that the drawdown
  under-statement is left deliberately (see the decision below).
- **`app/src/jurisdiction/de.rs`** — the worked Vorabpauschale example now says
  money paid in over the year is not a gain, so the user-facing statement is no
  longer a slight lie.

**The `withdrawn` decision, recorded.** Adding `withdrawn` would have closed the
opposite (drawdown) direction in the same two files at near-zero marginal cost,
and the temptation was real. It was declined because it would be a *fidelity*
claim I cannot back: §18 InvStG caps the Vorabpauschale by the **per-share**
price movement over the year, and units sold in-year get **no** Vorabpauschale
at all (their gain is taxed on disposal instead). The current value cap,
un-corrected for withdrawals, lands nearer that law than a flow-corrected value
cap would — it is wrong for the wrong reason, but closer to the right number.
Modelling it faithfully means the per-share cap, which is a separate, larger
piece of work (and is the "realised-gain cap during drawdown" that
`de-tax`'s own "does not model" list in `CLAUDE.md` already names). So this fix
deliberately touches only the accumulation direction, and the `de-tax` comment
says so at the line.

**How it is pinned.** `de-tax`'s test uses the file's own numbers (fund
100,000 → 110,000 having received 20,000 of deposits: was €149.81, now €0).
`calc`'s side needed care — `MOCK_LEVY` caps at *nothing*, so it would have
accepted the new field and never read it, leaving the plumbing untested against
the system invariant 14 pins it with. So `MOCK_LEVY` gained an off-by-default
`OPT_CAP_AT_GROWTH` option; one `calc` test switches it on and asserts a
deposit-fed 0%-return period is charged nothing (and, without the cap, *is*
charged — so the zero is the netting-off, not an inert path). Every existing
levy test is untouched, and the plumbing is pinned against a fictional system,
not German figures.

Verified: tax crates, `calc` (89), the app wasm compile-check, the headless UI
suite (54), and all three boundary greps — all green.

## 4. "Unused allowance" counts years you could never have used it — fixed

> **Fixed** by a later session (Opus 5), on the repository owner's decision. The
> finding and the original Assessment are kept intact below; the resolution is
> appended after them, including one correction to the Assessment's proposed
> mechanism. Both halves — the counter and the stub year — were done together,
> because the first turns out to need the second.

**Where:** `calc/src/engine.rs`, the `anchor` line.

Everyone gets a slice of income each year they pay no tax on. Anything unused is
added to an "allowance unclaimed" total, which exists to explain *why* one way
of taking your money beats another.

Germany needed the tax-year clock to start on day one, because its yearly fund
charge happens while you are still saving. But starting the clock also started
the allowance counter, right through the saving-up years — when you are taking
nothing out and so could not possibly have used it.

**Verified.** Twenty idle accumulation years bank **€280,308** of "unused
allowance" with nothing ever withdrawn.

Britain is unaffected: its clock only starts at the handover.

**There is a second problem with the same cause.** The tax year no longer lines
up with the day drawdown begins. Save for 30 months and the clock ticks at 12,
24 and 36 — so the first drawdown "year" is six months long but still carries a
full year of allowances.

### Assessment — implementation session

**This matters more than its size suggests.** `unused_allowance` is the
show-your-working column in the strategy comparison — the one that explains why
one withdrawal order beats another. A number inflated by years that had nothing
to spend does not merely look odd; it actively misleads about the thing that
column exists to justify.

The original note offers two options: `calc` stops the counter until drawdown, or
Germany's rules stop counting during accumulation. It rightly points out that the
second needs Germany to know which phase it is in, which it is not told.

**I would take neither. There is a third option that is cleaner than both:**
have `calc` record `unused_allowance()` at the handover and report only the
growth from that point on. No contract change, no new field, and — the important
part — **`de-tax` never needs to learn what phase it is in.** Phase knowledge
stays in `calc`, which already has it. Keeping tax systems ignorant of the
projection's shape is worth protecting; it is the property that lets a
jurisdiction crate be tested standalone.

On the stub-year half: the suggested `horizon % 12` one-liner does fix the
alignment, and it is correct that it changes reported numbers and must not apply
to Britain. I would treat it as a separate, smaller decision from the counter
problem — they share a cause but not a fix.

**One coupling to be aware of.** If you later model the Günstigerprüfung
properly (item 1), capital income starts consuming the Grundfreibetrag, which
changes both pension taxation *and* this number. **Items 1 and 4 should be
decided together**, or the second will silently undo assumptions made in the
first.

### Resolution — Opus 5 session

Both halves, as decided: **the counter is measured from the handover, and the
tax periods are aligned so that there is a handover to measure from.** The
Assessment treated those as separable ("a separate, smaller decision"). They are
not, and that is the one place this session disagrees with it — see below.

**What changed, and where.**

- **`calc/src/engine.rs`, the boundary test.** Periods were counted forwards
  from the anchor (`(i - anchor) % period_len`). They are now counted *outwards
  from the handover*, in both directions:
  `(i - horizon).rem_euclid(period_len) == 0`. One line, and for a pricing-only
  system — where the anchor *is* the handover — it is the same expression it
  replaced, so nothing about a UK projection moves. The symmetric form was
  chosen over a special-cased extra boundary because it is one uniform rule
  rather than a rule plus an exception, and because the short period it leaves
  is the **first** one, where the pot is smallest and a part period charged in
  full costs least.
- **`calc/src/engine.rs`, the baseline.** `project` records
  `unused_allowance()` once, at the handover boundary, and `Run` carries it. The
  reported figure is the closing figure minus that baseline.
- **`calc/src/types.rs`.** The field's doc already said "across the drawdown".
  It now says why that is load-bearing rather than incidental.
- **The glossary, both jurisdictions.** "Only the years money is being drawn
  count: a year in which nothing came out had no allowance to leave unclaimed."
  The old wording was not wrong, but it was the code that disagreed with it.

**Where this corrects the Assessment.** The Assessment's third option — record
`unused_allowance()` at the handover and report the growth from there — is the
right shape, and it is what was built. But *the handover* is not the right
instant, and the two halves are not independent:

- `unused_allowance()` is **banked + what is left in the currently open period**.
  Subtract it wholesale and you also subtract a full untouched period's
  allowance that belongs to the drawdown. The correct instant is immediately
  **before** the handover's `start_period()`, which is about to bank exactly that
  remainder — at that one moment, the figure equals the banked total. A line
  earlier and the periodic charge's own consumption of the closing period's
  allowance is counted as unclaimed; a line later and the fresh drawdown period's
  full allowance is subtracted away. All three variants look identical at the
  call site and differ by a year's allowance.
- That instant **only exists if a period closes at the handover**. With a growth
  period that is not a whole number of years, the straddling period belongs to
  neither phase and there is nothing to read. Doing the counter alone would have
  left the figure quietly undercounting by whatever the first drawdown months
  spent — a silent wrong number in the column that exists to explain things,
  which is worse than the loud wrong number it replaced.

**The trade, stated plainly.** Aligning the periods moves German figures for any
growth period that is not a whole number of years: the charge dates shift, and
the short first period is charged a full year's Vorabpauschale because the model
does not pro-rate a part period. That is a small bounded over-charge in the safe
direction, and it is the price of the column being trustworthy.

**How it is pinned.** Four tests, all against `taxkit`'s fictional systems, so a
German or UK rate change cannot break them:

- `unused_allowance_counts_only_the_drawdown_years` — under `MOCK_LEVY`, growth
  periods of 120, 240 and 125 months over the same drawdown all report the same
  figure, and it is exactly ten drawdown years' allowance.
- `a_period_closes_exactly_on_the_handover` — a 125-month growth period still
  levies a charge at month 125, visible in `charged_series`. This is the
  alignment half on its own.
- `an_accumulation_only_projection_claims_no_allowance` — nothing drawn, nothing
  reported.
- `a_pricing_only_system_measures_its_whole_session` — under plain `MOCK`, the
  growth period does not move the figure, because every period a pricing-only
  system runs is a drawdown period. This is the "Britain is unaffected" claim,
  pinned rather than asserted.

Both alignment-dependent tests were confirmed to **fail** against the old
forward-anchored boundary before the fix was kept, so neither is inert.

**Not done, and not proposed.** The coupling the Assessment flags stands
untouched: if the Günstigerprüfung (item 1) is ever modelled, capital income
starts consuming the Grundfreibetrag and this figure moves with it. The
mechanism here is indifferent to *what* consumes an allowance, and the tests
pin fictional figures, so that work does not have to revisit this one.

Verified: `calc` (93), de-tax 44, uk-tax 40, taxkit 23, app 86 native module
tests, the app wasm compile-check, the headless UI suite (54), and all three
boundary greps — all green.

## 5. The "year you start drawing" box shows a year it does not use

**Where:** `app/src/jurisdiction/de.rs`.

Germany asks what year you start drawing your pension, because that year locks
in how much of it is taxed, for life. The box offers the current year as a
suggestion — but only *displays* it. Nothing is handed to the maths, which
quietly falls back to whatever year the tax tables were written for.

**Confirmed, and currently latent.** Both say 2026 today, so nothing looks
wrong. It diverges the moment the calendar reaches 2027 before the tables are
refreshed: the box would say 2027 while the sums use 2026, with no warning.

### Assessment — implementation session

**Real but genuinely low priority** — it is invisible until a year boundary, and
the tax tables going stale already raises a warning of its own.

The original fix wrote the default into the option map from a render effect, and
was withdrawn because the write landed a moment late and leaked the jurisdiction
into the following test. That diagnosis is correct, and the fix was rightly
withdrawn rather than papered over.

**Recommended:** seed `base_year` into the options map **synchronously, where
`App` builds it from `ShareState`**, not from a render-time effect. There is no
late write to leak, so the ordering problem never arises. That is a smaller
change than the one that was tried, and it sidesteps the obstacle rather than
fighting it.

An alternative worth considering instead: drop the visible default entirely and
let the box start blank, showing the fallback year as placeholder text. Then
what is displayed and what is used can never disagree, because nothing is
displayed that has not been chosen. Slightly worse as a prompt, but honest by
construction.

---

# Also worth knowing

## The shared "which country are we in" memory is fragile

Surfaced by item 5, and it is worth separating out because it is not really
about item 5.

The app remembers the active jurisdiction in a single shared spot
(`convert::active_system()`, a thread-local) that anything can read at any time,
with nothing guaranteeing it is current when read. **The implementation session
(this file's second author) introduced this in Phase C1; it is that session's
own doing, not a review finding.** It was chosen to avoid threading a
tax-system parameter through about sixty call sites, which was a real trade —
but it left this edge.

**Partly addressed.** The browser suite no longer depends on it: its `kinds()`
helper read the thread-local *before* mounting, so once any test switched to
Germany the next test picked up the German catalogue while mounting a British
state. It now asks the catalogue for the jurisdiction the tests actually seed,
which makes the suite order-independent. That was a real latent fault — adding
one test that switched jurisdiction was enough to break an unrelated one.

**Not addressed.** The underlying weakness stands. It is not a production bug —
in the browser there is one `App`, and `build_input` sets the value on every
recomputation — but it is a sharp edge for tests and for any future caller that
is not `App`. If it trips anyone again, the fix is to make the active system a
reactive context (as the currency symbol already is) rather than a thread-local.

---

# Summary

| # | Fix | How bad | Blocking decision |
|---|-----|---------|-------------------|
| 1 | ~~No tax on German capital~~ | ~~Serious~~ | **Fixed.** Faithful version optional, decide with 4 |
| 2 | ~~Deposits counted as growth for the fund charge~~ | ~~Medium, bounded~~ | **Fixed.** Per-share cap optional, left deliberately |
| 3 | ~~Input boxes showed £~~ | ~~Medium~~ | **Fixed.** Sign placement still open |
| 4 | ~~Unused allowance counted while only saving~~ | ~~Medium, misleads~~ | **Fixed.** Counter and alignment both done |
| 5 | Start-year box shows one year, uses another | Small, latent | None — seed it synchronously |

Everything else the review found is already fixed and tested. Current state
(verified after the item 4 fix): calc 93, de-tax 44, uk-tax 40, taxkit 23, app
86 native module tests, 54 browser tests, the app wasm compile-check, and all
three boundary greps — all passing.
