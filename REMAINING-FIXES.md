# Still to fix

Five things came out of the code review. **Two are now fixed** (1 and 3); three
are still open (2, 4 and 5).

The original numbering is kept so earlier discussion still lines up. Each open
item now carries what was actually measured and a recommendation, so the
decision left to you is a genuine choice rather than an open question.

| # | Item | State | Next step |
|---|------|-------|-----------|
| 1 | No tax on German capital with no other income | **Fixed** `05ea147` | Optional: model it faithfully |
| 2 | Deposits counted as growth for the yearly fund charge | Open | Widen `PeriodPot` — cheaper than first thought |
| 3 | Input boxes showed £ under Germany | **Fixed** `05ea147` | Optional: decide £5 vs 5 € |
| 4 | Unused allowance counted during the saving-up years | Open | Decide who owns it — I suggest `calc` |
| 5 | Pension start-year box shows one year, uses another | Open | Seed it synchronously |

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

## 2. Germany's yearly fund charge treats your deposits as growth

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

### My view

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

## 4. "Unused allowance" counts years you could never have used it

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

### My view

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

## 5. The "year you start drawing" box shows a year it does not use

**Where:** `app/src/jurisdiction/de.rs`.

Germany asks what year you start drawing your pension, because that year locks
in how much of it is taxed, for life. The box offers the current year as a
suggestion — but only *displays* it. Nothing is handed to the maths, which
quietly falls back to whatever year the tax tables were written for.

**Confirmed, and currently latent.** Both say 2026 today, so nothing looks
wrong. It diverges the moment the calendar reaches 2027 before the tables are
refreshed: the box would say 2027 while the sums use 2026, with no warning.

### My view

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
with nothing guaranteeing it is current when read. **This was introduced in
Phase C1 and is my responsibility, not the review's.** It was chosen to avoid
threading a tax-system parameter through about sixty call sites, which was a
real trade — but it left this edge.

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
| 2 | Deposits counted as growth for the fund charge | Medium, bounded | None really — widen `PeriodPot`, 2 files |
| 3 | ~~Input boxes showed £~~ | ~~Medium~~ | **Fixed.** Sign placement still open |
| 4 | Unused allowance counted while only saving | Medium, misleads | Who owns it — I suggest `calc` at the handover |
| 5 | Start-year box shows one year, uses another | Small, latent | None — seed it synchronously |

Everything else the review found is already fixed and tested. Current state:
calc 88, de-tax 38, uk-tax 36, taxkit 23, app 73, 46 browser tests, the Trunk
production build, and all four boundary greps — all passing.
