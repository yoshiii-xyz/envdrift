# Research record

Date: 2026-09-03

## Current sources

- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
  defines the versioned machine-readable workspace and package output. The
  implementation uses format version 1 with `--no-deps`.
- [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html)
  documents hierarchical `.cargo/config.toml` discovery, Cargo home
  configuration, and target directory settings.
- [Cargo command reference](https://doc.rust-lang.org/cargo/commands/cargo.html)
  documents `CARGO_HOME`, the Cargo home cache layout, and local manifest
  commands.
- [Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)
  documents Cargo's build and toolchain-related environment surface.
- [cargo locate-project](https://doc.rust-lang.org/cargo/commands/cargo-locate-project.html)
  documents manifest discovery and workspace selection.
- [rustup overrides](https://rust-lang.github.io/rustup/overrides.html)
  documents precedence for command-line, environment, directory override, and
  rust-toolchain selection.
- [rustup environment variables](https://rust-lang.github.io/rustup/environment-variables.html)
  documents `RUSTUP_TOOLCHAIN` and related toolchain state.

## Evidence grades

Documented Cargo and rustup command behavior is evidence-backed for the
interfaces cited above. Cache directory presence, target artifact names, and
Git status are local observations. Freshness and causal conclusions remain
inferences or unresolved states.

## Decision

Build a deterministic offline scanner around Cargo's public machine-readable
metadata and manifest commands, with bounded filesystem inspection for cache,
target, source configuration, and toolchain evidence. Keep the report schema
versioned and make every unsupported or incomplete claim explicit. Do not
become an environment manager or add Python support in this release.

## Rejected options

- Running `cargo fetch` would change state and violate the no-network scan
  boundary.
- Reading rustup's internal settings file would couple the tool to a schema
  that the rustup documentation does not promise; use `rustup show` instead.
- Treating a target artifact as proof of a current build would overstate what
  a bounded filename and timestamp inspection can establish.
