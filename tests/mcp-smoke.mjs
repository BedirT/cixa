import assert from "node:assert/strict";
import { access, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const exec = promisify(execFile);
const root = new URL("..", import.meta.url).pathname;
const binary = join(root, "target", "debug", "cixa");
const directory = await mkdtemp(join(tmpdir(), "cixa-mcp-"));
const ownerFile = join(directory, "owner.token");
const agentFile = join(directory, "agent.token");
await exec(binary, ["init", "--data-dir", directory, "--owner-token-file", ownerFile]);
await exec(binary, ["create-agent", "--data-dir", directory, "--owner-token-file", ownerFile, "--agent-token-file", agentFile, "--mode", "bounded_autonomous"]);
const socketPath = join(directory, "cixa.sock");
const daemon = spawn(binary, ["serve", "--data-dir", directory, "--socket", socketPath], { cwd: root, stdio: "ignore" });
try {
  for (let attempt = 0; attempt < 100 && !(await exists(socketPath)); attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.equal(await exists(socketPath), true);
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [join(root, "packages/mcp-server/dist/index.js")],
    env: {
      ...process.env,
      CIXA_SOCKET_PATH: socketPath,
      CIXA_AGENT_TOKEN_FILE: agentFile,
    },
  });
  const client = new Client({ name: "smoke-agent", version: "0.1.0" });
  await client.connect(transport);
  const listed = await client.listTools();
  const names = listed.tools.map((tool) => tool.name);
  assert.equal(names.includes("cixa_get_status"), true);
  assert.equal(names.includes("cixa_get_budget"), true);
  assert.equal(names.includes("owner_set_emergency_stop"), false);
  const status = await client.callTool({ name: "cixa_get_status", arguments: {} });
  const payload = JSON.parse(status.content[0].text);
  assert.equal(payload.principal, "agent");
  const transactions = await client.callTool({
    name: "cixa_list_transactions",
    arguments: { cursor: null, limit: 25 },
  });
  const transactionPage = JSON.parse(transactions.content[0].text);
  assert.equal(transactionPage.transactions.length, 0);
  assert.equal(transactionPage.has_more, false);
  await client.close();
  console.log("MCP smoke assertions passed");
} finally {
  daemon.kill("SIGTERM");
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
