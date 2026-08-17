import { BrokerClient } from "cixa-sdk";

const client = new BrokerClient({
  socketPath: process.env.CIXA_SOCKET_PATH!,
  tokenFile: process.env.CIXA_AGENT_TOKEN_FILE!,
});

console.log(await client.getBudget());
console.log(await client.getReceiveInstructions());

