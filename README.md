# Flowmation

Flowmation is a terminal-based coding agent that connects to
Ollama-compatible model providers.

## Getting started

Install dependencies and start the development CLI:

```sh
pnpm install
pnpm dev
```

On a fresh installation, Flowmation creates its configuration in
`~/.work-agent` and displays:

```text
Welcome to flowmation. Before we can get started you will need to setup a provider and a model. Use /model to get started.
```

Run `/model` and answer the prompts for:

- A provider name
- An Ollama-compatible base URL
- A model name
- The model context window

The provider and model are saved to `~/.work-agent/models.json` as the active
defaults. The agent starts immediately after setup, without restarting the CLI.

Once a model is configured, `/model` lists available models. Use
`/model <name>` or `/model <provider>/<name>` to switch the active model.

## Commands

- `/help` shows command help.
- `/clear` clears the conversation context.
- `/model` lists models or starts first-run model setup.
- `/model <name>` switches models.
- `/exit` or `/quit` exits Flowmation.
- `/<skill-name>` loads a configured skill into the conversation.

## Development

```sh
pnpm test
pnpm run build
```

The CLI entry point is `src/index.ts`. REPL startup and command handling live in
`src/cli/FlowCli.ts`, while agent tools are registered in
`src/tools/ToolRegistry.ts`.
