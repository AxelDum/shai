# SHAI

shai is a coding agent, your pair programming buddy that lives in the terminal. Written in rust with love <3

![Shai CLI Screenshot](./docs/assets/shai.png)

## Features

- **Interactive coding agent** - Chat with shai in your terminal to write code, fix bugs, and get answers
- **Headless mode** - Pipe prompts directly into shai for scripting and automation
- **HTTP server** - Run shai as a service with OpenAI-compatible APIs and SSE streaming
- **Shell assistant** - Automatically suggests fixes when commands fail in your terminal
- **Project context** - Load project-specific information via `AGENTS.md` files (with `SHAI.md` legacy support)
- **MCP Support** - Configure specialized agents with MCP and OAuth support
- **Skills** - Extend shai with composable, on-demand procedural instructions
- **Multiple LLM providers** - Works with OVHCloud, OpenAI, and other compatible endpoints

## Installation

### Latest stable release

Install the latest release with the following command:

```bash
curl -fsSL https://raw.githubusercontent.com/ovh/shai/main/install.sh | sh
```

### Nightly version

Install the last [``unstable``](https://github.com/ovh/shai/releases/tag/unstable) version with the following command:

```bash
curl -fsSL https://raw.githubusercontent.com/ovh/shai/main/install.sh | SHAI_RELEASE=unstable sh
```

The `shai` binary will be installed in `$HOME/.local/bin`

## Quick Start

By default `shai` uses OVHcloud as an anonymous user meaning you will be rate limited! If you want to sign in with your account or select another provider, run:

```bash
shai auth
```

![shai auth](./docs/assets/auth.gif)

Once you have a provider set up, you can run shai:

```bash
shai
```

![shai](./docs/assets/shai-hello-world.gif)

## Usage

### Interactive Mode

Simply run `shai` to start the interactive coding agent. You can chat with shai and it will help you write code, fix bugs, and answer questions.

### Headless Mode

Shai can also run in headless mode without user interface. In that case simply pipe a prompt into shai, it will stream event in the stderr:

```bash
echo "make me a hello world in main.py" | shai
```

![shai headless](./docs/assets/shai-headless.gif)

You can also instruct shai to return the entire conversation as a trace once it is done:

```bash
echo "make me a hello world in main.py" | shai 2>/dev/null --trace
```

This is handy because you can chain `shai` calls:

```bash
echo "make me a hello world in main.py" | shai --trace | shai "now run it!"
```

### HTTP Server Mode

You can run shai as an HTTP service with SSE streaming support. This mode provides multiple API endpoints:

```bash
shai serve --port 3000
```

![shai http](./docs/assets/shai-http.png)

Available API endpoints:

- **POST /v1/chat/completions** - OpenAI Chat Completions API (ephemeral mode)
- **POST /v1/responses** - OpenAI Responses API (stateful/stateless)
- **GET /v1/responses/{id}** - Get response by ID
- **POST /v1/responses/{id}/cancel** - Cancel a response
- **POST /v1/multimodal** - Simple multimodal API (streaming)
- **POST /v1/multimodal/{session_id}** - Simple multimodal API (with session)

Options:

- `--port <PORT>` - Port to bind to (default: 3000)
- `--ephemeral` - Use ephemeral mode (spawn new agent per request)
- `[AGENT]` - Agent name to use for persistent session

### Shell Assistant

shai can also act as a shell assistant in case a command failed and will propose you a fix. This works by injecting command hook while monitoring your terminal output. Your last terminal output along with the last command and error code will be sent for analysis to the llm provider.

To start hooking your shell with shai simply type:

```bash
shai on
```

For instance:

![Shai CLI Screenshot](./docs/assets/shai-shell.png)

To stop shai from monitoring your shell you can type:

```bash
shai off
```

## Configuration

### Project Context File

Shai supports the [AGENTS.md](https://agents.md/) convention — a standard markdown file at the root of your project containing build steps, code style guidelines, testing instructions, and other context for AI coding agents.

Shai loads context from two files (in priority order):

1. **`AGENTS.md`** — the canonical project context file (recommended).
2. **`SHAI.md`** — legacy override/supplement (deprecated, use `AGENTS.md` instead).

Both files are loaded automatically if present at the git root. `AGENTS.md` is displayed first, followed by `SHAI.md` as an override.

For nested projects in a monorepo, place an `AGENTS.md` in each subdirectory — the closest one to the edited file takes precedence.

### Custom Agents (with MCP)

Instead of a single global configuration, you can create custom agent in a separate configuration.

[`.ovh.config`](./.ovh.config) contains an example of a custom configuration with an remote MCP server configured.

Place this file in `~/.config/shai/agents/ovh.config`, you can then list the agents available with:

```bash
curl https://raw.githubusercontent.com/ovh/shai/refs/heads/main/.ovh.config -o ~/.config/shai/agents/ovh.config
shai agent list
```

You can run shai with this specific agent with the `agent` subcommand:

```bash
shai agent ovh
```

### Temperature

Shai defaults to a sampling temperature of `0.0` (deterministic/greedy decoding). You can adjust this at runtime:

**In the TUI:**

```shai
/temp 0.3
```

**Via CLI flag (headless mode):**

```bash
shai --temperature 0.3 "fix this bug"
```

**Via agent config file:**

```json
{
  "name": "my-agent",
  "temperature": 0.3,
  ...
}
```

### Tool Usage Limits

Shai limits the number of tool calls the agent can make per user turn to prevent runaway loops. When the limit is reached, the agent wraps up its response instead of executing further tools.

This is controlled by the `max_tool_calls_per_turn` setting in the agent's compaction config (default: `100`):

```json
{
  "compaction": {
    "max_tool_calls_per_turn": 100
  }
}
```

Set it to `null` to remove the limit entirely:

```json
{
  "compaction": {
    "max_tool_calls_per_turn": null
  }
}
```

Other related compaction settings:

| Setting | Default | Description |
|---------|---------|-------------|
| `max_output_chars` | `8000` | Maximum characters per tool output before truncation |
| `max_trace_chars` | `50000` | Character threshold above which older trace entries are compacted |
| `max_cached_commands` | `50` | Number of recent bash commands cached for duplicate detection |
| `max_cached_reads` | `100` | Number of file reads cached for duplicate detection |

### OVHCloud Endpoints

OVHCloud provides compatible LLM endpoints for using shai with tools. Start by creating a [_Public Cloud_ project in your OVHCloud account](https://www.ovh.com/manager/#/public-cloud), then head to _AI Endpoints_ and retreive your API key. After setting it in shai, you can:

- choose [one of the models with function calling feature](https://endpoints.ai.cloud.ovh.net/catalog) (e.g., [gpt-oss-120b](https://endpoints.ai.cloud.ovh.net/models/gpt-oss-120b), [gpt-oss-20b](https://endpoints.ai.cloud.ovh.net/models/gpt-oss-20b), [Mistral-​Small-​3.2-​24B-​Instruct-​2506](https://endpoints.ai.cloud.ovh.net/models/mistral-small-3-2-24b-instruct-2506)) for best performance ;
- choose any other model forcing structured output (`/set so` option).

## Skills

Shai supports **skills** — composable, on-demand procedural instructions that extend its capabilities. Skills are loaded progressively: only their name and description are injected into the system prompt, and the full instructions are fetched when the model decides they're relevant.

### Creating a Skill

Create a directory under `.shai/skills/` with a `SKILL.md` file:

```
.shai/skills/
└── my-skill/
    └── SKILL.md
```

A `SKILL.md` file contains YAML frontmatter and markdown body:

```markdown
---
name: my-skill
description: A short description shown in the skill catalog
---

# My Skill

Detailed instructions for the model to follow when this skill is loaded.
```

### Skill Locations

| Location | Scope |
|----------|-------|
| `.shai/skills/` | Project-local (higher priority) |
| `~/.config/shai/skills/` | Global (user-wide) |

Project-local skills shadow global skills with the same name.

See [`docs/skills.md`](./docs/skills.md) for the full documentation.

## Development

### Build The Project

Simply build the project with `cargo`

```bash
git clone git@github.com:ovh/shai.git
cd shai
cargo build --release
```

## Benchmarks

Shai includes a measurement harness that runs the agent against scripted tasks (bug fixes, features, refactors, search) and collects token usage, tool call counts, compaction ratios, and verification results.

### Running a Benchmark

```bash
# Run a single benchmark script
cargo run --example measurement_harness -- shai-core/examples/scripts/bugfix.json

# Run all scripts in a directory
cargo run --example benchmark_runner -- shai-core/examples/scripts/

# Run multiple scripts
cargo run --example benchmark_runner -- shai-core/examples/scripts/bugfix.json shai-core/examples/scripts/feature.json
```

The `benchmark_runner` compiles `measurement_harness` automatically, runs each script sequentially, and prints a summary table with task name, category, difficulty, status, token count, and duration.

### Benchmark Scripts

Each benchmark is defined as a JSON file with the following format:

```json
{
  "name": "task-name",
  "category": "bugfix",
  "difficulty": "medium",
  "goal": "Initial prompt describing the task for the agent",
  "prompts": [],
  "setup": "sh -c 'curl ... | tar xz && cd project'",
  "verify": [
    { "command": "cargo check", "expected_exit_code": 0 }
  ]
}
```

| Field        | Description                                                                  |
|--------------|------------------------------------------------------------------------------|
| `name`      | Display name for the task                                                    |
| `category`  | Task category (`bugfix`, `feature`, `refactor`, `search`)                   |
| `difficulty` | Difficulty level (`easy`, `medium`, `hard`)                                 |
| `goal`      | The initial prompt sent to the agent                                         |
| `prompts`   | Optional additional turns (follow-up prompts) to send after the goal        |
| `setup`     | Optional shell command to prepare a fixture directory (cloned from a repo)  |
| `verify`    | List of commands to run after the agent completes; all must exit 0 to pass   |

### Environment Variables

| Variable                  | Default   | Description                                            |
|---------------------------|-----------|--------------------------------------------------------|
| `HARNESS_TURN_TIMEOUT_MS`| `120000`  | Maximum time (ms) per turn before timing out           |

### Output

Each benchmark run creates a `benchmarks-<timestamp>/` directory containing:
- One subdirectory per task (named after the script file stem)
- `runtime.log` — detailed agent execution log
- `session.log` — summary of token usage, tool calls, and verification results
- `diff.patch` — git diff of changes made by the agent

The harness also prints a JSON report wrapped between `<<<HARNESS_JSON_BEGIN>>>` and `<<<HARNESS_JSON_END>>>` markers for programmatic consumption.

