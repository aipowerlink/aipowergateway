# aipowergateway

> LAN compute sharing gateway — share your model access (DeepSeek, Kimi, Zhipu GLM) with your team over the local network. Rust + system tray.

## Overview

AIPowerLink gateway lets one person (the **leader**) share their LLM API access with others (the **members**) on the same LAN:

- Members install the client, auto-discover the leader, enter the password, and start calling models — **zero config**
- One binary, dual role: `--role server` (leader) or `--role client` (member)
- **Dual protocol**: OpenAI-compatible and Anthropic-compatible (Claude Code ready)
- **Multi-backend**: share DeepSeek, Kimi, Zhipu GLM simultaneously — route by model name
- Leader sees per-member token usage, can kick/ban members instantly
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

# Set share password (default: aipowergateway)
AIPOWERLINK_PASSWORD=mysecret aipowergateway --role server
```

After starting:
- Console: open http://127.0.0.1:39091/ in a browser
- Members auto-discover via UDP broadcast (port 39090)
- Fingerprint: first 8 chars of the password hash (members can pre-verify)

### Member (client role)

```bash
aipowergateway --role client
# Auto-discover leader -> enter password -> call models
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
aipowergateway config set port 39091
aipowergateway config set password mysecret   # detected as secret
aipowergateway config list                    # secrets shown as [set]
aipowergateway config get password
```

## Custom Roles

```bash
# Built-in roles are read-only; clone to customize
aipowergateway role clone server my-leader
aipowergateway role list    # server(system) client(system) my-leader(user)
aipowergateway --role my-leader   # start with custom role
```

## System Tray

- Leader: open console / start / pause sharing / change password / quit
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