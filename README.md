# aipowergateway

> LAN compute sharing gateway — share your model access (DeepSeek, Kimi, Zhipu GLM, CodeBuddy) with your team over the local network. Rust + system tray.

## Overview

AIPowerLink gateway lets one person (the **leader**) share their LLM API access with others (the **members**) on the same LAN:

- Members install the client, auto-discover the leader, and start calling models — **passwordless, zero config**
- One binary, dual role: `--role server` (leader) or `--role client` (member)
- **Dual protocol**: OpenAI-compatible and Anthropic-compatible (Claude Code ready)
- **Multi-backend**: share DeepSeek, Kimi, Zhipu GLM, CodeBuddy simultaneously — route by model name
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

> **Binds 127.0.0.1 by default** — the admin console, the OpenAI-compatible and the
> Anthropic-compatible endpoints all serve **this machine only**. To share with LAN
> members run `config set bind 0.0.0.0` once to expose the endpoints; sharing itself works
> through **gateway-to-gateway communication** — member gateways (`--role client`) discover
> the leader via UDP and connect to its dedicated gateway channel on `0.0.0.0:39092`
> (member-access endpoints only, token-gated; never the admin console or config API).

```bash
# Local mock backend (verify the flow)
aipowergateway --role server

# Share DeepSeek
AIPOWERLINK_DEEPSEEK_API_KEY=sk-xxx aipowergateway --backend deepseek

# Share CodeBuddy (Tencent Copilot key; CODEBUDDY_API_KEY alias also accepted)
AIPOWERLINK_CODEBUDDY_API_KEY=ck-xxx aipowergateway --backend codebuddy

# Share multiple backends at once
AIPOWERLINK_DEEPSEEK_API_KEY=sk-ds AIPOWERLINK_KIMI_API_KEY=sk-kimi aipowergateway --backend deepseek,kimi,zhipu

# Passwordless: members connect without a password (0.2.0+)
```

### Model settings (panel, DeepSeek-Harness style)

Open the console (`http://127.0.0.1:39091/`) → **Models**. Here you can:

- **Add provider** — pick DeepSeek / Kimi / Zhipu / CodeBuddy (or **Add custom provider** for any OpenAI-compatible endpoint) and fill the API key directly, or reference an env var by name.
  - cc-switch style **standard presets**: choosing a built-in provider auto-fills its official base URL and standard model list (e.g. `deepseek-chat`, `deepseek-reasoner`) — add/remove models as chips, or hit **Use standard models** to reset.
- **Add executor** (`pair://` / `agent://` presets) — connect a **supply-side executor** as an upstream provider: the home PAIR cluster (NVIDIA PAIR, idles home compute) or an aipoweredge-agent node. Pick the card, fill only the OpenAI-compatible `base_url` (the executor's `GET {base_url}/models` endpoint, key optional for local executors) and save; the gateway auto-fetches the real model list and persists it, so routing starts immediately. Cards are tagged `pair://` / `agent://` in the list.
- **Edit / Delete** — models (a provider can serve several), base URL and key survive edits that don't touch them; changes are saved to `data_dir/backends.yaml` and hot-applied to routing **without restart**.
- **Test** — cc-switch style connectivity check: from the form (tests what you typed, nothing is saved) or from any card (uses the saved key), the gateway issues `GET {base_url}/models` with a 5s timeout and reports latency on success, or the HTTP status with auth hints (401/403/429) / connection error on failure. CodeBuddy has no `/models` endpoint (Tencent returns 404) and only accepts streaming, so it is probed with a minimal streaming chat instead. Mock backends are verified locally without any network.
- **Auto-connection & status dot** (DeepSeek Harness style) — saving a backend immediately probes it, and opening the panel re-probes every configured backend in the background. Each card carries a status dot: **green** = configuration valid (hover shows latency), **red** = last test failed (hover shows the reason), **grey** = not tested yet.
- **Health polling** (supply-side executors) — executors (`pair://` / `agent://`) poll the endpoint **continuously** (`GET {base_url}/models`, interval & thresholds configurable, default 15s / 3 / 10). A per-card **four-state dot** overlays the connection test: **green = healthy**, **yellow = degraded** (consecutive failures, still routed), **red = removed** (too many failures — excluded from routing and the `GET /v1/models` catalog), **grey = polling off / not probed yet** (hover shows last failure and failure count). Configure per backend from the card: enable/disable, interval, degrade and remove thresholds (`PUT /api/backends/:id/polling`, available in the API too). Any single success returns a backend to healthy and resets the failure streak; polling is off by default for cloud providers and is zero-cost when nothing is enabled (no scheduled task spawned).
- **Usage metering & C2C fields (reserved)** — usage records carry the **provider dimension** (`provider=pair|agent` for executor traffic, alongside the cloud provider name), stored per member and returned in the panel/members API as `providerTokens`; `/api/usage/export` keeps the member-level CSV. Backend entries may carry **reserved C2C fields** `splitRatio` (sharing ratio) and `listingId` (compute-market listing id) — persisted in `backends.yaml` and echoed back via `GET /api/backends`, but **no split/revenue logic is implemented yet** (mode C). Quotas (RPM/TPM/day limits) are enforced **uniformly in the routing layer for every provider** — executors get the same 429 `quota_exceeded` as cloud providers, with no bypass.
- **Fetch the provider's actual model list** (cc-switch style) — the **Fetch models** button probes the endpoint with the current form values (OpenAI-compatible `GET {base_url}/models` → `data[].id`, deduplicated) and fills the model chips with the real list the model server offers. CodeBuddy cannot list models (`/models` is 404), so its probe returns the official catalog (`hy4-preview`, `deepseek-v4-flash`). Saving a provider **without** an explicit model list auto-fetches the real list and persists it, so the provider immediately serves exactly the models the server exposes; explicitly configured model lists are never overwritten. **Filling in an API key also triggers the fetch automatically** (DeepSeek Harness style): a second after you stop typing, the latest model list from the model server replaces the chips — no button needed (mock has no network; custom providers need a base URL first).

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

Member machines run their **own local gateway**; member tools never hit the leader directly —
the member gateway talks to the leader gateway (gateway-to-gateway):

```bash
aipowergateway --role client --data-dir <member-data-dir>
# listens on http://127.0.0.1:39091 (config set member_port to change)
# UDP auto-discovers the leader and forwards every call over its gateway channel (39092)
```

Local tools (cc-switch / Cherry Studio / Claude Code) point at the **member's own**
`http://127.0.0.1:39091`:
- `POST /auth/token` `{"machineName":"..."}` — exchanges the machine name for the
  member token (passwordless, 12h TTL), issued by the leader
- `GET /v1/models` / `POST /v1/chat/completions` / `POST /v1/messages` — forwarded
  to the leader over the gateway channel with your Bearer token
- No leader found yet → `503 no leader discovered`; leader switching is automatic (last-seen wins)

## Supported Protocols (choose one)

| Protocol | Endpoint | Client examples |
|----------|----------|-----------------|
| **OpenAI-compatible** | `POST /v1/chat/completions` | Any OpenAI-compatible tool (curl, Cursor, Open WebUI) |
| **Anthropic-compatible** | `POST /v1/messages` (SSE streaming) | Claude Code via `ANTHROPIC_BASE_URL` |

### Claude Code

```bash
export ANTHROPIC_BASE_URL=http://<leader-ip>:39091
export ANTHROPIC_AUTH_TOKEN=<member-token>
export ANTHROPIC_MODEL=deepseek-chat   # or kimi-2.7-code / hy4-preview / deepseek-v4-flash, etc.
```

### Model catalog (what the leader shares)

```bash
curl http://<leader-ip>:39091/v1/models
# e.g. deepseek-chat / kimi-2.7-code / glm-4-flash / hy4-preview / deepseek-v4-flash
```

## Supported Official Models

| Provider | Env var | Default model |
|----------|---------|---------------|
| DeepSeek | `AIPOWERLINK_DEEPSEEK_API_KEY` | deepseek-chat |
| Kimi (Moonshot) | `AIPOWERLINK_KIMI_API_KEY` | moonshot-v1-8k |
| Zhipu GLM | `AIPOWERLINK_ZHIPU_API_KEY` | glm-4-flash |
| CodeBuddy (Tencent Copilot) | `AIPOWERLINK_CODEBUDDY_API_KEY` (or `CODEBUDDY_API_KEY`) | deepseek-v4-flash |
| Custom | `AIPOWERLINK_BASE_URL` + `AIPOWERLINK_MODEL` | — |

Model-name prefix routing: `deepseek-*` -> DeepSeek, `kimi-*` -> Kimi, `glm-*` -> Zhipu. CodeBuddy has no shared model-name prefix (its models `hy4-preview` / `deepseek-v4-flash` are routed by exact name).

## Configuration

```bash
# Read/write config (secrets auto-encrypted and redacted)
aipowergateway config set port 39091          # listening port (default 39091)
aipowergateway config set bind 0.0.0.0       # LAN sharing (default is 127.0.0.1 = local-only)
aipowergateway config list                    # secrets shown as [set]
```

### Link encryption (leader policy)

`link.encrypt` controls leader-side link encryption (AES-256-GCM over the member↔leader
link, negotiated with the `x-aipg-enc: v1` header):

- `aes-gcm` (leader default): negotiated — encrypted requests are decrypted, plaintext
  requests pass through untouched (full backward compatibility with old members)
- `enforce`: mandatory — `POST /api/control {"action":"link-encrypt","mode":"enforce"}`
  or the console switch makes the leader reply **426 Upgrade Required** to any unencrypted
  `/v1/*` model request (admin `/api/*` and `/auth/*` stay plaintext-reachable so the
  console/token exchange can never lock itself out); encrypted traffic works as usual
- `off`: plaintext pass-through only

```bash
aipowergateway config set link.encrypt aes-gcm   # leader default (negotiated)
aipowergateway config set link.encrypt enforce   # mandatory: unencrypted /v1/* → 426
```

The console's Controls tab also has a three-state Link encryption switch; the change is
persisted to `link-encrypt.json` in the data dir and takes precedence over the config on
restart. Members read the same key on their side: `off` (default) sends plaintext, any
other value encrypts cross-network deep-link traffic.

### Load whitelist redline (mining / deepfake)

Of the four compliance red lines (mining / deepfake / data egress / unlicensed payments),
**mining and deepfake load blocking lives on the gateway side**: the cloud never touches
content, and the gateway runs an **in-memory only** keyword check
(`crates/lan-share/src/policy.rs`) over the plaintext going upstream. Mining/deepfake
requests get **403** `blocked by load whitelist: mining|deepfake`.

- **Zero knowledge**: the check happens only inside the gateway process — nothing is
  persisted, uploaded, or logged (tracing records only the category and a counter, never
  the content itself)
- **On by default** (the red line is enforced from the first line of code); disable via
  the console Controls tab "Load redline block" switch or
  `POST /api/control {"action":"load-policy","enabled":false}`; persisted to
  `load-policy.json` (file takes precedence on restart)
- Member-gateway share channels reuse the same policy, so forwarded member traffic is
  covered automatically

### Rule execution engine (model = rule-set name)

A 1→N token distribution option: one gateway owner can bundle a set of real upstream
models behind a logical name and hand that name to any number of users. The `model` field
in a request may then be a **rule-set name** instead of a real upstream model: the
gateway resolves the name into an ordered list of candidate real models and picks the
cheapest one whose context fits the request. Users keep using a stable name and never see
real model names. Rule sets are fully local (no cloud dependency); cloud-hosted
management is a possible future addition, not a prerequisite.

- Rule sets load from `data_dir/model-rule-set.json` at startup (single object or array;
  missing/corrupt file → empty resolver, gateway keeps serving; UTF-8 BOM tolerated)
- Schema: `{id, name, version, rules:[{match_model, order, strategy,
  candidates:[{model, max_prompt_tokens}], fallback:[...]}]}`
- Rule selection: `match_model=="*"` first, else exact `match_model`, else smallest
  `order`; `strategy: token_tier` (default) filters candidates whose
  `max_prompt_tokens` fits the estimated prompt tokens, sorts ascending
  (smallest context = cheapest first, unbounded candidates last, `fallback` appended),
  `strategy: fixed` keeps original candidate order; a rule name with no rule matched
  (or a non-rule model) keeps original behavior (routed as a real model name)
- Token estimation is heuristic (roughly prompt chars / 4) — no real tokenizer
- Fallback: non-streaming requests loop through candidates, advancing to the next on an
  unroutable candidate or a retryable upstream error (429/5xx/timeout/conn), 502 when all
  fail; streaming and Anthropic (`/v1/messages`) requests use candidate[0] only
- Matched responses carry `X-APL-Rule` (rule name, empty when unmatched) and
  `X-APL-Upstream` (final real model) headers
- Rule-set names appear in `/v1/models` (both OpenAI & Anthropic formats), `/api/models`,
  `/api/info` (`rules` / `ruleSetCount`)
- Management: `GET /api/rules` lists loaded sets; `POST /api/rules` saves
  `model-rule-set.json` and hot-reloads without restart; the console Controls tab has a
  "Rule sets" editor card
- Telemetry: per-member usage gains a `ruleSetTokens` dimension (`rule name → tokens`),
  aggregated without double-counting the member total

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
    |  sends model name: deepseek-chat / kimi-2.7-code / hy4-preview
Leader gateway aipowergateway (auth + metering + broadcast + console)
    |-- deepseek-* -> DeepSeek
    |-- kimi-*     -> Kimi
    |-- glm-*      -> Zhipu GLM
    |-- CodeBuddy  -> exact model name (hy4-preview / deepseek-v4-flash; no prefix)
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