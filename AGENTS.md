# Agent instructions

## Project context

- Build command: `cargo build --locked`
- Test command: `cargo test --all-targets --locked`
- Lint command: `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features --locked -- -D warnings`
- Type check command: `cargo check --all-targets --locked`
- Key configuration files: `Cargo.toml`, `Cargo.lock`, `.github/workflows/`
- Architecture constraint: one Linux-first Rust CLI and library; no workspace members

## Scope

Envdrift compares Cargo declarations with local evidence. Keep the MVP
Cargo-only, offline, deterministic, and bounded. Do not add environment
installation, package management, Python or uv support, hosted services, or a
frontend.

## Verification

After filesystem edits, read the changed files back. Before a release, run the
full commands listed in README.md, inspect `git diff --check`, and record exact
results under the private `qa/evidence/` directory. Do not call unresolved or
environment-limited observations passes.

## Safety and privacy

Never read or commit registry credentials, tokens, private keys, or login
files. Keep scans offline. Hash external paths and redact credential-like
values. Do not modify a user's Cargo cache, toolchain, or project source as
part of scanning.
