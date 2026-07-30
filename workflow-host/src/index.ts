#!/usr/bin/env node

import { RpcConnection } from "./rpc.js";
import { WorkflowHostServer } from "./server.js";

const connection = new RpcConnection(process.stdin, process.stdout);
const server = new WorkflowHostServer(connection);

try {
  await connection.start(server.handleRequest);
} catch (error) {
  process.stderr.write(
    `Workflow host failed: ${
      error instanceof Error ? error.message : String(error)
    }\n`,
  );
  process.exitCode = 1;
}
