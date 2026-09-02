---
name: update-german-tax-rules
description: Refresh the Germany tax figures in the de-tax crate — the §32a tariff coefficients, surcharge rates, allowances, the Vorabpauschale Basiszins, the cohort Besteuerungsanteil, the account catalogue — against current published sources, and stamp them with today's date. Use when asked to update the German tax rules, the German tax year, the de-tax crate, or the rates a German drawdown projection uses; or when the app shows a stale-figures warning under Germany. For the United Kingdom, use update-uk-tax-rules instead.
---

# Updating the German tax rules

## What you are editing, and what you are not

`de-tax/src/tables.rs` is **data**: every euro threshold is a whole `i64`, every
rate is basis points (`u32`), and every §32a tariff coefficient is an `i64`
number of *hundredths of a euro* (so `914.51` is written `91_451`). An update is
a change of integer literals and nothing else.

`de-tax/src/tarif.rs` (the §32a walker) **and** `de-tax/src/engine.rs` (the trait
impls and dispatch) are **mechanism**. **Do not edit either as part of a routine
update.** A change that needs them is a decision for the user, not something to
implement unprompted — report it and stop (see step 5).

`de-tax/src/glossary.rs` is **data too, and in scope** — the terms the UI shows
a reader, and what they mean. It lives beside the rules precisely so an update
re-reads it: after changing a figure, check that the entry describing that rule
still describes it. Three standing rules there:

- **No rates and no thresholds in a definition.** Figures live in `tables.rs`;
  a definition that quotes one is a second place to update and a silent way to
  go stale. Name the allowance, do not price it.
- **Descriptive, never advisory.** Say what a rule does and which figure it
  moves — never what a reader should do about it.
- **The German term stays the `term`; the English gloss goes in `also`.** The
  German word is what appears everywhere else in the interface, so it is what a
  reader is looking up. A test pins each account entry's `term` to the
  catalogue's own label.

Adding an account kind means adding its glossary entry, with the entry's `id`
equal to the kind's; a coverage test fails otherwise.

Everything else in the repo is off-limits here. `calc`, `taxkit` and `app` are
written against the `taxkit` traits and never name a jurisdiction; if an update
seems to need a change there, something has gone wrong.

## Procedure

### 1. Establish where you are starting from

The German tax year is the **calendar year**, so there is no 6-April trap: a
date's tax year is simply its year. Read the current state:

```bash
grep -n "label:\|as_of:\|source_note:" de-tax/src/tables.rs
```

If `LATEST.label` already names the current year and `as_of` is recent, say so
and ask whether the user wants a re-check anyway rather than doing redundant work.

### 2. Research, one line item at a time

Work down this checklist, which mirrors the `TaxYear` struct field for field.
Search for each item separately — a single "German tax rates" search returns a
summary that blurs boundaries, and it is exactly the figures that changed which
such a page is most likely to get wrong.

| Field(s) | What to confirm |
| --- | --- |
| `grundfreibetrag_eur`, `zone2_top_eur`, `zone3_top_eur`, `zone4_top_eur` | The §32a zone boundaries (E0–E3), on the zvE |
| `zone2_a_cents`, `zone2_b_cents` | Zone 2's `(a·y + b)·y` coefficients, in hundredths of a euro |
| `zone3_a_cents`, `zone3_b_cents`, `zone3_c_cents` | Zone 3's `(a·z + b)·z + c` coefficients, in hundredths |
| `zone4_sub_cents`, `zone5_sub_cents`, `upper_rate_bp`, `top_rate_bp` | The linear zones: `0.42·x − sub` and `0.45·x − sub`, and the two flat rates |
| `soli_bp`, `soli_freigrenze_eur`, `soli_milderung_bp` | Solidaritätszuschlag rate, the Freigrenze **on the tax** (single), and the Milderungszone cap |
| `kirchensteuer_bp` | The 8% / 9% church-tax rates |
| `kapest_bp`, `sparer_pauschbetrag_eur` | Abgeltungsteuer rate and the Sparer-Pauschbetrag (single) |
| `basiszins_bp`, `vorab_faktor_bp` | The Vorabpauschale Basiszins (a separate BMF-Schreiben each January) and the 70% factor |
| `besteuerungsanteil` | The cohort table: the taxable share for a pension *starting* in each listed year |

Rules for sourcing:

- **Prefer `gesetze-im-internet.de`** (§32a, §32d, §20, §22 EStG; InvStG) **and
  `bundesfinanzministerium.de`.** A commercial summary is corroboration, never
  the sole basis for a change.
- **Watch the effective date.** German changes arrive by *Jahressteuergesetz* or
  the *Inflationsausgleichsgesetz*, usually late in the *preceding* year. Put a
  figure in the year it actually applies to (`ab dem Veranlagungszeitraum …`).
- If a figure cannot be confirmed from a good source, **say so and leave it
  unchanged** rather than carrying over a guess.

### 3. Add a new tax year — never edit an existing one

Old tables cost nothing and are the only record of what the figures used to be.

- Add a new `TY_YYYY` const modelled on the existing one, and a matching
  `BESTEUERUNGSANTEIL_YYYY` cohort slice.
- **Prepend** it to `TAX_YEARS` (newest first) and point `LATEST` at it.
- Set `as_of` to today's date, and put the URLs you actually used in `source_note`.

### 4. The §32a coefficients are the transcription trap — transcribe them as a set

The single most important rule here. The five zones must join up: the tax value
**and its marginal rate** are continuous at each zone boundary, and the coefficients
are tuned together to make that so. **Mixing a coefficient from one year with a
boundary from another produces a tariff that looks plausible and is wrong** — and
mis-prices every pension withdrawal silently.

The four invariant tests in `tables.rs`/`tarif.rs` are the guard: 14% just above
the Grundfreibetrag; the marginal rate continuous at the zone 2/3 boundary
(≈23.97% both sides); 42% at the top of zone 3; and a monotone (income-tax)
marginal throughout. **Run them and believe them** — if a published set fails
them, the set is wrong (or mis-transcribed), not the test.

Other jurisdiction-specific traps:

- Figures are very often quoted for **Zusammenveranlagung** (doubled). The table
  wants the **single**-assessment figures; splitting is derived.
- Online "Grenzsteuersatz" tables are rounded. Use the statute's coefficients.
- The Soli **Freigrenze** is a threshold on the **tax**, not on income.
- The **Besteuerungsanteil is a cohort figure** — the share for a pension
  *starting* in a given year, fixed for life. Extend the table with a new row for
  the new start-year; never overwrite an existing cohort's share.

### 5. Update the account catalogue only where it is safe, and report the rest

Adding an `AccountKind` to `DE_ACCOUNTS` is in scope **only** when its taxation
matches an existing `WithdrawalTax`/`Treatment` shape. It needs a matching
`DE_TREATMENT` entry and a place in `DE_CONVENTIONAL_ORDER`; the tests enforce all
three. Note any limitation in the `note` field rather than dropping it — it is
shown to the user.

Stop and describe, rather than implementing, anything that needs `engine.rs` or
`tarif.rs`:

- a new **kind** of taxation (a new `WithdrawalTax` variant), or a structural
  change the fields cannot express;
- the Vorabpauschale's realised-gain cap during drawdown; social contributions
  (Kranken-/Pflegeversicherung) on pension income; a genuine annual
  Günstigerprüfung election; or capital-loss buckets — all currently
  simplifications or non-modelled, each a decision, not a figure.

### 6. Verify

**There is no Rust toolchain on this machine** — `cargo` is never run bare. Start
with the tax crates, which is where a bad table shows up:

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test -p taxkit -p de-tax --features taxkit/mock --target-dir /target
```

Then the whole workspace, to be sure nothing downstream shifted:

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test --workspace --features taxkit/mock --target-dir /target
```

`de-tax`'s tests check the tables are internally consistent — the §32a
continuity invariants, the cohort table ascending, every account having exactly
one treatment and one place in the order, `needs_cost_basis` matching a gain-taxed
treatment, `TAX_YEARS` newest-first. A failure there is usually a transcription
error, not a broken test. `calc`'s tests run against a **fictional** tax system on
purpose, so they should be unaffected; if one fails, something leaked across the
boundary — investigate rather than adjusting the test.

### 7. Report as a diff table

Finish with a table of what changed, so the user can check your work without
reading the diff:

| Field | Was | Now | Source |
| --- | --- | --- | --- |

List separately: anything you could not confirm and therefore left alone, and
anything you are reporting under step 5.

## Note on framing

This is a tool for curiosity, not tax advice, and the app says so. Keep the `note`
fields factual and descriptive — what a wrapper is and what the model does not
cover — and never evaluative about which account someone should use.
