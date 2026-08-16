# ADR 0001: Rust Core and Local JSON IPC

## Status

Accepted.

## Decision

Keep money arithmetic, policy evaluation, authorization, transaction state, ledger, audit chain, and simulated/manual provider boundaries in a small Rust crate. Use a Rust daemon with a versioned JSON-line protocol over a private Unix-domain socket. Adapt that protocol to MCP, TypeScript, Python, and the CLI.

## Rationale

The security decisions are deterministic and benefit from a small typed core. A process boundary prevents the agent-facing JavaScript adapter from owning policy or secrets. JSON is a deliberately narrow interoperability format, not the source of truth for money arithmetic.

## Consequences

The default daemon is Unix-specific. Windows deployments need a named-pipe adapter before production use. The local API is easy to inspect and test, but privileged local attackers remain out of scope.

