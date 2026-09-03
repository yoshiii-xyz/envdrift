# Design

## Evidence model

Reports contain stable observations keyed by a subject and state. Values are
limited to versions, safe source labels, bounded counts, digests, and explicit
status words. Observations are sorted before serialization.

## Collection path

1. Locate the selected Cargo manifest.
2. Parse Cargo.toml and Cargo.lock.
3. Ask Cargo for workspace metadata with `--offline` and `--no-deps`.
4. Search project and ancestor `.cargo/config.toml` or `.cargo/config` files,
   then Cargo home configuration.
5. Search for the nearest rust-toolchain file using rustup precedence.
6. Inspect bounded Cargo home, target, and Git workspace state.
7. Record runtime identities and unresolved limitations.

No collector changes the project, cache, registry, or toolchain. Output paths
are the only intentional filesystem writes.

## Explanation

`explain` maps observations by `(subject, state)` and reports added, removed,
or changed values. It does not infer a causal fact from a changed digest.
Missing evidence remains unresolved.
