# aipowergateway

> LAN compute sharing gateway — share your model access (DeepSeek, Kimi, Zhipu GLM) with your team over the local network. Rust + system tray.

## Overview

AIPowerLink gateway lets one person (the **leader**) share their LLM API access with others (the **members**) on the same LAN:

- Members install the client, auto-discover the leader, and start calling models — **passwordless, zero config**
- One binary, dual role: `--role server` (leader) or `--role client` (member)
- **Dual protocol**: OpenAI-compatible and Anthropic-compatible (Claude Code ready)
- **Multi-backend**: share DeepSeek, Kimi, Zhipu GLM simultaneously — route by model name
- Leader sees per-member token usage and source IP / gateway ID; can ban & unban members (persisted)
- Works offline on LAN — no cloud dependency

## Quick Start

### Build

```bash
# Requires: Rust 1.94+ (MSVC on Windows) + Node 18+ (for web console)
cargo build --release -p aipg-cli

# Build web console (optional)
cd web && npm install && npm run build
```

### Leader (server role)

```bash
# Local mock backend (verify the flow)
aipowergateway --role server

# Share DeepSeek
AIPOWERLINK_DEEPSEEK_API_KEY=sk-xxx aipowergateway --backend deepseek

# Share multiple backends at once
AIPOWERLINK_DEEPSEEK_API_KEY=sk-ds AIPOWERLINK_KIMI_API_KEY=sk-kimi aipowergateway --backend deepseek,kimi,zhipu

# Passwordless: members connect without a password (0.2.0+)
```

### Model settings (panel, DeepSeek-Harness style)

Open the console (`http://127.0.0.1:39091/`) → **Models**. Here you can:

- **Add provider** — pick DeepSeek / Kimi / Zhipu (or **Add custom provider** for any OpenAI-compatible endpoint) and fill the API key directly, or reference an env var by name.
  - cc-switch style **standard presets**: choosing a built-in provider auto-fills its official base URL and standard model list (e.g. `deepseek-chat`, `deepseek-reasoner`) — add/remove models as chips, or hit **Use standard models** to reset.
- **Edit / Delete** — models (a provider can serve several), base URL and key survive edits that don't touch them; changes are saved to `data_dir/backends.yaml` and hot-applied to routing **without restart**.
- **Test** — cc-switch style connectivity check: from the form (tests what you typed, nothing is saved) or from any card (uses the saved key), the gateway issues `GET {base_url}/models` with a 5s timeout and reports latency on success, or the HTTP status with auth hints (401/403/429) / connection error on failure. Mock backends are verified locally without any network.
- **Auto-connection & status dot** (DeepSeek Harness style) — saving a backend immediately probes it, and opening the panel re-probes every configured backend in the background. Each card carries a status dot: **green** = configuration valid (hover shows latency), **red** = last test failed (hover shows the reason), **grey** = not tested yet.
- **Fetch the provider's actual model list** (cc-switch style) — the **Fetch models** button probes the endpoint with the current form values (OpenAI-compatible `GET {base_url}/models` → `data[].id`, deduplicated) and fills the model chips with the real list the model server offers. Saving a provider **without** an explicit model list auto-fetches the real list and persists it, so the provider immediately serves exactly the models the server exposes; explicitly configured model lists are never overwritten. **Filling in an API key also triggers the fetch automatically** (DeepSeek Harness style): a second after you stop typing, the latest model list from the model server replaces the chips — no button needed (mock has no network; custom providers need a base URL first).

Config is stored as a `providers` list in `backends.yaml` (like DSH `providers:`). Direct keys are stored in the file and shown masked (`sk-***abcd`); env-var references never touch disk and display as `env:NAME`. CLI flags (`--backend` / env vars) only seed initial entries — the file wins afterwards.
After starting:
- Console: open http://127.0.0.1:39091/ in a browser
- Members auto-discover via UDP broadcast (port 39090)

### Connection info (panel)

Open the **Connect** page in the console — it shows everything needed to point a client
(cc-switch / Cherry Studio / curl) at this gateway:

- **OpenAI-compatible endpoint** — the base URL with this machine's auto-detected LAN IP
- **Member token** — a copyable curl command that exchanges a machine name for a token
  (passwordless; one machine name per device so the panel meters and governs per machine)
- **Exposed models** — the live model list served by GET /v1/models
- **cc-switch fields** — ready-to-fill provider preset (name / API address / API key)

### Member (client role)

```bash
aipowergateway --role client
# Auto-discover leader -> connect (passwordless) -> call models
```

## Supported Protocols (choose one)

| Protocol | Endpoint | Client examples |
|----------|----------|-----------------|
| **OpenAI-compatible** | `POST /v1/chat/completions` | Any OpenAI-compatible tool (curl, Cursor, Open WebUI) |
| **Anthropic-compatible** | `POST /v1/messages` (SSE streaming) | Claude Code via `ANTHROPIC_BASE_URL` |

### Claude Code

```bash
export ANTHROPIC_BASE_URL=http://<leader-ip>:39091
export ANTHROPIC_AUTH_TOKEN=<member-token>
export ANTHROPIC_MODEL=deepseek-chat   # or kimi-2.7-code, etc.
```

### Model catalog (what the leader shares)

```bash
curl http://<leader-ip>:39091/v1/models
# e.g. deepseek-chat / kimi-2.7-code / glm-4-flash
```

## Supported Official Models

| Provider | Env var | Default model |
|----------|---------|---------------|
| DeepSeek | `AIPOWERLINK_DEEPSEEK_API_KEY` | deepseek-chat |
| Kimi (Moonshot) | `AIPOWERLINK_KIMI_API_KEY` | moonshot-v1-8k |
| Zhipu GLM | `AIPOWERLINK_ZHIPU_API_KEY` | glm-4-flash |
| Custom | `AIPOWERLINK_BASE_URL` + `AIPOWERLINK_MODEL` | — |

Model-name prefix routing: `deepseek-*` -> DeepSeek, `kimi-*` -> Kimi, `glm-*` -> Zhipu.

## Configuration

```bash
# Read/write config (secrets auto-encrypted and redacted)
aipowergateway config set port 39091          # listening port (default 39091)
aipowergateway config set bind 127.0.0.1      # local-only (default 0.0.0.0 = LAN sharing)
aipowergateway config list                    # secrets shown as [set]
```

## Custom Roles

```bash
# Built-in roles are read-only; clone to customize
aipowergateway role clone server my-leader
aipowergateway role list    # server(system) client(system) my-leader(user)
aipowergateway --role my-leader   # start with custom role
```

## Member Governance

Passwordless access is governed instead of guarded:

- **Visibility** — the leader console shows each member's machine name, display name,
  **source IP** and **gateway ID** (`name:port`), online status and token usage
- **Ban** — the leader can ban a member from the console: the member and its source IP are
  blocked, all of its tokens are revoked, and the ban is persisted to `banned.json` in the
  data dir (survives restarts)
- **Unban** — removes the ban; the member can reconnect

## System Tray

- Leader: open console / start / pause sharing / quit (passwordless, 0.2.0+)
- Member: leader list / connection status / rename / usage / quit
- `--no-tray`: CLI-only mode

## Startup

- Single instance: a second `aipowergateway` launch prints `already running` and exits
- Autostart: `aipowergateway autostart enable|disable|status` (Windows registry / Linux XDG / macOS login item)

## Architecture

```
Member (OpenAI or Anthropic interface)
    |  sends model name: deepseek-chat / kimi-2.7-code
Leader gateway aipowergateway (auth + metering + broadcast + console)
    |-- deepseek-* -> DeepSeek
    |-- kimi-*     -> Kimi
    |-- glm-*      -> Zhipu GLM
    `-- mock-*     -> local mock
```

### Modules

| Crate | Responsibility |
|-------|-----------------|
| `aipg-runtime` | Microkernel: Module trait, Host, event bus, roles, i18n, data dir |
| `aipg-lan-share` | Leader: dual-protocol API, auth, members, usage, broadcast, routing, web |
| `aipg-lan-client` | Member: discovery, connect, dual-protocol calls, identity, usage |
| `aipg-config` | Config store: SQLite, role partitions, Vault encryption, redaction |
| `aipg-lan-tray` | System tray (tray-icon) |
| `aipg-cli` | CLI entry (aipowergateway) |

## Platforms

- Windows / Linux / macOS (cross-platform tray)
- Console opens in system browser

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).

---

中文版：[README.zh-CN.md](README.zh-CN.md)