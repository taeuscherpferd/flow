# OpenAI API provider TODO

Status: proposed

## Goal

Add a provider named `openai-api` for usage-based API access without changing the
existing `openai` provider, which is reserved for ChatGPT subscription access
through the official Codex app server.

The provider should support OpenAI Platform and compatible services by letting a
user configure:

- A base URL, defaulting to `https://api.openai.com/v1`.
- A wire schema, initially `responses` or `chat-completions`.
- A model name and context window.
- An authorization token supplied through an environment variable or an OS-backed
  credential store. Raw tokens must not be written to `models.json` or logs.

## Proposed setup

Use `/model openai-api` for an interactive wizard and
`/model openai-api/<name>` for an already configured model. The wizard should
explain that this provider uses metered API billing and is unrelated to ChatGPT
subscription limits.

A future configuration shape could resemble:

```json
{
  "providers": {
    "openai-api": {
      "baseUrl": "https://api.openai.com/v1",
      "schema": "responses",
      "tokenSource": { "type": "environment", "name": "OPENAI_API_KEY" },
      "models": [{ "name": "example-model", "contextWindow": 128000 }]
    }
  }
}
```

## Work items

1. Extend provider configuration with typed schema and credential-source fields
   while keeping existing Ollama-compatible configuration backward compatible.
2. Add a secret resolver abstraction with environment-variable support and an
   OS credential-store implementation before accepting tokens interactively.
3. Implement the provider in a separate crate with cancellation, structured tool
   calls, thinking controls where supported, and structured provider errors.
4. Add the `openai-api` setup flow and route it through the shared CLI provider
   factory used by both interactive and scheduled execution.
5. Cover request serialization, bearer authentication, secret redaction,
   non-2xx responses, cancellation, model switching, and scheduled workflows.
6. Document billing boundaries and migration from manually configured compatible
   endpoints.

## Acceptance criteria

- Selecting `openai` can never use an API key or usage-based API billing.
- Selecting `openai-api` never reads or changes the Codex/ChatGPT login.
- Tokens are absent from configuration files, debug output, error messages, and
  test snapshots.
- Interactive and scheduled runs use identical provider construction.
