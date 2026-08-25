# Investment Projection

A single-page tool that projects the future value of a group of investments.
Each holding is entered as its **value today** (principal plus any historical
compounding already baked in), an **annualised return**, and an optional
**monthly deposit** (an ongoing recurring investment). Enter a horizon (in
months or years) and it extrapolates each holding forward, sums them into a
portfolio, and charts the result.

A **mode switch** flips the whole tool between two questions:

- **Building it up** — grow the portfolio over the horizon. Goal-seek asks *what
  monthly top-up reaches £X* or *how long until £X*.
- **Drawing it down** — grow for the horizon, then spend the **projected** pot
  down over a second period, taking a monthly withdrawal from the whole
  portfolio (split across holdings pro-rata and rebalanced each month). Goal-seek
  asks *how much can I withdraw and empty it exactly on time* or *how long a
  given withdrawal lasts*. The chart runs continuously across both phases.

Amounts can be typed however you like — `10000`, `10,000` and `£10,000` all
parse. The chart is scrubbable: point at it, or focus it and use the arrow
keys, to read the projected value at any month along the way. It follows your
system light/dark preference.

**Live demo:** <https://d3anparker.github.io/investment-projection/>

> **Not financial advice.** This is a mathematical extrapolation from a rate you
> supply — for entertainment and curiosity, not planning. Past performance does
> not predict future results.

## Architecture: Rust all the way down

Money should not be calculated in binary floating point (where `0.1 + 0.2`
famously isn't `0.3`). This app is written **entirely in Rust**, compiled to
**WebAssembly**, as a Cargo workspace of two crates:

- [`calc/`](calc/src/lib.rs) — a **pure library** that does every financial
  calculation in exact base-10 decimals (`rust_decimal`). No UI, no WASM
  specifics, no floating point. It also owns input parsing and validation, so
  invalid input comes back as a typed error naming the field that caused it.
  Unit-tested natively (`cargo test`).
- [`app/`](app/src/main.rs) — a [Leptos](https://leptos.dev) (client-side) UI
  that owns the reactive form and *formats* the `Decimal`s `calc` returns. It
  calls `calc::calculate()` directly with shared types and performs no
  arithmetic of its own.

The workspace root is a *virtual* manifest (`[workspace]` only, no package), so
both crates are named peers.

Because both layers are Rust, there is no JS↔WASM string boundary and no
JavaScript to keep in sync — the UI and the core share the same types. Nothing
is sent anywhere and nothing is stored; reload to start fresh. The page loads
with a worked example.

## Run it (Docker — no toolchain needed)

```bash
docker compose up --build
```

Then open <http://localhost:8080>. The build uses [Trunk](https://trunk-rs.github.io/trunk/)
to compile the app to WebAssembly and serves the static output via nginx. Stop
with `Ctrl+C`.

## Deploying (GitHub Pages)

Every push to `main` publishes the site to GitHub Pages via
[`.github/workflows/deploy.yml`](.github/workflows/deploy.yml). The workflow runs the
test suite (`cargo test --workspace`) and, only if it passes, builds the app with Trunk
and deploys the static output with the official Pages actions — no branch to maintain.

The one Pages-specific detail is the build flag `trunk build --release --public-url ./`.
Trunk's default output references assets with root-absolute paths (`/app-<hash>.wasm`),
which 404 when the site is served from a project subpath like
`https://<user>.github.io/investment-projection/`; `--public-url ./` makes those
references relative so they resolve under the subpath. There is no router, so relative
paths are unambiguous.

First-time setup: create a public repo, push, then set **Settings → Pages → Source** to
**GitHub Actions** and re-run the workflow.

## How the numbers work

Each period (years or months) is converted to whole months in `Decimal`
(fractional periods are rounded there, not in the UI). The growth period is
clamped to 1–1200 months (100 years); in drawdown mode the growth and drawdown
periods together are capped at 1200. Each holding compounds its value-today
forward at its **annualised rate**: the monthly factor is `(1 + annual)^(1/12)`,
and each month the running value is compounded and then the **monthly deposit**
is added at month end (so today's value is unaffected — the money isn't invested
yet). Month 0 is today's value, so the series has `totalMonths + 1` points; the
portfolio line is the sum across holdings. Fractional exponents use
`rust_decimal`'s decimal `powd`, so no `f64` enters the calculation.

In **drawdown mode** the accumulation runs exactly as above for the growth
period, producing the *handover pot* (`series[growthMonths]`). After that, each
month the running value is compounded and then a single portfolio-level
**withdrawal** is taken, apportioned across the holdings in proportion to their
current value and rebalanced every month — so the whole portfolio empties
together if the draw outpaces the returns. The withdrawal is capped at the pot,
so the reported total never exceeds what was there. It is one continuous series;
the handover is just the month deposits stop and withdrawals begin.

**Projected growth** is reported as *returns only*: the final value less today's
value *and* less the net cash you moved in (deposits minus withdrawals), so
neither your own deposits nor money you withdrew is counted as investment
performance. Its percentage is that gain over the capital actually deployed
(today's value plus all deposits) — a simple return on capital, not a
money-weighted IRR. The UI states that denominator ("of £39,000 put in") rather
than leaving a bare percentage to be interpreted.

The per-holding table shows deposits (and, in drawdown mode, the handover value
and the amount withdrawn) as their own columns for the same reason. A row
reading `£10,000 → £53,881.86 at +7.00%` looks wrong on its own — £10,000 at 7%
for ten years is £19,671 — until you can see the £24,000 of monthly deposits
that bridge the gap.

## Accessibility

Accessibility here is deliberate rather than incidental. The following are
implemented and have been checked against the running app — this is not a claim
of full WCAG conformance, which would need a proper audit:

- Every colour pair passes AA contrast in **both** light and dark themes.
- Validation errors name the control they belong to (`aria-invalid` +
  `aria-describedby`), rather than stranding a sentence at the foot of the form.
  Announcements are debounced and polite, so a screen reader isn't interrupted
  mid-word while you are still typing.
- A half-typed number never blanks the results — the last good figures stay on
  screen, dimmed and marked `aria-busy`.
- Removing a holding moves focus to the next remove button (or to
  "+ Add investment"), so keyboard users aren't dropped back at the top of the
  page.
- The chart is exposed as a slider with `aria-valuetext`, so its intermediate
  values are reachable by keyboard, not just by hovering a mouse.
- Gain and loss are distinguished by an explicit `+`/`-` sign and by the label
  wording, not by green-vs-red alone.

## Developing

A [devcontainer](.devcontainer/devcontainer.json) (Rust + `wasm32` target +
Trunk) is provided. Inside it, or on any machine with Rust, the `wasm32` target,
and Trunk installed:

```bash
./dev.sh          # trunk serve with live reload on http://localhost:8080
cargo test -p calc   # run the calculation-core unit tests (native, fast)
cargo test           # the above plus the pure app-module tests
# Headless-browser UI suite for the App component (needs a browser + geckodriver;
# see test.Dockerfile for the containerised run):
cargo test -p app --target wasm32-unknown-unknown --test ui
```

## Layout

| Path                 | What it is                                                        |
| -------------------- | ---------------------------------------------------------------- |
| `Cargo.toml`         | Virtual workspace manifest (`[workspace]` only) + shared profile |
| `calc/src/lib.rs`    | Pure exact-decimal calculation core + its unit tests             |
| `app/src/main.rs`    | Leptos UI: mount + the reactive form/results components          |
| `app/src/format.rs`  | `Decimal` → display-string formatting (with tests)              |
| `app/src/chart.rs`   | Hand-built SVG portfolio chart + its plot geometry (with tests)  |
| `app/index.html`     | Trunk entry point (Trunk runs from `app/`)                       |
| `app/styles.css`     | Styles; single source of the light and dark palettes, referenced by the chart |
| `Dockerfile`         | Multi-stage: Trunk build → serve with nginx                      |
| `docker-compose.yml` | One-command run                                                  |
| `dev.sh`             | Local `trunk serve` helper (`cd`s into `app/`)                   |
| `.github/workflows/deploy.yml` | GitHub Actions: test, then build with Trunk and deploy to Pages |
