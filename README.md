# Flowmation

Flowmation is a terminal coding agent with a Rust-owned application core and a
small Node.js workflow host. Rust owns configuration, agents, models, tools,
authorization, persistence, workflow orchestration, scheduling, and the CLI.
The Node host preserves trusted `WORKFLOW.js` and `WORKFLOW.ts` modules,
including their Node API and workflow-local dependency access.

## Requirements

- Rust 1.96 or newer
- Node.js 24 or newer and pnpm for the workflow compatibility host. The
  `node` executable must be available on `PATH`.
- An Ollama-compatible provider, an OpenAI-compatible Chat Completions API, or
  the official OpenAI Codex CLI for ChatGPT subscription access

The current CLI initializes workflow discovery before its first normal agent
turn so it can register `run_workflow`. Consequently, interactive chat as well
as explicit workflow commands requires a built host and a compatible Node
runtime. Configuration, model setup, and non-workflow repository operations
remain Rust-only.

## Build, test, and install

Install the workflow-host dependencies and build the complete workspace:

```sh
pnpm install --frozen-lockfile
pnpm run build
```

The root build runs:

```sh
cargo build --workspace
```

The CLI crate's Cargo build script compiles the workflow host first and stages
it with its production Node dependencies beside the executable at
`target/debug/workflow-host/`. Release builds also embed that package in the
executable as a fallback.

Run the Rust and workflow-host suites:

```sh
pnpm test
```

The Rust test script builds the ignored workflow-host distribution first, so it
also works from a clean checkout. Run the suites independently with:

```sh
pnpm run test:rust
pnpm --dir workflow-host test
```

Cargo also handles the workflow-host build when invoked directly:

```sh
cargo test --workspace --all-features
```

Formatting and lint checks are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run the CLI from the checkout:

```sh
cargo run -p flowmation-cli
```

Build the release distribution:

```sh
pnpm run build:release
```

Install a release executable with the embedded workflow host from the checkout:

```sh
cargo install --path crates/flowmation-cli
```

The build stages the production workflow host in `target/release/workflow-host/`
for directory-based packages and embeds the same package in the release
executable. A release binary first uses a sibling host when present; otherwise,
it extracts its embedded host under `~/.work-agent/runtime/`. It never depends
on a source path from the build machine.

An explicit host override remains available for development and custom
packaging:

```sh
FLOWMATION_WORKFLOW_HOST=/absolute/path/to/workflow-host/dist/index.js flowmation
```

Debug binaries fall back to the checkout's `workflow-host/dist/index.js` when a
staged sibling is unavailable. Host paths are simplified before being passed to
Node so Windows verbatim paths remain compatible with the Node module loader.

## Workspace architecture

| Crate/package | Responsibility |
| --- | --- |
| `flowmation-domain` | Compatibility-sensitive IDs and records, state enums, model configuration, schema validation, five-field cron behavior, fingerprints, and input history. |
| `flowmation-application` | Provider/tool interfaces, authorization, agents, registries, workflow callbacks and durable execution, scheduling services, and UI-neutral events/cancellation. |
| `flowmation-sqlite` | `runs.sqlite` repositories, ordered migrations, application adapters, schedule leases/occurrences/notifications, and legacy compatibility. |
| `flowmation-http` | Shared cancellable JSON HTTP transport used by provider adapters. |
| `flowmation-ollama` | Ollama-compatible HTTP provider adapter. |
| `flowmation-openai-compatible` | OpenAI-compatible Chat Completions adapter with environment-backed bearer authentication. |
| `flowmation-codex` | OpenAI subscription adapter using the official Codex app server and its cached login. |
| `flowmation-workflow-host` | Rust child-process client and typed, versioned bidirectional JSON-RPC protocol. |
| `flowmation-cli` | Terminal adapter, first-model setup, raw-mode line editor, run management, and the internal schedule worker. |
| `flowmation-test-support` | Reusable fake providers, brokers, clocks, repositories, and event collectors. |
| `workflow-host/` | Remaining Node 24+ workflow loader and authoring SDK. |

The application crate has no terminal or SQLite dependency. Those concerns are
adapters, allowing another UI or persistence implementation to use the same
domain and application services.

## First run and configuration

On first launch Flowmation creates the global scaffold under
`~/.work-agent/`. If no model is configured, `/model` starts an interactive
provider setup with three choices:

- `ollama` for local Ollama-compatible models;
- `openai` for OpenAI models covered by a ChatGPT subscription through Codex;
- `openai-api` for OpenAI Platform or another OpenAI-compatible API endpoint.

### OpenAI through a ChatGPT subscription

Run `/model` to see configured models and any additional OpenAI models available
to the current ChatGPT subscription authenticated through Codex.
If Codex is not signed in, the listing points to `/model openai`, which starts
the device-code flow and then shows the live model catalog:

```text
OpenAI models require ChatGPT sign-in. Run /model openai to sign in.
Open https://auth.openai.com/codex/device
Enter the one-time code: ABCD-1234
```

Open the address printed by Codex, sign in to ChatGPT, and enter the one-time
code. Device-code login may first need to be enabled in ChatGPT Security
Settings or by a workspace administrator.

Flowmation does not implement OpenAI OAuth, read `~/.codex/auth.json`, or store
ChatGPT tokens. It starts the official Codex app server, which owns login,
credential storage, refresh, model access, and subscription usage limits.
Install and authenticate Codex before setup if preferred:

```sh
npm install -g @openai/codex
codex login --device-auth
codex login status
```

If `codex` is not on `PATH`, set `FLOWMATION_CODEX_BIN` to the executable or
launcher path. Windows pnpm `codex.ps1` launchers are supported directly.
Flowmation gets the picker-visible models and the recommended default from the
Codex app server instead of maintaining a hard-coded OpenAI model list. Run
`/model openai/<name>` to add a model shown by `/model` and switch the active
conversation to it immediately. OpenAI context settings are managed
automatically; the setup flow does not ask for a context-window value.

This is distinct from OpenAI Platform API access. The `openai` provider rejects
API-key and non-OpenAI Codex authentication so it cannot silently switch to
usage-based billing.

> **Security warning:** The Codex app server remains an agent runtime and may
> invoke Codex built-in or configured tools that Flowmation did not advertise.
> Flowmation requests a read-only sandbox and never requests approval escalation,
> but it cannot currently enforce a Codex-side tool allowlist. Use this provider
> only when the local Codex configuration and execution environment are trusted.

### OpenAI-compatible APIs

Run `/model openai-api` to configure OpenAI Platform or a service exposing the
[OpenAI Chat Completions schema](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create).
The wizard asks for a provider name, base URL, model, context window, and the
name of an environment variable containing the API key. For example:

```sh
export OPENAI_API_KEY="your-key"
```

The default base URL is `https://api.openai.com/v1` and the default credential
source is `OPENAI_API_KEY`. Enter `none` at the credential prompt for an
unauthenticated compatible endpoint such as a locally hosted server. API keys
are read when a request runs; only the environment-variable name is stored in
`models.json`. Interactive conversations and scheduled workflows use the same
provider factory and credential resolution.

Credential sources are accepted only in the global `~/.work-agent/models.json`
file. A project may select a provider defined globally or define an
unauthenticated provider, but project model configuration cannot name an
environment variable to forward to an endpoint.

This provider uses metered API billing and never reads or changes the Codex or
ChatGPT login. It currently targets the broadly supported Chat Completions wire
format so it can also work with compatible services such as OpenRouter, Groq,
Together, and vLLM. Endpoint-specific headers and the Responses API are not yet
configurable. See [docs/openai-api-provider.md](docs/openai-api-provider.md) for
the configuration contract and current limitations.

Configuration is loaded from:

```text
~/.work-agent/
<launch-directory>/.work-agent/
```

Project model and application values override or extend global values
according to the Rust merge rules. Project `AGENTS.md` follows the global
instructions, while a project `SOUL.md` replaces the global soul.

The main agent includes `create-skill`, `create-workflow`, and `create-schedule`
skills for authoring and scheduling reusable project automation. Invoke them
with `/create-skill <request>`, `/create-workflow <request>`, or
`/create-schedule <request>`.
Same-named skills under global or project `skills/` override the embedded
versions; newly created resources are discovered after Flowmation restarts.

## Configured agents

Agent packages live under:

```text
~/.work-agent/agents/<name>/
<project>/.work-agent/agents/<name>/
```

A project package atomically replaces a same-named global package. Required
files are `AGENT.yaml`, `SOUL.md`, and `AGENTS.md`; `CONTEXT.md`, `context/`,
and `skills/` are optional. The package uses the legacy SHA-256 fingerprint,
and symbolic links are rejected.

`/agent` lists packages, `/agent <name>` opens that agent's project-scoped
conversation, and `/agent main` returns to the coordinator. Conversations are
stored in `~/.work-agent/runs.sqlite`; system messages are rebuilt and never
persisted.

Interactive agents may use any registered tool in their allowlist without a
per-call permission prompt. Their tool loop continues until the model returns a
tool-free response, the provider fails, or the user cancels the turn; there is
no default iteration cap. Scheduled agent execution remains non-interactive and
denies effectful model tools. Workflow `confirm` policies and workflow-authored
human approval steps still prompt separately.

The runtime supports specialist chat and isolated workflow-created agent
sessions. Coordinator delegation tools and interactive agent-package-local
workflow discovery are not implemented. See [docs/agents.md](docs/agents.md).

## Workflows and the Node host

Interactive workflows are discovered from:

```text
~/.work-agent/workflows/<name>/WORKFLOW.ts
<project>/.work-agent/workflows/<name>/WORKFLOW.js
```

Exactly one entry file may exist in a workflow directory. Project workflows
replace same-named global workflows. Directory and exported names must match
and use lowercase kebab-case.

Rust fingerprints the complete directory, asks the Node host to inspect the
module, validates input, creates the durable run, owns all callbacks and
durability records, and commits the final status. The host imports the trusted
module and runs its `run` callback.

The protocol is newline-delimited JSON-RPC 2.0 over stdio, version `1`. Rust
performs `host.handshake` before using workflow methods, and SDK operations
return to Rust as `sdk.*` requests. Only protocol messages use stdout;
diagnostics use stderr.

Workflow code is trusted local code. It can use Node APIs and local
dependencies, so Node 24+ remains required for workflows. See
[docs/workflows.md](docs/workflows.md).

## SQLite compatibility

Rust opens the existing `~/.work-agent/runs.sqlite` in place. It retains the
legacy 5-second busy timeout, WAL mode, foreign-key enforcement, table and
column names, JSON formats, status strings, timestamps, indexes, and schedule
status trigger.

Six ordered Rust migrations create or complete workflow storage, add
agent/trigger metadata, create schedule storage, install the schedule-run
trigger, create conversation storage, and add explicit cron/one-shot timing.
Existing rows are not rewritten;
historical runs default to `agent_name = 'main'` and a manual trigger. See
[docs/migration.md](docs/migration.md).

## CLI commands

The Rust REPL implements:

- `/help`
- `/agent [name]`
- `/clear`
- `/model [name]`
- `/model openai` and `/model openai/<name>`
- `/model openai-api`
- `/workflows`
- `/workflow <name> [input]` and `/<workflow-name> [input]`
- `/<skill-name> [message]`
- `/runs`
- `/run <id>`
- `/resume <id>`
- `/cancel <id>`
- `/workflow-debug [on|off]`
- `/schedules`
- `/schedule create <json>`
- `/schedule <id>`
- `/schedule pause|resume|delete <id>`
- `/exit` and `/quit`

Run commands are scoped to the launch project and active agent. `/resume`
replays waiting or interrupted runs with checkpoint, effect, and human-step
durability. `/cancel` changes the durable status, but it cannot signal an
execution owned by a different process.

Normal text is sent to the active agent. On a TTY the cross-platform Rust line
editor uses Crossterm on Linux, macOS, and Windows. It supports cursor movement,
deletion, persistent history navigation, wrapped-row redraw, dimmed
slash-command completion, and the two-stage Ctrl+C behavior. Press Tab or Right
Arrow at the end of the input to accept the suggested built-in, workflow, or
skill command. When the agent loads a skill during a normal turn, the spinner
names that skill and the CLI prints a persistent `Used skill` summary before the
answer. Non-TTY stdin uses line-oriented input.

The coordinator and specialists whose tool policy includes `create_schedule`
can create confirmed cron or one-shot workflow schedules with the model tool.
The explicit `/schedule create <json>` command is also available to the user.
The optional `agent` field defaults to the active agent; the coordinator can use
it to target a specialist workflow. Schedule reauthorization remains an
application service and is not exposed by the CLI.

## Schedule worker

The executable includes a Rust worker:

```sh
cargo run -p flowmation-cli -- worker --once
cargo run -p flowmation-cli -- worker --database /path/to/runs.sqlite --once
cargo run -p flowmation-cli -- worker
```

`--once` performs one leased tick. Without it, the worker polls every 15
seconds until Ctrl+C. It recovers non-terminal occurrences, coalesces downtime
into the oldest due occurrence, validates the authorized source fingerprint
before module evaluation, and executes through the same Node host and durable
Rust callbacks.

The CLI does not automatically detach or install this worker. Run it under a
process supervisor for unattended schedules. See
[docs/scheduling.md](docs/scheduling.md) and
[docs/security.md](docs/security.md) for the operational boundaries.
