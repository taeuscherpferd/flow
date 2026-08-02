# OpenAI-compatible API provider

Status: implemented

## Goal

The `openai-api` setup path provides usage-based API access without changing the
existing `openai` provider, which remains reserved for ChatGPT subscription
access through the official Codex app server.

The provider supports OpenAI Platform and compatible services by letting a
user configure:

- A base URL, defaulting to `https://api.openai.com/v1`.
- The OpenAI Chat Completions wire schema.
- A model name and context window.
- An authorization token supplied through an environment variable, or no
  authentication for local compatible endpoints. Raw tokens are not written to
  `models.json` or logs and are redacted from provider errors.

## Setup

Use `/model openai-api` for an interactive wizard. After setup, use the normal
`/model <provider>/<model>` command to switch to a configured model. The wizard
explains that this provider uses metered API billing and is unrelated to ChatGPT
subscription limits.

The resulting configuration shape resembles:

```json
{
  "providers": {
    "openai-api": {
      "kind": "openai-compatible",
      "baseUrl": "https://api.openai.com/v1",
      "tokenSource": { "type": "environment", "name": "OPENAI_API_KEY" },
      "models": [{ "name": "example-model", "contextWindow": 128000 }]
    }
  }
}
```

## Implementation

- Provider configuration uses a required `kind` and an optional `tokenSource`.
- `EnvironmentSecretResolver` reads credentials only at request time.
- `flowmation-openai-compatible` owns request mapping, structured tool calls,
  reasoning-effort mapping, error handling, redaction, and cancellation.
- The shared CLI provider factory is used by interactive and scheduled runs.
- The `openai` provider name is reserved and always routes through the Codex
  subscription adapter, even if a hand-edited configuration assigns another
  kind.

## Acceptance criteria

- Selecting `openai` can never use an API key or usage-based API billing.
- Selecting `openai-api` never reads or changes the Codex/ChatGPT login.
- Tokens are absent from configuration files, debug output, error messages, and
  test snapshots.
- Interactive and scheduled runs use identical provider construction.

## Future extensions

- Add the Responses API as a selectable schema.
- Add optional endpoint-specific headers.
- Add an OS credential-store source without accepting raw tokens in setup.
