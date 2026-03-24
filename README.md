<h1 align="center">gws</h1>

**One CLI for all of Google Workspace — built for humans and AI agents.**<br>
Drive, Gmail, Calendar, and every Workspace API. Zero boilerplate. Structured JSON output. 40+ agent skills included.

> [!NOTE]
> This is **not** an officially supported Google product.

<p>
  <a href="https://www.npmjs.com/package/@googleworkspace/cli"><img src="https://img.shields.io/npm/v/@googleworkspace/cli" alt="npm version"></a>
  <a href="https://github.com/googleworkspace/cli/blob/main/LICENSE"><img src="https://img.shields.io/github/license/googleworkspace/cli" alt="license"></a>
  <a href="https://github.com/googleworkspace/cli/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/googleworkspace/cli/ci.yml?branch=main&label=CI" alt="CI status"></a>
  <a href="https://www.npmjs.com/package/@googleworkspace/cli"><img src="https://img.shields.io/npm/unpacked-size/@googleworkspace/cli" alt="install size"></a>
</p>
<br>

```bash
npm install -g @googleworkspace/cli
```

`gws` doesn't ship a static list of commands. It reads Google's own [Discovery Service](https://developers.google.com/discovery) at runtime and builds its entire command surface dynamically. When Google Workspace adds an API endpoint or method, `gws` picks it up automatically.

> [!IMPORTANT]
> This project is under active development. Expect breaking changes as we march toward v1.0.

## Contents

- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Why gws?](#why-gws)
- [Authentication](#authentication)
- [AI Agent Skills](#ai-agent-skills)
- [Advanced Usage](#advanced-usage)
- [Environment Variables](#environment-variables)
- [Exit Codes](#exit-codes)
- [Architecture](#architecture)
- [Troubleshooting](#troubleshooting)
- [Development](#development)

## Prerequisites

- **Node.js 18+** — for `npm install` (or download a pre-built binary from [GitHub Releases](https://github.com/googleworkspace/cli/releases))
- **A Google Cloud project** — required for OAuth credentials. You can create one via the [Google Cloud Console](https://console.cloud.google.com/) or with the [`gcloud` CLI](https://cloud.google.com/sdk/docs/install) or with the `gws auth setup` command.
- **A Google account** with access to Google Workspace

## Installation

```bash
npm install -g @googleworkspace/cli
```

> The npm package bundles pre-built native binaries for your OS and architecture.
> No Rust toolchain required.

Pre-built binaries are also available on the [GitHub Releases](https://github.com/googleworkspace/cli/releases) page.

Or build from source:

```bash
cargo install --git https://github.com/googleworkspace/cli --locked
```

A Nix flake is also available at `github:googleworkspace/cli`

```bash
nix run github:googleworkspace/cli
```

On macOS and Linux, you can also install via [Homebrew](https://brew.sh/):

```bash
brew install googleworkspace-cli
```

## Quick Start

```bash
gws auth login             # opens browser → Google consent → done
gws drive files list --params '{"pageSize": 5}'
```

## Why gws?

**For humans** — stop writing `curl` calls against REST docs. `gws` gives you `--help` on every resource, `--dry-run` to preview requests, and auto‑pagination.

**For AI agents** — every response is structured JSON. Pair it with the included agent skills and your LLM can manage Workspace without custom tooling.

```bash
# List the 10 most recent files
gws drive files list --params '{"pageSize": 10}'

# Create a spreadsheet
gws sheets spreadsheets create --json '{"properties": {"title": "Q1 Budget"}}'

# Send a Chat message
gws chat spaces messages create \
  --params '{"parent": "spaces/xyz"}' \
  --json '{"text": "Deploy complete."}' \
  --dry-run

# Introspect any method's request/response schema
gws schema drive.files.list

# Stream paginated results as NDJSON
gws drive files list --params '{"pageSize": 100}' --page-all | jq -r '.files[].name'
```

## Authentication

```bash
gws auth login    # opens browser → Google consent → done
```

No GCP project, no client secret, no `gcloud` needed. The CLI uses a Cloud Function proxy (same as [gemini-cli-extensions/workspace](https://github.com/gemini-cli-extensions/workspace)) to handle OAuth token exchange server-side. Tokens are refreshed automatically through the same proxy.

Credentials are encrypted at rest (AES-256-GCM) with the key stored in your OS keyring. Use `gws auth status` to check the current auth state.

### Headless / CI

1. Complete interactive auth on a machine with a browser.
2. Export credentials:
   ```bash
   gws auth export --unmasked > credentials.json
   ```
3. On the headless machine:
   ```bash
   export GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE=/path/to/credentials.json
   gws drive files list   # just works
   ```

### Pre-obtained Access Token

Useful when another tool (e.g. `gcloud`) already mints tokens for your environment.

```bash
export GOOGLE_WORKSPACE_CLI_TOKEN=$(gcloud auth print-access-token)
```

### Precedence

| Priority | Source                 | Set via                                 |
| -------- | ---------------------- | --------------------------------------- |
| 1        | Access token           | `GOOGLE_WORKSPACE_CLI_TOKEN`            |
| 2        | Credentials file       | `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` |
| 3        | Encrypted credentials  | `gws auth login`                        |
| 4        | Plaintext credentials  | `~/.config/gws/credentials.json`        |
| 5        | Application Default Credentials | `GOOGLE_APPLICATION_CREDENTIALS` or `gcloud auth application-default login` |

Environment variables can also live in a `.env` file.

## AI Agent Skills

The repo ships 100+ Agent Skills (`SKILL.md` files) — one for every supported API, plus higher-level helpers for common workflows and 50 curated recipes for Gmail, Drive, Docs, Calendar, and Sheets. See the full [Skills Index](docs/skills.md) for the complete list.

```bash
# Install all skills at once
npx skills add https://github.com/googleworkspace/cli

# Or pick only what you need
npx skills add https://github.com/googleworkspace/cli/tree/main/skills/gws-drive
npx skills add https://github.com/googleworkspace/cli/tree/main/skills/gws-gmail
```

<details>
<summary>OpenClaw setup</summary>

```bash
# Symlink all skills (stays in sync with repo)
ln -s $(pwd)/skills/gws-* ~/.openclaw/skills/

# Or copy specific skills
cp -r skills/gws-drive skills/gws-gmail ~/.openclaw/skills/
```

The `gws-shared` skill includes an `install` block so OpenClaw auto-installs the CLI via `npm` if `gws` isn't on PATH.

</details>

## Gemini CLI Extension

1. Authenticate the CLI first:

   ```bash
   gws auth setup
   ```

2. Install the extension into the Gemini CLI:
   ```bash
   gemini extensions install https://github.com/googleworkspace/cli
   ```

Installing this extension gives your Gemini CLI agent direct access to all `gws` commands and Google Workspace agent skills. Because `gws` handles its own authentication securely, you simply need to authenticate your terminal once prior to using the agent, and the extension will automatically inherit your credentials.

## Advanced Usage

### Multipart Uploads

```bash
gws drive files create --json '{"name": "report.pdf"}' --upload ./report.pdf
```

### Pagination

| Flag                | Description                                    | Default |
| ------------------- | ---------------------------------------------- | ------- |
| `--page-all`        | Auto-paginate, one JSON line per page (NDJSON) | off     |
| `--page-limit <N>`  | Max pages to fetch                             | 10      |
| `--page-delay <MS>` | Delay between pages                            | 100 ms  |

### Google Sheets — Shell Escaping

Sheets ranges use `!` which bash interprets as history expansion. Always wrap values in **single quotes**:

```bash
# Read cells A1:C10 from "Sheet1"
gws sheets spreadsheets values get \
  --params '{"spreadsheetId": "SPREADSHEET_ID", "range": "Sheet1!A1:C10"}'

# Append rows
gws sheets spreadsheets values append \
  --params '{"spreadsheetId": "ID", "range": "Sheet1!A1", "valueInputOption": "USER_ENTERED"}' \
  --json '{"values": [["Name", "Score"], ["Alice", 95]]}'
```

### Helper Commands

Some services ship hand-crafted helper commands alongside the auto-generated Discovery surface. Helper commands are prefixed with `+` so they are visually distinct and never collide with Discovery-generated method names.

Time-aware helpers (`+agenda`, `+standup-report`, `+weekly-digest`, `+meeting-prep`) automatically use your **Google account timezone** (fetched from Calendar Settings API and cached for 24 hours). Override with `--timezone`/`--tz` on `+agenda`, or set the `--timezone` flag for explicit control.

Run `gws <service> --help` to see both Discovery methods and helper commands together.

```bash
gws gmail --help      # shows +send, +reply, +reply-all, +forward, +triage, +watch …
gws calendar --help   # shows +insert, +agenda …
gws drive --help      # shows +upload …
```

**Full helper reference:**

| Service | Command | Description |
|---------|---------|-------------|
| `gmail` | `+send` | Send an email |
| `gmail` | `+reply` | Reply to a message (handles threading automatically) |
| `gmail` | `+reply-all` | Reply-all to a message |
| `gmail` | `+forward` | Forward a message to new recipients |
| `gmail` | `+triage` | Show unread inbox summary (sender, subject, date) |
| `gmail` | `+watch` | Watch for new emails and stream them as NDJSON |
| `sheets` | `+append` | Append a row to a spreadsheet |
| `sheets` | `+read` | Read values from a spreadsheet |
| `docs` | `+write` | Append text to a document |
| `chat` | `+send` | Send a message to a space |
| `drive` | `+upload` | Upload a file with automatic metadata |
| `calendar` | `+insert` | Create a new event |
| `calendar` | `+agenda` | Show upcoming events (uses Google account timezone; override with `--timezone`) |
| `script` | `+push` | Replace all files in an Apps Script project with local files |
| `workflow` | `+standup-report` | Today's meetings + open tasks as a standup summary |
| `workflow` | `+meeting-prep` | Prepare for your next meeting: agenda, attendees, and linked docs |
| `workflow` | `+email-to-task` | Convert a Gmail message into a Google Tasks entry |
| `workflow` | `+weekly-digest` | Weekly summary: this week's meetings + unread email count |
| `workflow` | `+file-announce` | Announce a Drive file in a Chat space |
| `events` | `+subscribe` | Subscribe to Workspace events and stream them as NDJSON |
| `events` | `+renew` | Renew/reactivate Workspace Events subscriptions |
| `modelarmor` | `+sanitize-prompt` | Sanitize a user prompt through a Model Armor template |
| `modelarmor` | `+sanitize-response` | Sanitize a model response through a Model Armor template |
| `modelarmor` | `+create-template` | Create a new Model Armor template |

**Examples:**

```bash
# Send an email
gws gmail +send --to alice@example.com --subject "Hello" --body "Hi there"

# Reply to a message
gws gmail +reply --message-id MESSAGE_ID --body "Thanks!"

# Append a row to a spreadsheet
gws sheets +append --spreadsheet SPREADSHEET_ID --values "Alice,95"

# Show today's calendar agenda
gws calendar +agenda

# Upload a file to Drive
gws drive +upload ./report.pdf --name "Q1 Report"

# Morning standup summary
gws workflow +standup-report

# Show today's agenda in a specific timezone
gws calendar +agenda --today --timezone America/New_York
```

### Model Armor (Response Sanitization)

Integrate [Google Cloud Model Armor](https://cloud.google.com/security/products/model-armor) to scan API responses for prompt injection before they reach your agent.

```bash
gws gmail users messages get --params '...' \
  --sanitize "projects/P/locations/L/templates/T"
```

| Variable                                 | Description                  |
| ---------------------------------------- | ---------------------------- |
| `GOOGLE_WORKSPACE_CLI_SANITIZE_TEMPLATE` | Default Model Armor template |
| `GOOGLE_WORKSPACE_CLI_SANITIZE_MODE`     | `warn` (default) or `block`  |

## Environment Variables

All variables are optional. See [`.env.example`](.env.example) for a copy-paste template.

| Variable | Description |
|---|---|
| `GOOGLE_WORKSPACE_CLI_TOKEN` | Pre-obtained OAuth2 access token (highest priority) |
| `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` | Path to OAuth credentials JSON (user or service account) |
| `GOOGLE_WORKSPACE_CLI_CONFIG_DIR` | Override config directory (default: `~/.config/gws`) |
| `GOOGLE_WORKSPACE_CLI_SANITIZE_TEMPLATE` | Default Model Armor template |
| `GOOGLE_WORKSPACE_CLI_SANITIZE_MODE` | `warn` (default) or `block` |
| `GOOGLE_WORKSPACE_CLI_LOG` | Log level for stderr (e.g., `gws=debug`). Off by default. |
| `GOOGLE_WORKSPACE_CLI_LOG_FILE` | Directory for JSON log files with daily rotation. Off by default. |
| `GOOGLE_WORKSPACE_PROJECT_ID` | GCP project ID override for quota/billing and fallback for helper commands |

Environment variables can also be set in a `.env` file (loaded via [dotenvy](https://crates.io/crates/dotenvy)).

## Exit Codes

`gws` uses structured exit codes so scripts can branch on the failure type without parsing error output.

| Code | Meaning | Example cause |
|------|---------|---------------|
| `0` | Success | Command completed normally |
| `1` | API error | Google returned a 4xx/5xx response |
| `2` | Auth error | Credentials missing, expired, or invalid |
| `3` | Validation error | Bad arguments, unknown service, invalid flag |
| `4` | Discovery error | Could not fetch the API schema document |
| `5` | Internal error | Unexpected failure |

```bash
gws drive files list --params '{"fileId": "bad"}'
echo $?   # 1 — API error

gws unknown-service files list
echo $?   # 3 — validation error (unknown service)
```

## Architecture

`gws` uses a **two-phase parsing** strategy:

1. Read `argv[1]` to identify the service (e.g. `drive`)
2. Fetch the service's Discovery Document (cached 24 h)
3. Build a `clap::Command` tree from the document's resources and methods
4. Re-parse the remaining arguments
5. Authenticate, build the HTTP request, execute

All output — success, errors, download metadata — is structured JSON.

## Troubleshooting

### API not enabled — `accessNotConfigured`

If a required Google API is not enabled for your GCP project, you will see a
403 error with reason `accessNotConfigured`:

```json
{
  "error": {
    "code": 403,
    "message": "Gmail API has not been used in project 549352339482 ...",
    "reason": "accessNotConfigured",
    "enable_url": "https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=549352339482"
  }
}
```

`gws` also prints an actionable hint to **stderr**:

```
💡 API not enabled for your GCP project.
   Enable it at: https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=549352339482
   After enabling, wait a few seconds and retry your command.
```

**Steps to fix:**

1. Click the `enable_url` link (or copy it from the `enable_url` JSON field).
2. In the GCP Console, click **Enable**.
3. Wait ~10 seconds, then retry your `gws` command.

## Development

```bash
cargo build                       # dev build
cargo clippy -- -D warnings       # lint
cargo test                        # unit tests
./scripts/coverage.sh             # HTML coverage report → target/llvm-cov/html/
```

## License

Apache-2.0

## Disclaimer

> [!CAUTION]
> This is **not** an officially supported Google product.
