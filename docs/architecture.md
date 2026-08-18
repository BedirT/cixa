# Architecture

## System Boundary

![Cixa Docker-first architecture](assets/cixa-architecture.svg)

Docker Compose is the reference deployment. The same invariants apply to a native installation, but Docker supplies the identities, mounts, read-only filesystems, process limits, and browser runtime consistently.

The agent, merchant page, provider output, notifications, screenshots, and model traces are untrusted. The broker, owner console, authenticated state, owner-created merchant profiles, short-lived card helper, and reviewed checkout adapter belong to the owner boundary. KOHO remains outside Cixa and is authoritative only when the owner checks it through the official app.

## Container And Process Model

| Process | UID | Network | Writable data | Responsibility |
| --- | ---: | --- | --- | --- |
| `cixa-init` | `0` | None | Both fresh named volumes | One-shot ownership and initialization. It exits before normal operation. |
| `cixa-broker` | `10000` | Merchant egress | Owner data and agent IPC | Authentication, policy, ledger, audit, provider execution, helper grants, and checkout browser lifecycle. |
| `cixa-console` | `10000` | Loopback HTTP | Owner data and agent token directory | Owner UI, agent provisioning, provider setup, payment-session lifecycle, profiles, approvals, and reconciliation. |
| `cixa-mcp` | `10001` | None | None | Stateless stdio translation from agent-safe MCP tools to the local agent socket. |

The owner services share one trusted UID but not one process. The agent bridge uses a distinct UID and only supplemental IPC GID `12000`. Manual-provider requests also validate the Unix peer UID, so mounting the socket without the expected identity is insufficient.

## Storage Model

`cixa-owner-data` is mounted only into trusted owner services. It contains:

- the HMAC-authenticated state envelope and separate audit key;
- owner and dashboard credentials;
- the owner-only Unix socket;
- checkout profiles and helper key material;
- durable helper-redemption records.

`cixa-agent-ipc` is the only volume visible to an agent-side process. It contains:

- `cixa.sock`, owned by the owner UID and IPC group with mode `0660`;
- scoped agent capability files with mode `0640`;
- no owner token, owner socket, state, policy file, audit key, browser, profile, or payment material.

Capability values are not Compose environment secrets. The generated MCP configuration contains only a token filename inside the read-only agent volume.

## Request Path

1. The software agent calls a `cixa_*` MCP tool.
2. The network-disabled MCP container reads its scoped token file and sends one bounded v1 request through `cixa.sock`.
3. The broker checks peer identity, capability hash, scope, expiry, revocation, rate limit, and intent ownership.
4. Typed domain code evaluates the exact final total, currency, merchant, items, redirect chain, fulfillment, consent facts, authority mode, active session, provider balance freshness, and all budgets.
5. The broker returns a denial, an owner-approval state, or a policy-validated intent. The model cannot promote its own state.

Owner requests use a different socket, credential, connection pool, and operation set. Agent RPC rejects owner operations even if a request attempts to name one.

## Controlled Checkout Critical Section

1. The agent requests execution once for a policy-validated or owner-approved intent.
2. The broker loads the unique owner profile for the canonical merchant and validates every executable and configuration path.
3. The broker creates an intent-bound, helper-bound, broker-UID-bound signed grant.
4. The checkout executor independently validates the approved request before card retrieval.
5. The broker reserves authority, moves the intent to `executing`, appends audit evidence, and durably synchronizes state before browser submission.
6. The helper redeems the grant once and sends the volatile card object directly to the short-lived adapter process.
7. A fresh Playwright process and context enforce public network destinations, exact navigation origins, approved processor frames, visible checkout facts, and one submit.
8. The process is destroyed and the broker records the result as ambiguous until owner reconciliation.

The agent never receives Playwright, CDP, DOM, browser profiles, screenshots, traces, console output, network bodies, clipboard contents, or payment values from this process.

## Purchase State Model

| State | Permitted next step |
| --- | --- |
| `draft` / `proposed` | Broker validation only. |
| `policy_validated` | One execution request. |
| `approval_required` | Owner approve or deny. |
| `approved` | One execution request. |
| `funds_reserved` / `executing` | Provider critical section only. |
| `provider_pending` | Provider outcome or owner reconciliation. |
| `unknown` | `reconciliation_required` only. Never execute again. |
| `reconciliation_required` | Owner records settled, declined, or refunded evidence. |
| `settled`, `declined`, `failed`, `cancelled`, `refunded` | Terminal according to the domain transition table. |

Before any external side effect, the verified `final_total` is used for policy limits, reservations, submission, reconciliation, ledger events, and receipts. Restart recovery changes interrupted `executing` work to `unknown`; there is no transition from `unknown` back to execution.

## Provider And Receiving Decisions

The simulator is deterministic and never contacts a merchant. The manual prepaid-card provider stores a non-secret reference, masked last four, controlled-checkout flag, and freshness-limited owner balance snapshot. It does not automate KOHO login or use a private API.

A merchant page is not authenticated issuer evidence. Even a visible success message produces an unknown result until the owner checks KOHO and reconciles the intent. Receiving follows the same evidence rule: an agent may share public instructions, but only owner-verified arrival evidence can become spending authority.

## Owner Versus Agent Interfaces

| Capability | Agent | Owner |
| --- | --- | --- |
| Read effective budget and status | Yes | Yes |
| Read public receiving instructions | Yes | Yes |
| Create and execute a bounded intent | Yes, policy-bound | Through owner workflows |
| Cancel own unexecuted intent | Yes | Yes |
| Create, rotate, or revoke agents | No | Yes |
| Change policy, limits, or authority | No | Yes |
| Approve an exception or merchant | No | Yes |
| Configure card session or profiles | No | Yes |
| Record or verify income | No | Yes |
| Reconcile an unknown transaction | No | Yes |
| Read credentials or audit key | No | No normal UI or agent path |
| Emergency stop | No | Yes |

The detailed rationale is recorded in [`docs/adr/`](adr), with operational boundaries in [Docker deployment](docker.md), [Credential handling](credential-handling.md), and the [Threat model](../THREAT_MODEL.md).
