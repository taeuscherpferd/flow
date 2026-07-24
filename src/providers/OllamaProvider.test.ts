import assert from "node:assert/strict";
import test from "node:test";
import { OllamaProvider } from "#src/providers/OllamaProvider.js";
import type {
  ChatCompletionRequest,
  ThinkingMode,
} from "#src/providers/types.js";

interface RecordedRequestBody {
  messages: Array<{
    role: string;
    content: string;
    thinking?: string;
  }>;
  think?: boolean | "low" | "medium" | "high";
}

async function recordRequest(
  thinking?: ThinkingMode,
  historicalThinking?: string,
): Promise<{
  body: RecordedRequestBody;
  responseThinking?: string;
}> {
  let body: RecordedRequestBody | undefined;
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (_input, init) => {
    body = JSON.parse(String(init?.body)) as RecordedRequestBody;
    return new Response(
      JSON.stringify({
        message: {
          role: "assistant",
          content: "answer",
          thinking: "reasoning",
        },
        done: true,
      }),
      {
        status: 200,
        headers: { "Content-Type": "application/json" },
      },
    );
  };

  try {
    const request: ChatCompletionRequest = {
      model: "test-model",
      messages: [
        {
          role: "assistant",
          content: "previous answer",
          ...(historicalThinking === undefined
            ? {}
            : { thinking: historicalThinking }),
        },
      ],
      ...(thinking === undefined ? {} : { options: { thinking } }),
    };
    const result = await new OllamaProvider("http://ollama.test").chat(request);
    assert.ok(body);
    return {
      body,
      ...(result.message.thinking === undefined
        ? {}
        : { responseThinking: result.message.thinking }),
    };
  } finally {
    globalThis.fetch = originalFetch;
  }
}

test("maps thinking modes to Ollama's top-level think field", async () => {
  const cases: Array<{
    mode?: ThinkingMode;
    expected?: boolean | "low" | "medium" | "high";
  }> = [
    {},
    { mode: "default" },
    { mode: "off", expected: false },
    { mode: "on", expected: true },
    { mode: "low", expected: "low" },
    { mode: "medium", expected: "medium" },
    { mode: "high", expected: "high" },
  ];

  for (const testCase of cases) {
    const { body } = await recordRequest(testCase.mode);
    if (testCase.expected === undefined) {
      assert.equal("think" in body, false);
    } else {
      assert.equal(body.think, testCase.expected);
    }
  }
});

test("retains response thinking and replays historical thinking", async () => {
  const { body, responseThinking } = await recordRequest(
    undefined,
    "previous reasoning",
  );

  assert.equal(responseThinking, "reasoning");
  assert.equal(body.messages[0]?.thinking, "previous reasoning");
});
