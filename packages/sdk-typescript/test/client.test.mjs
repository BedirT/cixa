import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:net";
import { BrokerClient } from "../dist/index.js";

test("TypeScript client uses the v1 line protocol and token file", async () => {
  const directory = await mkdtemp(join(tmpdir(), "agent-treasury-sdk-"));
  const tokenFile = join(directory, "agent.token");
  const socketPath = join(directory, "broker.sock");
  await writeFile(tokenFile, "synthetic-token\n", { mode: 0o600 });
  const server = createServer((socket) => {
    socket.on("data", (chunk) => {
      const request = JSON.parse(chunk.toString());
      assert.equal(request.api_version, "v1");
      assert.equal(request.token, "synthetic-token");
      socket.end(`${JSON.stringify({ api_version: "v1", request_id: request.request_id, ok: true, data: { principal: "agent" } })}\n`);
    });
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  const client = new BrokerClient({ socketPath, tokenFile });
  assert.deepEqual(await client.getStatus(), { principal: "agent" });
  await new Promise((resolve) => server.close(resolve));
});

