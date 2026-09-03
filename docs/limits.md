# Limits

- The MVP is tested on Linux only.
- Cargo metadata is requested with `--offline`; an incomplete local cache can
  prevent metadata resolution and is reported as unresolved.
- Cache searches use bounded directory walks and do not prove that a package
  can be fetched or built.
- Target inspection recognizes common debug and release artifact names. A
  present artifact does not prove freshness or successful use by the current
  configuration.
- Cargo fingerprint files are not parsed as a stable public API.
- Only selected Cargo config keys are rendered. The full config digest can
  show that unrendered state changed without identifying the cause.
- Absolute local paths are flagged as non-portable even when they point inside
  the current workspace.
- Git status is observed but no source-control mutation is performed.
- Python, uv, environment installation, remote caches, hosted services, and
  physical-device validation are out of scope.
