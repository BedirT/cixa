# Architecture

## Principals and Trust Boundaries

```mermaid
flowchart TB
  subgraph Untrusted[Untrusted boundary]
    Agent[Agent process]
    Merchant[Merchant page and content]
    ProviderOutput[Provider and notification output]
  end
  subgraph Local[Owner device]
    Adapter[MCP / SDK / CLI adapter]
    Broker[Rust broker]
    Owner[Owner CLI and dashboard]
    Secrets[Owner-controlled secret provider]
    Data[Private state, audit log, audit key]
  end
  Agent --> Adapter
  Merchant --> Agent
  ProviderOutput --> Agent
  Adapter -->|agent token over local IPC| Broker
  Owner -->|owner token| Broker
  Broker --> Data
  Broker --> Secrets
```

The agent, merchant, notifications, screenshots, and model traces are never trusted for security decisions. The broker revalidates amount, currency, origin, fulfillment, and policy immediately before the simulated provider call.

## Process Architecture

```mermaid
flowchart LR
  MCP[MCP stdio process] -->|JSON-RPC tool call| SDK[SDK client]
  SDK -->|v1 newline JSON| AgentSocket[Bounded agent socket 0600]
  Owner[Owner CLI and dashboard] -->|owner token| OwnerSocket[Independent owner socket 0600]
  AgentSocket --> Daemon[treasury daemon]
  OwnerSocket --> Daemon
  Daemon --> Mutex[Single process state lock]
  Mutex --> State[Atomic authenticated state envelope]
  Mutex --> Audit[Audit entries + separate audit.key]
  Daemon --> Provider[Simulated provider]
```

The daemon owns an exclusive data-directory lock for its lifetime and serializes requests in one process. CLI mutations route through the independently admitted owner socket or acquire the same lock when the daemon is offline. The agent socket rejects owner operations, and the owner socket rejects non-owner credentials. State and audit entries are covered by an HMAC-authenticated envelope; writes use a random private temporary file, `sync_all`, atomic rename, and parent-directory synchronization. Both sockets are local by construction and are not public TCP listeners.

Before a provider call, the broker persists `funds_reserved` and `executing`. The verified `final_total`, rather than the earlier requested estimate, is the authoritative value for policy limits, reservations, provider submission, reconciliation, ledger events, and receipts. If the process exits before the final outcome is durable, restart recovery changes `executing` to `unknown` and forbids automatic resubmission. The intent ID is the provider idempotency key in the simulated adapter.

## Purchase State Machine

```mermaid
stateDiagram-v2
  [*] --> draft
  draft --> proposed
  proposed --> policy_validated
  proposed --> approval_required
  proposed --> failed
  proposed --> cancelled
  approval_required --> approved
  approval_required --> cancelled
  policy_validated --> funds_reserved
  approved --> funds_reserved
  funds_reserved --> executing
  executing --> provider_pending
  executing --> settled
  executing --> declined
  executing --> failed
  executing --> unknown
  provider_pending --> settled
  provider_pending --> declined
  provider_pending --> unknown
  unknown --> reconciliation_required
  reconciliation_required --> settled
  reconciliation_required --> declined
  settled --> refunded
```

There is no transition from `unknown` back to `executing`. A timeout after submission is an owner reconciliation task, not a retry opportunity.

## Secure Checkout Critical Section

```mermaid
sequenceDiagram
  participant A as Agent
  participant B as Broker
  participant E as Executor
  participant S as SecretProvider
  participant P as Provider
  A->>B: create intent
  B->>B: deterministic policy and schema validation
  A->>B: execute intent
  B->>E: suspend agent control and validate origin/total
  B->>B: reserve funds and revalidate policy
  B->>S: owner-controlled just-in-time secret request
  S-->>E: volatile secret, not agent-readable
  E->>P: submit exactly once
  P-->>E: approved, pending, declined, or ambiguous
  E-->>B: sanitized outcome
  B->>S: clear transaction secret
  B->>B: append ledger and audit; destroy/sanitize context
  B-->>A: sanitized receipt or reconciliation state
```

The default simulator exercises the state shape with synthetic facts and no money. The manual provider always requires owner approval and never trusts agent claims. The default real-card workflow is a two-phase owner-manual handoff. For owner-reviewed hosted-field integrations, the experimental Playwright adapter independently observes configured checkout facts, excludes agent RPC with the broker lock, leaves capture channels disabled, and destroys its fresh context. Unknown forms or observations return an explicit unsupported or ambiguous result.

## Provider Abstraction

```mermaid
classDiagram
  class PaymentProvider {
    <<interface>>
    provider_id()
    available_balance()
    authorize(intent)
  }
  class SimulatedProvider
  class ManualPrepaidCardProvider
  PaymentProvider <|.. SimulatedProvider
  PaymentProvider <|.. ManualPrepaidCardProvider
```

The manual adapter is selectable through the owner CLI. It stores a non-secret credential reference and a freshness-limited balance snapshot, not a login or private issuer session. Estimated or expired snapshots cannot authorize spending. It returns an ambiguous/manual outcome for submission so the owner controls the real checkout and reconciliation.

## Owner Versus Agent Interfaces

| Capability | Agent | Owner |
| --- | --- | --- |
| Read effective budget | Yes | Yes |
| Read public receiving instructions | Yes | Yes |
| Create and execute a bounded intent | Yes, policy-bound | Yes, through owner workflow |
| Cancel own unexecuted intent | Yes | Yes |
| Create or revoke agents | No | Yes |
| Change policy or limits | No | Yes |
| Approve exception | No | Yes |
| Record or verify income | No | Yes |
| Reconcile unknown transaction | No | Yes |
| Read credentials or audit key | No | No normal agent or dashboard path |
| Emergency stop | No | Yes |
