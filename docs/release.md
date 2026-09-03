# Release

The 0.1.0 MVP was published to crates.io and released on GitHub on
2026-09-03. The release evidence is recorded privately under
`qa/evidence/iteration-2026-09-03.md`.

## Dry run

Run from a clean checkout:

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --locked
cargo package --locked
cargo audit
```

Run the bounded fuzz gate with an explicit nightly toolchain:

```bash
timeout --foreground 60s env RUSTUP_TOOLCHAIN=nightly cargo fuzz run report -- -max_total_time=10
```

Inspect the result and record the exact command and output under private
`qa/evidence/`. Generated corpus and target directories must stay ignored.

## Publication order used for 0.1.0

The local gate, installed smoke test, hosted CI, security, and CodeQL checks
passed before publication. The crate was published with `cargo publish
--locked`, verified with `cargo info` and a fresh `cargo install`, then tagged
as `v0.1.0`. The tag package workflow passed before the GitHub release was
created.

For a future release, use this order:

```text
local gate
installed smoke test
review and commit
push main
hosted CI, security, and CodeQL
cargo publish --dry-run --locked
cargo publish --locked
verify registry availability
push annotated tag
wait for release package workflow
verify public install and smoke test
record evidence
```

Do not overwrite a published crate version. Yank or deprecate only through the
registry's documented process, and publish a corrected version after a new
full gate.
