# syntax=docker/dockerfile:1
# Image for the headless-browser UI suite (app/tests/ui.rs). Build once, then
# run the documented `docker run ... cargo test --test ui` command against it;
# the named cargo/target volumes keep repeat runs incremental. Separate from the
# release Dockerfile, which is Trunk/nginx and needs no browser.
FROM rust:1-bookworm

RUN rustup target add wasm32-unknown-unknown \
 && apt-get update \
 && apt-get install -y --no-install-recommends firefox-esr ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

# geckodriver — pinned, like TRUNK_VERSION. wasm-bindgen-test-runner drives
# Firefox through it.
ARG GECKODRIVER_VERSION=v0.36.0
RUN curl -fsSL "https://github.com/mozilla/geckodriver/releases/download/${GECKODRIVER_VERSION}/geckodriver-${GECKODRIVER_VERSION}-linux64.tar.gz" \
      | tar -xzf- -C /usr/local/bin \
 && chmod +x /usr/local/bin/geckodriver

# wasm-bindgen-test-runner must match the wasm-bindgen version *exactly* or it
# refuses the module ("schema version mismatch"). Derive the version from
# Cargo.lock so there is no fourth constant to keep in step. Prefer the prebuilt
# binary; fall back to a from-source install if the release layout ever differs.
COPY Cargo.lock /tmp/Cargo.lock
RUN WB=$(grep -A1 '^name = "wasm-bindgen"$' /tmp/Cargo.lock | grep '^version' | head -1 | cut -d'"' -f2) \
 && echo "wasm-bindgen ${WB}" \
 && ( curl -fsSL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WB}/wasm-bindgen-${WB}-x86_64-unknown-linux-musl.tar.gz" \
        | tar -xzf- --strip-components=1 -C /usr/local/bin \
        "wasm-bindgen-${WB}-x86_64-unknown-linux-musl/wasm-bindgen-test-runner" \
      || cargo install wasm-bindgen-cli --locked --version "${WB}" )

# wasm-bindgen-test-runner reads this to find the driver; the runner supplies
# its own headless + no-sandbox capabilities.
ENV GECKODRIVER=/usr/local/bin/geckodriver
