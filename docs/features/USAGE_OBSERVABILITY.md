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

Migration 12 adds component-aware facts while preserving migrations 1 through 11 and
older aggregate `TurnUsage` records. Claude stream parsing is stateful for one CLI turn.
Repeated thinking or text chunks with the same exact assistant `message.id` contribute
once, while every distinct assistant message remains a separate token component. Each
component retains its actual reported model, input and output tokens, cache-read and
cache-write categories, and optional provider occurrence time. The terminal result UUID
identifies a separate turn-level cost fact, so total turn cost is never assigned to the
last assistant message.

Managed turn facts are normalized before transcript sources during every scan. Managed
and transcript token components reconcile only when provider session and assistant
message identity match exactly. Timestamp skew and one missing optional cost may enrich
that fact. If a transcript was committed first, the compatible fact is promoted in place
and a new outbox sequence advertises the authoritative revision. Conflicting provider,
provider instance, subscription, model, cost, or token measurements remain separate and
keep source coverage Partial with a durable `managed_transcript_mismatch` diagnostic.
Replays, restarts, Hub synchronization, and component-aware backfill are idempotent.
Existing result-UUID and assistant-message pairs are not merged by timestamp or value.
Migration 12 records scopes containing both parser-v1 Claude transcript facts and
legacy managed aggregates as `legacy_reconciliation_debt`, keeping coverage Partial
until an explicit reviewed repair is applied to a clone.

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

## Capacity evidence and synchronization

Migration 9 stores immutable capacity revisions by subscription, environment, provider
instance, window, evidence scope, source, and observation time. A revision carries exact
integer used and remaining fractions, reset time, provider version, confidence, schema
version, and maximum age. Used and remaining values must total one million when both are
reported. Unsupported, unavailable, or semantically unverified provider data remains
Unknown rather than receiving a plausible default.

Codex account rate-limit fixtures justify Reported evidence and use exact decimal-to-
integer conversion. Grok's parser fixture remains test coverage only, so live Grok quota
collection is disabled. Claude and Antigravity report Unknown capacity until a defensible
provider source exists.

After a successful usage synchronization pass, the Hub requests one bounded capacity
refresh. The node returns only the subscription metadata referenced by that snapshot
set, allowing a fresh Hub database to satisfy referential integrity before storing the
revisions. Authenticated transport context replaces any claimed environment identity.
The Hub then pushes its newest bounded view back to the same node with the schema version
and age policy; the node stamps it as Hub evidence.

Projection keeps only the newest revision for each evidence identity. Fresh Reported
evidence ages to Estimated and then Unknown. Reset, stale, clock-skewed, conflicting, and
offline evidence remains explicit. Conflicts choose the lowest known remaining fraction.
An active local hard limit forces zero until its verified reset even if its ordinary age
expires, and stale Hub evidence cannot raise it.

## Provider health and login

`provider.health` probes configured instances concurrently with a bounded semaphore and
per-probe deadline. Probe child processes are killed if their future is cancelled. The
result persists and reports environment, exact instance, provider, enabled state,
installation, resolved executable, version, authentication state and confidence, masked
account and subscription label, runtime state and reason code, check time, expiry, last
successful check, retryability, and structured login guidance. Unknown authentication
is never classified Healthy. Health tests assert that no provider session is started.

Login is a separate explicit action. Claude and Codex first return a short-lived one-time
preview containing the exact environment, instance, isolated profile, flow, and safely
quoted command. A matching launch opens the Terminal surface with private profile and
environment values, then consumes the preview only after successful input. Launch and
remote reachability failures stay structured, visible, and retryable; a failed input
closes the newly opened terminal. Antigravity continues to use its dedicated browser
OAuth flow, and unverified generic provider login launchers remain unavailable.

## Usage, identity, and organization policy UI

Usage is a top-level GPUI rail destination. Its state comes from a pure
`pandamux-client` reducer that distinguishes loading, empty, ready, partial, stale,
offline, malformed-source, migration-quarantine, and server-error results. Date,
environment, provider, named subscription, agent, schedule, model, and source filters
map directly to `usage.query`. Token totals are primary. Provider-reported spend and
API-equivalent estimates stay separate, and missing prices, capacity, freshness, or
health evidence render as Unknown or Not reported.

Every aggregate includes source coverage and freshness. The surface provides a complete
usage data table and text capacity summaries with confidence, evidence sources, as-of
age, reset state, verified hard-limit state, and discount or exclusion reasons. Filter
choices accumulate across narrowed result pages so an active filter does not disappear.

The subscription editor joins opaque transport keys to user-defined subscription and
provider-instance names. It stores only an optional masked account label and rejects an
email-shaped label that does not contain masking. Thread chrome, Usage tables, capacity
cards, and organization matrix cells present names rather than opaque provider or
subscription keys.

Organization settings edit allowed named instances and subscriptions, exact integer
budget limits, concurrency ceilings, organization tier defaults, and Broadside artifact
or transcript permissions. Removing a last valid named route is blocked, and server-side
impact review blocks removal of a provider used by an enabled schedule. Project
assignment editing remains disabled because the current desktop has no canonical project
catalog. The same surface creates and edits explicit failover pools. Each member binds a
named provider instance and subscription to one server-derived model and capability set;
requirements cover instruction delivery, tools, approvals, reasoning, and the access
ceiling. Incompatible members are rejected before settings are committed.

Project-bound local and remote thread creation and turns, schedule validation and run,
and read-only MCP proposal validation resolve exactly one organization before provider
selection. Missing or ambiguous assignment fails closed. Runtime failover applies that
policy before capability, model, installation, authentication, health, local hard-block,
and concurrency filters. Only eligible candidates receive capacity, health, concurrency,
locality, and preference scores. Every selected and rejected candidate keeps a durable,
deterministic explanation. For remote project turns and continuations, the Hub replaces
caller input with its canonical organization and referenced-pool snapshot. The
authenticated node validates that snapshot, every pool, and every member authorization
again before routing. Broadside sharing still has no runtime action. Remote schedule
storage or execution, orchestrator schedules, and continue-existing-thread schedules are
explicitly unavailable.

## Capacity interruption and continuation

Migration 10 adds durable turn attempts, tool ledgers, continuations, and delivery
records. Additive migration 11 attaches the exact execution configuration and probed
provider version to each attempt. A completed retry becomes the authoritative provider,
subscription, and model source for managed usage while the exhausted attempt remains
immutable evidence. Provider adapters advance a monotonic attempt boundary before the server
forwards visible output, tool activity, subagent activity, or an approval request. A
capacity signal at the safe boundary may retry once on another eligible pool member. The
exhausted provider is excluded, and the second attempt always starts a fresh session.

Capacity exhaustion after any unsafe boundary never silently replays the turn. PandaMUX
persists `InterruptedForCapacity` with the exact provider evidence, partial output,
checkpoint, routing decision, and tool ledger. Its continuation verifies the worktree is
still at the recorded checkpoint, sends a bounded handoff in a fresh provider session,
and retires any source native resume token. The continuation target is linked before the
handoff is sent, which makes duplicate requests restart-safe. Approval delivery is
at-most-once and durable; completed tool results are immutable and accept an identical
redelivery without creating another result.

Advanced settings independently gate usage scanning, Hub synchronization,
capacity-aware routing, and automatic failover. Scanner and Hub switches apply at the
next process start. Capacity routing applies to the next turn, and automatic failover
applies to the next capacity decision. Raw observations, read-only usage queries, and
legacy tables remain intact when any switch is off.

## Accessibility boundary

New icon-only controls have accessible names and tooltips. Buttons and inputs retain
keyboard semantics, the subscription editor moves focus to its first field and restores
the prior focus after save or cancel, status always includes text or symbols beyond
color, and the Usage page wraps controls inside a vertical scroll surface. It uses no
new animation, and errors remain visible until a retry or successful reload.

The current GPUI primitives used here do not expose a live-region or semantic table API,
so automated tests cannot prove error announcement or screen-reader table navigation.
The later exact 0.60.0 Windows UI Automation probe opened Usage, found named,
focusable navigation, filter, and subscription controls after AccessKit activation,
and completed a 20-step Tab path. The subscription editor received typed input,
and keyboard Cancel closed it and restored focus to its originating button.
UI Automation did not report focus on the Edit control, and its Invoke action
on Cancel only moved focus. The probe did not test
announcement of values or tables, 200 percent scaling, reduced motion, forced
colors, or a screen reader. These remain acceptance gaps. See
`docs/operations/PHASE4_ACCEPTANCE.md`.

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

Checkpoint 4F has local source verification plus exact 0.60.0 native Windows
startup and partial UI Automation evidence, Galahad live turns for all three
must-have providers, a Hub-to-node SSH turn, and a bounded Hub usage pull. The
deferred physical R0.5 laptop disconnect and sleep rows, full native Phase 4E
interaction and accessibility acceptance, weekly reconciliation, the local plus
Galahad live failover matrix, and later release gates still block production
acceptance and release. The exact results and limitations are recorded in
`docs/operations/PHASE4_ACCEPTANCE.md`.
