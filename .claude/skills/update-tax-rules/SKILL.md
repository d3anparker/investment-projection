---
name: update-tax-rules
description: Refresh the UK tax figures in the uk-tax crate — rates, thresholds, allowances, the account catalogue — against current published sources, and stamp them with today's date. Use when asked to update the tax rules, the tax year, the tax tables, the uk-tax crate, or the rates/allowances a drawdown projection uses; or when the app is showing a stale-figures warning.
---

# Updating the UK tax rules

## What you are editing, and what you are not

`uk-tax/src/tables.rs` is **data**: every threshold is a whole number of pounds
(`i64`), every rate is basis points (`u32`). A rate update is a change of integer
literals and nothing else.

`uk-tax/src/engine.rs` is **mechanism**: how those figures become a rate schedule
and how a withdrawal is priced against it. **Do not edit it as part of a routine
update.** If a change needs it, that is a decision for the user, not something to
implement unprompted — report it and stop (see step 5).

Everything else in the repo is off-limits here. `calc` and `app` are written
against the `taxkit` traits and never name a jurisdiction; if an update seems to
need a change there, something has gone wrong.

## Procedure

### 1. Establish where you are starting from

Work out today's date and which UK tax year it falls in — the year runs 6 April
to 5 April, so 5 April and 6 April are in *different* years. Then read the
current state:

```bash
grep -n "label:\|as_of:\|source_note:" uk-tax/src/tables.rs
```

If `LATEST.label` already names the current tax year and `as_of` is recent, say
so and ask whether the user wants a re-check anyway rather than doing redundant
work.

### 2. Research, one line item at a time

Work down this checklist, which mirrors the `TaxYear` struct field for field.
Search for each item separately — a single "UK tax rates" search returns a
summary that blurs the year boundaries, and it is exactly the figures that
changed which such a page is most likely to get wrong.

| Field | What to confirm |
| --- | --- |
| `personal_allowance_gbp`, `pa_taper_threshold_gbp`, `pa_taper_divisor` | The allowance, the income at which it starts being withdrawn, and the rate of withdrawal |
| `bands_england`, `bands_wales`, `bands_northern_ireland` | Rates and thresholds **on taxable income**, i.e. after the allowance |
| `bands_scotland` | Scotland's own bands — also on taxable income, so subtract the personal allowance from published gross figures |
| `bands_savings`, `bands_dividends` | UK-wide for every jurisdiction, including Scottish taxpayers |
| `basic_rate_limit_gbp` | Top of the UK basic-rate band — the pivot between the two capital gains rates |
| `dividend_allowance_gbp`, `psa_gbp`, `savings_starting_rate_gbp` | The dividend allowance, the personal savings allowance by band, the 0% starting rate band |
| `cgt_annual_exempt_gbp`, `cgt_rate_basic_bp`, `cgt_rate_higher_bp` | Annual exempt amount and both rates |
| `pcls_bp`, `lump_sum_allowance_gbp`, `normal_minimum_pension_age`, `mpaa_gbp` | Tax-free pension fraction, the lifetime cap on it, the access age, the money purchase annual allowance |
| `isa_allowance_gbp`, `lifetime_isa_allowance_gbp`, `junior_isa_allowance_gbp` | Subscription limits |

Rules for sourcing:

- **Prefer `gov.uk` and `gov.scot`.** A commercial summary is corroboration, never
  the sole basis for a change.
- **Watch the effective date.** Budget announcements usually take effect the
  *following* April, so a figure "announced" is often not a figure "in force".
  The table describes a tax year; put a figure in the year it actually applies to.
- **Scottish thresholds are usually published gross.** The struct wants them net
  of the personal allowance. Getting this wrong shifts every Scottish band.
- If a figure cannot be confirmed from a good source, **say so and leave it
  unchanged** rather than carrying over a guess.

### 3. Add a new tax year — never edit an existing one

Old tables cost nothing and are the only record of what the figures used to be.

- Add a new `TY_YYYY_YY` const modelled on the existing one.
- **Prepend** it to `TAX_YEARS` (newest first) and point `LATEST` at it.
- Set `as_of` to today's date.
- Put the URLs you actually used in `source_note`.

If a genuinely identical year has already been recorded, updating its `as_of` and
`source_note` in place is right — you re-checked it, you did not discover a new
year.

### 4. Update the account catalogue only where it is safe

Adding an `AccountKind` to `UK_ACCOUNTS` is in scope **only** when its taxation
matches an existing `WithdrawalTax` variant. It needs a matching entry in
`UK_TREATMENT` and a place in `UK_CONVENTIONAL_ORDER`; the tests enforce all
three, so a half-added account fails loudly.

Note where a limitation belongs in the `note` field rather than being silently
dropped — it is shown to the user.

### 5. Report what you could not do

Stop and describe, rather than implementing, anything that needs `engine.rs`:

- a new **kind** of taxation (a new `WithdrawalTax` variant)
- a structural change the fields cannot express — a limit that varies by age, a
  band that depends on something not modelled
- an account whose taxation does not match any existing treatment

These change behaviour, not figures, and they deserve a decision.

### 6. Verify

**There is no Rust toolchain on this machine** — `cargo` is never run bare.
Every command goes through Docker. Start with the tax crates, which is where a
bad table shows up:

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test -p taxkit -p uk-tax --features taxkit/mock --target-dir /target
```

Then the whole workspace, to be sure nothing downstream shifted:

```bash
docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test --workspace --features taxkit/mock --target-dir /target
```

`uk-tax`'s tests check the tables are internally consistent — bands ascending from
zero, no rate at or above 100%, the taper exactly exhausting the allowance,
`TAX_YEARS` newest-first, every account having exactly one treatment and one
place in the order. A failure there is usually a transcription error, not a
broken test.

`calc`'s tests run against a **fictional** tax system on purpose, so they should
be unaffected by anything you do here. If a `calc` test fails after a table
update, something has leaked across the boundary — investigate rather than
adjusting the test.

### 7. Report as a diff table

Finish with a table of what changed, so the user can check your work without
reading the diff:

| Field | Was | Now | Source |
| --- | --- | --- | --- |

List separately: anything you could not confirm and therefore left alone, and
anything you are reporting under step 5.

## Note on framing

This is a tool for curiosity, not tax advice, and the app says so. Keep the
`note` fields factual and descriptive — what a wrapper is and what the model does
not cover — and never evaluative about which account someone should use.
