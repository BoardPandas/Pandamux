# Security Scan Report

## Summary

- Scope: Repository-wide secret and dependency hygiene; OWASP and input-validation review scoped to the Phase 5B native Windows hardening diff. Generated `target/` output and `.git/` internals were excluded.
- Critical: 0
- High: 0
- Medium: 0
- Low: 2

The changed access-control, managed-path, Git-process, durability, and fixture code fails closed under the reviewed error paths. No command injection, path traversal, unsafe deserialization, credential logging, authorization bypass, or new network trust was found. Native tests exercise protected DACL creation, foreign-owner and broad-ACE rejection, junction rejection, atomic Git recovery, exact bundle bytes, and long managed refs.

## Critical Findings

None.

## High

None.

## Medium

None.

## Low

[LOW] Local credential artifacts: ignored `tmp/` acceptance data

  Finding: Credential-shaped data is present in three ignored local Phase 4 acceptance files: `tmp/phase4-p4c5-live-20261007/pandamux-data/profiles/codex/default/auth.json`, `tmp/p4m43/data/profiles/codex/default/auth.json`, and `tmp/phase4-model-routing-372f2ae-0.60.43/data/profiles/codex/default/auth.json`. The files are ignored by the repository and have never been tracked. No credential values were read into this report.
  Impact: Another local process, backup, or accidental archive of the worktree could expose stale or live authentication material.
  Remediation: Preserve them only while their acceptance evidence is required. When that evidence is no longer needed, remove the exact task-owned directories through the approved cleanup process and rotate any credential that may have left the intended host.

[LOW] Software supply chain: yanked transitive dependency versions at `Cargo.lock:1019` and `Cargo.lock:10029`

  Finding: `cargo deny check advisories` reports `chacha20 0.10.1` and `yoke-derive 0.8.3` as yanked. It reports no security advisories, and both versions are transitive dependencies in the committed lockfile.
  Impact: Yanked versions no longer receive normal ecosystem selection and can complicate reproducibility or future updates, even when no vulnerability is currently published.
  Remediation: Evaluate compatible upstream dependency updates in a dedicated dependency checkpoint, retain the lockfile, and rerun the full workspace matrix plus `cargo deny check advisories` before accepting any graph change.

## Verification

- No tracked dotenv, private-key, certificate-key, keystore, or credential-container filename was found.
- `.env` and `.env.local` are covered by repository ignore rules; no dotenv file exists in the scanned worktree.
- Value-shaped secret scanning found only the three ignored local acceptance files listed above.
- Literal-assignment hits in tracked Rust sources are synthetic redaction and runtime tests.
- CI workflows use secret or environment references rather than inline credential literals.
- `Cargo.lock` is tracked and direct registry dependencies remain exact-version pinned.
- `cargo deny check advisories` exited successfully with no advisories and the two yanked-version warnings recorded above.
