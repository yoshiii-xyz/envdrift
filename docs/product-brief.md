# Product brief

Envdrift compares a Rust project's declarations and lockfile with the actual
local toolchain, cache, source configuration, target metadata, and installed
state.

The first release is Cargo-only and Linux-first. It supports `scan`,
`explain`, and JSON `export`. Evidence is labeled `declared`, `locked`,
`downloaded`, `built`, `installed`, `observed`, or `unresolved`.

The MVP does not install or change environments, manage packages, support
Python or uv, call hosted services, or claim that a local state is portable.
