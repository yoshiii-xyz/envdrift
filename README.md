# envdrift

Envdrift compares a Rust project's Cargo declarations and lockfile with local toolchain, cache, source, target, and installed state.

Status: published 0.1.0 MVP

CI: [workflow](https://github.com/joshiii-xyz/envdrift/actions/workflows/ci.yml)
Release: [v0.1.0](https://github.com/joshiii-xyz/envdrift/releases/tag/v0.1.0)

## Install

Install the published binary with:

```bash
cargo install envdrift --version 0.1.0 --locked
```

## Quick start

```bash
envdrift scan
envdrift export --format json --output .envdrift/baseline.json
envdrift explain --baseline .envdrift/baseline.json
```

Run the export again after a controlled environment or project change, then
compare the new report with the saved baseline.

## What it solves

Envdrift makes local Rust project state explicit. It reports whether evidence
was declared, locked, downloaded, built, installed, observed, or unresolved.
It is Cargo-only and does not manage environments or install packages.

## How it works

The scanner locates the manifest, invokes `cargo metadata --format-version 1
--no-deps --offline`, parses Cargo.toml and Cargo.lock, searches Cargo's
hierarchical config files, discovers rust-toolchain files using rustup's
nearest-file rules, and inspects bounded local cache and target metadata.
Runtime identities come from installed Cargo, rustc, and rustup commands.

`explain` compares stable JSON observations from a saved baseline with a new
offline scan. Missing, malformed, or unmapped evidence stays unresolved.

## Commands or library API

```text
envdrift scan [--format text|json] [--output PATH]
envdrift explain [--baseline PATH]
envdrift export --format json [--output PATH]
envdrift version
```

The Rust library exposes `scan_project`, `explain_reports`, and the versioned
`Report` and `Observation` types for local integrations.

## Output and exit codes

JSON reports are deterministic and use schema version 1. Paths inside the
workspace are relative labels. External paths are represented by a digest.
Configuration values with credential-like names and URL credentials are
omitted or redacted.

Exit code 0 means the requested operation completed. Exit code 2 represents a
CLI, manifest, report, or output error. A successful scan can still contain
observations with state `unresolved`.

## Safety, privacy, and data handling

The scan invokes Cargo with `--offline` and reports
`network_access_attempted: false`. It does not read Cargo credential files or
write to a project's source tree unless an explicit `--output` path is given.
It hashes external paths and file contents rather than storing their raw
locations or values. Reports can still reveal project structure and should be
treated as local evidence.

## Limits and non-goals

The MVP supports Cargo projects on Linux. Cache and target inventories are
bounded. Cargo fingerprint internals are not interpreted as a stable public
interface. A present target file does not prove that a package is current, and
an absent cache entry does not prove that no network fetch could succeed.
Python, uv, environment installation, hosted services, and human usability
testing are outside this release. See [docs/limits.md](docs/limits.md).

## Testing and development

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --locked
cargo package --locked
cargo audit
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the change workflow and
[docs/release.md](docs/release.md) for release evidence.

## Release and support status

Version 0.1.0 is published on crates.io and released on GitHub. It is a
Linux-first MVP and makes no production-readiness claim.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before proposing a change.

## License

Envdrift is released under the [MIT License](LICENSE).
