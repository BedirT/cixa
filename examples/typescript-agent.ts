import { BrokerClient } from "agent-treasury-sdk";

const client = new BrokerClient({
  socketPath: process.env.TREASURY_SOCKET_PATH!,
  tokenFile: process.env.TREASURY_AGENT_TOKEN_FILE!,
});

console.log(await client.getBudget());
console.log(await client.getReceiveInstructions());

