# Phase 4 Acceptance Record

Date: 2026-09-29
Updated: 2026-10-07

Status: **Accounting passed, P4-C3 native Windows acceptance is reopened,
P4-C5 remains Partial, and Phase 4 live acceptance is incomplete.** Exact
0.60.15 commit `3a77382` passed the P4-C1 implementation, P4-C2 local candidate,
and P4-C4 cloned-store reconciliation gates. Exact 0.60.22 commit `fee9e02`
passed every P4-C3 row that was executed, including limited-user interaction,
accessibility, visual, and current normal-user IPC checks. P4-C3 was reopened
after the omitted successful provider-login launch path left its Terminal
surface undiscoverable and noninteractive during manual use. Exact 0.60.24
commit `581113e` proved the repaired panel, terminal binding, and automatic
focus natively, but PowerShell did not execute the injected login command
because ConPTY received a line feed instead of carriage return. Exact 0.60.25
proved command execution visually, then failed because painted Terminal output
was absent from UI Automation. Exact 0.60.26 commit `b696aad` exposed bounded
visible output as a semantic Log and passed the successful-login interaction,
High Contrast, and disabled-animation rows. Its genuine Narrator Speech Recap
did not announce READY or ECHO, so it is rejected for P4-C3. Version 0.60.27
makes the output Log a dedicated keyboard focus stop. Its exact native build,
smoke, remote routing, successful-login interaction, 200 percent scaling, High
Contrast, and disabled-animation rows passed, but direct UI Automation focus
did not reach the Log and the advertised Tab path remained PTY input. It is
also rejected. Exact 0.60.28 commit `c0200a9` added an explicit F6 input/output
focus toggle and passed native build, smoke, remote login lifecycle,
successful-login interaction, 200 percent scaling, High Contrast, and
disabled-animation rows. Narrator displaced focus to the PandaMUX window,
where the terminal-scoped F6 handler could not run, so 0.60.28 is rejected.
Exact 0.60.29 commit `2b3b806` passed native build, smoke, remote lifecycle,
successful-login interaction, 200 percent scaling, High Contrast, and
disabled-animation rows, but its root visual handler still had no GPUI focus
path under Narrator and did not receive F6. Exact 0.60.30 commit `9dae237`
passed native build, smoke, remote lifecycle, and standard successful-login
interaction, but Narrator still consumed or withheld F6 before PandaMUX
received it. Exact 0.60.31 commit `0394eb3` passed native build, smoke, remote
lifecycle, and standard successful-login interaction. Its explicit controls
were visible and invokable, but invoking `Read output` under Narrator left UI
Automation focus on the PandaMUX window, so it is rejected. Exact 0.60.32
commit `952b280` passed native build, smoke, remote lifecycle, and standard
successful-login interaction, but direct UI Automation Invoke still left focus
on the PandaMUX window because the styled control fell back to synthesized
pointer input under Narrator. It is also rejected. Exact 0.60.33 commit
`58452e7` passed native build, smoke, remote lifecycle, and standard
successful-login interaction, but its direct AccessKit handler deferred the
focus update and UI Automation Invoke still left focus on the PandaMUX window.
It is rejected. Exact 0.60.34 commit `1f1df21` passed native build, smoke,
remote lifecycle, and standard successful-login interaction, but Windows UI
Automation still restored window-level focus after the synchronous
accessibility callback. It is rejected. Exact 0.60.35 commit `b54f8e3` passed
native build, smoke, remote lifecycle, and standard successful-login
interaction, but its delayed retry requested the output handle that GPUI
already considered focused. GPUI elided that retry while Windows UI Automation
still reported the PandaMUX window, so 0.60.35 is rejected. Exact 0.60.36
commit `6fd5d02` made the delayed move a distinct GPUI focus transition and
passed native build, smoke, remote lifecycle, and standard interaction, but UI
Automation still reported the PandaMUX window. It is rejected. Exact 0.60.37
commit `f21bc2d` explicitly activated the native PandaMUX window and passed
native build, smoke, remote lifecycle, and standard interaction, but Narrator
still reported window-level focus. Instrumented diagnostics then proved that
PandaMUX already owned the foreground window and had internally entered its
visible output-reading state; native activation was not the missing condition.
It is rejected. Exact 0.60.38 commit `d328ad8` then kept real keyboard focus on
the terminal container and published the output Log through GPUI's supported
active-descendant pattern. Its complete local gate, native build, host smoke,
remote lifecycle, and standard successful-login interaction passed. Ordinary
UI Automation honored the active descendant, but genuine Narrator did not speak
READY. Instrumented follow-ups proved that PandaMUX retained native foreground,
active, and keyboard focus while synchronized Narrator navigation reached the
terminal header controls and skipped the semantic Log group. It is rejected.
Exact 0.60.39 commit `8b7b880` changed that node to AccessKit's dedicated
Terminal role and passed the complete local gate, native build, host gates, and
standard successful-login interaction. Windows reported a focusable UI
Automation Document, but genuine Narrator visited that Document without
speaking its parent-only accessible name and then left PandaMUX. It is rejected.
Exact 0.60.40 commit `d4c9dd8` retained the Document, passed its local gate,
native build, and host gates, but failed the standard guest row because its
label-only semantic child was absent from Windows UI Automation. The 0.60.41
source candidate gives that child an AccessKit value backed by a synthetic text
run so both the child and containing Document can expose TextPattern content.
It has not passed exact native Windows acceptance.
Exact
0.60.17 commit `2f56a27` passed the scanner, Hub
synchronization, conservative capacity projection, route-explanation, and
rollback-control portions of P4-C5. Real provider quota exhaustion and real
interruptions after output or external side effects were not available, so
their live rows remain open even though the focused diagnostic fixtures pass.
The final documentation archive after those rows stabilize and the deferred
physical matrix also remain open. No push, deployment, production activation,
or release occurred.

## Source and preservation boundary

- Exact executable source: clean `codex/recovery-r0` commit
  `96aad1d16f357f1bc70d46b9889ad403550a4d1a`, version `0.60.0`.
- Exact native Usage accessibility follow-up source: clean `codex/recovery-r0`
  commit `2be212f9a290637add9a4f5c662ca3155da98a0c`, version `0.60.3`.
- Exact accounting source: clean `codex/recovery-r0` commit `3a77382`, version
  `0.60.15`. This documentation-only follow-up uses workspace version `0.60.16`;
  no 0.60.16 native or live result is claimed.
- Exact P4-C5 durability source: clean `codex/recovery-r0` commit `2f56a27`,
  version `0.60.17`. The documentation follow-up uses workspace version
  `0.60.18`; no 0.60.18 executable, native, or live result is claimed.
- Exact P4-C3 native Windows source: clean `codex/recovery-r0` commit
  `fee9e02275dceafd87c25a36af0732cd38c47076`, version `0.60.22`. This
  documentation-only follow-up uses workspace version `0.60.23`; no 0.60.23
  executable, native, live, production, or release result is claimed.
- First reopened P4-C3 repair source: clean commit
  `581113eb6daadd87827b8c730ff81f69a4bf9ab9`, version `0.60.24`. Its exact
  native Windows build and smoke passed, and its focused Terminal surface was
  visible from Settings. The controlled login command did not execute, so the
  attempt is rejected for P4-C3 acceptance.
- Second reopened P4-C3 repair source: clean commit
  `8f0ec2040832cf135d20293eaf61f3e5847d9629`, version `0.60.25`. Its exact
  native Windows build, smoke, remote login-terminal routing test, panel,
  binding, focus, command execution, and visible READY output passed. The
  painted output was absent from UI Automation, so the attempt is rejected for
  P4-C3 acceptance.
- Third reopened P4-C3 repair source: clean commit
  `b696aad3823df70b741bffcf9241c3ebfa2b4619`, version `0.60.26`. Its exact
  native build, smoke, remote routing, successful-login interaction, High
  Contrast, and disabled-animation rows passed. Narrator Speech Recap did not
  announce the controlled READY or ECHO output, so the attempt is rejected.
- Fourth reopened P4-C3 repair source: clean commit
  `5e00715e1b3b9e343d192cee254348e3d9a3b7c5`, version `0.60.27`. Its exact
  native build, smoke, remote routing, successful-login interaction, 200
  percent scaling, High Contrast, and disabled-animation rows passed. Direct
  UI Automation focus did not move to the output Log, and the advertised Tab
  path remained PTY input, so the attempt is rejected.
- Fifth reopened P4-C3 repair source: clean commit
  `c0200a9a38fcdccf1cff12fe51f4143562f217ab`, version `0.60.28`. Its exact
  native build, headless and limited-user smoke, remote login lifecycle,
  successful-login interaction, 200 percent scaling, High Contrast, and
  disabled-animation rows passed. Narrator displaced focus to the PandaMUX
  window, where the terminal-scoped F6 handler could not run. UI Automation
  focus and real pointer recovery did not restore terminal child focus, so the
  attempt is rejected.
- Sixth reopened P4-C3 repair source: clean commit
  `2b3b806a9d3e0f1c1c3709039a23f642dd4a9b2a`, version `0.60.29`. Its exact
  native build, headless and limited-user smoke, remote login lifecycle,
  successful-login interaction, 200 percent scaling, High Contrast, and
  disabled-animation rows passed. Narrator reproduced the PandaMUX-window
  focus state, and F6 still did not reach the root visual handler because GPUI
  had no child focus path, so the attempt is rejected.
- Seventh reopened P4-C3 repair source: clean commit
  `9dae2374aae7351626416e5973c224eaa8cfb096`, version `0.60.30`. Its exact
  native build, headless and limited-user smoke, remote login lifecycle, and
  standard successful-login interaction passed. Narrator reproduced the
  PandaMUX-window focus state, and F6 still did not reach the application
  interceptor, so the attempt is rejected.
- Eighth reopened P4-C3 repair source: clean commit
  `0394eb36ed3a93dd2c01099a0abab1adcf5d0d52`, version `0.60.31`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Its explicit controls were visible and
  invokable, but Narrator activation of `Read output` left focus on the
  PandaMUX window, so the attempt is rejected.
- Ninth reopened P4-C3 repair source: clean commit
  `952b28037aaa9ce9bd19a94cf26831f1b1ef38b3`, version `0.60.32`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Direct UI Automation Invoke of the
  visible, invokable `Read output` control still left focus on the PandaMUX
  window because its styled button used synthesized pointer fallback under
  Narrator, so the attempt is rejected.
- Tenth reopened P4-C3 repair source: clean commit
  `58452e70dbe2c1ca14bdf05270300bcc6112b4d1`, version `0.60.33`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Direct UI Automation Invoke reached its
  AccessKit Click handler, but the deferred focus update still left focus on the
  PandaMUX window under Narrator, so the attempt is rejected.
- Eleventh reopened P4-C3 repair source: clean commit
  `1f1df21eaba68c2874b30ed94e02459a6f96a641`, version `0.60.34`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Direct UI Automation Invoke reached the
  synchronous AccessKit focus handler, but Windows still restored focus to the
  PandaMUX window under Narrator, so the attempt is rejected.
- Twelfth reopened P4-C3 repair source: clean commit
  `b54f8e39c4e3b60e328b9a4b58df70d0558bd776`, version `0.60.35`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. The delayed accessibility retry was a
  no-op because GPUI still considered the output handle focused after Narrator
  restored platform focus to the PandaMUX window. The attempt is rejected.
- Thirteenth reopened P4-C3 repair source: clean commit
  `6fd5d027945312a7ba066e9fc42e5bd37dc05fe5`, version `0.60.36`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Its AccessKit Click handler made the
  delayed target move a distinct GPUI focus transition, but UI Automation still
  reported the PandaMUX window. The attempt is rejected.
- Fourteenth reopened P4-C3 repair source: clean commit
  `f21bc2da89ce25975cf422f16906170729c28778`, version `0.60.37`. Its exact
  native build, headless and limited-user smoke, remote lifecycle, and standard
  successful-login interaction passed. Narrator still reported the PandaMUX
  window after invoking `Read output`. Assisted diagnostics proved the window
  was already foreground before and after Invoke, while the visible instruction
  and focus border proved the direct handler had changed GPUI into output mode.
  Native activation addressed the wrong layer, so the attempt is rejected.
- Fifteenth reopened P4-C3 repair source: clean commit
  `d328ad8d6c59ba0375c1715b9ee63f42c4fbf581`, version `0.60.38`. Its complete
  local gate, native build, host smoke, remote lifecycle, and standard
  successful-login interaction passed. Ordinary UI Automation honored the
  active-descendant output focus, but genuine Narrator did not speak READY.
  Native-focus instrumentation and synchronized Narrator navigation isolated
  the remaining defect to the semantic Log group, so the attempt is rejected.
- Sixteenth reopened P4-C3 repair source: clean commit
  `8b7b8804999f217ef8f03fb501b679463908e990`, version `0.60.39`. Its complete
  local gate, native build, host headless and limited-user smoke, remote
  lifecycle, and standard successful-login interaction passed. Windows
  reported `terminal-output-log` as a focusable UI Automation Document, but
  Narrator visited it without speaking its parent-only accessible name. The
  attempt is rejected.
- Seventeenth reopened P4-C3 repair source: clean commit
  `d4c9dd891cf9cb42eb919bc45237f5ade0860352`, version `0.60.40`. Its complete
  local gate, exact native Windows build, host headless and limited-user smoke,
  and remote lifecycle passed. The guest standard row confirmed the Document
  parent but found no `terminal-output-text` child, so the attempt is rejected
  before Narrator.
- Current reopened P4-C3 repair source: workspace version `0.60.41` in the
  isolated recovery worktree. It retains the Terminal/Document parent, gives
  the Label child the bounded output as its value, and backs it with a synthetic
  text run for UIA TextPattern. Its complete local source gate passed with 437
  workspace tests and two intentional real-shell ignores, but it has not passed
  exact native Windows acceptance; no native, live, production, or release
  result is claimed.
- The intermediate 0.60.2 commit `8a52d2a21da1d5d0f69540864a6162d2b30375bc`
  repaired direct UI Automation activation for Save and Cancel. The 0.60.3
  follow-up exposed the editor text fields on semantic nodes tied to their real
  editing focus. This documentation follow-up uses workspace version `0.60.4`.
  No 0.60.4 Windows or Galahad executable is claimed as tested here.
- Original checkout: `/home/brandon/Repos/Pandamux` at recovery baseline
  `9511d0e22d44` with the unfinished Phase 4 prototype preserved. R0.1 archived
  a binary-safe patch and untracked-file tar, recorded their hashes, and verified
  every captured path by byte comparison in a detached restore worktree before
  source editing. See `tasks/plan-rewrite.md` R0.1 for the archive location and
  exact hashes. The original dirty checkout was not reset, cleaned, or staged.
- The Galahad acceptance root was
  `/home/brandon/.pandamux/acceptance/phase4-96aad1d`; its node runtime,
  database, profiles, and test repository were separate from the existing
  `/home/brandon/.pandamux/run` production runtime. The production 0.54.11
  daemon remained running throughout. The test node was stopped after acceptance,
  and its private database and artifacts remain for inspection.
- The Windows host built from an isolated clean source clone under
  `C:\ProgramData\PandaMUX\acceptance\phase4-96aad1d\source`. The host's
  existing `C:\Github\Pandamux` checkout was not edited. The guest
  `SF-C4AD-VM` used `C:\ProgramData\PandaMUX\acceptance\phase4-96aad1d` and
  `C:\Users\C4adUser\AppData\Local\PandaMUX-Phase4-96aad1d` only.

## Exact artifacts and startup

The Linux artifacts were built from the clean commit with publication disabled:

```bash
scripts/r0-node-artifacts.sh build
```

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux musl `pandamux-server` | 17,497,312 | `5e1471fad2af3de5f95e72f21ac09b764b052a4d976c90469c2cac3165a58c2b` |
| Linux musl `pandamux` CLI | 2,020,336 | `32a80cdc6253ebee945a969397065158ae673372a07a17b0496fe93529970072` |
| Windows GNU `pandamux-server.exe` | 28,433,717 | `06479f8bd295f3a4cb7eefaf3bfc48f4b2a4970fd690254c7432db8124931688` |
| Windows GNU `pandamux-cli.exe` | 4,562,000 | `60acc30f7a50aaa120e51186190a7f536ff91b5a34de6f8cc81013520ca11160` |
| Windows MSVC `pandamux.exe` | 27,208,704 | `a2762195c586a7042b61bbd53c0199e2b6170431c8ad9f9c0c17968dfbfce2c9` |

The Linux hashes were reverified on Galahad after transfer. The Windows hashes
were reverified on `SF-C4AD-VM` after transfer. The Windows desktop was built
natively on `COLT-SHINE` with MSVC, the Windows SDK shader compiler, and
`cargo build --locked --release -p pandamux-desktop`. Windows server and CLI
were cross-built with
`cargo build --locked --release --target x86_64-pc-windows-gnu -p pandamux-server -p pandamux-cli`.
The Linux-to-Windows desktop cross-build did not complete because the pinned
GPUI Windows build expects its generated shader file from a native Windows
build script. Native MSVC desktop build succeeded instead.

The clean guest lacked `VCRUNTIME140.dll`, so the first desktop launch failed
with Windows `0xC0000135`. An x64 Microsoft VC142 runtime DLL was copied
app-locally alongside the test executable, not installed globally; both copies
hashed to `270ba17bf89cc2eb6e6ce536eb78e4614ab5499f653ab7e07fe9e3d33bf3b6e3`.
With that DLL, `pandamux.exe --headless` exited 0. A scheduled task in the
interactive `SF-C4AD-VM\C4adUser` session ran `pandamux.exe --smoke`; its
`smoke.exit` is `0`, and `smoke.log` records that the window initialized and
rendered cleanly. The exact guest server and CLI also passed authenticated
`hello`, `ping`, and `identify` under the SupportForge SYSTEM session. That
SYSTEM test is not a substitute for the 0.60.0 normal-user named-pipe security
matrix. The earlier exact 0.54.12 normal-user matrix is recorded in
`docs/operations/PHASE3_ACCEPTANCE.md`.

## Exact 0.60.15 accounting and reconciliation acceptance

The clean accounting candidate is commit `3a77382`, version `0.60.15`. Its
isolated Galahad root is
`/home/brandon/.pandamux/acceptance/phase4-3a77382-0.60.15-p4-c4`.
The retained source at
`/home/brandon/.pandamux/acceptance/phase4-96aad1d` was copied before migration
or scan. The exact candidate artifacts used on Galahad were:

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux `pandamux-server` | 18,204,088 | `d4f9b3d04eb5f8218b109c0ec24a0bb15f9b176d485a5e53e0f016f5edfbea91` |
| Linux `pandamux-cli` | 2,034,000 | `aebaf76070d0f2fe0320ba6f31a78a33d4e526a2dd73b82bdcdfaf21c409108f` |

The same clean commit also produced local release artifacts and passed the
desktop and control-plane floor:

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux desktop `pandamux` | 60,149,672 | `14ee6a5ee522ac816fa3a03e2b8c13cbc0c22f11dcc22575050ca59ec628512a` |
| Linux `pandamux-server` | 18,800,232 | `c33dffcdda711be81dea814e04720bb9e12c7211f8e160cd5bb90bd88fb437f2` |
| Linux `pandamux-cli` | 2,174,664 | `68cc5bea36456d7879948dccc456cec99ade3d586d5da01ffa63b2eacb5ea344` |

`pandamux --headless` returned 0 with `ALL CHECKS PASSED`. An isolated exact
server and CLI completed authenticated `system.hello`, `system.ping`, and
`system.identify` with protocol 3, node role, version 0.60.15, and the expected
test environment. Formatting, strict all-target and all-feature Clippy, 423
workspace tests with zero failures and two intentional real-shell ignores, the
Rust boundary guard, the Claude wiring guard, and `cargo deny` all passed.
Dependency policy retained non-failing warnings for duplicate dependency
versions, one unmatched license allowance, and yanked transitive `chacha20
0.10.1` and `yoke-derive 0.8.3`.

### Clone, migration, and source fitness

Before migration, the clone matched the retained source database, WAL, SHM,
Claude corpus, and Codex corpus by SHA-256. It held migration 11, 23 usage
observations, 23 outbox rows, 3 quarantine rows, and 24 scanner cursors. The
source roots contained 15 Claude JSONL files totaling 2,527,996 bytes and 19
Codex JSONL files totaling 4,707,708 bytes. Their stable content digests were
`ee02fa6bfcc7cef98dfb7fdfea07970ece2337508640777e6c64ad2cdab2ba09`
and `eeaa3ac29f326609bcf845bfb2a89377c3d8c500c34ba6e02068e527857addb1`.

Only clone-local absolute paths were remapped. The first exact startup applied
migration 12 and rescanned the copied corpus. It stabilized at 43 observations,
43 outbox rows, 6 quarantine rows, 58 file cursors, 4 retained exact-ID Codex
model mismatches, and 1 retained Claude legacy-debt row. A second startup left
all counts and normalized observation digest
`85ad392b70959681dbacbac1826fed2f289a76d3fb5d0518da5b6733813ef305`
unchanged. Product `usage.query` coverage remained Partial for Claude with
`legacy_reconciliation_debt` and for Codex with
`managed_transcript_mismatch`. A no-op scanner pass never upgraded those
persistent debts to Complete in the product projection.

### Fresh exact-ID Claude turns

The simple acceptance turn
`turn-9facf3ef-bb3b-4d67-96aa-52fef7b6400f` completed with one exact component,
message `msg_011CfnFf39xkCP2qMfGKvk4N`, actual model `claude-opus-5-5`, 2 input,
11,698 cache-read, 9,115 cache-write, and 16 output tokens. Its separate terminal
cost fact was 75,588 provider-reported micro-USD.

The tool acceptance turn
`turn-a36b0236-cb11-4be1-a671-01107a0439fe` used Bash exactly once for `pwd`.
The durable ledger recorded success and the isolated project directory. It
retained two exact components:

| Message ID | Model | Input | Cache read | Cache write | Output |
|-|-|-:|-:|-:|-:|
| `msg_011CfnFfEMfF8Mjztzp1o2dx` | `claude-opus-5-5` | 2 | 20,813 | 19,898 | 74 |
| `msg_011CfnFfR3bTaU9q76NijVpt` | `claude-opus-5-5` | 2 | 40,711 | 3,294 | 14 |

Its separate terminal cost fact was 275,204 provider-reported micro-USD. A
sanitized extraction from the copied Claude transcript contained exactly those
three message IDs. Session, message, model, input, output, cache-read,
cache-write, and reasoning fields matched all three managed components exactly.
No new mismatch row appeared. This directly verifies that final
`message_delta.usage` values replace smaller preliminary stream values on the
same message component.

After live ingestion, repeated scan and restart remained at 48 observations,
48 outbox rows, 6 quarantine rows, 59 file cursors, 4 retained mismatches, and 1
retained legacy-debt row. The normalized digest remained
`906f31c0167bff1453e9788403654d1a878b94edacacbd46e6211eeafdbc2f7b`.
An authenticated `usage.query` returned all 48 facts in one page. Its aggregate
of 304,523 input, 667,885 cache-read, 297,219 cache-write, 4,110 output, and 0
reasoning tokens, plus 422,740 provider-reported micro-USD across 3 cost facts,
matched direct SQL exactly.

### Historical seven-day result

The retained historical window is the inclusive 604,800,000 millisecond span
from `2026-09-22T22:22:42.754Z` through the last pre-rerun observation at
`2026-09-29T22:22:42.754Z`. The UTC boundary avoids host timezone ambiguity.
The Oct 7 acceptance turns are intentionally outside this historical window.

`usage.query` returned 43 retained facts totaling 304,517 input, 594,663
cache-read, 264,912 cache-write, 4,006 output, and 0 reasoning tokens. One
legacy cost-bearing fact contributed 71,948 provider-reported micro-USD. The
query aggregate matched direct SQL exactly. Old ambiguous result UUID and
assistant message-ID facts were not merged or repaired. They remain visible in
the raw query and are excluded from any claim of a reconciled external total.

No independent provider statement was retained for Claude, Codex, or
Antigravity. The same-event managed cost is not an independent Claude report,
and an API-equivalent estimate is not provider-reported spend. Claude, Codex,
and Antigravity external reconciliation therefore each report **Unknown**. The
two percent comparison threshold is not applicable. Antigravity's absence from
the normalized window is not translated into zero usage.

### Preservation and cleanup

Final hashes of the retained `node.db`, `node.db-shm`, and empty `node.db-wal`
were respectively
`8392e444508c6c598312e97adf1c761dafc8674dcc704c2f7cdca6aa97de806f`,
`fd4c9fda9cd3f9ae7c962b0ddf37232294d55580e1aa165aa06129b8549389eb`,
and `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
identical to the pre-clone values. The two corpus digests also remained
identical, and no process held the retained database open. The original dirty
checkout remained unchanged.

An unsupported `pandamux-server --version` probe briefly launched the exact
candidate against Galahad's default hub path because the server ignores unknown
flags. Immutable inspection proved that `hub.db` was newly created by the probe,
contained zero usage observations, contained two scanner diagnostics, and passed
SQLite integrity checking. No prior database was overwritten. The exact file
was moved recoverably into the isolated acceptance root at
`accidental-default-probe/hub.db`, with SHA-256
`e07863853d6b6a9ff27972c1171d3bfff9a66d4ae81307e12aaadf6db462fa29`.
The default hub path and runtime files were restored to their prior absent state,
and no acceptance process remained running. Version 0.60.17 adds side-effect-free
server help and version actions and rejects unknown, incomplete, conflicting, or
invalid startup arguments so the same probe cannot launch a default Hub again.

### Exact 0.60.3 native accessibility repair artifacts

The follow-up source moved from Linux to COLT-SHINE as a prerequisite-bound Git
bundle with SHA-256
`401089dc7c06f3097e2292f20262582c93a4131dcee12e707c1ef0afb35a18b5`.
The host decoded and verified the bundle, cloned the already accepted isolated
source, fetched only the repair commit, checked out exact commit `2be212f`, and
proved a clean worktree before and after building. The host's existing
`C:\Github\Pandamux` checkout was not edited.

Native MSVC release builds used Visual Studio 2019 Build Tools, Rust 1.98.1,
the Windows SDK shader compiler, four Cargo jobs, and locked dependencies:

```powershell
cargo build --locked --release -p pandamux-desktop
cargo build --locked --release -p pandamux-server -p pandamux-cli
```

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,189,760 | `2f9fb97a6bc6a9c9633fe5987ce75931f58556ca6439e5a563344dfd2f51cb9e` |
| Windows MSVC `pandamux-server.exe` | 13,798,400 | `21b1aa84aae621610d5e8b9d7bd43651f9ebcee591b25e20197bddbfa41adef6` |
| Windows MSVC `pandamux-cli.exe` | 1,323,520 | `342834e38bfccd54177b094ff45fe6adb4b9aeee19ba9921b3cf4e18c03a7302` |

The exact `pandamux.exe --headless` run exited 0 and reported all checks passed.
Build scripts, full logs, exit files, source, bundle, probe scripts, results, and
the assembled `run-bin` remain under
`C:\ProgramData\PandaMUX\acceptance\phase4-2be212f`.

## Galahad live provider and Hub evidence

The isolated 0.60.0 node answered authenticated CLI `ping` and `identify`.
Its test project was a separate Git repository under the acceptance root.
Subscription keys were explicitly mapped to named instances, and an isolated
organization and compatible pool allowed Codex and Claude. No production
subscription mapping, policy, or node database was changed.

| Scenario | Result | Deciding record |
|-|-|-|
| Codex direct pool turn | Pass | `turn-5dc7d424-4012-479a-9906-03f85f64dd16` completed with `P4F_LIVE_OK`; attempt `attempt-c7f2ae5c-8010-484a-a00f-d0e4edc5bc76` retained provider kind `codex`, version `0.158.0`, exact selected instance, and managed usage of 14,256 input, 10 output, and 12,288 cache-read tokens |
| Claude selected after test pool preference change | Pass | `turn-aca21df8-cf70-4a00-a7d3-d85bdb843dfd` completed with `P4F_CLAUDE_PREFERRED_OK`; attempt `attempt-a3028860-f259-45e8-b73b-d20b2093593c` retained provider kind `claude` and model `claude-3-7-sonnet-latest`. The route gave Claude score 1,750 and rejected the saturated Codex instance for `concurrency_limit_reached` |
| Antigravity exact instance | Pass | `turn-f4798eea-8b15-4abe-930e-acc737a91156` completed with `P4F_ANTIGRAVITY_OK`; attempt `attempt-6b9eaac2-c69b-4c1d-b951-d4f967566fe8` retained provider kind `antigravity` and pinned ACP version `agy_acp_server_1.1.1` |
| Best-effort providers | Disabled by design | Cursor, Grok, and OpenCode health reports retained their configured disabled reasons; no live turn was claimed |

The offline Antigravity status probe reported `authenticated=false` even though
the isolated profile held a private token and a real ACP turn succeeded. The
probe only reports detected credentials, not a verified live login. The real
turn, not that status field, decides the live row.

Read-only inspection of the retained node database found four Codex and one
Claude `managed_turn` observations. The accepted direct Codex turn recorded
14,256 input, 10 output, and 12,288 cache-read tokens in both the turn and
its exact managed observation. The accepted Claude turn recorded 2 input and
22 output tokens in both places. The Hub-routed Codex turn also matched at
15,201 input, 12 output, and 12,288 cache-read tokens. The accepted
Antigravity turn had no provider-reported token usage and no managed usage
observation; its usage is Unknown, not zero. The node also retained 13 Codex
and five Claude transcript observations and three Codex `record_too_large`
quarantines. These quarantines remain visible source-coverage debt, and the
database comparison is not an independent weekly provider report.

A temporary Rust harness used `Server::connect_remote_environment` and the
production `pandamux-server proxy` command over the real SSH connection:

```bash
cargo run --locked -p pandamux-server --example phase4_live_acceptance
```

The harness created an in-memory Hub with a project policy allowing only
`codex-default`, then used the isolated Galahad node. Its final run returned:

```text
REMOTE_IDENTIFY_OK role="node"
HUB_POLICY_DENY_OK
HUB_REMOTE_CREATE_OK thread="thread-3ea2bc31-e4d2-4b2f-af8c-2f2787724b1f"
HUB_REMOTE_GET_OK
HUB_USAGE_SYNC_OK items=4
HUB_REMOTE_SEND_OK turn="turn-f084a577-7777-423d-90af-9769663b9c74"
HUB_REMOTE_TURN_OK
HUB_REMOTE_DISCONNECT_OK
```

The first extended run reached the same identity, policy, thread, and four-item
usage checks but rejected the new Codex turn because three earlier acceptance
sessions occupied the configured three-session limit. Raising only the isolated
test instance limit to four allowed the final run. The harness source was
removed after testing; the accepted output and node database remain as the
record. This proves a bounded live Hub pull and a Hub-routed turn, not
long-duration reconnect, retention-gap backfill, or weekly reconciliation.

## Exact 0.60.17 P4-C5 durability and control acceptance

P4-C5 is **Partial** on clean commit `2f56a27`, version `0.60.17`. Scanner
durability, Hub resumption and bounded backfill, capacity projection and route
explanations, and all four rollback controls passed in isolated roots. The
fixture-backed failover matrix also passed. Real external quota exhaustion and
real interruption after output or a side effect were not available, so P4-C5
rows 4 through 6 remain live-acceptance gaps.

The retained Galahad evidence root is
`/home/brandon/.pandamux/acceptance/phase4-2f56a27-0.60.17-p4-c5`. It contains
the final databases, pre-injection and pre-toggle backups, timestamped logs,
controlled evidence, exact artifact hashes, and a manifest covering 837 files.
The publication-disabled Linux artifacts were:

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux musl `pandamux-server` | 17,674,928 | `35600df4d2b04daaf5c748d868d51daad2d6dc476e44010cf579e985d1350dbc` |
| Linux musl `pandamux` CLI | 2,022,696 | `37d5f4cd4dd95378427ff8bda69f6ffe484319249c3b248d62ddd85bf91d2ea3` |

### Scanner and Hub durability

The live scanner matrix used synthetic Claude transcript records under the
isolated scanner profile. It passed startup, unchanged restart, partial trailing
record, completion, rename, rotation, truncation, replacement, permission
denial, and permission recovery. Partial input did not advance the committed
record or create an observation. Rotation, truncation, and replacement produced
the expected new facts without duplicating earlier facts. Permission denial
reported `source_permission_denied`, retained all six observations, and returned
to Complete after access was restored.

Three separate Hub processes then synchronized two facts per bounded pass. Their
durable high-water marks advanced from 2 to 4 to 6, with follow-up work true,
true, then false. Each explicit disconnect persisted
`usage_sync_unreachable`. After node outbox compaction retained only sequence 6,
a fresh Hub detected the retention gap and stopped after three bounded pages
with high-water 0, one inserted fact, and a durable backfill cursor. A new Hub
process resumed that cursor, completed seven further pages, inserted the other
five facts, and reached Ready at high-water 6. The six-fact node and Hub digest
matched at
`bded6d83039f8c665940bd3ebe5b364df06c902271eb945a2896774df9628f56`
without a manual usage-row edit.

The scanner rollback test added one seventh record while scanning was disabled.
After restart, both direct SQLite and authenticated `usage.query` remained at
six facts. Re-enabling scanning and restarting ingested exactly the queued
record, producing seven facts, four file cursors, and a retained outbox window
of sequences 6 through 7. The Hub-sync rollback test held its existing six
facts while synchronization was disabled, then reached seven facts and
high-water 7 immediately after re-enable. The final node and Hub logical fact
digest matched at
`0d9dfd15644e8382525ffa4df4514c18bbef22c091cb2ca3d84ede2ab4e33eac`.
All final scanner, capacity, and Hub databases returned `ok` from SQLite
integrity checking.

### Capacity evidence and route decisions

Authenticated `capacity.query` replayed real retained Codex quota evidence at
fixed evaluation instants. The `codex primary` window projected:

| State | Confidence | Remaining | Reason |
|-|-|-:|-|
| Fresh | Reported | 210,000 millionths | Current retained provider evidence |
| Aging | Estimated | 210,000 millionths | `capacity_confidence_age_decayed` |
| Stale | Unknown | Unknown | `capacity_snapshot_stale` |
| Reset | Unknown | Unknown | `capacity_window_reset` |

Claude returned Unknown rather than a fabricated quota. Querying the disconnected
Hub returned `env-p4-c5-scanner` in `offlineEnvironments` while preserving its
stale Unknown capacity projection. Four controlled snapshots were inserted only
into the cloned capacity store through `Store::save_capacity_snapshots`. Their
source names begin with `p4_c5_controlled_`. Conflicting 300,000 and 700,000
remaining values selected the safer 300,000 and reported
`capacity_evidence_conflict`. A local zero-remaining hard limit overrode a Hub
900,000-remaining report, projected `hard_limited`, and reported both
`capacity_evidence_conflict` and `local_hard_limit_active`.

With capacity routing enabled, the exact route rejected Codex for
`capacity_hard_limit`; Claude was independently rejected because authentication
was unverified, so the request failed closed with `no_eligible_failover_route`.
With only capacity routing disabled, the next isolated route selected Codex with
score 1,750 despite the controlled hard-limit snapshot. The switch was restored
immediately. The provider turn later failed against stale cloned authentication
and was canceled; it is not provider-success or capacity-failure evidence.

The retained clone also produced one real Claude launch that failed on expired
OAuth and one real Codex launch that failed on workspace discovery authorization.
Neither error represented quota exhaustion. No attempt was made to spend toward
a provider limit merely to force acceptance. Therefore these runs do not close
the real pre-output retry row or any real post-side-effect interruption row.

### Diagnostic failover and rollback coverage

Five focused `pandamux-server` lifecycle tests passed on 0.60.17. They prove a
safe pre-output fixture retries once onto an allowed pool member, disabling
automatic failover persists an explicit interruption, text, file edit, command,
network action, and approval boundaries never replay automatically, an audited
continuation remains idempotent after manager restart, and a node honors the
Hub policy snapshot without a local organization copy. These fixtures preserve
the expected checkpoint, partial output, tool ledger, capacity evidence, and
continuation state, but they remain diagnostic support rather than live-provider
acceptance.

All four rollback settings were exercised independently. Scanner and Hub sync
behavior were proven across process starts. Capacity routing changed the next
route and was restored. Automatic failover persisted false and then true while
all 10 retained attempts and 43 retained capacity-clone observations remained
queryable; its interruption behavior is covered by the focused fixture above.
The final persisted scanner, Hub-sync, capacity-routing, and automatic-failover
flags were all enabled.

### Isolation correction and cleanup

One scanner-toggle restart initially omitted the required
`PANDAMUX_DATA_DIR`. Relative profile paths therefore resolved against the
normal Galahad PandaMUX data directory and the scanner read 20 unrelated
historical records. This was an acceptance-harness error, not accepted product
evidence. No normal-profile file changed in the measured window. The failed
scanner and capacity databases were preserved under
`failed-restart-missing-data-dir`, both working clones were restored from their
pre-test backups, and the complete scanner, Hub, capacity, and rollback rows
were rerun with each live process showing its isolated data directory in
`/proc/<pid>/environ`.

A separate local probe accidentally used a stale pre-0.60.17 binary from the
release target directory. File birth time proved that it created a new empty
default `hub.db`; no prior database was overwritten. The process was stopped,
the database passed integrity checking, and it was moved recoverably into the
local acceptance tree with SHA-256
`9d0431a682890469daf636dadd8509e354bbcf9c939774b5dbd12d64278d27f5`.
The default path was absent afterward. The copied exact 0.60.17 artifact then
printed help without starting a server.

Both Galahad acceptance daemons stopped gracefully, no process retained the
P4-C5 root, and the retained source database still hashed to
`8392e444508c6c598312e97adf1c761dafc8674dcc704c2f7cdca6aa97de806f`.
The original dirty checkout remained at `9511d0e22d44` with the same nine
modified and three untracked prototype paths. No production settings, database,
deployment, activation, or release was changed.

## Native Usage accessibility probe

An interactive `C4adUser` scheduled task launched the exact Windows desktop
from its isolated profile and queried Windows UI Automation. The first query
returned only the top-level window because GPUI activates its AccessKit tree
on demand. After allowing the requested refresh frame, a second query found
12 named, keyboard-focusable navigation controls, including `Usage and
capacity`. Invoking that control through UI Automation exposed 26 descendant
elements, including 24 keyboard-focusable controls. They included the Usage
retry action, eight named filter controls, and six named subscription-edit
controls. A subsequent native 20-step Tab sequence reached the retry action,
all eight filters, and all six subscription-edit buttons in a stable order.
Opening the Claude subscription editor exposed named `Subscription name` and
`Masked account label` Edit controls plus Save and Cancel buttons. A test
character changed the unsaved name field from `Claude Code` to `Claude CodeX`,
proving that keyboard input reached the field. No save was invoked.

The 0.60.0 keyboard Cancel path passed: Enter on the focused Cancel button
closed the editor and restored accessibility focus to `Edit subscription for
Claude Code`. Its screen-reader path remained **unverified** because UI
Automation reported the top-level `PandaMUX` window as focused, and its Invoke
action on Cancel only focused the button. Probe results `ui-probe.json` through
`ui-probe-9.json` remain under the guest acceptance LocalAppData directory.

The exact 0.60.3 follow-up ran in the active limited
`COLT-SHINE\B_StL` console session with a new isolated LocalAppData profile per
run. The target behavior was deliberately narrow: UIA focused the existing Edit
button and keyboard Enter opened the editor, then the application had to publish
the name field as focused without a corrective UIA focus call. A single UIA
Invoke on Cancel then had to close the editor and restore focus to the original
Edit button.

The first strict 0.60.3 run was a transient miss: keyboard input still changed
`Claude Code` to `Claude CodeX`, direct UIA Cancel closed the editor, and no
process leaked, but UIA reported the top-level window instead of the input. A
fresh diagnostic run then passed before its later explicit focus diagnostic,
and two additional fresh-profile strict repeats passed without any corrective
focus call. In all three passing runs:

- the focused element was named `Subscription name`, had type
  `ControlType.Edit`, AutomationId `usage.subscription.name`, and
  `HasKeyboardFocus=true`;
- the same node was keyboard-focusable, a control and content element, and
  exposed the UIA Value pattern;
- keyboard input changed the unsaved value from `Claude Code` to
  `Claude CodeX`;
- one UIA Invoke on Cancel closed the editor; and
- focus returned to `Edit subscription for Claude Code` with
  `HasKeyboardFocus=true`.

The diagnostic task and both strict repeat tasks returned 0, were unregistered,
and left no exact acceptance process running. This closes the functional editor
focus and direct Cancel rows with repeat evidence. The initial transient miss
remains recorded and should be watched during the remaining screen-reader and
full-matrix runs rather than being treated as nonexistent.

This 0.60.3 result is native control exposure, keyboard focus, and invocation
evidence only. It does not establish announcement of usage values or table text
to a screen reader, 200 percent scaling, reduced motion, forced colors, or
visual non-color state. Those rows remained open for 0.60.3 and were later
closed by the exact 0.60.22 matrix below.

## Exact 0.60.22 P4-C3 native Windows acceptance, reopened

P4-C3 is **Reopened**. Clean commit
`fee9e02275dceafd87c25a36af0732cd38c47076`, version `0.60.22`, passed all rows
that the original matrix ran against the same desktop, server, and CLI hashes.
The matrix covered login preview and cancellation but omitted successful login
launch, terminal discovery, output, input, and close. The retained host
evidence root is
`C:\ProgramData\PandaMUX\acceptance\phase4-fee9e02-0.60.22-p4-c3`.
Its original `p4-c3-acceptance-summary.json` is 14,897 bytes and has SHA-256
`88cb877059ba08bc2cba1e2cc9903ab1e87bf20897b3cd7d34b11ae0a29c9ed2`.
A later contract audit found that its native table evidence covered the empty
state and headers but not a populated row or cells. The exact-candidate
follow-up closes that clause in the 8,252-byte
`populated-row-followup\p4-c3-populated-row-addendum.json`, SHA-256
`ed36c4221827d5377035dab393bd6ff4923dbef183b77087e1b9799a8933ebb6`.
The original summary and this addendum support their recorded rows, but they no
longer support an overall P4-C3 Pass.

### Post-acceptance successful-login failure

On 2026-10-07, manual use of the retained 0.60.22 candidate confirmed a
provider login preview and selected `Open login terminal`. The server returned
success and the desktop banner reported that the terminal opened. No terminal
was visible from Settings, and selecting the title-bar Surfaces control produced
no visible result. Source inspection established four connected defects:

- Settings intentionally omitted the Surfaces panel while the control still
  toggled its hidden state.
- The desktop discarded `ProviderLoginLaunchResult.terminal_id` instead of
  binding it to a Terminal surface.
- The Terminal surface had no live attach loop or keyboard-input path to its
  server-owned PTY.
- A login terminal created on a remote node was not registered in the Hub's
  terminal-owner map, so follow-up terminal requests would route locally.

Version 0.60.24 was the first source repair candidate. It keeps Surfaces available in
Settings, moves the confirmation card near the top of Provider settings,
automatically opens and focuses the returned Terminal surface, attaches bounded
ring-buffer output, forwards keyboard input, exposes an explicit close action,
keeps errors visible, and registers forwarded login terminal IDs. Local Rust
checks do not replace the reopened native row.

### Rejected 0.60.24 native repair attempt

Clean commit `581113eb6daadd87827b8c730ff81f69a4bf9ab9`, version `0.60.24`,
was built by `COLT-SHINE\B_StL` in the isolated root
`C:\ProgramData\PandaMUX\acceptance\phase4-581113e-0.60.24-p4-c3-repair`.
The source was clean before and after the build, and the 26,182-byte transfer
bundle had SHA-256
`b5a45a4d284efce1e6a7b2622c24f0ce4e533326229b7ab646c528047e5aae33`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,908,608 | `484c91f24151b6c496a06c266a9b58237f12c12fff54ffc85f7d3e2d9afda6f3` |
| Windows MSVC `pandamux-server.exe` | 14,327,808 | `6725d6b9ee2373c8c52944faa0d54f4b8dabc18415ef62074e1fa4e1f241d380` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `c6993fbebabe7ec45603eb543bff243cd9b38649016d55a07b6497395dd6d27a` |

The clean limited-user headless check and authenticated Hub and CLI smoke
passed. The latter reported version 0.60.24, the expected SID-scoped pipe, Hub
role, Windows platform, and successful `hello`, `ping`, and `identify`, then
stopped the server and removed its runtime credential. The 1,904-byte
`build-result.json` has SHA-256
`9ce04733cbb2acef0b1354ac98b5c9ff77028c1e7176c308f3b74f508f0d7ab0`;
the 1,817-byte `limited-user-smoke.json` has SHA-256
`b2d08e4c59456ca39b0bbd6780641ac928f917c74f4cafa9d617532ef10725f5`.

The second controlled UI attempt exposed a visible confirmation without
scrolling, kept Settings visible, opened the Terminal surface in Surfaces, and
reported the interactive Terminal group as keyboard-focused. The inspected
125,009-byte screenshot, SHA-256
`8e211f1df9e446da722bd07c148700c3095863bcd08c9f7b49f6e223f46235af`,
also showed the reviewed PowerShell command at a continuation prompt. The
controlled fixture never printed its ready marker, so input, panel retention,
and close were not credited. The 9,879-byte rejected result has SHA-256
`78af875beb190aef1b08f595991995908fe50bb95b686cab89b010573a04a2a0`.
The first attempt searched only the old short UIA action name and is retained
as a harness rejection.

### Rejected 0.60.25 native repair attempt

Clean commit `8f0ec2040832cf135d20293eaf61f3e5847d9629`, version `0.60.25`,
was built by `COLT-SHINE\B_StL` in the isolated root
`C:\ProgramData\PandaMUX\acceptance\phase4-8f0ec20-0.60.25-p4-c3-repair`.
The source was clean before and after the build. The 5,212-byte transfer bundle
had SHA-256
`fbe339fce879dbb4d1c72e59ec996671e06d0a74129c75deb04ce3eb4c9fce3e`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,909,120 | `d6bbcc40985146772fe8419fbc68956ddd73373d56e549b55af6e956e1174248` |
| Windows MSVC `pandamux-server.exe` | 14,328,320 | `74cd22e0c278bb34cf59caaf76f4ba4743b542a7023af591bb45f2c7c59ffe6a` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `1aab1a7c1782eefa637a27434981e03bfa84853d586a44101c601a197f34e64a` |

The exact limited-user headless check and authenticated Hub and CLI smoke
passed. The latter verified the version, SID-scoped pipe, `hello`, `ping`, and
`identify`, then stopped the server and removed its runtime credential. The
native mock-remote test also passed provider-login terminal registration plus
subsequent attach, input, and close routing with the source clean before and
after. The 1,903-byte `build-result.json`, 476-byte `headless-smoke.json`,
1,817-byte `limited-user-smoke.json`, and 402-byte
`remote-login-routing-test.json` have SHA-256 values
`f973a25f6af8d0f1fb32ca130fe317a9097d5a99819344a58ca93f8b6fdd26cb`,
`d45baad7a2f243500751ba3b98b6eb71e18c01257e261701d78ef23ffb665b9e`,
`7f25d097f4aed69d9296a78a9c276285e3d0b3b861d4b7ac8a8c9d6ff389864c`,
and `9c2afc4305c3ad255850916416676a95dd1e667699fd7e11f672523577b69478`.

At 96 DPI, the controlled login confirmation was visible without scrolling,
Settings remained visible, Surfaces opened, and the interactive Terminal group
received keyboard focus. The inspected 127,130-byte screenshot, SHA-256
`5120f9adc2a2b8d01f350c5412362f956d38bf732da4e1f9b93659e54b123ef8`,
showed the reviewed command executing and the controlled fixture's READY and
PROMPT output. The output was not present in the Windows UI Automation tree,
so the harness could not validate it and Narrator could not inspect it. Input,
panel retention, explicit close, and the condition matrix were not credited.
The 10,028-byte rejected result has SHA-256
`9f536b19336c1dfab45f662df99e8d7cad6408405ea1c667d97892f360998fbd`.

Version 0.60.26 exposed the bounded visible grid as a semantic Log and as the
focused Terminal's accessible description. It passed the interaction, High
Contrast, and disabled-animation rows, but Narrator Speech Recap did not speak
the controlled READY or ECHO output.

### Rejected 0.60.27 native repair attempt

Clean commit `5e00715e1b3b9e343d192cee254348e3d9a3b7c5`, version `0.60.27`,
was built by `COLT-SHINE\B_StL` in the isolated root
`C:\ProgramData\PandaMUX\acceptance\phase4-5e00715-0.60.27-p4-c3-repair-run2`.
The source was clean before and after the build. The 4,796-byte transfer bundle
had SHA-256
`f2d2fa76a3a3864a8cb5bcf09485416438b6562f17dfbc8e26a87bc70f3861cc`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,907,584 | `06c44736ac0bdc5d8551fa4d150a1007bf5019fcc45a823d25f68ccb849d1cc2` |
| Windows MSVC `pandamux-server.exe` | 14,326,784 | `596f547452ae2f2c072fd86414e2538cb68545b9ac2448b35a28a98d3a190015` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `37d2208562d30435d6f4fd02ff3d378b98613225d12b4fca902076d1a57b48f3` |

The exact clean build, headless check, limited-user Hub and CLI smoke, and
mock-remote login-terminal lifecycle test passed. Native lab runs at 100 and
200 percent scaling passed successful launch, visible confirmation, automatic
Terminal discovery, READY and ECHO output, input, panel reopen, retained
output, and explicit close. Separate High Contrast and disabled-animation rows
also passed and were restored after inspection.

The genuine Narrator run then exposed that calling UI Automation `SetFocus()`
on the reported focusable Log left focus on the PandaMUX window. Source review
also confirmed that ordinary Tab was still forwarded to the PTY, so the visible
instruction to Tab to the Log described an unreachable route. Narrator output
was not credited and the attempt is rejected. Exact 0.60.28 added an explicit
F6 toggle between terminal input and the output Log, retained Shift Tab as a
return path from the Log, and preserved ordinary Tab as PTY input. Its native
build, smoke, remote login lifecycle, successful-login interaction, 200
percent scaling, High Contrast, and disabled-animation rows passed. Narrator
then displaced focus to the PandaMUX window, where the terminal-scoped F6
handler could not run. UI Automation focus and real pointer recovery did not
restore the child focus, so Narrator output was not credited and 0.60.28 is
rejected. Exact 0.60.29 gave the Log an application-owned focus handle and
moved F6 to the root visual element. Its native build, smoke, remote lifecycle,
successful-login, 200 percent, High Contrast, and disabled-animation rows
passed. Narrator reproduced window-only focus, but the visual root still had no
GPUI focus path and did not receive F6, so 0.60.29 is rejected. Exact 0.60.30
moved F6 to an application-level interceptor. Its native build, smoke, remote
lifecycle, and standard successful-login interaction passed, but Narrator
still consumed or withheld F6 before PandaMUX received it, so 0.60.30 is
rejected. Exact 0.60.31 commit `0394eb3` added explicit accessible `Type here`
and `Read output` controls and passed native build, smoke, remote lifecycle,
and standard successful-login interaction, but Narrator invoking `Read output`
left UI Automation focus on the PandaMUX window. Exact 0.60.32 commit
`952b280` deferred the focus request and passed native build, smoke, remote
lifecycle, and standard successful-login interaction, but direct UI Automation
Invoke still left focus on the PandaMUX window. Source review traced that
failure to the styled button's synthesized-pointer accessibility fallback.
Exact 0.60.33 replaced that fallback with direct AccessKit Click handlers for
both visible focus controls and passed the standard successful-login row, but
its accessibility handler deferred the focus update and UI Automation Invoke
still left focus on the PandaMUX window under Narrator. Exact 0.60.34 focused
the requested terminal target synchronously and again passed the standard row,
but Windows UI Automation still restored window-level focus after that callback.
Exact 0.60.35 repeated the requested focus 100 milliseconds after platform
Invoke dispatch and again passed the standard row. Narrator still left UI
Automation focus on the PandaMUX window because GPUI already considered the
output handle focused and elided the same-handle retry. Version 0.60.36 first
focused the invoked control, making the delayed move to the output Log a real
GPUI focus transition. Exact 0.60.36 passed the standard row, but UI Automation
still reported the PandaMUX window. Exact 0.60.37 activated the native window
and passed the same standard row, but the Narrator result was unchanged.
Assisted diagnostics proved PandaMUX already owned the foreground window and
had visibly entered output-reading mode, isolating the defect to accessibility
focus publication. Exact 0.60.38 used GPUI's active-descendant pattern instead
of moving real keyboard focus to the output child. Its complete local gate,
native build, host smoke, remote lifecycle, and standard successful-login row
passed. Ordinary UI Automation honored the active descendant, but Narrator
still did not speak READY. Instrumentation proved PandaMUX retained native
foreground, active, and keyboard focus. Narrator navigation synchronized to
that focus, reached all three terminal header controls, and skipped the
semantic Log group. Exact 0.60.39 changed that output node to AccessKit's
dedicated Terminal role and passed the complete local gate, native build, host
gates, and standard successful-login interaction. Windows reported a focusable
UI Automation Document, but Narrator visited it without speaking its
parent-only accessible name and then left PandaMUX. Exact 0.60.40 retained that
Document, but the standard row rejected it because its label-only semantic
child was absent from the Windows UI Automation tree. Version 0.60.41 gives the
Label child a value backed by a synthetic text run so UIA can expose TextPattern
content. P4-C3 stays open until an exact 0.60.41 or later Windows artifact
passes successful launch, discovery, output, input, close, panel
reopen, and the retained accessibility and scaling matrix.

### Rejected 0.60.40 native repair attempt

Clean commit `d4c9dd891cf9cb42eb919bc45237f5ade0860352`, version `0.60.40`,
passed the complete local gate with 437 workspace tests and two intentional
real-shell ignores. Its verified 6,288-byte incremental source bundle has
SHA-256
`eb28800f7924fe966210f32d662df5e49fce0659c64e7dcface429b6c8dff0f5`.
The exact native Windows build result has SHA-256
`95559a670d8568a6d3c1d454a94877c2c153d929021fc0320bcf79af3199ff46`.
The source was clean before and after its Rust 1.98.1 MSVC build.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,972,096 | `15212f31ec59d64bea3ef164e6f1cc63644359501c56682c967ef60811659fa9` |
| Windows MSVC `pandamux-server.exe` | 14,328,832 | `b7036cfeec043750850a9bb9fe63e7f5cb4d73841a8fc30d72427918a8af37e9` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `1aaec764c1155d83f38949d98ed9e44c27dac3103486957501fb2c675fb5fd02` |

The host headless, limited-user IPC, and exact release-mode remote terminal
lifecycle gates passed. Their result hashes are
`ed3063b60fb1b928021b171744e4fc76d867ff683276c4a5f7477d0c3f174dcb`,
`ea8a2e95ce28c59cfe9310eac573714e6b1ade96f7344f423ebe50b35e4bb38f`,
and `86797fd274f95cf5bda06260d274849f230dd5030a198824344f41072739c381`.
They are retained in the isolated host evidence root
`C:\ProgramData\PandaMUX\acceptance\phase4-d4c9dd8-0.60.40-p4-c3-terminal-text-build`.

The exact artifacts and unchanged guest-local `VCRUNTIME140.dll` were copied to
the isolated guest root
`C:\ProgramData\PandaMUX\acceptance\phase4-d4c9dd8-0.60.40-p4-c3-terminal-text-lab`.
The standard 100 percent row was deliberately strengthened to require
`terminal-output-log` as `ControlType.Document` and the proposed
`terminal-output-text` child as `ControlType.Text`. Its rejected result has
SHA-256
`0d66954abbe5c89d27c426ce38d12a1e48c73a90729bbea2b549e8131d4b1c2b`.
It verified 96 DPI, High Contrast off, UI effects enabled, visible confirmation
within 495 pixels of the window top, automatic terminal discovery and focus,
READY output, and the focusable Document parent. No `terminal-output-text`
child existed in the Windows UI Automation tree, so the harness stopped before
input or Narrator. Desktop, server, and runtime-credential cleanup succeeded.
This rejects the label-only child implementation without weakening the
Narrator acceptance requirement.

### Rejected 0.60.39 native repair attempt

Clean commit `8b7b8804999f217ef8f03fb501b679463908e990`, version `0.60.39`,
passed the complete local gate with 437 workspace tests and two intentional
real-shell ignores. Its 7,034-byte source bundle has SHA-256
`cbbac8c344f3ccbb63399cfb17404b71d246ab9448584bc21c867f54acced07c`.
The exact native Windows build result has SHA-256
`1bd9016ba37fa6d09bd1226946617a5bcad12b737c7782f91e22195d7a62ba8e`.
The source was clean before and after its Rust 1.98.1 MSVC build.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,969,024 | `468622c9f78f436ed6dbf121ea91435622eaea0ffbb5a7a6eb2dd95086cbe360` |
| Windows MSVC `pandamux-server.exe` | 14,326,784 | `39562afa56e9d86a6448fe7463a76b81afcbe36f9010d943ce3cc7ddcabfd35f` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `756b441097570d8f1887c871dd91b7b7ea4539304f3a0cbd74749b91125041d1` |

The host headless, limited-user IPC, and exact release-mode remote terminal
lifecycle gates passed. Their result hashes are
`83939f48b503e70da5b65deae31808d88fec7ac4cba425227f64400190f58623`,
`8f27e0f7def09da4d774d488a36fa49303c6ef3e9b655e9620870c1e8a6718cd`,
and `36a466ca26aad2f37dde8d7c9480a92b77263e5411a2614edc8bba69b7aa0845`.
They are retained in the isolated host evidence root
`C:\ProgramData\PandaMUX\acceptance\phase4-8b7b880-0.60.39-p4-c3-terminal-role-build`.

The first guest standard-row harness result has SHA-256
`470e527da25fda1e31fde0c70bed354048878292c9888ca81bf343e69e112527`.
It stopped before UI launch because the transfer omitted the guest-local
`VCRUNTIME140.dll`; status `0xC0000135` and a direct version probe isolated the
harness dependency omission. The result was preserved, the unchanged runtime
DLL was restored beside the binaries, and a fresh Run2 profile was used. Run2
passed and has SHA-256
`67c63963796aa6c7b4a2ea693c716683dba92d6d7c4642d6d5466ca244350982`.
It verified confirmation placement, automatic Surfaces discovery, READY, F6
focus on `terminal-output-log`, real keyboard ECHO, panel close and reopen
retention, explicit terminal close, and cleanup. Windows reported the output as
`ControlType.Document`.

The genuine Narrator result has SHA-256
`985f50a1121e44d02312504f930956444d5011b0394c94db8e797d0f656ea4b4`.
The bounded run synchronized Narrator to system focus and copied every spoken
phrase while traversing 180 items. It reached the terminal group and all three
header controls. At the next item, the Document produced no terminal phrase;
Narrator then moved into another application and never spoke READY. Desktop,
server, fixture, runtime credential, and Narrator cleanup all succeeded. This
rejects 0.60.39 and isolates the remaining defect to a Document with only a
parent accessible name and no explicit traversable text child.

### Rejected 0.60.38 native repair attempt

Clean commit `d328ad8d6c59ba0375c1715b9ee63f42c4fbf581`, version `0.60.38`,
passed the complete local gate with 437 workspace tests and two intentional
real-shell ignores. Its verified source bundle has SHA-256
`12079d16dacf53bfda4c70f58327ee530ab2b37c872032c3ee5e1ec624d7cd56`.
The exact native Windows build result has SHA-256
`5e75e816550ba70b7544c0d05f5b647554e58d6384d6f11bc4dc6cc013e4d031`.

| Artifact | SHA-256 |
|-|-|
| Windows MSVC `pandamux.exe` | `849d92f61caf400ea1bd420eb90a311c85e3b9de93333106f709c8c844406846` |
| Windows MSVC `pandamux-server.exe` | `f1485e068eef69813c4949e0851643743d29cdff519c3cd2881d4a1df80a8965` |
| Windows MSVC `pandamux-cli.exe` | `156db0615efb31e3ba2539631c7a1fed4ff4be5295b6fd6e439a089ca2597b38` |

The host headless, limited-user smoke, and remote-lifecycle results have
SHA-256 values
`7c09ffbe0d06a0a2990f702deef90ad9e1e56581d1c883fa79ea39bee8b8d489`,
`3227f4cf1dd1453ac478b9fa78c8360b2f5419d262a65e50555361f3b38ce254`,
and `340aab2dd9a751fddcbe68b81c242efe5bba6a483623ef1801024ec3111755e1`.
The standard successful-login row passed and its retained result has SHA-256
`b4e1e186d513e770b58b5a535ed7f094fb99163233fb65e305db963f33653df7`.
Ordinary UI Automation reported `terminal-output-log` after F6.

The genuine Narrator row failed and its retained result has SHA-256
`736e4ff6fab2c9c98381157da5d7aa51c5788f46461e6658d460f8f2506e44a0`.
Speech Recap did not contain READY. A forced-read diagnostic result with
SHA-256 `37cabb33a3094e7111746c665ba09085322363d0d39c3f42fae720a0034be2d4`
contained only process and window chatter. Native-focus instrumentation, result
SHA-256 `1230645193954f9f4c91493df6845cd5904105c478f081d7e4464515d5864acb`,
proved that the PandaMUX HWND remained foreground, active, and the keyboard
focus target from 0 through 1000 milliseconds after `Read output` while UI
Automation still reported the root. Finally, synchronized Narrator navigation,
result SHA-256 `bfe9bc76719a80ba214f060bc482064ed246a80844f88414ec0b2d90598bfd89`,
reached the terminal region and its `Focus terminal input`, `Focus terminal
output`, and `Close terminal` controls, then jumped to another application
without visiting the semantic Log. This rejects 0.60.38 and identifies the
platform-role mapping, rather than native window activation or keyboard focus,
as the next repair boundary.

### Rejected 0.60.29 native repair attempt

Clean commit `2b3b806a9d3e0f1c1c3709039a23f642dd4a9b2a`, version `0.60.29`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-2b3b806-0.60.29-p4-c3-repair-build-run2`.
The source was clean before and after the build. The 5,083-byte transfer bundle
had SHA-256
`3f891c9c2b3779a91ce0f1f30600562e83a3c5a5d839714a403af59c411e975f`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,933,696 | `e4873c676cf474ee4ee089e1d1ac5634912c5f793ad04f2beab14edf3e9bc592` |
| Windows MSVC `pandamux-server.exe` | 14,328,832 | `dabe983ff49dfe6ca2e7bd39225c160ca09a2dea4b86b5fec9127fd496284000` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `3bc90301afc4c01018039b2925451fd4a2b9e7f8482e9507120c9b1b176b0876` |

The exact build, headless check, limited-user authenticated Hub and CLI smoke,
and mock-remote login-terminal lifecycle test passed. Their result files have
SHA-256 values
`3d58956ccb872d8f3e4e572946b9eb25f28c482d4ccba31e20c9922cae581b95`,
`d18c4c643aa89876045bd03f72258e599086d2518e54d5e8883405e369b22f68`,
`dce3c89a343f816b379aeb80df0da826fae5595d82f6b1ee07f584d0c5b8c222`,
and `eff22c236784b8e95ceed10806b637f543b28d1fc3c0286bed87e86d4c55b8d7`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-2b3b806-0.60.29-p4-c3-repair-lab`.
The successful-login result at 100 percent, High Contrast wrapper, disabled
animations wrapper, and true 200 percent result all passed at SHA-256
`e256d5e66444b84c5d2958d6a8df4b6a2b388d490be84372cb6258c6b232218d`,
`13212a04dd32049ff51bccfca93cab33b66eecb117916a215e2138c3642c2f2d`,
`3c63774b51959e8c0b8a37bc0fab45837bfa17ed70e80868e09d47f402a20511`,
and `b5c4dc7a02f314a582e1be3c8986ce7037219ac351011a37d8f28a114a87cc66`.
Those rows verified visible confirmation, automatic Terminal discovery, READY
and ECHO output, F6 input/output switching, retained output after closing and
reopening Surfaces, keyboard close, process cleanup, and state restoration.
Their reopened-panel screenshots were visually inspected.

The genuine Narrator run reproduced the exact unresolved state: before F6,
Windows UI Automation reported the `PandaMUX` window rather than a terminal
child as focused. One real F6 keystroke left focus at that window instead of
moving it to `terminal-output-log`. The cleaned-up rejected Narrator result has
SHA-256
`9087b84fb8b2dd84fcf89be91df2ae5132bd68417a6a1a483228d71303683dea`.
The root visual handler still depended on a GPUI focus dispatch path, so this
is a product failure rather than accepted Narrator evidence.

### Rejected 0.60.30 native repair attempt

Clean commit `9dae2374aae7351626416e5973c224eaa8cfb096`, version `0.60.30`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-9dae237-0.60.30-p4-c3-repair-build`.
The source was clean before and after the build. The 5,656-byte transfer bundle
had SHA-256
`2afe9abc9f985945daede8dba272d6e5311fef8d6656c3668330ab8e9a4f1f31`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,933,184 | `3a977f9caef5e4794cc42bcee386419a7d78012ef22ba329d19ec322906509db` |
| Windows MSVC `pandamux-server.exe` | 14,328,832 | `7917fac8fc1365a18a194d7ea66a75dfbb005a5871485d78e8d616da1a30bc7e` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `21bb2c383126e662c22424b7ccba2ca9ba5f78167bd2e3458936456a3dc52d49` |

The exact build, headless check, limited-user authenticated Hub and CLI smoke,
and mock-remote login-terminal lifecycle test passed. Their result files have
SHA-256 values
`f0906d93ca88a946f769d8194bf5821943be20f27e67f991d20676097201680c`,
`75786031fd472b1ab42f49898c11dd310745860e7e10ee2c463f41c14355637b`,
`7bc3014b5f77467e7c54a2102a2183c0e6417f5ffbb94d6400492470c0ce1072`,
and `0557af652ac4bdf8684c7b96410ec6032de504ff3ed52262da675f2bd50ddcf5`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-9dae237-0.60.30-p4-c3-repair-lab`.
The standard successful-login result passed at SHA-256
`853e0ace4249501349137d4627440cd29311bf27d5554d36896fcfde230f22cc`.
It verified visible confirmation, automatic Terminal discovery, READY and ECHO
output, F6 input/output switching, retained output after closing and reopening
Surfaces, keyboard close, process cleanup, and 96 DPI state.

The genuine Narrator run again reproduced the exact unresolved state: before
F6, Windows UI Automation reported the `PandaMUX` window rather than a terminal
child as focused. One real F6 keystroke left focus at that window instead of
moving it to `terminal-output-log`, even though the handler was registered as
an application-level keystroke interceptor. The cleaned-up rejected Narrator
result has SHA-256
`0288451926a846ef3786295fe59d7deaade31cf7ab2b9f1de54b9edb3d2d8307`.
The next candidate therefore exposes explicit accessible controls instead of
assuming assistive technology will pass the shortcut through.

### Rejected 0.60.31 native repair attempt

Clean commit `0394eb36ed3a93dd2c01099a0abab1adcf5d0d52`, version `0.60.31`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-0394eb3-0.60.31-p4-c3-repair-build`.
The source was clean before and after the build. The 5,594-byte transfer bundle
had SHA-256
`5332cb1ab405880d43cbfd106c2db84ba8d0abcf1c7ef4f5e843f3d3b9d6b485`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,944,448 | `1954b864627ea8f942a5f4b4aca69e700694fc479e3158b1ff57aa196f253c86` |
| Windows MSVC `pandamux-server.exe` | 14,327,296 | `73137f3c40a806e16ffe6f96e894620fbdb5d6da3ecf80464971c98f4803d793` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `e717a2e5bae65f14e4cb300507794524d1790d2e2b9d8d39bcd3d88157ff9089` |

The exact build result passed at SHA-256
`12fcdbc4ccd3d9a79a0f25eaa1e74326b8492bc3655746aa158cd6095e6908b3`.
The first headless wrapper emitted `ALL CHECKS PASSED` but failed to capture its
exit and cleanup fields; its preserved incomplete-run manifest has SHA-256
`af6f45c090d3dfdd5334c1422ccfb2bca42355412311f5584f8a26242dedcbfc`
and is not acceptance evidence. A fresh-profile run with explicit stage
snapshots passed at SHA-256
`e50638bf6c4e3bd3efb8d01078a9269aed1353cc3d59ae2d03029fe97a4458f2`.
The limited-user authenticated Hub and CLI smoke and exact remote terminal
lifecycle test passed at SHA-256
`5a78b19720b03d4b9e32ade60ba1e7972c36d263878a40b353dd2571ccc1e1a4`
and
`b8dec64287d157c4982583f01db68290382057a299a6caf7a1a3a07eb39aff23`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-0394eb3-0.60.31-p4-c3-repair-lab`.
The standard successful-login row passed at SHA-256
`01a8c372960449ceed305bc8cbb542a458369bb62f0136bc93b7cdd07014c4b7`.
It verified visible confirmation, automatic Terminal discovery, visible and
invokable `Type here` and `Read output` controls, F6 input/output switching,
READY and ECHO output, retained output after closing and reopening Surfaces,
keyboard close, process cleanup, and 96 DPI state.

The genuine Narrator run established window-level focus and confirmed both
explicit controls remained visible and exposed InvokePattern. Invoking `Read
output` still left UI Automation focus on the `PandaMUX` window rather than
`terminal-output-log`, so no Speech Recap result was credited. The cleaned-up
rejected result has SHA-256
`95ce918915a1d737fde8609ce7b5a89cc18a0d3736761208914f11173010aa8d`.
Version 0.60.32 therefore defers the requested terminal focus until the
accessibility button activation has completed.

### Rejected 0.60.32 native repair attempt

Clean commit `952b28037aaa9ce9bd19a94cf26831f1b1ef38b3`, version `0.60.32`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-952b280-0.60.32-p4-c3-repair-build`.
The source was clean before and after the build. The 5,511-byte transfer bundle
had SHA-256
`9aaa0c6e0010842e943ed6bbc4c1f2f0dd6f72b89b668046a68161d8dd93d414`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,945,472 | `c788eb03239dd9371482d4271542a533c6b6c934077e628461ef3dde69389b4f` |
| Windows MSVC `pandamux-server.exe` | 14,328,832 | `6d86dce29dfeccc171ea44bab837039f1671755c19c120ca688628cefe86e93f` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `48dedc2fa5e004585bc19dfd232c6dc6d025b1296a32b0af08c2fceab64f38b5` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`a20e8f8369955738aabfee9a0c21e4b33bd32be572007002f059ea89f86ea16e`,
`0c9bb8ce7843f191288e917badc25396fa28e12606eff26c48133180168e7f9f`,
`3ba19efc47dd78ca22bb5f205f0acd7eee7926cd2f1f3252cea1ceb2f358b34c`,
and `0fc3c78c54803ea766e8fbff75725477bbe14e613d17804b46e31171e8dba550`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-952b280-0.60.32-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`29cb9d48e3b1b0f613b1f0d6adac1f6ef2ceac1b1a51f9ba8146e3b075058493`.
It verified exact artifact identity, visible confirmation, automatic Terminal
discovery, visible and invokable `Type here` and `Read output` controls, F6 in
both directions, READY and ECHO output, retained output after closing and
reopening Surfaces, keyboard close, fixture shutdown, and complete cleanup.

The primary genuine Narrator run invoked `Read output` through its reported UI
Automation InvokePattern without using F6. The control remained visible and
invokable, but `terminal-output-log` did not receive focus; UI Automation still
reported the `PandaMUX` window. The cleaned-up rejected result has SHA-256
`7b99a6dc3d8ef725ff61dc4af398d96e9af5638b435a3feeaf85fdd5339397b7`.
A second run confirmed that UI Automation could not first place keyboard focus
on the button and has SHA-256
`2e9fc42a25f27529d4546dd909802a27c3d7980231434910d957beda79089bfe`.
Two preserved pointer diagnostics are not acceptance evidence, but they showed
that real injected pointer input neither focused the Log nor changed the
visible mode status; their result hashes are
`3e0303163e4cd525dc65862faf6dc0c0729aca9d4a3b8f67c98333e0fb2a180f`
and
`26a9e9ffe74f6dcea60620c5e42b0d1a5b5e816cae80518dd5e27d317435f851`.
All four runs stopped the desktop and server and removed the runtime
credential. Source review traced the failed Invoke path to the styled button's
synthesized-pointer fallback. Version 0.60.33 gives both focus controls direct
AccessKit Click handlers while retaining deferred terminal focus.

### Rejected 0.60.33 native repair attempt

Clean commit `58452e70dbe2c1ca14bdf05270300bcc6112b4d1`, version `0.60.33`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-58452e7-0.60.33-p4-c3-repair-build`.
The source was clean before and after the build. The 6,406-byte transfer bundle
had SHA-256
`02189a75945eed8cfaa179ee435d877de3264185cfc7414fb19196c526a0001e`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,962,368 | `867a34c325863c5978b1c425db5abd5de6ffbd9d8d181999fd4e656638768770` |
| Windows MSVC `pandamux-server.exe` | 14,324,736 | `c5efd5c5b59b33cf81b19df04eeafce0c58b10bf026ba0b544f78e3fb7b0f23d` |
| Windows MSVC `pandamux-cli.exe` | 1,408,512 | `2cde6ed02cc5550f16488d6155384a668c2d0440ea797c2cc46cc1d629f464cf` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`7943c117492d3ceba525431725c88dc3687b37dbb39498cb2654afecd2da5560`,
`7a31c8d31de35fd38e19cc776a255c6c82c5b5554604e796e690efb19dbd6f4f`,
`7bd4d1bf9d7d70086ce36e99e4122d916ff72fc52cf6d2ea1fca5e865215b67a`,
and `e3935f99bd29323fd896d26289b73d8cc24b724e231b2513b3878379c020eea2`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-58452e7-0.60.33-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`d8c9858e7308bd9ee65c474199e33ba2dc4b36ead2a3a35dfae4c78a14cb9f19`.
It verified exact artifact identity, visible confirmation, automatic Terminal
discovery, visible and invokable `Type here` and `Read output` controls, F6 in
both directions, READY and ECHO output, retained output after closing and
reopening Surfaces, keyboard close, fixture shutdown, and complete cleanup.

The primary genuine Narrator run invoked `Read output` through its reported UI
Automation InvokePattern without using F6. The control remained visible and
invokable, but `terminal-output-log` did not receive focus; UI Automation still
reported the `PandaMUX` window. The cleaned-up rejected result has SHA-256
`c37dcde22738b4edf6235e9ff70a8b35f717c22307f790ce03805c77a3fb3a65`.
No Speech Recap output was credited. The run stopped the desktop and server and
removed the runtime credential. Source review confirmed that the direct
AccessKit handler still deferred its focus update. Version 0.60.34 performs
that accessibility focus synchronously, matching GPUI's own Focus action path;
pointer activation remains deferred.

### Rejected 0.60.34 native repair attempt

Clean commit `1f1df21eaba68c2874b30ed94e02459a6f96a641`, version `0.60.34`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-1f1df21-0.60.34-p4-c3-repair-build`.
The source was clean before and after the build and exact native routing test.
The 4,766-byte incremental transfer bundle required exact 0.60.33 commit
`58452e70dbe2c1ca14bdf05270300bcc6112b4d1` and had SHA-256
`c8a1d704275ef3cd290e3b2ca13098e2dd5e7d45552ba1a715f27174581d6f7a`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,960,832 | `c1a93ff4e08ffb55c2549d3b1b852f947c497f7eef34b31a3dc1bbc344c73d3d` |
| Windows MSVC `pandamux-server.exe` | 14,328,320 | `ae553ea5e59c43231dd23b1ee7ccdd04d1674dd73da5119c3e97e8ee2c7888b9` |
| Windows MSVC `pandamux-cli.exe` | 1,408,512 | `bb658ddde8ba4549b7f4a109e5c146ad26f8bd3f76c80109f8d07be72aceeeb1` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`a9f20e0ff2e83424e61364e9f08008752a5654b3544d7a8ef1245ce12d2636ba`,
`8f693a84e1208d04170d3fb74b5d05ca09295bcf1278e55331156143140d1858`,
`e65925ff76438278da28169ccd3e539b0d231ba3e01a40ae6528a12a389bc1fa`,
and `62ff4633f2209863ca5bd0f52e0873f7e1ba8009a81b11cd69b52298e4dc29da`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-1f1df21-0.60.34-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`5e0cf9eb02332fc95894b03d2f4e4b04fc0da554bd2f4b832acb13788fe4a548`.
It verified exact artifact identity, visible confirmation, automatic Terminal
discovery, visible and invokable focus controls, F6 in both directions, READY
and ECHO output, retained output after closing and reopening Surfaces, keyboard
close, fixture shutdown, and complete cleanup.

The primary genuine Narrator run invoked `Read output` through its reported UI
Automation InvokePattern without using F6. The window-level focus precondition
was present and the control remained visible and invokable, but the synchronous
focus request did not persist: UI Automation still reported the `PandaMUX`
window instead of `terminal-output-log`. The cleaned-up rejected result has
SHA-256
`da3b4500b0a28d7a15f97a39cda1359be3cb77c0da6c10562967ba0794c88227`.
No Speech Recap output was credited. The run stopped Narrator, the desktop,
server, and fixture and removed the runtime credential. Version 0.60.35 keeps
the immediate request and repeats focus once on the application thread 100
milliseconds later, after platform Invoke dispatch completes.

### Rejected 0.60.35 native repair attempt

Clean commit `b54f8e39c4e3b60e328b9a4b58df70d0558bd776`, version `0.60.35`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-b54f8e3-0.60.35-p4-c3-repair-build`.
The source was clean before and after the build and exact native routing test.
The 5,039-byte incremental transfer bundle required exact 0.60.34 commit
`1f1df21eaba68c2874b30ed94e02459a6f96a641` and had SHA-256
`260d08617b4cce2ef07e3a26a6e284ac18a284e73cb6445c8e233199a5e77ad8`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,959,808 | `3eab2f1ac86e44982666cb4fcc6f1254eb4111e8e71e1425e1f7209d2d14389a` |
| Windows MSVC `pandamux-server.exe` | 14,326,784 | `aaef1dc84df415bf55d2844d4703a6cdfdb5efe8ee5367486446a4a461fe8181` |
| Windows MSVC `pandamux-cli.exe` | 1,408,512 | `75ba13ccf139d5ea4e58a295a0efd44fcf79457ec8e5ce9d8aedf2dfadf23d2c` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`9a4fc8d220513b6f87f425fe75faebb1a473390b9e43fccdd2dd1a1b45e35fe2`,
`276d295a9d15f36949932fedd1282d9152234827c3c28e3c25d0f42d223fc9c5`,
`40ab825406ab7e3990e831165bc91f47b617a9520dd8ff114232b46738ecf245`,
and `6e50176f6cfe7dd37a01e39e2f3c857f5f84a2b4302383f13f10d868b5a09ac1`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-b54f8e3-0.60.35-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`ed8ce413342f85b0c8fdb562573a3b5c6031b35ee6c0272a703b49f2068ea875`.
It verified exact artifact identity, visible confirmation, automatic Terminal
discovery, visible and invokable focus controls, F6 in both directions, READY
and ECHO output, retained output after closing and reopening Surfaces, keyboard
close, fixture shutdown, and complete cleanup.

The primary genuine Narrator run reached the window-level focus precondition
and found the visible, invokable `Read output` control. Invoking it still left
UI Automation focus on `PandaMUX`, so the harness stopped before sending any
Narrator read command and no Speech Recap output was credited. The exact error
was `Read output did not recover the terminal output Log from Narrator window
focus; focused: PandaMUX`. The cleaned-up rejected result has SHA-256
`02ca9ed61aed7df420f019b669b128a6ba65020a597af6ec73f0b81ad2f8006b`.
The run stopped Narrator, the desktop, server, and fixture and removed the
runtime credential. Source review found that the immediate request made GPUI
consider the output handle focused, so the delayed request for that same handle
returned early even though Windows had restored external focus to the window.
Version 0.60.36 first focuses the invoked control, then performs the delayed
move to the requested terminal target as a distinct GPUI focus transition.

### Rejected 0.60.36 native repair attempt

Clean commit `6fd5d027945312a7ba066e9fc42e5bd37dc05fe5`, version `0.60.36`,
was built by `COLT-SHINE\B_StL` from an isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-6fd5d02-0.60.36-p4-c3-repair-build`.
The source was clean before and after the build and exact native routing test.
The 5,410-byte incremental transfer bundle required exact 0.60.35 commit
`b54f8e39c4e3b60e328b9a4b58df70d0558bd776` and had SHA-256
`8f79843e7d5e1c5379ab59d17efc0e85a74bfb584e471c1f187bb64f33287893`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,960,320 | `d1a4668dfe82e7400fdfb3659cb45b6cefd85c73b09a829b9d5065003d51c233` |
| Windows MSVC `pandamux-server.exe` | 14,326,784 | `56a7fb8b160088243a206bd181ff82182e1ddc44bbdba61c73e630bb18bdeced` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `530b1abab938e407ade4703b213f4fbcef3a96dfd882b9375a0eb99d6c3edb10` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`24e6b90bb2db9ec8eea0aaf1695ce80f101f5728b189054962bab60e1382f8be`,
`6c742bffdb00f85c476aea136647af2b6d3dd26c014593bbf46e52310ff889b2`,
`ef4cefa15a2f38eed1d81775af4972b76b173368ae9d5ce4925814ef69b2570f`,
and `be87ab7c174859b042d3c257a946ad3bdb849c685bc0929c072af9acfeef7025`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-6fd5d02-0.60.36-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`582071c5ddd898b74f2e77c733e9d82c3fdbd7e48f8ec4d7f5a46ca1ca88865c`.
It verified exact artifact identity, visible confirmation, automatic Terminal
discovery, visible and invokable focus controls, F6 in both directions, READY
and ECHO output, retained output after closing and reopening Surfaces, keyboard
close, fixture shutdown, and complete cleanup.

The primary genuine Narrator run reached the window-level focus precondition
and found the visible, invokable `Read output` control. Invoking it still left
UI Automation focus on `PandaMUX`, so the harness stopped before sending any
Narrator read command and no Speech Recap output was credited. The exact error
was `Read output did not recover the terminal output Log from Narrator window
focus; focused: PandaMUX`. The cleaned-up rejected result has SHA-256
`4bf74e330dc9318934a349874e03ffe2ac4dcb9ff759ed64d531ecacd899a7fb`.
The run stopped Narrator, the desktop, server, and fixture and removed the
runtime credential. At that point native foreground ownership was still only a
hypothesis, so version 0.60.37 explicitly activated PandaMUX before the delayed
move. The later 0.60.37 diagnostics below disproved that hypothesis.

### Rejected 0.60.37 native repair attempt

Clean commit `f21bc2da89ce25975cf422f16906170729c28778`, version `0.60.37`,
was built by `COLT-SHINE\B_StL` from isolated source under
`C:\ProgramData\PandaMUX\acceptance\phase4-f21bc2d-0.60.37-p4-c3-repair-build-run2`.
The source was clean before and after the build and exact native routing test.
The 5,017-byte incremental transfer bundle required exact 0.60.36 commit
`6fd5d027945312a7ba066e9fc42e5bd37dc05fe5` and had SHA-256
`c25b99cf8173202c2556a40e1f19995f3774356990446990dacab5d48eebc30f`.
An earlier preserved preparation root failed before Cargo because the bundle
advertised its candidate as `HEAD` while the inherited wrapper fetched a branch
ref. Run 2 fetched `HEAD`, then still checked out and verified the full exact
commit before building.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,960,832 | `817120f4a26761f59d80c436b71124a40a2aad2c72e8cc292646385983e01e4f` |
| Windows MSVC `pandamux-server.exe` | 14,324,736 | `a43a5af97d8fdfd8c717767c3f28515e7d4789fbcf2991f315e797b766935c13` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `a308529b6579cd13f498fa82a1e30291567f19c5ee40cd22a8ecb4956a37a2d4` |

The exact build, fresh-profile headless check, limited-user authenticated Hub
and CLI smoke, and mock-remote login-terminal lifecycle test passed. Their
result files have SHA-256 values
`a0a2646b01629a6dd582e9ded57a70add6a58f83b388cb7282e22bbb9f97c46e`,
`80b70bf4ce18fa79c3effcdb0ca87152c46493a686e28820d1e115ba9b4254c7`,
`5a564c303b12b1f9e3bde54997bb6d45d837e81d52a35fd8eaa8ed30df047165`,
and `c1de568f4bc06c10406935a7e91c672912a2fdc5d0d39bbccbf73370de2c7dcb`.

The native lab root was
`C:\ProgramData\PandaMUX\acceptance\phase4-f21bc2d-0.60.37-p4-c3-repair-lab`.
The standard 100 percent successful-login row passed at SHA-256
`a22bc032fd4ac6d28ff22f35a8df9d0e32b3f41f4e0fd736941308a6003d489d`.
It verified exact artifact identity, confirmation placement, automatic
Terminal discovery, visible and invokable focus controls, F6 in both
directions, READY and ECHO output, retained output after closing and reopening
Surfaces, keyboard close, fixture shutdown, and complete cleanup.

The genuine Narrator row invoked the visible `Read output` control but UI
Automation still reported `PandaMUX`, so no Narrator read command or Speech
Recap capture was credited. The cleaned-up rejected result has SHA-256
`98f9ecc46421dd7cee472f9420956408525df2497f2e43d25b66350a204297cb`.
Two explicitly non-acceptance diagnostic runs then isolated the failure. Their
results have SHA-256 values
`22f41ab7d4eda6f8bb8d5866b2626fa1d1a0e467628ba5ffd44c98f60853122e`
and `a01723466cd9de5cb08551fd8541721cae84eb51816fe33281171446223ff9a3`.
Before and after Invoke, PandaMUX's HWND was already foreground and matched the
desktop process. External `SetForegroundWindow`, direct UI Automation Focus,
delivered F6, and a real pointer click did not change the global UI Automation
root. Crucially, the post-action screenshot visibly said `Reading terminal
output` and painted the output focus border. That proves the direct AccessKit
handler ran and GPUI changed its internal focus state; native foreground
activation addressed the wrong layer. Exact 0.60.38 retained real keyboard
focus on the terminal container and published the output Log as its active
descendant, the framework-supported composite focus pattern. Ordinary UI
Automation honored that pattern, but synchronized Narrator navigation skipped
the Log group. Version 0.60.39 therefore exposes the same bounded output as an
AccessKit Terminal, which Windows maps to a UI Automation Document.

### Build and limited-user startup

`COLT-SHINE\B_StL` built the clean source with Rust 1.98.1 and Cargo 1.98.1
for `x86_64-pc-windows-msvc`. The source was clean before and after the build,
and the verified source bundle had SHA-256
`5708d7235154a63ec458832517c4619f13b25c4c1d9ea217e4b1eb85bde275c6`.

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,581,440 | `d3ff7f61aa665f8caa46e09a68f157791ef6d43d7445d4f926b0a6747e1f4440` |
| Windows MSVC `pandamux-server.exe` | 14,316,544 | `3679477bd511722555a365348ac10ed315cdf3c77e3e65db37c49e2934bc66ab` |
| Windows MSVC `pandamux-cli.exe` | 1,407,488 | `850211de884bc917feed8da24617d52574364c57a22eb55f914c110cf0fbbabe` |
| App-local guest `VCRUNTIME140.dll` | 178,616 | `d1f4225df2cd877dbf130d5668a021dce3f94118455ff5ec952061c30afc9ce7` |

The exact desktop headless check returned 0 as the limited user. A temporary
exact server then created the expected SID-scoped pipe and reported version
0.60.22. Exact CLI `version`, authenticated `hello`, `ping`, and `identify`
all returned 0; `identify` reported the Windows Hub role. The server stopped
and its runtime credential was removed. The result file
`limited-user-smoke.json` has SHA-256
`1d0d19f0896a9f98b674d1028fd03097f8b34f3f7710a65b8e1b494c2264b6bd`.

### Offline, keyboard, semantic, and visual matrix

The desktop-only 96 DPI Offline run used a bin directory containing no server.
It exposed a persistent named alert with the full server-discovery reason,
`Usage status: Offline`, and keyboard-focusable Retry. Invoking Retry exposed
`Usage status: Loading`, returned to the same explicit failure, and spawned no
server process. The screenshot was visually inspected, the desktop stopped,
and `offline-matrix.json` passed at SHA-256
`bb8648048041715519e6b1c28503dde24b75e4642a48ac6bc8459de447e54a99`.

The final 100 percent matrix ran at 96 DPI in the isolated `P4C3Kb100` guest
profile. Its keyboard sequence reached Retry, all eight filters, the Usage data
table, and `Edit subscription for Claude Code`. The table accepted focus and
published this complete accessible name:

`Usage observations table. 0 observations. Columns: Provider instance;
Subscription; Model; Tokens; Source; Observed.`

The editor traversed `Subscription name`, `Masked account label`, Save, and
Cancel. Cancel closed it and returned focus to the originating Edit control.
Provider health, separate login preview and cancellation, policy controls, the
bottom sharing actions, and Save visibility also passed. Five final dark and
light screenshots were inspected for hierarchy, contrast, focus, labels, and
non-color status. `uia-keyboard-100pct.json` passed at SHA-256
`d551c6f29b5851003762feb43e78db4f4980a72cf2bcc2dd6c735f3ba1da1616`.
The retained guest archive has SHA-256
`107594581e3c191856dec1acf26f46585680dbf7c6c9d5f689b31792050ea9e8`.

Because that profile intentionally contained zero observations, a later audit
refused to infer populated-row semantics from the header-only result. A fresh
limited-user run copied the isolated zero-row acceptance profile into a new
profile and inserted one clearly labeled controlled fixture only into the copy.
The source database remained byte-identical at SHA-256
`429fda04c1eb95fe11920116e8807f61ace2e43f09f2c5c08f613bd258d824d2`,
with integrity OK and zero observations before and after preparation. The new
copy passed integrity checking with one 1,234-token observation. This fixture
is native accessibility evidence only; it is not live ingestion evidence.

The unchanged 0.60.22 artifacts then exposed `ControlType.Table` with the name
`Usage observations table. 1 observation. Columns: Provider instance;
Subscription; Model; Tokens; Source; Observed.` The table received keyboard
focus. UI Automation exposed the named header row, all six column headers, one
`ControlType.DataItem` row, and six ordered `ControlType.DataItem` children.
Windows localizes the GPUI `TableCell` nodes as `item`, so the accepted cell
classification uses their populated-row ancestry and column order. Their text
was `Claude Code`, `Acceptance Claude`, `claude-sonnet-4-5`, `1,234`, `managed
turn`, and `2m ago`. The row's complete accessible name matched all six fields.
The focused screenshot was visually inspected for focus, legibility, clipping,
and explicit non-color status and has SHA-256
`4e58d8d8bc41c0030f1f043e557e579f7619e07c23f80f72353c637818b50b82`.
The passing UIA result has SHA-256
`d0933c29810615961a02dca39404f85d914f103c1540969059038e9b1ab0fa0a`.

The final 200 percent matrix verified system, application, and configured DPI
at 192 on a 2560 by 1440 desktop. It repeated keyboard traversal and editor use,
applied six vertical wheel steps, observed 156 pixels of horizontal table
movement, then applied two more vertical steps. Provider actions, login,
organization policy, bottom controls, and Save stayed reachable with no clipped
required action. Seven final screenshots were visually inspected in dark and
light themes. `uia-200pct-matrix.json` passed at SHA-256
`cdc8ed82b0ad9e2147a052217dd58bd86378ae4a3ca5aa4449516aba80cdaee5`.
The retained guest archive has SHA-256
`93a7f101628066a64be07115ca4c3865c3a9e2317ee0c7ff98f9001833c18092`.

### Narrator, High Contrast, and disabled animations

Genuine Windows Narrator ran separately at 200 percent. Speech Recap visibly
contained the Usage status, the complete zero-row table name and all six column
names, `Subscription name`, `Masked account label`, Save, Cancel, and the
returned `Edit subscription for Claude Code` focus. After Check Health it also
contained `Settings status: Provider health checks complete. No interactive
login was started.` and the complete named failure reasons for configured
providers. UI Automation supplied structure and focus assertions, while Speech
Recap supplied the separate screen-reader evidence. The final
`narrator-matrix.json` recorded `mechanicalPassed=true`, stopped the exact
runtime, removed its credential, and has SHA-256
`ff134b1973f526deb18bb1e2c42662cac5069e5765d380589fe761926c1fe439`.
The retained Narrator archive has SHA-256
`bf7b82ee050bb1065f861f304d58e12d9102fe891859803acb38d5f1e19f9229`.

Windows High Contrast Black changed from disabled to enabled for the isolated
run. The explicit Offline text, full alert, Retry boundary, selected navigation,
focus, and status remained visible without relying on a background tint. The
original scheme and comparable flags were restored with no restoration error.
`high-contrast-matrix.json` passed at SHA-256
`93ce020abde0c15ec90982cdffe8791f73db8fb9a82efd34f9d65d733210c5d1`.

The animations-disabled run set Windows UI effects and client-area animation
off. Retry retained focus, Loading remained observable, the repeated Offline
state remained explicit, and no server spawned. The exact original flags were
restored with no restoration error. `animations-disabled-matrix.json` passed at
SHA-256
`de07de56dee7c80a9e79d3779ea0c05f43e8a7080ee0a2052dc26a67991ac487`.
Both alternate-mode screenshots were visually inspected.

### Current normal-user named-pipe security

The 0.60.22 server and CLI passed the normal-user local controls under
`COLT-SHINE\B_StL`: the pipe name contained the exact user SID, legitimate
authenticated ping worked, an unapproved PowerShell client was disconnected
and logged, SYSTEM received Access Denied, a second server instance was
rejected, fake PowerShell server image and mismatched runtime PID were rejected,
and the legitimate post-probe ping worked. A separate anti-squatting repeat
recorded exit code 1 after 1,076 milliseconds while both legitimate pings
succeeded.

The independent `SF-C4AD-VM` peer reached host TCP 445 over the temporary
Default Switch but received Access Denied when opening the host pipe. The host
server remained healthy and answered another legitimate ping before cleanup.
The local matrix intentionally leaves its embedded remote field Unrun and sets
only `passedLocalControls=true`; the separate remote result closes that row.
Their SHA-256 values are:

| Evidence | SHA-256 |
|-|-|
| `named-pipe-matrix.json` | `891339f8211641edc3e713a3957ed9270a6cacd2142d826bc6d3e6df271aa1d9` |
| `named-pipe-antisquat-repeat3.json` | `ca1f5aab26ba9c9d3efc9886de4e79c181c0575c3e76db89219d7966a1ed2150` |
| `named-pipe-remote-rejection.json` | `a254997ea8f64031aa4d8a48c5a621a025c2da3046b0329d9178429b4c108019` |

### Rejected harness attempts and cleanup

Rejected attempts remain in the evidence root. The first guest launch lacked
the app-local MSVC runtime DLL. A deterministic login-stub path was absent in an
old and new binary comparison, so the identical login failure was classified as
a harness issue, not a regression. A 200 percent focus helper used Restore and
shrunk the maximized window; replacing it with Show and rerunning the unchanged
candidate passed. None of these attempts counts toward acceptance.

The first populated-row follow-up harness also remains preserved. It expected
the localized control type `cell`, even though Windows UIA exposes GPUI
`TableCell` nodes as `ControlType.DataItem` with localized type `item` under the
correctly named row. That attempt exposed the complete product row and cleaned
up, but its classifier returned 1. A fresh profile and corrected ancestry-based
classifier returned 0 without changing the candidate. The rejected attempt is
archived inside the populated-row addendum and does not count toward acceptance.

Every temporary host and guest `PandaMUX-P4C3-*` scheduled task was exported and
unregistered. Guest PandaMUX and Narrator process counts were zero, temporary
autologon values were cleared, and the host had no exact-candidate process or
runtime credential. `SF-C4ad-c4f68c3c` was turned off and restored to checkpoint
`pre-rvr-3.271.0.1-4d42541b-20260929`. Independent final verification recorded:

- VM state `Off` and status `Operating normally`;
- one disconnected network adapter with `SwitchName=null`;
- Guest Service Interface disabled;
- video `Default`, 1920 by 1200;
- all eight pre-existing checkpoints intact, including the restored checkpoint;
- zero host `PandaMUX-P4C3-*` tasks and exact-candidate processes; and
- host High Contrast disabled with UI effects enabled.

`lab-state-after-restoration.json` has SHA-256
`0b6db3561b2320441d9c89abd895fe1a0341dc3abc515c8cae6d87190627eee3`.
Both populated-row follow-up tasks were exported and unregistered. Final checks
found zero exact desktop or server processes, no run 2 runtime credential, High
Contrast disabled, and UI effects enabled. The isolated zero-row source and two
seeded profile copies remain retained for audit. The follow-up did not start or
edit the restored Hyper-V guest.
The original checkout and retained Galahad acceptance database were not edited.
No push, CI, signing, packaging, deployment, production activation, or release
occurred.

## Static verification and remaining gates

The source commit passed `cargo fmt --all --check`, strict workspace Clippy,
`cargo test --workspace --quiet` with 412 passed, 0 failed, and two intentionally
ignored real-shell terminal tests, Windows GNU all-target type checking,
Rust crate-boundary and Claude wiring guards, built server and CLI smoke,
and desktop headless smoke. These are separate from the live artifact results.
The 0.60.1 documentation and dependency-policy follow-up also passed workspace
formatting, strict Clippy, and the full 412-test workspace suite with the same
two intentional ignores.

The 0.60.2 and 0.60.3 accessibility repairs each passed local formatting,
strict workspace Clippy, the full workspace suite with 412 passed, 0 failed,
and the same two intentional real-shell ignores, the Rust boundary guard,
Claude wiring guard, dependency policy, and desktop headless startup. The exact
0.60.3 commit additionally passed the native MSVC builds and UIA follow-up above.

Exact 0.60.17 commit `2f56a27` passed formatting, strict workspace and
all-target Clippy, 427 workspace tests with zero failures and two intentional
real-shell ignores, the Rust boundary guard, the Claude wiring guard, and
dependency policy. The five focused failover tests were rerun separately during
P4-C5 and all passed. The exact musl server and CLI hashes above were reverified
after transfer and the server completed authenticated version 0.60.17 RPCs on
Galahad. These checks support, but do not replace, the live rows.

Exact 0.60.22 commit `fee9e02` passed formatting, strict all-target workspace
Clippy, the complete serial workspace test suite with zero failures and two
intentional real-shell ignores, the Rust boundary guard, and the Claude wiring
guard. The clean native build and every P4-C3 row above ran after that local
verification. The documentation-only 0.60.23 follow-up separately passed
formatting, strict all-target workspace Clippy, 429 workspace tests with zero
failures and the same two intentional real-shell ignores, the Rust boundary
guard, and the Claude wiring guard. It makes no 0.60.23 executable, native,
live, production, or release claim.

Exact 0.60.37 commit `f21bc2d` passed formatting, strict all-target workspace
Clippy, 436 workspace tests with zero failures and two intentional real-shell
ignores, the Rust boundary guard, the Claude wiring guard, desktop headless
verification, and diff hygiene before its native build. Exact 0.60.38 passed
the same gate with 437 workspace tests, including the new terminal-mode
persistence and reset coverage, before its rejected exact native run. The
exact 0.60.39 candidate passed formatting, strict all-target and all-feature
workspace Clippy, 437 workspace tests with zero failures and the same two
intentional real-shell ignores, the Rust boundary guard, the Claude wiring
guard, desktop headless verification, and diff hygiene before its rejected
exact native run. Exact 0.60.40 passed that same complete local gate before its
rejected exact native run. The current 0.60.41 source candidate also passed
that gate with 437 tests and the same two intentional real-shell ignores. Local
results do not substitute for the exact native build or Narrator row.

`cargo-deny` 0.20.2 could not parse the old `deny.toml` syntax. The follow-up
configuration change removed deprecated keys and scoped seven existing
non-allowlisted transitive licenses to their exact crates and versions. With
that change, the exact command
`cargo deny check advisories licenses bans sources --hide-inclusion-graph`
returned `advisories ok, bans ok, licenses ok, sources ok`. The current exact
0.60.15 run emitted non-failing yanked-version warnings for transitive
`chacha20 0.10.1` and `yoke-derive 0.8.3`. The
global license allowlist remains narrow and does not admit GPL, AGPL, or LGPL.
A deliberate configuration tradeoff is `unmaintained = "workspace"`: the current
checker errors for unmaintained direct workspace dependencies but does not
report transitive unmaintained crates, replacing the removed general `"warn"`
setting. A release still needs a separate shipped-license and notice review,
especially for the scoped MPL-2.0 exceptions.

Phase 4 cannot yet be marked exited under `tasks/plan-rewrite.md`:

1. Build an exact 0.60.41 or later native Windows candidate and rerun the
   expanded P4-C3 successful-login row: confirmation placement, automatic
   Terminal discovery and focus from Settings, retained output, keyboard input,
   explicit close, panel close and reopen, remote terminal ownership, 100 and
   200 percent scaling, High Contrast, disabled animations, and Narrator.
2. Close the remaining live P4-C5 failover rows with a real pre-output capacity
   failure, real interruptions after text, file edit, command, network action,
   and approval, plus restart while real continuation or delivery work is
   pending. Exact 0.60.17 already passes scanner durability, reconnect and
   bounded backfill, capacity projection and route explanations, rollback
   controls, and the complete diagnostic fixture matrix.
3. Complete the P4-C7 archive and maintained-document synchronization after the
   remaining P4-C5 and P4-C6 live rows stabilize. This record already reflects
   the current P4-C3 Reopened and P4-C5 Partial results.
4. Complete the explicitly deferred R0.5 30-minute physical laptop disconnect
   and sleep checks before production acceptance or release.

The acceptance VM is still a shared lab. Its existing network attachment and
unrelated Hyper-V checkpoints were left untouched during the 0.60.0 matrix. No
installer, updater, signed artifact, deployment, or production activation was
tested.

During the 0.60.3 follow-up, the guest adapter was temporarily attached to the
existing Default Switch only to assess guest availability. Final cleanup on
COLT-SHINE verified the adapter back in its original disconnected state with no
switch, no exact acceptance process, no acceptance scheduled task, and a clean
exact source clone. No checkpoint was created, applied, or changed.
