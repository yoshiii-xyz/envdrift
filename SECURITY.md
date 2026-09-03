# Security policy

Envdrift is local diagnostic software. It does not need network access to scan
and passes `--offline` to Cargo. Reports may disclose project layout and
should be handled as sensitive local build evidence.

Do not include credentials, registry login files, private keys, or unredacted
reports in an issue. Report a security problem privately to the repository
owner before public disclosure.
