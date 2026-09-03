# Operations

## Baseline workflow

```bash
envdrift export --format json --output .envdrift/baseline.json
# make one controlled local change
envdrift explain --baseline .envdrift/baseline.json
```

Keep `.envdrift/` private. Reports are evidence records, not authority to
install a missing package or change a toolchain.

## Offline behavior

The scanner never invokes a package download command and passes `--offline` to
Cargo metadata. It reports whether the scan attempted network access as a
stable boolean field.

## Bounded work

File inventories stop at 50,000 entries. Individual evidence files are hashed
only through an 8 MiB prefix. A report that reaches a bound records a prefix
digest or unresolved state rather than implying complete coverage.
