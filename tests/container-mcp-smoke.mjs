import assert from "node:assert/strict";
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const root = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const project = process.env.CIXA_DOCKER_PROJECT;
assert.ok(project, "CIXA_DOCKER_PROJECT is required");

const transport = new StdioClientTransport({
  command: "docker",
  args: [
    "compose", "--project-directory", root, "--project-name", project,
    "run", "--rm", "--no-deps", "-T",
    "-e", "CIXA_AGENT_TOKEN_FILE=/run/cixa-agent/tokens/container-test.token",
    "cixa-mcp",
  ],
});
const client = new Client({ name: "cixa-container-smoke", version: "0.1.0" });

try {
  await client.connect(transport);
  const tools = await client.listTools();
  const names = tools.tools.map((tool) => tool.name);
  assert.equal(names.includes("cixa_get_status"), true);
  assert.equal(names.includes("cixa_get_budget"), true);
  assert.equal(names.some((name) => name.startsWith("owner_")), false);
  const status = await client.callTool({ name: "cixa_get_status", arguments: {} });
  const statusPayload = JSON.parse(status.content[0].text);
  assert.equal(statusPayload.principal, "agent");
  const capabilities = await client.callTool({ name: "cixa_get_capabilities", arguments: {} });
  const capabilityPayload = JSON.parse(capabilities.content[0].text);
  assert.equal(capabilityPayload.cannot.includes("change_policies"), true);
  assert.equal(capabilityPayload.cannot.includes("add_cards"), true);
  assert.equal(capabilityPayload.cannot.includes("disable_safeguards"), true);
} finally {
  await client.close();
}
