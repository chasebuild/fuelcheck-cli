# ⛽ Fuelcheck CLI

Fuelcheck CLI is a Rust command-line tool that fetches usage and cost data from multiple AI providers. It emits text or JSON output compatible with CodexBar-style dashboards and scripts. (heavily inspired by [CodexBar](https://github.com/steipete/CodexBar))

<img width="1418" height="946" alt="image" src="https://github.com/user-attachments/assets/9306bc2d-4426-4281-9983-aac10860986c" />


**Features**
- Multi-provider usage checks with optional status badges.
- JSON and JSON-only output for automation.
- Local cost scan for supported providers.
- Live TUI watch mode for continuous refresh.
- Configurable sources per provider (oauth, web, api, cli, local).

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

**Provider Setup (from CodexBar docs)**
Fuelcheck CLI uses the same credentials and tokens described in CodexBar provider docs. For cookie-based providers, you must supply a raw `Cookie:` header. A quick way to capture it is:
- Open the provider site in your browser.
- Open DevTools → Network and reload.
- Select a request to the provider domain and copy the `Cookie` request header.
- Paste the header into `cookie_header` or set the matching env var.

Setup shortcuts:
- `fuelcheck-cli setup` will detect Codex, Claude, and Gemini CLI credentials when present.
- `fuelcheck-cli setup --claude-cookie`, `--cursor-cookie`, `--factory-cookie` can prefill cookie headers.

Below is a per-provider setup summary based on CodexBar behavior and the CLI implementation.

### Codex
- Auth: run `codex` to create `~/.codex/auth.json` (or set `CODEX_HOME`).
- Source: OAuth only in this CLI build (`--source oauth` or `auto`).
- Multi-account: use `token_accounts` with Codex OAuth access tokens.

### Claude
- OAuth: run `claude` so `~/.claude/.credentials.json` exists (or Keychain on macOS).
- Web: set `cookie_header` or `CLAUDE_COOKIE` with a `sessionKey` cookie from `claude.ai`.
- Source: OAuth or Web (`--source oauth|web`).
- Multi-account: use `token_accounts` with OAuth access tokens.

### Gemini
- Auth: run `gemini` to create `~/.gemini/oauth_creds.json` and `~/.gemini/settings.json`.
- Source: API only (`--source api` or `auto`).

### Cursor
- Web cookies: set `cookie_header` or `CURSOR_COOKIE` from a `cursor.com` request.
- Source: Web only (`--source web` or `auto`).
- Multi-account: use `token_accounts` with per-account cookie headers.

### Factory (Droid)
- Web cookies: set `cookie_header` or `FACTORY_COOKIE`/`DROID_COOKIE` from `app.factory.ai`.
- Optional bearer token: set `api_key` or `FACTORY_BEARER_TOKEN`.
- Source: Web only (`--source web` or `auto`).

### z.ai
- API token: set `api_key` or `Z_AI_API_KEY`.
- Optional host overrides: `Z_AI_API_HOST` or `Z_AI_QUOTA_URL`.
- Source: API only (`--source api` or `auto`).

### MiniMax
- Preferred: set `api_key` or `MINIMAX_API_KEY`.
- Cookie fallback: set `cookie_header` or `MINIMAX_COOKIE`.
- Optional overrides: `MINIMAX_HOST`, `MINIMAX_REMAINS_URL`, or `region` in config.

### Kimi
- Token: set `api_key` or `KIMI_AUTH_TOKEN` from the `kimi-auth` cookie.
- Source: API only (`--source api` or `auto`).

### Kimi K2
- API key: set `api_key` or `KIMI_K2_API_KEY` (also accepts `KIMI_API_KEY` or `KIMI_KEY`).
- Source: API only (`--source api` or `auto`).

### Copilot
- API token: set `api_key` or `COPILOT_API_TOKEN` (also accepts `GITHUB_TOKEN`).
- Token should have access to Copilot usage for your account.
- Source: API only (`--source api` or `auto`).

### Kiro
- Install and log in to `kiro-cli` using AWS Builder ID.
- The CLI runs `kiro-cli chat --no-interactive "/usage"`.
- Source: CLI only (`--source cli` or `auto`).

### Vertex AI
- Run `gcloud auth application-default login` and set a project with `gcloud config set project`.
- Requires Cloud Monitoring quota access in the selected project.
- Source: OAuth only (`--source oauth` or `auto`).

### JetBrains AI
- Use a JetBrains IDE with AI Assistant enabled.
- The CLI reads `AIAssistantQuotaManager2.xml` from the IDE config directory.
- Source: Local only (`--source local` or `auto`).

### Amp
- Cookie header from `https://ampcode.com/settings`.
- Set `cookie_header` or `AMP_COOKIE`/`AMP_COOKIE_HEADER`.
- Source: Web only (`--source web` or `auto`).

### Warp
- API key from `https://app.warp.dev/settings/account`.
- Set `api_key` or `WARP_API_KEY`/`WARP_TOKEN`.
- Source: API only (`--source api` or `auto`).

### OpenCode
- Cookie header from `https://opencode.ai`.
- Set `cookie_header` or `OPENCODE_COOKIE`/`OPENCODE_COOKIE_HEADER`.
- Optional workspace override: `workspace_id` or `CODEXBAR_OPENCODE_WORKSPACE_ID`.
- Source: Web only (`--source web` or `auto`).

**Contributing**
See `CONTRIBUTING.md` for development workflow and style guidance.
