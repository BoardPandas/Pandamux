<!-- PAGE_ID: pandamux_15_glossary -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [README.md:1-274](../README.md#L1-L274)
- [CLAUDE.md:172-215](../CLAUDE.md#L172-L215)
- [crates/pandamux-core/src/lib.rs:1-114](../crates/pandamux-core/src/lib.rs#L1-L114)
- [crates/pandamux-protocol/src/lib.rs:1-96](../crates/pandamux-protocol/src/lib.rs#L1-L96)
- [crates/pandamux-providers/src/lib.rs:1-68](../crates/pandamux-providers/src/lib.rs#L1-L68)
- [crates/pandamux-server/src/lib.rs:1-58](../crates/pandamux-server/src/lib.rs#L1-L58)
- [crates/pandamux-remote/src/lib.rs:1-22](../crates/pandamux-remote/src/lib.rs#L1-L22)
- [crates/pandamux-desktop/src/lib.rs:1-60](../crates/pandamux-desktop/src/lib.rs#L1-L60)

</details>

# Glossary

> **Related Pages**: [Overview](OVERVIEW.md), [Architecture](core/ARCHITECTURE.md)

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_product -->
## Product Terms

These terms describe the user-visible work model and the durable domain records shared across PandaMUX components.

| Term | Definition |
|---|---|
| **PandaMUX** | A native, chat-first desktop for supervising local and remote AI coding agents through a GPUI client and server-owned control plane ([README.md:1-6](../README.md#L1-L6)) |
| **Thread** | The durable conversation and execution context selected in the desktop or CLI. Core exports its identity, status, workspace, origin, provider instance, model, and agent reference through the thread domain API ([lib.rs:103-106](../crates/pandamux-core/src/lib.rs#L103-L106)) |
| **Turn** | One requested provider interaction inside a thread, represented with input, status, outcome, usage, and events ([lib.rs:40-43](../crates/pandamux-core/src/lib.rs#L40-L43), [lib.rs:103-106](../crates/pandamux-core/src/lib.rs#L103-L106)) |
| **Agent** | An AI coding-agent definition or running reference associated with work. Core distinguishes authored definitions and changes from the agent instance attached to a thread ([lib.rs:25-27](../crates/pandamux-core/src/lib.rs#L25-L27), [lib.rs:103-106](../crates/pandamux-core/src/lib.rs#L103-L106)) |
| **Project** | A normalized local or remote work location used for thread grouping, policy assignment, and repository actions ([lib.rs:58-67](../crates/pandamux-core/src/lib.rs#L58-L67)) |
| **Environment** | A local machine or SSH-backed execution target with an explicit status and stable ID ([lib.rs:39-39](../crates/pandamux-core/src/lib.rs#L39)) |
| **Surface** | A desktop-side view or tool area. The implemented right-side surfaces include agents, diffs, confined files, pull requests, terminals, floor-plan data, and artifacts ([README.md:182-193](../README.md#L182-L193)) |

Sources: [README.md:1-6](../README.md#L1-L6), [README.md:182-193](../README.md#L182-L193), [lib.rs:25-67](../crates/pandamux-core/src/lib.rs#L25-L67), [lib.rs:103-106](../crates/pandamux-core/src/lib.rs#L103-L106)
<!-- END:AUTOGEN pandamux_15_glossary_product -->

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_runtime -->
## Runtime Terms

Runtime terms distinguish state ownership from presentation and local execution from remote composition.

| Term | Definition |
|---|---|
| **Hub** | The `pandamux-server` role on the desktop machine. It owns SQLite, provider processes, local PTYs, attachments, event ordering, and environment routing ([CLAUDE.md:197-201](../CLAUDE.md#L197-L201)) |
| **Node daemon** | A `pandamux-server daemon` process on a remote Linux environment. It keeps node work alive independently of the laptop connection ([README.md:258-260](../README.md#L258-L260)) |
| **Desktop projection** | Client-maintained rendering state derived from server events. The desktop submits intents but does not own providers or durable state ([CLAUDE.md:187-195](../CLAUDE.md#L187-L195)) |
| **Terminal grid** | The desktop-owned rendered terminal state. `pandamux-desktop` exports terminal view batching and rendering functions, while the server owns PTYs and byte replay ([lib.rs:49-51](../crates/pandamux-desktop/src/lib.rs#L49-L51), [README.md:190-192](../README.md#L190-L192)) |
| **Runtime record** | Private `server.json` discovery metadata containing endpoint and separate capabilities for clients ([CLAUDE.md:197-207](../CLAUDE.md#L197-L207)) |
| **Environment transport** | The server abstraction used to reach local or remote execution targets; the server crate exports both the router and proxy-backed transport composition ([lib.rs:31-33](../crates/pandamux-server/src/lib.rs#L31-L33), [lib.rs:40-47](../crates/pandamux-server/src/lib.rs#L40-L47)) |
| **Proxy tunnel** | The SSH exec or stream-local bridge used to carry Hub-to-node protocol traffic without opening a remote TCP listener ([lib.rs:14-20](../crates/pandamux-remote/src/lib.rs#L14-L20), [README.md:258-260](../README.md#L258-L260)) |

Sources: [CLAUDE.md:172-207](../CLAUDE.md#L172-L207), [README.md:190-192](../README.md#L190-L192), [README.md:258-260](../README.md#L258-L260), [lib.rs:31-47](../crates/pandamux-server/src/lib.rs#L31-L47), [lib.rs:14-20](../crates/pandamux-remote/src/lib.rs#L14-L20), [lib.rs:49-51](../crates/pandamux-desktop/src/lib.rs#L49-L51)
<!-- END:AUTOGEN pandamux_15_glossary_runtime -->

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_protocol -->
## Protocol and Security Terms

PandaMUX uses one versioned JSON-RPC family for local clients and a separate versioned proxy envelope for remote composition.

| Term | Definition |
|---|---|
| **JSON-RPC** | The request and response envelope used by the local control plane. The protocol crate exports request IDs, requests, responses, standard errors, and the JSON-RPC version ([lib.rs:48-49](../crates/pandamux-protocol/src/lib.rs#L48-L49)) |
| **Protocol version** | The PandaMUX application-level compatibility number exchanged by `system.hello` and reported by identification ([lib.rs:69-72](../crates/pandamux-protocol/src/lib.rs#L69-L72)) |
| **Capability** | A server-issued bearer value presented during the first `system.hello`. Capabilities identify a bounded client scope; request JSON cannot upgrade the server-assigned context ([CLAUDE.md:201-209](../CLAUDE.md#L201-L209)) |
| **Control scope** | The capability for the local desktop and interactive CLI. MCP methods are excluded from this scope ([CLAUDE.md:203-207](../CLAUDE.md#L203-L207)) |
| **MCP scope** | The distinct, read-only adapter scope used by the CLI's MCP stdio mode. It admits hello, ping, and purpose-built MCP tools ([CLAUDE.md:205-215](../CLAUDE.md#L205-L215)) |
| **Request context** | Server-owned peer role, authenticated environment, client identity, and capability scope assigned by the accepting transport ([CLAUDE.md:201-209](../CLAUDE.md#L201-L209), [lib.rs:42-44](../crates/pandamux-server/src/lib.rs#L42-L44)) |
| **Proxy protocol** | The separately versioned request, response, cancellation, and replay envelope used across the SSH bridge ([lib.rs:54-54](../crates/pandamux-protocol/src/lib.rs#L54), [README.md:258-260](../README.md#L258-L260)) |

The security boundary is the operating-system user, not a hostile same-user sandbox. FullAccess providers deliberately inherit host-user authority ([CLAUDE.md:211-215](../CLAUDE.md#L211-L215)).

Sources: [CLAUDE.md:197-215](../CLAUDE.md#L197-L215), [README.md:250-260](../README.md#L250-L260), [lib.rs:48-72](../crates/pandamux-protocol/src/lib.rs#L48-L72), [lib.rs:42-44](../crates/pandamux-server/src/lib.rs#L42-L44)
<!-- END:AUTOGEN pandamux_15_glossary_protocol -->

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_provider -->
## Provider and Routing Terms

Provider terms distinguish a backend implementation from a configured account or runtime and from the route selected for one turn.

| Term | Definition |
|---|---|
| **Provider driver** | The adapter that discovers models, reports health, starts sessions, supervises the provider process, and translates provider events. Drivers exist for Claude, Codex, Antigravity, Cursor, Grok, and OpenCode ([lib.rs:30-55](../crates/pandamux-providers/src/lib.rs#L30-L55), [lib.rs:61-63](../crates/pandamux-providers/src/lib.rs#L61-L63)) |
| **Provider instance** | A stored, explicit provider configuration selected by ID. Threads select only enabled, authenticated instances with driver-backed model discovery ([README.md:184-189](../README.md#L184-L189)) |
| **Provider profile** | An isolated on-disk configuration directory applied to one provider child through provider-specific environment variables ([lib.rs:56-60](../crates/pandamux-providers/src/lib.rs#L56-L60)) |
| **Provider session** | A supervised live interaction with a provider driver, represented by the common `ProviderSession` trait or a provider-specific session implementation ([lib.rs:30-34](../crates/pandamux-providers/src/lib.rs#L30-L34), [lib.rs:61-63](../crates/pandamux-providers/src/lib.rs#L61-L63)) |
| **Route** | The exact provider instance, model, subscription boundary, and attempt selected for a turn. Core exports authentication, candidates, evaluations, decisions, scores, attempts, and continuation records ([lib.rs:77-83](../crates/pandamux-core/src/lib.rs#L77-L83)) |
| **Failover pool** | An explicit set of eligible provider-instance and subscription members with capability requirements and ordering preferences ([lib.rs:53-57](../crates/pandamux-core/src/lib.rs#L53-L57)) |
| **Side-effect boundary** | Durable attempt evidence that determines whether an interrupted provider action may retry, fail, or require a fresh continuation. The routing API exports attempts, tool-ledger entries, and continuation delivery state ([lib.rs:77-83](../crates/pandamux-core/src/lib.rs#L77-L83)) |

Sources: [README.md:184-189](../README.md#L184-L189), [lib.rs:53-57](../crates/pandamux-core/src/lib.rs#L53-L57), [lib.rs:77-83](../crates/pandamux-core/src/lib.rs#L77-L83), [lib.rs:30-63](../crates/pandamux-providers/src/lib.rs#L30-L63)
<!-- END:AUTOGEN pandamux_15_glossary_provider -->

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_usage -->
## Usage and Capacity Terms

Usage terms describe measured provider consumption; capacity terms describe whether a provider or subscription is currently serviceable for routing.

| Term | Definition |
|---|---|
| **Usage observation** | A normalized, durable fact derived from a managed turn or provider transcript. Core exports observation identity, fact kind, source kind, token counts, unknown fields, and validation errors ([lib.rs:107-113](../crates/pandamux-core/src/lib.rs#L107-L113)) |
| **Usage subscription** | The quota and billing boundary associated with provider usage, distinct from the provider instance that executes work ([lib.rs:110-113](../crates/pandamux-core/src/lib.rs#L110-L113)) |
| **Usage cursor** | Durable scanner or synchronization progress used to avoid double-counting and to detect retention gaps. Phase 4 records transactional cursors, outbox state, and per-node high-water marks ([README.md:20-22](../README.md#L20-L22)) |
| **API-equivalent estimate** | A computed micro-USD estimate based on effective-dated integer pricing. It remains separate from provider-reported spend ([README.md:20-22](../README.md#L20-L22), [lib.rs:110-114](../crates/pandamux-core/src/lib.rs#L110-L114)) |
| **Capacity snapshot** | Schema-versioned quota evidence for a provider subscription, carrying confidence, freshness, evidence scope, and reset semantics ([README.md:23-26](../README.md#L23-L26), [lib.rs:107-110](../crates/pandamux-core/src/lib.rs#L107-L110)) |
| **Capacity projection** | The conservative routing view derived from one or more capacity revisions. Unverified provider capacity remains Unknown ([README.md:23-26](../README.md#L23-L26), [lib.rs:107-114](../crates/pandamux-core/src/lib.rs#L107-L114)) |
| **Coverage gap** | Explicit evidence that a scanner or Hub synchronization interval is incomplete, stale, or outside retained history rather than silently treated as zero usage ([README.md:20-26](../README.md#L20-L26)) |

Provider success also fails closed when a started tool lacks a durable result, preserving the ledger rather than inventing success or retrying unsafely ([README.md:35-42](../README.md#L35-L42)).

Sources: [README.md:20-46](../README.md#L20-L46), [lib.rs:107-114](../crates/pandamux-core/src/lib.rs#L107-L114)
<!-- END:AUTOGEN pandamux_15_glossary_usage -->

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_release -->
## Release and Evidence Terms

Release vocabulary is deliberately strict so source completion, automated checks, and production acceptance are never collapsed into one claim.

| Term | Definition |
|---|---|
| **Implementation** | Source changes that encode the requested behavior. Implementation alone does not establish a passing build, deployment, or live behavior ([CLAUDE.md:362-377](../CLAUDE.md#L362-L377)) |
| **Verification** | Directly observed checks against the exact source or artifact, such as formatting, tests, build, smoke, or a native acceptance row ([CLAUDE.md:364-377](../CLAUDE.md#L364-L377)) |
| **Artifact evidence** | Commit, version, size, and hash evidence for a built binary set. The R0 harness records and re-verifies these values but leaves publication disabled ([README.md:262-266](../README.md#L262-L266)) |
| **Commit and push** | Separate Git states: a local commit records work, while a push places that commit on a remote ref. Neither implies deployment or acceptance ([CLAUDE.md:364-377](../CLAUDE.md#L364-L377)) |
| **Deployment** | Placement or activation of an exact artifact in a target environment. It remains distinct from source implementation and from live acceptance ([CLAUDE.md:362-377](../CLAUDE.md#L362-L377)) |
| **Live acceptance** | Execution of the defined matrix against the exact environment and artifact. Claims require the recorded row evidence, not inference from local tests ([CLAUDE.md:364-377](../CLAUDE.md#L364-L377)) |
| **Phase 4 exit matrix** | The current acceptance contract for provider observability and routing. Both physical P4-C6 rows passed; real pre-output capacity exhaustion and the final post-P4-C5 archive remain open ([README.md:169-181](../README.md#L169-L181)) |
| **Released or shipped** | A status that requires the release and acceptance gates, not merely a versioned source tree. The remaining Phase 4 rows prevent that claim for the current recovery baseline ([README.md:175-181](../README.md#L175-L181)) |

At commit `a41c5bd40861fb37de7a53252d96dcb055828362`, this glossary documents implemented contracts and recorded evidence boundaries. It does not claim Phase 4 production acceptance, release, deployment, or shipment.

Sources: [README.md:169-181](../README.md#L169-L181), [README.md:262-266](../README.md#L262-L266), [CLAUDE.md:361-382](../CLAUDE.md#L361-L382)
<!-- END:AUTOGEN pandamux_15_glossary_release -->

---
