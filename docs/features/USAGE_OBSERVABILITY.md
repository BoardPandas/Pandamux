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

## Hub synchronization and retention

The Hub persists its default database at `data/hub.db`. Its owned asynchronous service
pulls bounded `usage.export` pages from every registered node at connection time and on
a periodic fallback. The authenticated transport assigns the environment id, so a node
cannot select another environment in an observation. Each page upsert and its per-node
high-water advance share one SQLite transaction. Duplicate facts are idempotent,
out-of-order pages cannot skip an unseen sequence, and reconnect starts from the last
committed sequence.

Nodes retain outbox rows until the Hub acknowledges them. Acknowledged rows compact
against all three default limits: 50,000 rows, 30 days, and 64 MiB of serialized
observation payload. Unacknowledged rows are never deleted to satisfy a retention limit.
If a Hub requests a sequence older than the retained window, the node returns an
explicit coverage gap instead of silently advancing it.

Coverage gaps and changed node-store identities enter a durable bounded backfill. The
Hub stores the snapshot sequence, run id, and observation cursor after every page. A
restart resumes that cursor. Only the final backfill transaction removes facts absent
from the replacement node store and advances the high-water mark to the snapshot. Newer
outbox revisions are then pulled incrementally.

## Aggregation and pricing

`usage.query` returns a bounded fact page while SQLite computes totals across the full
filtered set. Indexed predicates cover time, environment, provider, provider instance,
subscription, agent, schedule, model, and source. Provider-specific token categories
are aggregated through SQLite JSON tables, so the server never loads the full fact set
into memory.

Pricing revisions are provider-scoped, source-dated, effective-dated, and identified by
an opaque version id. Each rule matches one exact canonical model id plus explicit exact
aliases. Query-time selection uses the observation time and newest applicable effective
revision. Updating a pricing revision changes only the estimate projection; normalized
usage facts do not change.

Rates are integer micro-USD per million tokens. For each effective rule group, PandaMUX
multiplies integer token counts by integer rates with checked arithmetic, sums at full
precision, and rounds once to the nearest micro-USD with exact halves rounded upward.
Group results are added with checked arithmetic. A nonzero category without an explicit
rate, including an unknown provider-specific category, makes the overall API-equivalent
estimate absent. Provider-reported spend remains a separate value. Coverage reports the
unknown-price token count and its millionths share.

## Diagnostics and privacy

Usage queries expose source coverage, parser version, last scan and success times,
latest source time, lag, files and bytes scanned, retained quarantine count, observation
sources, included environments, node synchronization state, and one bounded safe error
code. Node synchronization metadata contains no provider files or account labels. Usage
results do not expose:

- raw transcript records or excerpts
- transcript or provider profile paths
- scanner parser state or platform file identifiers
- provider environment overrides or credentials
- unmasked account labels

Raw transcript access and scanner filesystem work remain on the node that owns the
provider profile.

## Current boundary

Checkpoint 4C is local implementation and verification evidence. Capacity semantics,
provider health confidence, and actionable login are Checkpoint 4D. The deferred
physical R0.5 laptop disconnect and sleep rows still block production acceptance and
release.
