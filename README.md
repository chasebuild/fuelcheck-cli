# ⛽ Fuelcheck CLI

Fuelcheck CLI is a Rust command-line tool that fetches usage and cost data from multiple AI providers. It emits text or JSON output compatible with CodexBar-style dashboards and scripts. (heavily inspired by [CodexBar](https://github.com/steipete/CodexBar))

<img width="1418" height="946" alt="image" src="https://github.com/user-attachments/assets/9306bc2d-4426-4281-9983-aac10860986c" />


**Features**
- Multi-provider usage checks with optional status badges.
- JSON and JSON-only output for automation.
- Local cost scan for supported providers.
- Codex local session analytics (`daily`, `monthly`, `session`) via `cost --report`.
- Live TUI watch mode for continuous refresh.
- Configurable sources per provider (oauth, web, api, cli, local).

**Install**
Install from crates.io:
```bash
cargo install fuelcheck-cli
```

Build from source:
```bash
cargo build --release -p fuelcheck-cli
```
The binary will be at `target/release/fuelcheck-cli`.

Install locally:
```bash
cargo install --path cli
```

Run directly during development:
```bash
cargo run -p fuelcheck-cli -- --help
```

**Workspace Layout**
- `core/` (`fuelcheck-core`): provider integrations, config/domain models, cost/usage/report logic.
- `cli/` (`fuelcheck-cli`): command parsing, orchestration, exit/error policy.
- `ui/` (`fuelcheck-ui`): text renderers, report renderers, live TUI watch mode.

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

Compute Codex local reports from `CODEX_HOME/sessions` (or `~/.codex/sessions`):
```bash
fuelcheck-cli cost --report daily --provider codex
fuelcheck-cli cost --report monthly --provider codex --since 20250901 --until 20250930
fuelcheck-cli cost --report session --provider codex --timezone America/New_York
```

JSON report output (single provider keeps ccusage-style top-level keys):
```bash
fuelcheck-cli cost --report daily --provider codex --json --pretty
```

JSON report output (multiple providers returns provider wrapper):
```bash
fuelcheck-cli cost --report daily --provider codex --provider claude --json --pretty
```

Run the live watch TUI:
```bash
fuelcheck-cli usage --watch
```

Validate or inspect config:
```bash
fuelcheck-cli config validate
fuelcheck-cli config dump --pretty
```

**Configuration**
The default config path is `~/.codexbar/config.json`. Override it with `--config` on any command.

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

Provider fields supported by the CLI:
- `id`: provider id (see list below).
- `enabled`: true/false to include by default.
- `source`: one of `auto`, `oauth`, `web`, `api`, `cli`, `local`.
- `cookie_header`: raw `Cookie:` header for web-based providers.
- `api_key`: API token for API-based providers.
- `region`: provider-specific region hint (used by z.ai and MiniMax).
- `workspace_id`: OpenCode workspace override.
- `token_accounts`: optional multi-account list for Codex, Claude, and Cursor.

Example with token accounts:
```json
{
  "providers": [
    {
      "id": "claude",
      "enabled": true,
      "token_accounts": {
        "active_index": 0,
        "accounts": [
          { "label": "Work", "token": "sk-ant-oat..." },
          { "label": "Personal", "token": "sk-ant-oat..." }
        ]
      }
    }
  ]
}
```

**Provider IDs**
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
- `--watch` requires text output.
- `cost --report` currently implements Codex local reports; unsupported providers return provider-level errors in output.

**Provider Setup**
Provider-specific authentication/setup instructions are documented in [`PROVIDER.md`](PROVIDER.md).

**Contributing**
See `CONTRIBUTING.md` for development workflow and style guidance.
