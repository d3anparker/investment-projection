# Already drawing — a plan

Every projection this app can express begins **today, with a growth phase**. You
save, you hand over, you draw down. A holder who is already two years into their
drawdown cannot say so.

The arithmetic for that person is not wrong — today's values already contain the
last two years, so projecting forward from them is exactly right. What is missing
is the *concept*: nothing in the model, the controls or the wording admits that
drawing may have started before the projection does. This is a plan for adding
it.

---

# Who wrote this, and what kind of claim each part is

Written by an **Opus 5 session**, from a conversation with the repository owner
about the German start-year control. It is a *proposal*, not a finding, and the
two are separated below on purpose:

- **"Verified"** means read out of the code, with the file and line given. Take
  those as facts about the repository as it stands.
- **"Proposed" / "Recommended"** is this session's judgement, and should be
  argued with rather than followed.

Nothing here has been implemented. No code was changed to produce it.

**If you add to this file,** append an attributed section rather than editing an
existing one. The value of a plan like this is the record of who concluded what,
on what evidence; rewriting it in a later voice destroys that.

---

# Why this is not just "type 0 in the growth box"

It nearly is, and that is the trap.

The data model already supports it: `horizon_value` is a `String` on
`CalcInput`, the share codec already round-trips `"0"`, and a projection with no
growth phase is a legal shape for the month loop. One validation rule is all
that stands in the way.

But shipping it as "type 0" would mean:

- **nobody finds it.** There is no wording anywhere that suggests the growth
  period may be zero, and a user who has already retired has no reason to think
  a calculator about *growing* money is for them;
- **the page then lies in five places.** A zero growth period leaves labels and
  alt text reading "After 0 years of growth", "growing to £847,000 after 0
  years", and a "Value at handover" column that duplicates today's value;
- **the inputs still ask the wrong questions.** "Age when it starts" and "Year
  drawing starts" are future-tense controls being handed past facts.

The concept is new even though the arithmetic is not. That is the whole of the
work.

---

# What already works — verified

Each of these was read out of the code and needs no change. They are recorded
because they are the reason this feature is small rather than large.

1. **A past year is already accepted where it matters, and the control is
   already honest about it.** The German start-year box is a plain number input
   with no bound, and `besteuerungsanteil_for` (`de-tax/src/tables.rs:175`)
   clamps below its first cohort row and above its last. `start_year` has
   **exactly one consumer** (`de-tax/src/engine.rs:311`), so a past year cannot
   disturb anything else — no uprating, no threshold scaling, no allowance.

   The box also no longer pre-fills a year. It shows only a year that was
   *chosen*, with the fallback as placeholder text, so it neither asserts a
   future year nor contradicts what the sums use. That change was made for its
   own reasons, but it is a precondition for this feature: a box defaulted to
   the current year would fight a holder entering 2024, and the fix is why there
   is no default left to fight.

2. **The share link needs no version bump.** `horizon_value` is a `String`, so
   `"0"` encodes and decodes today. A new field would be `#[serde(default)]`
   anyway, per the codec's stated preference.

3. **The chart already copes.** `chart_svg` filters the handover divider to
   `h > 0` (`app/src/chart.rs:174`), so a zero-length growth phase draws no
   spurious dashed rule and no "drawdown" label at the y-axis.

4. **The tax-period machinery lands correctly at zero.** With no growth phase
   the anchor and the handover coincide at month zero
   (`calc/src/engine.rs:649`), so the unused-allowance baseline is never
   captured (`calc/src/engine.rs:788`) and the whole session counts as drawdown
   — which is the right answer, because every period *is* a drawdown period.
   This falls out of invariant 15 rather than needing a special case.

5. **The goal-seek already reports the right thing.** `Solution::Depletes` is
   `m - horizon_months` (`calc/src/solve.rs:346`), which at zero is the absolute
   month, which is also the drawdown month. Correct without change.

6. **One ambiguity dissolves.** `SessionSpec::age` means "age when the drawdown
   begins". For someone who started at 60 and is now 62 that is currently an
   unanswerable question; with the drawdown beginning *now*, it is simply their
   age today.

## And one thing that gets *better*

The workaround available today — set the growth period to its one-month floor —
is **worse than a true zero under Germany**, not merely uglier.

Tax periods are counted outwards from the handover
(`calc/src/engine.rs:740`), so a one-month growth period closes a period at
month 1 and levies a **full year's Vorabpauschale immediately**, on the whole
pot. A zero-length growth period puts the first boundary at month 12, where it
belongs.

So this feature does not merely tidy the workaround; it removes a real
over-charge that the workaround incurs. (The over-charge is bounded and in the
safe direction, which is why it is a wrinkle and not a bug — see invariant 15's
note that a part period is charged in full.)

---

# What blocks it, or reads wrong — verified

| # | Where | What happens |
|---|-------|--------------|
| 1 | `calc/src/engine.rs:267` | `horizon_months_of` rejects anything below 1 month. **The only hard blocker.** Shared by `calculate` and all three solvers. |
| 2 | `app/src/format.rs:99` → `app/src/summary.rs:36` | `horizon_label(0)` is `"0 years"`, so the summary card reads "After 0 years of growth". |
| 3 | `app/src/results.rs:144` | The chart's accessible description reads "growing to £X after 0 years, then drawn down to £Y". |
| 4 | `app/src/summary.rs:33` | `handover_total` is today's total, so the card duplicates the headline figure. |
| 5 | `app/src/results.rs:185` | The per-row table shows a "handover" column identical to the value-today column. |
| 6 | `app/src/app.rs:707`, `:732`, `:818` | "Grow for", "then draw down for", "Age when it starts" — all future-tense. |

Note that (2)–(6) are all **presentation** — wording that reads wrongly at zero,
and two cards that would duplicate a figure. Only (1) is a rule, and nothing
here is a correctness problem: the arithmetic is already right.

---

# The shape of the feature — proposed

Three ways to expose it. They differ in discoverability and in how much of the
existing model they disturb.

### Option A — a third top-level mode (recommended)

`.mode-switch` gains a third radio: **"Already drawing it down"**. `calc::Plan`
gains a variant carrying the drawdown fields but no growth period.

- **For:** the mode switch's stated job is to reconfigure the whole page
  context, and this genuinely does — it removes a period control and changes
  what two others mean. A new `Plan` variant makes every `match` site fail to
  compile until it is handled, which is exactly the discipline the codebase uses
  for `StrategyChoice`. Discoverable by someone who would never think to type 0.
- **Against:** the biggest change. Three `Plan` variants, a three-way radio
  group, and every `match plan` site touched. A link written under the new mode
  and opened by an older build would decode to Deposits and quietly project
  something else — a non-issue for a single-version static site, but worth
  recording.

### Option B — a toggle inside drawdown mode

A checkbox: "I have already started drawing". Hides the growth period, switches
the labels, reveals the past-fact controls.

- **For:** no new `Plan` variant; the state is derivable from
  `horizon_months == 0`, so nothing new goes in the share link.
- **Against:** deriving the toggle's position from the horizon box is fragile in
  both directions (type 0 by hand and the toggle flips under you), and storing
  it separately means two sources for one fact. A control that changes the
  meaning of other controls also sits awkwardly beside the app's own rule that
  a control must appear exactly when the projection uses it.

### Option C — drop the floor and nothing else

Allow `horizon = 0` in drawdown mode, fix the five wording sites to read
sensibly at zero, and stop.

- **For:** by far the smallest. Honest — it adds no concept, it removes an
  arbitrary restriction.
- **Against:** solves the arithmetic and none of the discoverability. Nobody
  will find it, and the past-fact inputs stay future-tense.

**Recommended: A.** The owner's framing is the deciding argument — this is a new
concept, and a new concept that only exists as an unusual value in an existing
box is not really in the product. C is a legitimate first phase *of* A, and is
sequenced that way below.

---

# Phases

Each phase leaves the app shippable.

### Phase 1 — make zero legal (`calc`)

- `horizon_months_of` takes the mode into account: the 1-month floor stays for
  **deposits** (a zero-length accumulation is genuinely meaningless there) and
  drops for **drawdown**. The existing `guards_reject_bad_input` test
  (`calc/src/tests.rs:157`) is a deposits-mode case and stays green untouched.
- The two `.expect("horizon >= 1 guarantees a point")` messages
  (`calc/src/engine.rs:1000-1001`, `calc/src/solve.rs:151`) still hold — a
  drawdown of at least one month keeps two points in the series — but their
  stated *reason* becomes wrong and must be re-worded.
- Tests: a zero-horizon drawdown projects, reconciles (invariant 5), and reports
  `handover_total == current_total`; the same run under `MOCK_LEVY` levies its
  first charge at month 12, not month 0; `unused_allowance_total` counts every
  period (the baseline is never taken).

### Phase 2 — the past facts

The start-year control is **already done** — it shows only a chosen year, with
the fallback as placeholder text, so nothing needs undoing here. What remains is
the wording around it and one limit worth stating plainly.

- **Do not reintroduce a default, in any form.** The tempting one is the
  *handover* year — today plus the growth period — and it is wrong precisely
  here: it assumes drawing begins when the projection begins, which is the
  assumption this feature exists to break. The year drawing started is a fact
  about the holder's life. The app cannot derive it, and a derived default would
  be a guess wearing a fact's clothing. This is recorded in CLAUDE.md as a rule
  about settings panels generally; it applies with extra force in this mode.
- **The honest limit.** A blank box still uses the tax tables' year, so someone
  who started drawing in 2024 and ignores the control gets 84.00% rather than
  83.00%. The placeholder makes that year *visible*; only entering a year makes
  it *right*. In a mode explicitly for people who started in the past, that gap
  matters more than it does today — so this mode is the place to consider
  prompting harder for it, not merely permitting it.
- **The label is future-tense** — "Year drawing starts" — and should read as a
  past question in this mode. It is a `de-tax` const (`BASE_YEAR_LABEL`), so
  wording that varies by mode means either a second const or an app-side choice
  between two the crate exports. Prefer the second: the words stay with the
  rules.
- The **age** control's label becomes "Age now" here, for the same reason.

### Phase 3 — the wording (`app`)

The five presentation sites in the table above, plus the mode's own labels.
Proposed wording, to be argued with:

- headline: "Drawing down from today" rather than a growth card;
- the handover card and the per-row handover column are **suppressed**, not
  relabelled — at zero they carry no information the value-today column lacks;
- chart alt text drops its growth clause entirely;
- "then draw down for" becomes "Draw down for".

### Phase 4 — tests

- Browser: mounting in the new mode shows no growth-period control, no handover
  card, no handover column, and the chart draws no phase divider.
- Browser: the German panel's year box is empty with a placeholder, and a typed
  past year survives a round trip through the share link.
- Native: `share` round-trips the new mode; an unknown mode still falls back to
  deposits (`convert::plan_from`'s existing permissive rule).

---

# Explicitly out of scope

Naming these so they are not quietly absorbed later:

- **Historical performance.** No past returns, no "what my pot did since 2024".
  The value-today figure is the only history the app has ever modelled, and this
  changes nothing about that.
- **Contributions during drawdown.** The model stops deposits at the handover;
  a zero-length growth phase means no deposits at all. Someone both drawing and
  paying in is a different feature.
- **Inflation or real-terms figures.** Already a documented rejection; being
  two years in does not change it.
- **A tax history.** Allowances used in previous years, carried-forward losses,
  Vorabpauschale already paid — all outside what `TaxSession` models, and all
  properly a much larger piece of work.

---

# Open decisions

1. **A, B or C** for the shape of the feature. (Recommended: A, with C as its
   first phase.)
2. **Does the 1-month floor stay for deposits mode?** Recommended yes — a
   zero-length accumulation is meaningless there, and keeping it means the
   existing guard test is untouched.
3. **Suppress or relabel** the handover card and column at zero. (Recommended:
   suppress.)
4. **How hard should this mode prompt for the start year?** Leaving it blank is
   legal and uses the tables' year, which for a holder who started in the past is
   quietly wrong. Options run from the placeholder it has today, through a note
   in this mode only, to treating a blank as an error. Recommended: a note, not
   an error — the app validates numbers, not omissions, and an error here would
   be the first control that refuses to project without an answer.
