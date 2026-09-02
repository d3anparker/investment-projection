# The jurisdiction glossary — a phased plan

A reader can currently see *what* a projection says and not *what any of it
means*. Account kinds carry a one-line `note`, the German panel carries two
sentences, and everything else — Vorabpauschale, Teilfreistellung, the handover,
"deployed", pro-rata — is a word on screen with no way to ask what it is. This
plan adds a **glossary that is part of what a tax-system implementation is**,
reachable from one button, shown in a modal that fills the screen.

It is written to be finished **across several merges**. Each phase is
independently mergeable, leaves the app working, and carries its own acceptance
gate. Tick the boxes as they land.

| Phase | What it delivers | Status |
| --- | --- | --- |
| **G1** | The contract (`taxkit::GlossaryEntry`) and both jurisdictions' entries. No UI. | ☑ landed |
| **G2** | The shared app terms, the modal, the button, the mobile-first styling. | ☑ landed |
| **G3** | Content projection: a jurisdiction may insert its own panel into the modal. | ☑ landed |
| **G4** | Filtering, docs, the two update skills, the CI guard. | ☑ landed |

---

## Why the content lives in the tax crates

The obvious shortcut is a big const in `app` with a `match` on the jurisdiction
id. It is wrong for the same reason `AccountKind::note` is not in `app`: the
words that explain a tax system are *part of that tax system*, they change when
its rules change, and they are exactly what a rate-update skill should be
re-reading. Splitting them from the rules guarantees they rot.

So the glossary follows the precedent already set by `AccountKind::note` and
`TaxSystem::source_note`: **the neutral contract names the shape of an
explanation and never the words.** `taxkit` gains a `GlossaryEntry` type and one
defaulted trait method; `uk-tax` and `de-tax` each write their own terms in
their own vocabulary; `app` renders whatever it is handed and names nobody.

This is not a weakening of the jurisdiction boundary — it is the boundary
working.

### Rejected: a per-jurisdiction glossary module in `app/src/jurisdiction/`

`app/src/jurisdiction/de.rs` already exists and could hold the German terms, so
this looks like the cheaper option. Rejected because:

- It puts the *content* one crate away from the rules it describes. A German
  rate update edits `de-tax/src/tables.rs`; if the glossary lived in `app`, the
  update skill would need a second scope in a second crate, and those skills are
  deliberately narrow.
- The tax crates are natively testable with no wasm target. A glossary in `app`
  can only be structurally tested in a build that drags in Leptos.
- A third jurisdiction would get a glossary *slot* for free but no glossary, and
  nothing would notice. As a trait method with a per-crate coverage test, each
  implementation is held to explaining its own catalogue.

What `app/src/jurisdiction/` **does** get is G3's projection slot: prose, worked
examples and layout that a `&'static [GlossaryEntry]` cannot express. Data in
the tax crate, bespoke presentation in the app — the same split as "numbers live
in `calc`, presentation lives in `app`", one level up.

### Rejected: making `glossary()` required rather than defaulted

Tempting, since "every implementation must explain itself" is the goal. But
`taxkit::mock` is a fictional system whose whole value is being minimal, and a
required method would make it write a glossary for a tax that does not exist.
Defaulted-empty, with the coverage test living in each *real* crate, buys the
same discipline without warping the contract — the same reasoning that leaves
`marginal_headroom` defaulted.

---

## Phase G1 — the contract and the content

**No UI. `app` is untouched. Gate: the tax-crate tests pass and the boundary
greps are clean.**

### G1.1 `taxkit::GlossaryEntry`

```rust
/// One term a reader meets in this system's labels, notes and figures, and
/// what it means.
///
/// The contract names the *shape* of an explanation and never the words: every
/// term, definition and grouping label is written by the implementation, as
/// `AccountKind::note` and `source_note` already are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlossaryEntry {
    /// Stable, opaque id. Never parsed; a UI uses it for anchors and keys.
    pub id: &'static str,
    /// The term as it appears in the interface.
    pub term: &'static str,
    /// An abbreviation, alternative name or translation. May be empty.
    pub also: &'static str,
    /// What it means, in plain language.
    pub definition: &'static str,
    /// Where the reader meets it — which card, column or figure this explains.
    /// May be empty for background terms.
    pub seen_in: &'static str,
    /// Grouping label, so a long list reads as sections. Entries sharing a
    /// topic are expected to be adjacent.
    pub topic: &'static str,
    /// Ids of related entries.
    pub see_also: &'static [&'static str],
}
```

`seen_in` is the field that turns a word list into "how to understand the data",
and it is the one a reviewer will be tempted to drop as redundant. It is not: it
is the only thing tying a term to the figure it explains.

### G1.2 The trait method

```rust
// on TaxSystem
/// Terms this system's own labels and figures use, and what they mean.
///
/// Defaulted empty: a system need not explain itself, and the mock does not.
/// Entries are in presentation order and grouped by `topic`.
fn glossary(&self) -> &'static [GlossaryEntry] { &[] }
```

Watch the CI word list: the doc comment above must not contain any term the
`taxkit names no jurisdiction` grep bans — which includes the substring `eur`.

### G1.3 `uk-tax` and `de-tax` entries

A new `glossary.rs` in each crate — **data, like `tables.rs`, not mechanism** —
and `glossary()` returning it. Roughly:

- **UK** — ISA, GIA, SIPP, the personal allowance and its withdrawal, marginal
  rate, capital gains and the annual exempt amount, tax-free cash and the lump
  sum allowance, the normal minimum access age, the four UK tax jurisdictions,
  frozen thresholds versus uprating.
- **Germany** — §32a and the Grundfreibetrag, Solidaritätszuschlag with its
  Freigrenze, Kirchensteuer, Abgeltungsteuer, Sparer-Pauschbetrag,
  Teilfreistellung, Vorabpauschale and the Basiszins, Splittingverfahren,
  Besteuerungsanteil and the cohort year, Rürup, bAV, the 12/62 rule.

Definitions are **descriptive, never advisory** — the rule the `note` fields
already follow. "Applies to X, so the figure in column Y is net of it", never
"you should".

### G1.4 Structural tests, per crate

- ids unique and non-empty; `term` and `definition` non-empty
- every `see_also` id resolves to an entry in the same glossary
- entries sharing a `topic` are contiguous
- **coverage**: every `AccountKind` the system advertises is mentioned by some
  entry — the test that stops a new account kind shipping unexplained

No figures are asserted, so a rate update cannot break them.

**Gate.** `cargo test -p taxkit -p uk-tax -p de-tax --features taxkit/mock`
green; the taxkit grep silent; `cargo tree -p calc` still naming no jurisdiction
crate; the app build untouched.

---

## Phase G2 — the modal

**Gate: the UI suite passes, including the new tests below; the wasm build is
clean; the stylesheet grep still finds no currency.**

### G2.1 `app/src/glossary.rs` — shared terms and pure helpers

The jurisdiction explains its taxes; nothing explains *this app's own model*. So
`app` owns a second const slice, of the same `taxkit::GlossaryEntry` type (one
type, one renderer, two sources): growth period and handover, drawdown,
deployed, growth versus return, pro-rata, the withdrawal strategies, gross
versus net, the periodic charge, why nothing is ranked, and why this is not
advice.

These are app concepts, so they name no jurisdiction and no currency — the
existing greps keep them honest.

The module is pure and natively tested: the section-grouping helper
(`sections(&[GlossaryEntry])`) and, in G4, the filter. The `web_sys` surface is
two calls (`show_modal`, `close`) and stays in the component.

### G2.2 The dialog

Native `<dialog>` driven by `show_modal()`, not a hand-rolled overlay. It gives
the focus trap, `Esc`, the inert background and the top layer for free, and
those are exactly the parts a hand-rolled modal gets wrong. The `firefox-esr` in
`test.Dockerfile` is ESR 128, so the UI suite can drive it.

- `aria-labelledby` on the dialog pointing at its own heading.
- The close control carries a visible text label, not only an icon glyph.
- Focus returns to the trigger on close — `showModal()`/`close()` do this, but
  it is worth a UI test, being the thing that silently regresses.
- The dialog renders **inside** the app root, not on `document.body`, so the UI
  suite's "query within the returned root" convention still holds.

### G2.3 The button

On the `.mode-switch` row, beside the jurisdiction picker: that row is the
page-context band, and the glossary explains both the mode and the jurisdiction.
Visible text "Glossary"; the accessible name *extends* it ("Glossary — what
these terms mean") rather than replacing it, per WCAG 2.5.3.

### G2.4 Styling, mobile first

The base rule is the small screen: `100dvw`/`100dvh`, no radius, no margin — a
full-bleed sheet. `dvh`, not `vh`, or mobile browser chrome crops the footer. A
`min-width` media query then constrains it to a large centred panel with a
`max-width` and a radius.

- Sticky header holding the title and the close control, so close is always
  reachable without scrolling back up.
- Body scrolls, with `overscroll-behavior: contain` so a flick at the end of the
  list does not scroll the page behind it.
- `::backdrop` tinted from the existing tokens; defined in **both** theme
  blocks, since `styles.css` is the single source of the palette for each.
- Respect `prefers-reduced-motion` for any open transition.
- Check contrast against `--panel`, not against white.

### G2.5 UI tests

Opening from the button shows the dialog; `Esc` closes it; closing returns focus
to the button; the German glossary shows German terms and the UK one does not;
switching jurisdiction swaps the terms; the shared app terms appear under both.

---

## Phase G3 — content projection

The slot the request asked for, and deliberately *after* the generic renderer,
so the generic path is the one that has to work.

```rust
// on Jurisdiction, beside settings_panel / notes_panel
/// Extra glossary content: prose, a worked example, anything a flat term list
/// cannot say. Projected into the modal after the generic entries.
pub glossary_panel: Option<fn(GlossarySlot) -> View>,
```

`GlossarySlot` follows the existing slot discipline — handed what it needs, and
may not compute, derive or round anything. Germany fills it with a worked
Vorabpauschale example (opening value, Basiszins, the charge, and where that
figure lands in the breakdown); the UK leaves it `None` and **no wrapper element
renders at all**, the same courtesy the region select and the other two panels
already get. A UI test pins that absence.

---

## Phase G4 — polish, docs, guards

- **Filter box** in the modal, pure and natively tested in `glossary.rs`.
  Matches `term`, `also` and `definition`; an empty result says so rather than
  showing a blank panel. Mobile keyboards make this worth more than it looks.
- **`CLAUDE.md`** — the glossary in the crate descriptions; the modal in the
  page-structure table; a gotcha for `<dialog>` and one for `dvh`.
- **`README.md`** — one line, since it is user-facing.
- **Both update skills** gain a step: when a rule changes, check whether its
  glossary entry still describes it. This is the payoff for putting the content
  in the tax crates, and should not be skipped.
- **CI** — the four existing greps already cover the boundary. Add nothing new
  unless G2/G3 introduce a literal; re-run all four.

---

## Verification

The commands from `CLAUDE.md`, unchanged. Each phase ends with the ones its gate
names; run the whole set before the final merge.

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test -p taxkit -p uk-tax -p de-tax --features taxkit/mock --target-dir /target
```

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo ip-wasm-test cargo test -p app --target wasm32-unknown-unknown --test ui --target-dir /target
```

Under Git Bash these need the `MSYS_NO_PATHCONV=1` prefix.

---

## What landed, and what a later author should know

All four phases are in. Two things worth recording because they are not
obvious from the diff:

- **No new CI guard was needed.** G4 planned for one and the four existing
  greps turned out to cover it: the terms live in the tax crates, so `app`
  still names no jurisdiction; `taxkit` gained a type and not a word; and a
  native test in `app/src/glossary.rs` covers the one thing grep could not —
  that the app's own entries print no currency symbol, the symbol being
  reactive and a literal being the thing that would go stale.
- **`\u{2014}`, not `--`, in a user-facing string.** The tax crates use `--`
  freely in *doc comments*, which is house style, and it is easy to carry that
  habit into a `definition` — where it renders as two hyphens on screen beside
  the em dashes the rest of the UI uses. Same for quotation marks: `\u{201c}`
  and `\u{201d}`, not `'`.

Left undone deliberately: nothing from the plan. The obvious next additions,
if the glossary is extended, are a per-entry deep link (which needs a scheme
that does **not** touch the location fragment — see the note in `term_of`) and
richer projected panels for a third jurisdiction.

## Who wrote what

Phases and rationale drafted in the session that added this file. The two
"Rejected" sections record decisions taken there, not settled repo doctrine, so
a later author is free to reopen them — but should append an attributed section
rather than rewriting these, so the record of what was decided on what grounds
survives. The convention `REMAINING-FIXES.md` uses.
