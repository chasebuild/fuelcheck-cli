# Provider Setup

Fuelcheck CLI uses the same credentials and tokens described in CodexBar provider docs. For cookie-based providers, you must supply a raw `Cookie:` header. A quick way to capture it is:
- Open the provider site in your browser.
- Open DevTools -> Network and reload.
- Select a request to the provider domain and copy the `Cookie` request header.
- Paste the header into `cookie_header` or set the matching env var.

Setup shortcuts:
- `fuelcheck-cli setup` will detect Codex, Claude, and Gemini CLI credentials when present.
- `fuelcheck-cli setup --claude-cookie`, `--cursor-cookie`, `--factory-cookie` can prefill cookie headers.

Below is a per-provider setup summary based on CodexBar behavior and the CLI implementation.

## Codex
- Auth: run `codex` to create `~/.codex/auth.json` (or set `CODEX_HOME`).
- Source: OAuth only in this CLI build (`--source oauth` or `auto`).
- Multi-account: use `token_accounts` with Codex OAuth access tokens.

## Claude
- OAuth: run `claude` so `~/.claude/.credentials.json` exists (or Keychain on macOS).
- Web: set `cookie_header` or `CLAUDE_COOKIE` with a `sessionKey` cookie from `claude.ai`.
- Source: OAuth or Web (`--source oauth|web`).
- Multi-account: use `token_accounts` with OAuth access tokens.

## Gemini
- Auth: run `gemini` to create `~/.gemini/oauth_creds.json` and `~/.gemini/settings.json`.
- Source: API only (`--source api` or `auto`).

## Cursor
- Web cookies: set `cookie_header` or `CURSOR_COOKIE` from a `cursor.com` request.
- Source: Web only (`--source web` or `auto`).
- Multi-account: use `token_accounts` with per-account cookie headers.

## Factory (Droid)
- Web cookies: set `cookie_header` or `FACTORY_COOKIE`/`DROID_COOKIE` from `app.factory.ai`.
- Optional bearer token: set `api_key` or `FACTORY_BEARER_TOKEN`.
- Source: Web only (`--source web` or `auto`).

## z.ai
- API token: set `api_key` or `Z_AI_API_KEY`.
- Optional host overrides: `Z_AI_API_HOST` or `Z_AI_QUOTA_URL`.
- Source: API only (`--source api` or `auto`).

## MiniMax
- Preferred: set `api_key` or `MINIMAX_API_KEY`.
- Cookie fallback: set `cookie_header` or `MINIMAX_COOKIE`.
- Optional overrides: `MINIMAX_HOST`, `MINIMAX_REMAINS_URL`, or `region` in config.

## Kimi
- Token: set `api_key` or `KIMI_AUTH_TOKEN` from the `kimi-auth` cookie.
- Source: API only (`--source api` or `auto`).

## Kimi K2
- API key: set `api_key` or `KIMI_K2_API_KEY` (also accepts `KIMI_API_KEY` or `KIMI_KEY`).
- Source: API only (`--source api` or `auto`).

## Copilot
- API token: set `api_key` or `COPILOT_API_TOKEN` (also accepts `GITHUB_TOKEN`).
- Token should have access to Copilot usage for your account.
- Source: API only (`--source api` or `auto`).

## Kiro
- Install and log in to `kiro-cli` using AWS Builder ID.
- The CLI runs `kiro-cli chat --no-interactive "/usage"`.
- Source: CLI only (`--source cli` or `auto`).

## Vertex AI
- Run `gcloud auth application-default login` and set a project with `gcloud config set project`.
- Requires Cloud Monitoring quota access in the selected project.
- Source: OAuth only (`--source oauth` or `auto`).

## JetBrains AI
- Use a JetBrains IDE with AI Assistant enabled.
- The CLI reads `AIAssistantQuotaManager2.xml` from the IDE config directory.
- Source: Local only (`--source local` or `auto`).

## Amp
- Cookie header from `https://ampcode.com/settings`.
- Set `cookie_header` or `AMP_COOKIE`/`AMP_COOKIE_HEADER`.
- Source: Web only (`--source web` or `auto`).

## Warp
- API key from `https://app.warp.dev/settings/account`.
- Set `api_key` or `WARP_API_KEY`/`WARP_TOKEN`.
- Source: API only (`--source api` or `auto`).

## OpenCode
- Cookie header from `https://opencode.ai`.
- Set `cookie_header` or `OPENCODE_COOKIE`/`OPENCODE_COOKIE_HEADER`.
- Optional workspace override: `workspace_id` or `CODEXBAR_OPENCODE_WORKSPACE_ID`.
- Source: Web only (`--source web` or `auto`).
