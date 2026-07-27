import assert from "node:assert/strict";
import test from "node:test";
import {
  CliPermissionController,
  type CliPermissionPrompt,
} from "#src/cli/CliPermissionController.js";
import { EOF } from "#src/ui/lineEditor.js";

interface PendingPrompt {
  prompt: string;
  resolve(answer: string | typeof EOF): void;
}

test("serializes concurrent permission confirmations", async () => {
  const pending: PendingPrompt[] = [];
  const prompt: CliPermissionPrompt = (message) =>
    new Promise((resolve) => {
      pending.push({ prompt: message, resolve });
    });
  let pauses = 0;
  let resumes = 0;
  const controller = new CliPermissionController(
    {
      pauseSpinner() {
        pauses += 1;
        return true;
      },
      resumeSpinner() {
        resumes += 1;
      },
      getScheduleController() {
        return undefined;
      },
    },
    prompt,
  );

  const first = controller.confirm("Allow first?", "first details");
  const second = controller.confirm("Allow second?", "second details");
  await Promise.resolve();

  assert.deepEqual(
    pending.map((entry) => entry.prompt),
    ["Allow first? [y/N] "],
  );
  pending[0]!.resolve("yes");
  assert.equal(await first, true);
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(
    pending.map((entry) => entry.prompt),
    ["Allow first? [y/N] ", "Allow second? [y/N] "],
  );
  pending[1]!.resolve("no");

  assert.equal(await second, false);
  assert.equal(pauses, 2);
  assert.equal(resumes, 2);
});
