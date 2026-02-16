# Fuelcheck CLI

Fuelcheck CLI is a Rust command-line tool that fetches usage and cost data from multiple AI providers. It emits text or JSON output compatible with CodexBar-style dashboards and scripts.

**Features**
- Multi-provider usage and status checks.
- JSON and JSON-only output for scripting.
- Local cost scan for supported providers.
- Optional TUI watch mode for live refresh.

**Install**
Build from source:
```bash
cargo build --release
```
The binary will be at `target/release/fuelcheck-cli`.

Install locally:
```bash
cargo install --path .
```

Run directly during development:
```bash
cargo run -- --help
```

**Quick Start**
Create a config by detecting local credentials:
```bash
fuelcheck-cli setup
```

Fetch usage (defaults to enabled providers in the config):
```bash
fuelcheck-cli usage
```

Fetch usage for a specific provider in JSON:
```bash
fuelcheck-cli usage --provider codex --format json --pretty
```

Compute local cost totals (from local logs when available):
```bash
fuelcheck-cli cost --provider codex
```

Run the live watch TUI:
```bash
fuelcheck-cli usage --watch
```

**Configuration**
The default config path is `~/.codexbar/config.json`. You can override it with `--config` on any command.

Minimal example:
```json
{
  "version": 1,
  "providers": [
    { "id": "codex", "enabled": true, "source": "oauth" },
    { "id": "claude", "enabled": true, "source": "oauth" }
  ]
}
```

Validate or inspect config:
```bash
fuelcheck-cli config validate
fuelcheck-cli config dump --pretty
```

**Providers**
Supported provider IDs:
- codex
- claude
- gemini
- cursor
- factory
- zai
- minimax
- kimi
- kimik2
- copilot
- kiro
- vertexai
- jetbrains
- amp
- warp
- opencode

Use `--provider` multiple times or `--provider all` to query more than one.

**Output Notes**
- Use `--format json` or `--json` for JSON output.
- Use `--json-only` to suppress all non-JSON output.
- Use `--json-output` to emit JSONL logs on stderr.

**Contributing**
See `CONTRIBUTING.md` for development workflow and style guidance.
