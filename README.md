<!-- Final banner: docs/assets/cixa-banner.png -->

<div align="center">

# Cixa

**A local fortress between software agents and real money.**

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-5c6ac4.svg)](LICENSE)
![Core: Rust](https://img.shields.io/badge/core-Rust-b7410e.svg)
![Transport: local Unix socket](https://img.shields.io/badge/transport-local%20Unix%20socket-2f855a.svg)
![Status: reference implementation](https://img.shields.io/badge/status-reference%20implementation-6b7280.svg)

</div>

Cixa is a local payment authorization gateway for software agents. It lets an agent ask for a purchase, then checks that request against limits you control before any payment can move forward.

The agent never gets your account login, raw card details, owner controls, or permission to raise its own budget. Cixa stays on your machine, listens on a Unix socket, and fails closed when something is unclear.

The name comes from Laz. **Cixa** means fortress or castle, and appears in the names of historical fortifications across the Laz region. It is pronounced roughly **JEE-kha**, with the `x` sounding like the `ch` in *Bach* or *loch*.

## Take It For A Spin

You only need Rust stable, Node.js 20+, npm, and Python 3.11+ for the local demo. It uses synthetic data and a simulated provider, so there is no account, API key, paid service, or real transaction involved.

```bash
git clone https://github.com/BedirT/cixa.git
cd cixa
npm ci
./scripts/demo
```

The demo walks through a valid bounded purchase, duplicate protection, over-budget denial, recurring-payment denial, currency substitution, a hostile checkout form, emergency stop, audit verification, and a secret-canary scan.

## A Quick Look

<table>
  <tr>
    <td width="50%">
      <img src="docs/assets/owner-dashboard.png" alt="Cixa owner dashboard showing local controls and provider status">
    </td>
    <td width="50%">
      <img src="docs/assets/checkout-lab.png" alt="Cixa hostile checkout laboratory used for local security tests">
    </td>
  </tr>
  <tr>
    <td align="center"><sub>Owner-only dashboard on loopback</sub></td>
    <td align="center"><sub>Hostile local checkout fixture</sub></td>
  </tr>
</table>

Both screens are local test surfaces. The dashboard loads no CDN assets, analytics, or third-party scripts, and the checkout lab never accepts real payment details.

## What Cixa Actually Does

- Checks amounts using integer minor units and explicit ISO 4217 currencies.
- Keeps owner, agent, broker, and payment-provider permissions separate.
- Supports observe-only, approval-required, bounded-autonomous, and disabled modes.
- Enforces budgets, merchant rules, currency rules, fulfillment allowlists, redirect checks, and recurring-payment denial.
- Gives agents scoped, expiring, revocable capability tokens. Stored state keeps only token hashes.
- Records an append-only ledger, an HMAC hash-chain audit log, and an authenticated state envelope.
- Treats uncertain payment outcomes as `unknown` or `reconciliation_required` instead of retrying and hoping for the best.
- Ships with a Rust CLI and daemon, MCP server, TypeScript SDK, dependency-free Python SDK, local owner dashboard, and simulated provider.

## How It Fits Together

```mermaid
flowchart LR
  A["Untrusted agent"] -->|"scoped capability"| M["MCP or SDK"]
  M -->|"local v1 JSON IPC"| C["Cixa broker"]
  O["Owner CLI or dashboard"] -->|"owner-only channel"| C
  C --> P["Policy and budget engine"]
  C --> L["Ledger and audit chain"]
  C --> S["Secret provider boundary"]
  C --> X["Simulated or manual provider"]
```

The agent-facing socket and owner-control socket are separate. Agent integrations get `cixa.sock` and a scoped token. They do not get `owner.sock`, the data directory, provider credentials, or the browser handoff.

For the deeper version, see [the architecture guide](docs/architecture.md) and [threat model](THREAT_MODEL.md).

## Run A Persisted Local Broker

Build the workspace, create a local data directory, and add a bounded agent:

```bash
cargo build --locked
mkdir -p .local

target/debug/cixa init \
  --data-dir .local \
  --owner-token-file .local/owner.token

target/debug/cixa create-agent \
  --data-dir .local \
  --owner-token-file .local/owner.token \
  --agent-token-file .local/agent.token \
  --mode bounded_autonomous

target/debug/cixa serve --data-dir .local
```

That same-user setup is for simulated local development. Manual-provider mode requires the separate Unix identities and group-isolated socket layout in [the deployment guide](docs/deployment.md).

### Connect An MCP Client

Build the packages, then run the MCP adapter inside the untrusted agent process:

```bash
npm run build

CIXA_SOCKET_PATH="$PWD/.local/cixa.sock" \
CIXA_AGENT_TOKEN_FILE="$PWD/.local/agent.token" \
node packages/mcp-server/dist/index.js
```

The MCP server exposes only the agent-safe surface:

| Tool | Purpose |
| --- | --- |
| `cixa_get_status` | Read sanitized broker status |
| `cixa_get_capabilities` | Read the agent's fixed capabilities |
| `cixa_get_budget` | Read the effective budget and provider labels |
| `cixa_get_receive_instructions` | Read owner-approved public receiving details |
| `cixa_create_purchase_intent` | Ask Cixa to validate a proposed purchase |
| `cixa_get_purchase_intent` | Read one sanitized intent |
| `cixa_execute_purchase_intent` | Execute an already authorized intent |
| `cixa_cancel_purchase_intent` | Cancel an unexecuted intent |
| `cixa_list_transactions` | List the agent's sanitized transactions |
| `cixa_get_receipt` | Read a receipt with personal details removed |

Policy edits, agent creation and revocation, approvals, deposits, reconciliation, provider setup, audit export, and emergency stop stay on the owner side.

### Use The SDKs

TypeScript:

```ts
import { BrokerClient } from "cixa-sdk";

const cixa = new BrokerClient({
  socketPath: process.env.CIXA_SOCKET_PATH!,
  tokenFile: process.env.CIXA_AGENT_TOKEN_FILE!,
});

console.log(await cixa.getBudget());
```

Python:

```python
from cixa import CixaClient

cixa = CixaClient(".local/cixa.sock", ".local/agent.token")
print(cixa.get_budget())
```

More complete examples live in [examples](examples) and [the agent integration guide](docs/agent-integration.md).

## The Serious Bit

Cixa is **not a bank**, wallet, issuer, payment processor, money transmitter, custodial service, KOHO API, KOHO partner, Mastercard product, Interac product, or universal checkout system. It does not claim PCI DSS compliance, formal certification, or compatibility with every merchant.

The first real-world reference path is deliberately **manual-only**: a user-owned Canadian KOHO prepaid virtual Mastercard plus an owner-configured Interac e-Transfer receiving address. Cixa does not automate a KOHO login, use a private KOHO API, send outgoing Interac e-Transfers, or run real-money tests.

A few rules are worth repeating:

- Treat the agent and every merchant page as hostile input.
- Never put real card details, login credentials, owner tokens, or agent tokens in source, prompts, environment variables, logs, screenshots, traces, or MCP output.
- A timeout after submission is ambiguous. Cixa quarantines it for owner reconciliation and does not retry automatically.
- The default secret-provider path never persists CVV.
- A local administrator, compromised kernel, browser runtime, issuer, merchant, or owner sits outside some guarantees.
- A green test run is useful evidence about this checkout, not a security warranty.

Read [SECURITY.md](SECURITY.md), [the credential guide](docs/credential-handling.md), and [the limitations](docs/limitations.md) before connecting anything beyond the simulator.

## Manual Provider Reference

The manual adapter stores a reference to an owner-controlled credential entry, not the card itself:

```bash
target/debug/cixa configure-manual-provider \
  --data-dir .local \
  --owner-token-file .local/owner.token \
  --credential-reference keychain://cixa/card \
  --provider-kind os-credential-store \
  --last-four 1111 \
  --balance-minor 5000 \
  --balance-status owner_confirmed
```

Real-world completion is two-phase and owner-driven. Cixa records `executing` before handoff, records uncertain completion as non-retryable, and requires the owner to reconcile the result. The details are in [the checkout adapter guide](docs/checkout-adapters.md) and [KOHO reference setup](docs/koho-setup.md).

## Verify The Whole Project

The full local gate also expects `pip-audit` 2.9.0, `cargo-audit` 0.22.2, and `gitleaks` 8.30.1. Install the pinned Python build graph from `requirements-build.lock`, then run:

```bash
./scripts/verify
```

That gate covers formatting, Rust build and Clippy, Rust and SDK tests, fuzz-harness compilation, installed-package smoke tests, persisted daemon integration, owner dashboard integration, adversarial scenarios, docs, dependency licenses, SBOM output, vulnerability scans, and secret-canary checks.

## Project Status

Cixa is an early security-focused reference implementation. The simulated path is complete and heavily tested. The manual path is intentionally cautious. There is no public network listener by default, no automated real provider, and no real transaction made by this repository or its test harness.

Before the first tagged release, persisted data from the old pre-release project name should be discarded and initialized again. Capability and reference domain separators changed as part of the rename.

## Contributing

Small, reviewable changes are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), and please read the nearest ADR before changing a trust boundary. Security reports belong in the private process described in [SECURITY.md](SECURITY.md), not in a public issue with sensitive details.

## License

Apache-2.0. See [LICENSE](LICENSE).
