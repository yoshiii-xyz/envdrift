# Contributing

Keep changes narrow and evidence-backed. Envdrift is a Cargo-only Linux-first
tool. New evidence sources must state their source, bound their scan, and keep
unknown results explicit.

Before opening a change, run:

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --locked
cargo package --locked
cargo audit
```

Add a deterministic fixture or adversarial test for behavior changes. Do not
include credentials, cache archives, target artifacts, or generated reports.

All changes are reviewed before commit and publication.
