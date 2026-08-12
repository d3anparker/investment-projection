# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A static, single-page tool that projects the future value of a group of investments. Each holding is entered as its **value today** (principal plus any historical compounding already baked in) plus a return figure — either an annualised rate, or a total return expected over the whole projection horizon — and an optional recurring monthly contribution. That value-today figure is projected straight forward; there is no historical holding period. No backend, no persistence; loads with an illustrative example. It is explicitly not financial advice — it extrapolates, and the UI says so.

Note on the growth figure: `growth` is returns-only — `projected_total − current_total − contributed_total` — so a user's own future deposits are never reported as investment gains; `growth_pct` measures that gain against deployed capital, reported as `deployed` (`current_total + contributed_total`) so the UI can state the denominator instead of showing a bare percentage. With no contributions this reduces to the old `projected − current`.

Two output fields exist purely so the figures **reconcile on screen**, and should not be dropped as redundant: `InvestmentResult::contributed` (a row showing `£10,000 → £53,881.86 at +7.00%` is nonsense without the `£24,000` of top-ups that bridge it) and `CalcOutput::deployed` (as above).

**The whole app is Rust compiled to WebAssembly** — a pure calculation library plus a [Leptos](https://leptos.dev) client-side UI. There is no JavaScript.

## Architecture

A **virtual Cargo workspace** (the root `Cargo.toml` is `[workspace]`-only — no root package) with two named member crates:

- **`calc/` (library)** — every financial calculation, in exact base-10 `Decimal` (`rust_decimal`). Pure: no UI, no WASM/`web_sys`, no floating point. Exposes typed `calculate(&CalcInput) -> Result<CalcOutput, CalcError>` plus the `Unit`/`Mode`/input/output types. It also owns *input parsing*: `parse_number` accepts numbers the way users type them (`1,234.56`, `£1,234`, `7 %`), so the UI never pre-cleans a string. This is the single source of numeric truth and is natively unit-testable.
- **`app/` (binary)** — the Leptos UI, and the web entry it owns (`app/index.html`, `app/styles.css`). Split into modules: `app/src/main.rs` (mount + the `App`/row components + input-string→`calc`-enum helpers), `app/src/format.rs` (`Decimal`→display-string formatting), and `app/src/chart.rs` (the hand-built SVG). It owns the reactive form state and **only formats** the `Decimal`s that `calc` returns. It must not compute, derive, round, or validate any number — that all belongs in `calc`. The one non-`calc` numeric code is `chart.rs`'s pixel geometry (cosmetic `f64`, never a reported figure). `format.rs` and `chart.rs` are pure and carry their own `#[cfg(test)] mod tests`.

The load-bearing rule: **numbers live in `calc`, presentation lives in `app`.** Because both are Rust they share types directly — there is no JSON/string boundary between them (a deliberate change from the earlier wasm-bindgen design).

## Reactive data flow (Leptos)

- Each investment row is a `Row` of per-field `RwSignal<String>`s (so typing in one cell doesn't disturb others); `rows` is an `RwSignal<Vec<Row>>`.
- A single `create_memo` (`outcome`) reads every input signal, builds a `calc::CalcInput`, and calls `calculate`. Reading the signals inside the memo is what makes the projection recompute on any edit; the memo caches so `calculate` runs once even though the error line and results panel both read it.
- Blank rows (empty value today, rate, AND contribution) are filtered out before calling `calculate`. **This breaks the index correspondence** between `CalcInput::investments` and the rows on screen, so the memo records a parallel `row_ids: Vec<usize>` in its `Outcome`; that is what lets a `CalcError`'s index be mapped back to the right `Row`.
- `calculate` never panics; invalid input comes back as `Err(CalcError)` carrying a `message` and an optional `field` (`Field::Investment { index, part }` or `Field::Horizon`). The UI shows the message in `.error-msg` *and* marks the named control with `aria-invalid` + `aria-describedby` + `.field-invalid`.
- The results panel renders `displayed`, a memo that holds the **last good** `CalcOutput` through a transient error (dimmed via `.stale` + `aria-busy`) rather than blanking. It only falls back to the empty state when there are genuinely no rows.
- The error is announced through a debounced sr-only `role="status"` region, not by making `.error-msg` itself live — recomputation happens on every keystroke, and a live region there talks over the user.
- Removing a row goes through `remove_row`, which also **moves focus** (see Accessibility). `Row` therefore carries its own `remove_btn: NodeRef<html::Button>`, created in `new_row` rather than inside the `For` body, so a *sibling* row's handler can reach it.

## Page structure

Three panels inside `.layout`:

| Panel | Heading | Holds |
| --- | --- | --- |
| `.panel-summary` | "Projection" | The four stat cards. `grid-column: 1 / -1` on desktop — full width above the other two |
| `.panel` | "Your investments" | The row editor and the horizon control |
| `.panel.results` | "Breakdown" | The chart (with scrubber) and the per-holding table |

The summary is hoisted out of the results column deliberately: the form is inherently shorter than the results, and side by side that left a ragged hole in the page. The lower two panels then `align-items: stretch` to equal height, and `.horizon` takes `margin-top: auto` to spend the form's spare height rather than leave a gap.

## Accessibility conventions

These are load-bearing and easy to regress silently — a change that "looks fine" can still break them.

- **Errors name a control, not just a problem.** `CalcError::field` drives `aria-invalid`, `aria-describedby=ERROR_ID` and `.field-invalid` on the offending input. A new error site in `calc` must pick a `Field`, or the message strands itself at the foot of the form again.
- **Never put a live region on something a memo rewrites per keystroke.** The visible `.error-msg` is inert; announcement goes through the separate debounced (`ANNOUNCE_DELAY`) sr-only `role="status"`. `role="alert"` here interrupts a screen reader mid-word on every character typed.
- **Destructive controls move focus.** `remove_row` focuses the successor row's remove button, or `+ Add investment` when the list empties; otherwise focus falls to `<body>` and a keyboard user restarts at the top of the page. No `request_animation_frame` is needed because the `For` is keyed by `Row::id`, so surviving rows keep their DOM nodes and their `NodeRef`s are already populated.
- **Direction never rests on colour alone.** Gain/loss uses `fmt_signed_money` (explicit `+`/`-`) *and* switches the label between "Projected growth" and "Projected loss". The green/red is a third, redundant cue.
- **The chart is a `role="slider"`, not a hover tooltip.** `aria-valuetext` carries the readout; arrows step a month, PageUp/Down a year, Home/End the ends. The visible `.chart-readout` mirrors it but is `aria-hidden` so the value is announced once, not twice.
- **Accessible names must contain the visible label** (WCAG 2.5.3). The remove button reads "Remove Global Equity Fund" while showing "Remove"; that's fine, but an `aria-label` that *replaces* rather than extends visible text breaks voice control.
- **Both themes must pass AA.** Check each token against the surface it actually sits on (`--panel` vs `--panel-2` vs `--bg`), not against white or black.
- **Hit targets** are enlarged with a centred `::after` overlay, so the pointer target reaches 44px without changing the drawn control or the grid row height.

## Build & run

**There is no Rust toolchain on the dev machine** — `cargo` and `rustc` are not on PATH, so no bare `cargo` command will run. Docker is installed, and every build and test goes through it. Never tell the user to run `cargo` directly, and never report a change as verified without having run one of the containerised commands below.

The two fast commands share three named volumes (auto-created on first use) holding the cargo registry, the rustup toolchain, and a Linux `target/`. They are what make repeat runs incremental — a *bind-mounted* target dir on this Windows host does not work, cargo recompiles everything every time. Budget ~2 min for the first cold run of each; after that both return in about a second.

- **Compile-check the app** — catches Leptos `view!` macro and type errors without the Trunk/WASM bundling step. This is the command to use while iterating:
  ```bash
  docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -v ip-rustup:/usr/local/rustup -w /repo rust:1-slim sh -c "rustup target add wasm32-unknown-unknown && cargo check -p app --target wasm32-unknown-unknown --target-dir /target"
  ```
- **Test the core** (native, no WASM):
  ```bash
  docker run --rm -v "${PWD}:/repo" -v ip-target:/target -v ip-cargo:/usr/local/cargo/registry -w /repo rust:1-slim cargo test -p calc --target-dir /target
  ```
- **Run the whole app** (Trunk build → nginx at http://localhost:8080):
  ```bash
  docker compose up --build
  ```
  `docker compose build` on its own is the end-to-end check — it runs the Dockerfile's `trunk build --release` over both crates. Roughly 7s for a source-change rebuild thanks to the Dockerfile's cache mounts (see gotchas); budget several minutes the first time on a machine with a cold BuildKit cache.
- **Dev loop** (needs Rust + `wasm32-unknown-unknown` target + Trunk, e.g. the `.devcontainer`): `./dev.sh` runs `trunk serve` with live reload on :8080. It `cd`s into `app/` first — see the Trunk gotcha below.
- Inside the devcontainer, or on any machine that does have the toolchain, the bare commands work directly: `cargo test -p calc`, whole-workspace `cargo test`, and `cargo build --target wasm32-unknown-unknown --release` from the workspace root as a fast compile check.
- **Deploy** is automatic: every push to `main` runs `.github/workflows/deploy.yml`, which gates on `cargo test --workspace` and then builds with Trunk and publishes to GitHub Pages (`https://<user>.github.io/investment-projection/`). The `TRUNK_VERSION` there must be kept in step with the `Dockerfile` and `.devcontainer/devcontainer.json` pin (`0.21.14`). This is a separate build path from the `Dockerfile`/nginx one, which stays as the local run.

`target/` (workspace-root, shared by both crates) and `app/dist/` are build output (git- and docker-ignored). Trunk emits a self-contained `dist/` (hashed `.js`/`.wasm`/`.css` + `index.html`); nginx serves it. The page must be served over `http://` — a `file://` load won't fetch the WASM.

## Gotchas & conventions

- **GitHub Pages serves from a subpath, so the Pages build needs `--public-url`.** Trunk's default output references its hashed assets with root-absolute paths (`/app-<hash>.wasm`), which 404 under `https://<user>.github.io/investment-projection/`. The deploy workflow builds with `trunk build --release --public-url ./` to make those references relative; this is safe because there is no router (single `mount_to_body`, no deep links). The local Dockerfile/nginx build serves from `/`, so it deliberately omits the flag. If a future Trunk normalises `./` back to `/`, switch the workflow to the explicit `--public-url /investment-projection/`.
- **Trunk must run from inside `app/`** (both `dev.sh` and the Dockerfile `cd` there). In a virtual workspace `cargo metadata` only resolves a root package when invoked from within the member crate; run from the workspace root, Trunk fails with "could not find the root package of the target crate". The web entry (`index.html`/`styles.css`) therefore lives in `app/`, not at the repo root.
- **Adding anything that computes/derives/rounds/validates a number goes in `calc/src/lib.rs`, returning a `Decimal`** (or an error), and `app/src/main.rs` formats it. Do not reach for `f64` or JS-style number handling in the UI.
- After changing `calc`, add or update a unit test in its `#[cfg(test)] mod tests` — the core is the one part with real test coverage; keep it that way. `format.rs` and `chart.rs` carry their own tests too; `main.rs` (the reactive/DOM layer) has none, so behaviour there is verified in a browser against the running build.
- **Input parsing is deliberately lenient and must stay non-guessing.** `parse_number` strips whitespace, grouping separators, currency symbols and a stray `%`; anything that is not a plain decimal afterwards still errors (`1.2.3`, `--5`). Note the en-GB assumption: `,` is a thousands separator, never a decimal point.
- **The Dockerfile's `trunk build` uses three BuildKit cache mounts** — the cargo registry, `/src/target`, and `/root/.cache` — which together keep `docker compose build` at ~7s instead of ~59s. `/root/.cache` is the non-obvious one: without it Trunk re-downloads `wasm-bindgen` and `wasm-opt` on every build, which alone was ~16s. They need the `# syntax=docker/dockerfile:1` header — don't drop it. `.dockerignore` excluding `target/` is correct and should stay: the host's `target/` holds Windows-native artifacts that are useless to the Linux build. Cache mounts are the fix, not un-ignoring it.
- **The devcontainer `dev.sh` path (Trunk live-reload) has two host-specific traps.** (1) `trunk serve` must bind `--address 0.0.0.0`, not its `127.0.0.1` default — bound to loopback it's only reachable inside the container, so VS Code's forwarded 8080 (and any host browser) gets nothing. (2) `CARGO_TARGET_DIR` is set to `/home/vscode/target` (container-local, `vscode`-owned) in `devcontainer.json`. Do **not** let cargo/Trunk build into the bind-mounted `target/`: the host dir's files can be root-owned, so the build dies with `failed to open .../target/debug/.cargo-build-lock: Permission denied (os error 13)` → `cargo build ... exit status: 101` → Trunk serves an empty `dist/` and every request 404s (the server still *listens*, so it looks like a networking bug but isn't). The bind-mounted target is also why cargo recompiled everything each run. If a stale root-owned `target/` predates this fix, `sudo chown -R vscode:vscode target` (or `rm -rf target`) unblocks it.
- **Breakpoint debugging works on `calc`, not `app`.** `.vscode/launch.json` has two CodeLLDB (`vadimcn.vscode-lldb`) configs that build and launch the `calc` unit-test binary natively — set a breakpoint in a `calc` test and step through the `Decimal` math. `app` is Rust→WASM in the browser, so there's no native process to attach lldb to; since `app` only formats what `calc` returns, reproduce any wrong number in a `calc` test and debug it there. (Genuine UI/reactive-layer bugs are browser-devtools/Chrome-DWARF territory, a heavier host-side setup that isn't wired into the devcontainer.) Debug builds honour `CARGO_TARGET_DIR`, so CodeLLDB finds the binary with no extra path config.
- `nginx.conf` relies on nginx's bundled `mime.types` and only overrides `.wasm` via a `location` block — don't replace the whole `types {}` map.
- The chart is a hand-built SVG string in `chart.rs` (`chart_svg`) set via `inner_html`; its colours reference the CSS custom properties (`var(--accent)` etc.) from `styles.css` because the SVG is in the live DOM. Keep the palette in `styles.css`, not duplicated in Rust — that is also what makes the light theme work for free, since `@media (prefers-color-scheme: light)` just redefines the same tokens.
- **The chart's label size is coupled to its rendered width.** Text is sized in viewBox units, so what the reader sees is `AXIS_FONT * width / 640`. `AXIS_FONT` (chart.rs) and `.chart-stage`'s `min-width`/`max-width` (styles.css) must be changed together, or the labels shrink below legibility (they were 8px on a phone) or balloon. The gutters `PL`/`PR` are sized against `AXIS_FONT` too, and `fmt_axis` abbreviates past a million so a long label can't outgrow `PL`.
- **The chart scrubber positions itself from `PLOT_*_FRAC`**, exported by `chart.rs` from the same viewBox constants the line is drawn with. Don't hard-code those percentages in CSS — the marker would drift off the data.
- **A `@media` block adds no specificity, so source order decides.** `styles.css` keeps its desktop overrides *after* the base rule they override. `.horizon { margin-top: auto }` sits in a second `@media (min-width: 960px)` block placed below the base `.horizon` rule for exactly this reason — moved up next to the other desktop rules, the later `margin-top: 22px` silently wins and the control stops pinning to the panel foot. Same trap applies to `.field-invalid:focus`, which needs to outrank `input:focus`.
- **`styles.css` is the single source of the palette for both themes.** `@media (prefers-color-scheme: light)` redefines the same custom properties, which is why the chart's SVG follows along without any Rust change — it references `var(--accent)` etc. from the live DOM. Adding a colour means adding it to *both* blocks.
- **Form controls with a non-default initial value need mount-safe binding, not `prop:value`.** In Leptos's `view!` expansion an element's props are set *before* its children mount, so `prop:value` on a `<select>` runs while its `<option>`s don't exist yet — a non-first initial value (e.g. a row defaulting to `"total"`) silently fails to apply and the control falls back to the first option. Drive selects from the options instead: `<option value="total" selected=move || sig.get() == "total">`. Text `<input>`s have the mirror problem (plus a caret-reset) and use the `bind_value` node-ref effect. Default-valued controls appear to work by coincidence because the fallback *is* their value, so test initial rendering with a non-default value.
- Leptos 0.6, stable Rust idioms (`.get()`/`.set()`, not the nightly call sugar). Pin/verify the Leptos version before using newer-version APIs.
