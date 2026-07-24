import assert from "node:assert/strict";
import test from "node:test";
import { SerializedWorkflowHumanAdapter } from "./SerializedWorkflowHumanAdapter.js";

test("serializes concurrent human requests", async () => {
  const firstResponse = Promise.withResolvers<string>();
  const started: string[] = [];
  const adapter = new SerializedWorkflowHumanAdapter({
    async request(prompt) {
      started.push(prompt.prompt);
      if (prompt.prompt === "First?") return firstResponse.promise;
      return "second";
    },
  });

  const first = adapter.request({ kind: "text", prompt: "First?" });
  const second = adapter.request({ kind: "text", prompt: "Second?" });

  await Promise.resolve();
  assert.deepEqual(started, ["First?"]);

  firstResponse.resolve("first");
  assert.equal(await first, "first");
  assert.equal(await second, "second");
  assert.deepEqual(started, ["First?", "Second?"]);
});

test("continues the request queue after a rejected prompt", async () => {
  const adapter = new SerializedWorkflowHumanAdapter({
    async request(prompt) {
      if (prompt.prompt === "Fails") throw new Error("cancelled");
      return "recovered";
    },
  });

  const failed = adapter.request({ kind: "text", prompt: "Fails" });
  const recovered = adapter.request({ kind: "text", prompt: "Continues" });

  await assert.rejects(failed, /cancelled/);
  assert.equal(await recovered, "recovered");
});
