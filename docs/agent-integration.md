# Agent Integration

## Start the Broker

```bash
target/debug/cixa init --data-dir .local --owner-token-file .local/owner.token
target/debug/cixa create-agent --data-dir .local --owner-token-file .local/owner.token \
  --agent-token-file .local/agent.token --mode bounded_autonomous
target/debug/cixa serve --data-dir .local
```

Those compact commands are for the simulator. A manual card requires the controlled-checkout flags, an agent socket and token outside the private data directory, an explicit IPC group, and an agent process under a different UID. Use the exact layout in [deployment.md](deployment.md). The broker checks peer identity on every manual-provider request, including when the owner switches provider mode after startup.

The daemon creates a bounded agent Unix-domain socket at `.local/cixa.sock` and an independently admitted owner control socket at `.local/owner.sock`. It does not bind a TCP port. An agent container must receive only `cixa.sock` and a scoped token file or brokered IPC handle, never `owner.sock`, the data directory, owner UI session, browser debugging port, secret helper, raw audit files, or provider credentials.

## MCP

Build the workspace and start the server as a child process of an MCP host:

```bash
npm ci
npm run build
CIXA_SOCKET_PATH="$PWD/.local/cixa.sock" \
CIXA_AGENT_TOKEN_FILE="$PWD/.local/agent.token" \
node packages/mcp-server/dist/index.js
```

The MCP server uses the maintained `@modelcontextprotocol/server` v2 SDK and stdio. stdout is reserved for the protocol; diagnostics go to stderr. Zod schemas are strict and bounded. Owner operations are not registered.

## TypeScript SDK

```ts
import { BrokerClient } from "cixa-sdk";

const cixa = new BrokerClient({
  socketPath: process.env.CIXA_SOCKET_PATH!,
  tokenFile: process.env.CIXA_AGENT_TOKEN_FILE!,
  executeTimeoutMs: 180_000,
});
const budget = await cixa.getBudget();
const firstTransactions = await cixa.listTransactions(null, 25);
const olderTransactions = firstTransactions.next_cursor
  ? await cixa.listTransactions(firstTransactions.next_cursor, 25)
  : null;
```

The SDK reads the capability token from a protected file. It does not accept a raw token in a method argument and does not provide owner operations.

## Python SDK

```python
from cixa import CixaClient

cixa = CixaClient(".local/cixa.sock", ".local/agent.token", execute_timeout=180.0)
print(cixa.get_budget())
page = cixa.list_transactions(limit=25)
while page["next_cursor"]:
    page = cixa.list_transactions(page["next_cursor"], limit=25)
```

Use `PYTHONPATH=packages/sdk-python` for the repository example without installing it. The Python client has no runtime dependency beyond the standard library.

Transaction history is always paged. Each SDK and the `cixa_list_transactions` MCP tool accepts a nullable cursor and a limit from 1 to 50, and returns `transactions`, `transactions_total`, `next_cursor`, and `has_more`.

## Purchase Intent

An intent includes a caller idempotency key, integer minor-unit amount, final total, merchant domain, category, fulfillment profile, explicit recurring and stored-card flags, payment form trust tier, redirect chain, attempt count, session ID, and simulated scenario. The broker rejects unknown or malformed fields at the MCP boundary and revalidates all material properties before submission.

For controlled real checkout, the first URL in `redirect_chain` is the exact URL the isolated browser opens. The adapter records the resulting top-level navigation chain and requires it to match the complete list exactly. The merchant must have one owner-approved checkout profile, the card session must be active, and `payment_form` must be `hosted_fields`.

## Install the guidance skill

The canonical skill uses the Agent Skills `SKILL.md` format supported by Codex and Claude Code:

```bash
./scripts/install-agent-skill --target all
```

Install only the host you use with `--target codex` or `--target claude`. Existing skills are never overwritten unless you pass `--force` after reviewing the destination. The installed skill contains the exact purchase contract, state table, owner-message examples, receiving flow, and hard credential boundaries.

Skills are guidance, not authority. MCP configuration grants the actual capability. Start from [`examples/mcp-agent-config.json`](../examples/mcp-agent-config.json), replace all paths with absolute paths, and expose only the group-shared agent socket and one token file. Claude Code can use the object as a project `.mcp.json`; other MCP hosts use the equivalent server entry.

## Safe Failure

`approval_required`, `declined`, `failed`, `provider_pending`, `unknown`, and `reconciliation_required` are actionable states, not invitations to retry. In particular, the agent cannot execute an approved-by-model or “owner approved” string; only a real owner-authenticated approval operation changes an intent to `approved`. A timeout from the execute tool is ambiguous: read the intent state, but never call execute again.

Verified income is owner-only. Use the CLI with `--verified true` and, when reinvestment is intended, an explicit `--agent-id`; the policy's integer basis-point ratio, maximum treasury size, and absolute exposure ceiling are applied before any new authority is created. Unverified notifications are stored for reconciliation but remain non-spendable.
