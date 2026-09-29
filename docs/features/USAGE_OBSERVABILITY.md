# Usage Observability and Transcript Privacy

PandaMUX records normalized usage facts on each environment. Managed PandaMUX turns are
authoritative. Claude and Codex transcript scanners cover external and historical use
without sending raw provider files to the Hub.

## Ownership and cadence

`pandamux-server` owns one dedicated blocking scanner worker per environment. It scans
at server startup, after a managed turn completes, and on a deterministic jittered
fallback interval. The trigger queue, trigger lifetime, managed-turn batch, directory
count, file count, bytes, records, record size, quarantine retention, and total scan
duration are bounded. Shutdown stops new work, drains an already accepted trigger within
the owned budget, and joins the worker.

Provider-specific JSON parsing is pure code in `pandamux-providers`. It does not open
files or own lifecycle state.

## Durable cursor contract

Migration 7 stores scanner cursors by environment, provider instance, source kind,
node-store identity, and platform file identity:

- Unix uses device and inode.
- Windows uses volume serial and file index from the opened file handle.

The canonical path is diagnostic state only. Cursor offset, a committed-prefix hash,
parser version, serialized parser state, observations, quarantine records, and the
source diagnostic commit transactionally. A changed node-store identity, parser
version, prefix, size, or file identity restarts parsing safely. Rename keeps the same
cursor, while truncate, replacement, and rotation are explicit states.

The scanner does not follow symlinks or Windows reparse points. An incomplete final
JSONL record never advances the cursor. A malformed or oversized complete record stores
only bounded reason codes and SHA-256 hashes, then parsing resumes at the next record.

## Reconciliation

Managed turn facts are normalized before transcript sources during every scan. Native
provider session and turn identifiers are preferred. When the same strong identity is
seen in both sources, the managed measurement and PandaMUX attribution win and only one
fact remains queryable. If a transcript was committed first, the fact is promoted in
place and a new outbox sequence advertises the authoritative revision. Replays are
idempotent, and measurement disagreement marks source coverage partial.

Missing timestamps, models, identities, or required token fields are not replaced with
plausible defaults. They produce absent fields and partial coverage diagnostics.

## Diagnostics and privacy

Usage queries expose source coverage, parser version, last scan and success times,
latest source time, lag, files and bytes scanned, retained quarantine count, and one
bounded safe error code. They do not expose:

- raw transcript records or excerpts
- transcript or provider profile paths
- scanner parser state or platform file identifiers
- provider environment overrides or credentials
- unmasked account labels

Raw transcript access and scanner filesystem work remain on the node that owns the
provider profile.

## Current boundary

Checkpoint 4B is local implementation and verification evidence. Hub synchronization,
indexed aggregation, effective-dated pricing, and cross-node coverage are Checkpoint 4C.
The deferred physical R0.5 laptop disconnect and sleep rows still block production
acceptance and release.
