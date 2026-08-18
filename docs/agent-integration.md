# Agent Integration

## Start Cixa

```bash
./scripts/cixa-docker up
./scripts/cixa-docker dashboard-token
```

Open the loopback owner console, create an agent, and choose a capability filename. The console writes the secret directly into the agent IPC volume. The browser never receives or displays it.

The Compose deployment creates a bounded agent Unix-domain socket in `cixa-agent-ipc` and an independently admitted owner socket in `cixa-owner-data`. It does not bind a broker TCP port. The agent-side MCP container mounts only the first volume, read-only. It never receives the owner socket, data directory, owner UI session, browser debugging port, secret helper, raw audit files, or provider credentials.

## MCP

Print the containerized MCP configuration for the token filename you created:

```bash
./scripts/cixa-docker agent-config research-runner.token
```

The generated entry starts `cixa-mcp` through Compose as a disposable UID `10001` container. It has no network, no owner volume, a read-only root filesystem, and a read-only agent IPC mount. The MCP server uses the maintained `@modelcontextprotocol/server` v2 SDK and stdio. stdout is reserved for the protocol; diagnostics go to stderr. Zod schemas are strict and bounded. Owner operations are not registered.

For a custom containerized agent, reproduce the `cixa-mcp` service boundary in `compose.yaml` rather than copying capability values into environment variables. [Docker deployment](docker.md) documents the volume and identity contract.

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

Skills are guidance, not authority. The scoped file and broker policy grant the actual capability. Use `scripts/cixa-docker agent-config` for the default deployment. [`examples/mcp-agent-config.json`](../examples/mcp-agent-config.json) is intentionally the advanced native-socket example.

## Safe Failure

`approval_required`, `declined`, `failed`, `provider_pending`, `unknown`, and `reconciliation_required` are actionable states, not invitations to retry. In particular, the agent cannot execute an approved-by-model or “owner approved” string; only a real owner-authenticated approval operation changes an intent to `approved`. A timeout from the execute tool is ambiguous: read the intent state, but never call execute again.

Verified income is owner-only. Use the CLI with `--verified true` and, when reinvestment is intended, an explicit `--agent-id`; the policy's integer basis-point ratio, maximum treasury size, and absolute exposure ceiling are applied before any new authority is created. Unverified notifications are stored for reconciliation but remain non-spendable.
