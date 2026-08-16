# Agent Integration

## Start the Broker

```bash
target/debug/treasury init --data-dir .local --owner-token-file .local/owner.token
target/debug/treasury create-agent --data-dir .local --owner-token-file .local/owner.token \
  --agent-token-file .local/agent.token --mode bounded_autonomous
target/debug/treasury serve --data-dir .local
```

The daemon creates a private Unix-domain socket at `.local/treasury.sock` by default. It does not bind a TCP port. An agent container should receive only the socket endpoint and a scoped token file or brokered IPC handle, not the data directory, owner UI session, browser debugging port, secret helper, raw audit files, or provider credentials.

## MCP

Build the workspace and start the server as a child process of an MCP host:

```bash
npm ci
npm run build
TREASURY_SOCKET_PATH="$PWD/.local/treasury.sock" \
TREASURY_AGENT_TOKEN_FILE="$PWD/.local/agent.token" \
node packages/mcp-server/dist/index.js
```

The MCP server uses the maintained `@modelcontextprotocol/server` v2 SDK and stdio. stdout is reserved for the protocol; diagnostics go to stderr. Zod schemas are strict and bounded. Owner operations are not registered.

## TypeScript SDK

```ts
import { BrokerClient } from "agent-treasury-sdk";

const treasury = new BrokerClient({
  socketPath: process.env.TREASURY_SOCKET_PATH!,
  tokenFile: process.env.TREASURY_AGENT_TOKEN_FILE!,
});
const budget = await treasury.getBudget();
```

The SDK reads the capability token from a protected file. It does not accept a raw token in a method argument and does not provide owner operations.

## Python SDK

```python
from agent_treasury import TreasuryClient

treasury = TreasuryClient(".local/treasury.sock", ".local/agent.token")
print(treasury.get_budget())
```

Use `PYTHONPATH=packages/sdk-python` for the repository example without installing it. The Python client has no runtime dependency beyond the standard library.

## Purchase Intent

An intent includes a caller idempotency key, integer minor-unit amount, final total, merchant domain, category, fulfillment profile, explicit recurring and stored-card flags, payment form trust tier, redirect chain, attempt count, session ID, and simulated scenario. The broker rejects unknown or malformed fields at the MCP boundary and revalidates all material properties before submission.

## Safe Failure

`approval_required`, `declined`, `failed`, `provider_pending`, `unknown`, and `reconciliation_required` are actionable states, not invitations to retry. In particular, the agent cannot execute an approved-by-model or “owner approved” string; only a real owner-authenticated approval operation changes an intent to `approved`.

Verified income is owner-only. Use the CLI with `--verified true` and, when reinvestment is intended, an explicit `--agent-id`; the policy's integer basis-point ratio, maximum treasury size, and absolute exposure ceiling are applied before any new authority is created. Unverified notifications are stored for reconciliation but remain non-spendable.
