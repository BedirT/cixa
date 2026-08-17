import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const [serverPath, socketPath, tokenFile] = process.argv.slice(2);
if (!serverPath || !socketPath || !tokenFile) throw new Error("demo MCP arguments are required");

const transport = new StdioClientTransport({
  command: process.execPath,
  args: [serverPath],
  env: {
    ...process.env,
    CIXA_SOCKET_PATH: socketPath,
    CIXA_AGENT_TOKEN_FILE: tokenFile,
  },
});
const client = new Client({ name: "system-demo", version: "0.1.0" });
await client.connect(transport);
const statusResult = await client.callTool({ name: "cixa_get_status", arguments: {} });
const budgetResult = await client.callTool({ name: "cixa_get_budget", arguments: {} });
await client.close();
process.stdout.write(JSON.stringify({
  status: JSON.parse(statusResult.content[0].text),
  budget: JSON.parse(budgetResult.content[0].text),
}));
