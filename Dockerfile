# syntax=docker/dockerfile:1
# --- Stage 1: build the Leptos app (Rust) to WebAssembly with Trunk ----------
FROM rust:1-bookworm AS build

RUN rustup target add wasm32-unknown-unknown \
 && cargo install trunk --locked --version 0.21.14

WORKDIR /src
COPY . .

# The web entry lives in the `app` crate. Trunk must run from that crate's
# directory: in a virtual workspace `cargo metadata` only resolves a root
# package when invoked from within the member, which Trunk needs to build it.
WORKDIR /src/app

# Trunk compiles the crate to WASM, runs wasm-bindgen + wasm-opt, and emits a
# self-contained static site (index.html + hashed .js/.wasm/.css) into dist/.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/root/.cache \
    trunk build --release

# --- Stage 2: serve the static site ------------------------------------------
FROM nginx:1.27-alpine

COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/app/dist /usr/share/nginx/html

EXPOSE 80
