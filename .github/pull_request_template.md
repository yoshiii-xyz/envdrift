## Change summary

<!-- Keep the Cargo-only Linux-first scope explicit. -->

## Evidence

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --all-targets --locked`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] `cargo package --locked`
- [ ] `cargo audit`
- [ ] Changed behavior has a deterministic fixture or adversarial test.
