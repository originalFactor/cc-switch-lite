<div align="center">

# CC Switch

### The All-in-One Manager for Claude Code, Claude Desktop, Codex, Gemini CLI & Grok Build

**Switch API providers in one click and manage MCP, Skills, and Prompts in one place — no more hand-editing JSON / TOML / YAML config files.**

[![Version](https://img.shields.io/github/v/release/originalFactor/cc-switch-lite?color=blue&label=version)](https://github.com/originalFactor/cc-switch-lite/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/originalFactor/cc-switch-lite/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/originalFactor/cc-switch-lite/total)](https://github.com/originalFactor/cc-switch-lite/releases/latest)

**[Download](#download--installation) · [Quick Start](#quick-start) · [Features](#features) · [FAQ](#faq)**

</div>

## Why CC Switch?

AI coding tools like Claude Code, Codex, and Gemini CLI each have their own configuration format. Switching API providers means hand-editing JSON, TOML, YAML, or `.env` files, and MCP, Skills, and prompts have to be maintained separately in every tool.

**CC Switch** brings all of this into a single desktop app: enter the endpoint and key, and switch in one click. Switching replaces only the connection details — endpoint, key, and model — so plugins, hooks, MCP, and comments you added yourself stay as they are.

- **One App, Five Tools** — Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build
- **No More Manual Editing** — Universal provider form: enter endpoint and key once; search models by name and domain, and the form knows providers like AWS Bedrock, NVIDIA NIM, and community relays
- **Direct or Routing — Your Choice** — Direct connects the tool straight to the provider; Routing forwards requests through your machine, converting API formats and failing over automatically, so you can use GPT in Claude Code and Claude in Codex. One switch in the app page header flips between the two and keeps the same provider
- **Centralized MCP, Skills & Prompts** — Add MCP servers and Skills once, then choose which tools to sync them to; prompts are maintained separately for each tool
- **Sessions You Can Read** — Turns each tool's session logs into a reading view: what each turn did, which files changed, and which step failed, all at a glance
- **Usage & Quotas at a Glance** — Track token usage and spending even without local routing; remaining subscription quota and account balances show right on provider cards and in the tray
- **Cross-Platform** — Native desktop app for Windows, macOS, and Linux, built with Tauri 2

## Download & Installation

### System Requirements

- **Windows**: Windows 10 and above
- **macOS**: macOS 12 (Monterey) and above
- **Linux**: x86_64 or ARM64 with glibc 2.35+ and WebKitGTK 4.1 — e.g. Ubuntu 22.04+, Debian 12+, and recent Fedora releases; RHEL / Rocky / Alma 8–9 are not supported yet

### Windows Users

Download the latest `CC-Switch-v{version}-Windows.msi` installer or `CC-Switch-v{version}-Windows-Portable.zip` portable version from the [Releases](../../releases) page. On Windows on ARM, download `CC-Switch-v{version}-Windows-arm64.msi` or `CC-Switch-v{version}-Windows-arm64-Portable.zip`.

### macOS Users

**Method 1: Install via Homebrew (Recommended)**

```bash
brew install --cask cc-switch
```

Update:

```bash
brew upgrade --cask cc-switch
```

**Method 2: Manual Download**

Download `CC-Switch-v{version}-macOS.dmg` (recommended) or `.zip` from the [Releases](../../releases) page. It's a Universal build that runs natively on both Apple Silicon and Intel Macs.

> **Note**: CC Switch for macOS is code-signed and notarised by Apple. You can install and open it directly.

### Arch Linux Users

**Install via paru (Recommended)**

```bash
paru -S cc-switch-bin
```

### Linux Users

Download the latest Linux build from the [Releases](../../releases) page:

- `CC-Switch-v{version}-Linux-x86_64.deb` / `-Linux-arm64.deb` (Debian/Ubuntu)
- `CC-Switch-v{version}-Linux-x86_64.rpm` / `-Linux-arm64.rpm` (Fedora and other RPM distros that ship WebKitGTK 4.1)
- `CC-Switch-v{version}-Linux-x86_64.AppImage` / `-Linux-arm64.AppImage` (any distro meeting the requirements above)

## Quick Start

### Basic Usage

1. **Add Provider**: Select the tool you want to manage in the sidebar, then click "+" (Add Provider) in the top-right corner of the page → Fill in the endpoint and key (adding always creates a universal provider that syncs to the apps you enable) — no more picking from a preset list
2. **Switch Provider**:
   - Main UI: Select provider → Click "Enable"
   - System Tray: One row per tool showing "name · mode · provider · quota"; click a provider name in its submenu to switch (Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build)
3. **Takes Effect**: Claude Code needs no restart; for Codex, Gemini CLI, and Grok Build, restart your terminal or the CLI tool; for Claude Desktop, restart the app itself (see FAQ)
4. **Back to Official Login**: Switch to the built-in official provider in the list (e.g. "Claude Official"), restart the tool, then follow its login/OAuth flow
5. **Routing (optional)**: The Claude Code, Codex, Gemini CLI, and Grok Build pages have one routing switch in the header. Turn it on to send the current provider's requests through local routing, turn it off to go back to direct with the same provider. Use Routing to use OpenAI- or Gemini-format providers in Claude Code, or Claude in Codex
6. **Manage Tools**: The "Apps" page in the sidebar shows each AI coding tool's version, install location, and source in one place; you can install, upgrade, upgrade all, or uninstall (and pick which duplicate to remove when a tool is installed in several places), and choose which tools appear in the sidebar

### MCP, Prompts, Skills, Projects & Sessions

- **MCP**: Click "MCP" in the sidebar → Add servers via templates or custom config (or "Import Existing") → Toggle sync for each tool
- **Prompts**: Click "Prompts" in the sidebar → Create prompts with the Markdown editor → Enable one to write it to that tool's prompt file
- **Skills**: Click "Skills" in the sidebar → "Discover Skills" → Search skills.sh or browse GitHub repos → One-click install to supported tools
- **Projects**: First turn on "Show project switcher" in "Settings → General", then on the Claude Code, Claude Desktop, or Codex page, open the project switcher at the top → "New project" to save the current configuration; later, pick it from the switcher to switch the whole setup at once
- **Sessions**: Click "Sessions" in the sidebar → Browse and search conversation history, open the reading view, or restore each tool's sessions

> **Note**: On first launch, CC Switch automatically imports your existing Claude Code, Codex, Gemini CLI, and Grok Build configuration as a provider named `default` and adds an official provider for each of these tools and Claude Desktop, so nothing you had configured is lost.

## Features

### Supported Features by Tool

| Tool | Providers | Local Routing | Tray Switching | MCP | Skills | Prompts | Sessions | Usage Stats |
| --- | --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Claude Code | Switch | ✓ | ✓ | ✓ | ✓ | CLAUDE.md | ✓ | ✓ |
| Claude Desktop | Switch | Model Mapping only | ✓ | – | – | – | – | Model Mapping only |
| Codex | Switch | ✓ | ✓ | ✓ | ✓ | AGENTS.md | ✓ | ✓ |
| Gemini CLI | Switch | ✓ | ✓ | ✓ | ✓ | GEMINI.md | ✓ | ✓ |
| Grok Build | Switch | ✓ | ✓ | ✓ | ✓ | AGENTS.md | ✓ | ✓ |

- **Switch**: only one provider is active at a time.
- **Local Routing**: CC Switch forwards requests on your machine and converts API formats; see [Local Routing & Failover](#local-routing--failover) below. Claude Desktop providers can use "Direct" or "Model Mapping"; with "Model Mapping", requests go through local routing.
- **Sessions**: Browse and search conversation history, open the reading view, and copy a resume command to continue a conversation.
- **Usage Stats**: Without local routing, usage is collected from each tool's local session logs; requests that go through local routing are counted as well.
- The MCP, Skills, Prompts, and Sessions panels opened from the Claude Desktop page apply to Claude Code.

### Provider Management

- **Universal provider form** — Every provider you add is a universal provider: fill in the endpoint and key once, enable the apps it should sync to, and one configuration is shared by Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build. The form searches models by name and domain, shows what the config file will look like after switching, and CC Switch automatically backs up each config file before rewriting it for the first time
- **Key fields only** — Switching replaces only the connection details such as the endpoint, key, and model; plugins, hooks, MCP, settings you added yourself, and comments stay as they are. The editor shows what the config file will look like after switching, and CC Switch automatically backs up each config file before rewriting it for the first time
- **Projects** — Save Claude Code's or Codex's current provider, MCP, Skills, and prompt files as a project (for Claude Desktop, only the provider is saved), then switch the whole setup in one click from the project switcher at the top of the page or from the tray; when you switch to another project, the current state is automatically saved back to the previous project. The project switcher is hidden by default; turn on "Show project switcher" in "Settings → General"
- **Third-party providers for Claude Desktop** — Connect directly to Anthropic-compatible endpoints; for non-Claude models, choose "Model Mapping" to map tiers like Sonnet, Opus, and Haiku to the provider's actual models through local routing
- **Universal providers** — One config syncs to all five apps: Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build
- One-click switching, system tray quick switching (Claude Code, Claude Desktop, Codex, Gemini CLI, Grok Build), drag-and-drop sorting, import/export

### Local Routing & Failover

- **API format conversion** — Local routing converts requests between Anthropic Messages, OpenAI Chat Completions, OpenAI Responses, and Gemini Native: Claude Code and Claude Desktop can use OpenAI- or Gemini-format providers, and Codex and Grok Build can use Chat Completions or Anthropic Messages providers
- **Per-tool toggle** — Claude Code, Codex, Gemini CLI, and Grok Build can each enter Routing mode separately with the routing switch in the app page header; it routes the provider that is currently in direct use. In Routing mode, switching providers takes effect immediately for subsequent requests (Codex, Gemini CLI, and Grok Build may still need a restart if the switch changes the model). When you quit CC Switch, it writes the direct configuration back first and reconnects the next time it starts
- **Auto-failover** — Configure a failover queue for each tool; when a request fails, CC Switch automatically moves on to the next provider in the queue, backed by a circuit breaker and provider health monitoring
- **Rectifier** — Automatically fixes certain requests that some upstreams can't handle (e.g. Thinking signatures, or falling back when images aren't supported)
- Official providers (e.g. Claude Official) can't go through local routing (except Codex's OpenAI Official)
- The routing service's status, listen address, and failover settings are all in "Settings → Local routing"

### MCP, Prompts & Skills

- **Unified MCP panel** — Manage all MCP servers in one place, choose which tools to sync each one to (Claude Code, Codex, Gemini CLI, Grok Build), import existing configs from each tool, and import via Deep Link
- **Prompts** — A per-tool prompt library with a Markdown editor; enabling a prompt writes it to that tool's prompt file (CLAUDE.md / AGENTS.md / GEMINI.md), and the file's existing content is saved back to the library first, so nothing is lost
- **Skills** — Search skills.sh, or install in one click from GitHub repos or ZIP files; check for updates and update all in one click; sync to each tool automatically, via symlinks only, or via copies only; you can optionally store them in `~/.agents/skills`
- All three panels support search, and MCP and Skills can also be enabled or disabled all at once per tool; providers, MCP servers, and prompts are edited full-page in the content area, and you're warned before leaving with unsaved changes

### Usage & Cost Tracking

- **Usage dashboard** — Works without local routing: by default it automatically scans each tool's local session logs and tracks requests, tokens, cache hit rate, output speed, and spending by provider and model, with trend charts, a full-year heatmap, and per-request logs
- **Quotas & balances** — Provider cards and the tray show what's left and the reset countdown, e.g. "5h 94% left": official subscription quotas (Claude, ChatGPT, Gemini, SuperGrok), Coding Plan 5-hour/weekly/monthly quotas (Kimi, Zhipu GLM, MiniMax, Volcengine Ark, etc.), and account balances (DeepSeek, OpenRouter, SiliconFlow, etc.); some need to be turned on first via "Configure usage query" on the provider card, and for other providers you can write a custom usage script. ChatGPT subscriptions also show remaining limit resets and the Codex Credits balance
- **Custom pricing** — Set per-model prices, or import them from models.dev

### Session Manager

- **Session reading view** — Parses each tool's session logs into structured content: text, thinking, tool calls, tool results, and images are shown separately; each turn of work collapses into a one-line summary (how many steps, how many files changed, how many failures), with failed steps marked in red; a conversation outline sits on the right, with in-session search, Markdown export, and copying a single turn or the whole conversation
- **List & resume** — Group by project or time, search titles, directories, messages, or session IDs, and delete in bulk; copy a resume command to continue a conversation, and on macOS resume in a terminal with one click

### System & Platform

- **Cloud sync** — Sync across devices via WebDAV (Jianguoyun, Nextcloud, Synology NAS, etc.) or S3-compatible storage (AWS S3, Cloudflare R2, Alibaba Cloud OSS, Tencent Cloud COS, etc.); you can also put the CC Switch configuration directory in a cloud drive folder such as Dropbox, OneDrive, or iCloud
- **CLI tool management** — On the "Apps" page in the sidebar, see the current and latest versions of command-line tools like Claude Code and Codex, install, upgrade, or upgrade all in one click, and diagnose duplicate installations; on Windows, it can also manage tools inside WSL (see FAQ)
- **Deep Link** (`ccswitch://`) — Import providers, MCP servers, and prompts, or add skill repositories, in one click via a link; imported providers become a unified provider synced to all five apps, and `enabled=true` switches only the app named in the link. Create links with the [online generator](https://ccswitch.io/deplink.html)
- **Built-in utilities** — Skip Claude Code's first-run confirmation, hide AI attribution, have the VS Code Claude Code extension follow CC Switch's provider switches, and more
- Dark / Light / System theme, auto-launch, auto-updater, atomic writes, auto-backups (see where every backup lives and how much space it takes in "Settings → Data"), i18n (zh/en)

## FAQ

<details>
<summary><strong>Which AI tools does CC Switch support?</strong></summary>

CC Switch supports five tools: **Claude Code**, **Claude Desktop**, **Codex**, **Gemini CLI**, **Grok Build**. Each tool has its own provider management; see [Supported Features by Tool](#supported-features-by-tool) for what each one supports.

</details>

<details>
<summary><strong>Do I need to restart the terminal after switching providers?</strong></summary>

It depends on the tool:

- **Claude Code**: supports hot-switching of provider data — no restart needed.
- **Codex, Gemini CLI, Grok Build**: restart your terminal or the CLI tool for changes to take effect (CC Switch reminds you after switching). In Routing mode, requests go to the new provider immediately, but all three tools may still need a restart if the switch changes the model.
- **Claude Desktop**: fully quit and reopen Claude Desktop; when using "Model Mapping", also keep CC Switch running.

</details>

<details>
<summary><strong>Will switching providers change my plugins, hooks, or other settings?</strong></summary>

No. When you switch providers for Claude Code, Codex, Gemini CLI, or Grok Build, CC Switch replaces only the **key fields** in the config file: the endpoint, key, model name, and API protocol (plus the reasoning effort for Codex and the auth method for Gemini CLI), along with a few compatibility options that belong to the provider (such as Claude Code's "Disable Artefact Tool" and the context window). Plugins, hooks, permissions, MCP, environment variables you added yourself, comments, and formatting all stay as they are and apply to every provider.

You can change these shared settings in the tool itself or by editing the config file by hand. You can also edit any provider in CC Switch: the editor shows "what the config file will look like after switching to this provider". When you save, the key fields are stored in that provider, and every other change is written to the config file and applies to every provider.

So the old "Common Config Snippet" is no longer needed, and its buttons have been removed. Settings that were in your snippet before the upgrade were already written into the config file when you switched, so they stay. Before CC Switch rewrites each config file for the first time, it also backs up the original to `~/.cc-switch/backups/live-first-write/`.

</details>

<details>
<summary><strong>I changed the model inside the tool — why does it go back after I switch away and back?</strong></summary>

The model is a key field and belongs to the provider. A model you pick inside the tool (such as with `/model` in Claude Code) stays in effect until the next switch; when you switch, the model in the config file is replaced with the one saved in the target provider, and CC Switch doesn't save the model you picked back to the previous provider. To keep using a model long-term, edit that provider in CC Switch.

Older versions saved the whole config file back to the provider when you switched away. That no longer happens: it froze plugins and other shared settings into one provider, so they were lost when you switched to another one.

</details>

<details>
<summary><strong>Why can't I delete the provider currently in use?</strong></summary>

CC Switch follows a "minimal intrusion" design principle — even if you uninstall the app, your tools will continue to work normally.

So for tools that use one active provider at a time (Claude Code, Claude Desktop, Codex, Gemini CLI, Grok Build), the system always keeps one configuration in use, because deleting all configurations would make the corresponding tool unusable; to delete it, switch to another provider first. If you rarely use a tool, you can hide it from the sidebar on the "Apps" page. To switch back to official login, see the next question.

</details>

<details>
<summary><strong>How do I switch back to official login?</strong></summary>

In CC Switch, the provider lists for Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build each include a built-in official provider (**Claude Official**, **Claude Desktop Official**, **OpenAI Official**, **Google Official**, **Grok Official**). If you deleted it, create a universal provider for that app. After switching to the official provider, follow the tool's own login flow (e.g. `/login` in Claude Code, `codex login` for Codex), and then you can freely switch between the official provider and third-party providers.

Note: official subscriptions use the tool's own login and don't go through local routing. Official providers can't be selected in Routing mode (Codex's OpenAI Official cards are the exception); switch back to Direct first.

</details>

<details>
<summary><strong>After I start routing, why does my config file point to 127.0.0.1?</strong></summary>

In Routing mode, the tool's requests first go to CC Switch's local routing (`http://127.0.0.1:15721` by default), and CC Switch then forwards them to the provider you selected. That's why the tool's config file only contains the local address and the placeholder key `PROXY_MANAGED`; for Claude Code, the model name is also written as a fixed alias such as `claude-sonnet-5` (the `/model` menu still shows the real model name). The real provider address, key, and model are all stored in CC Switch.

In the request logs under "Usage" in the sidebar, you can see "requested model → actual model" for each request.

In Routing mode, switching changes the provider that routing uses; the provider you were using before you entered Routing stays the same and is labeled "Direct" on its card. When you click "Back to direct", the config file is written back to this direct provider's configuration. Quitting CC Switch also writes back the direct provider first, and it reconnects the next time CC Switch starts.

</details>

<details>
<summary><strong>Can I use OpenAI-compatible APIs, Gemini, or local models in Claude Code?</strong></summary>

Yes, but you need to use Routing. When editing the provider, set "Upstream Format" under "Advanced Options" to match the provider's API: choose "OpenAI Chat Completions" for services that only offer a Chat Completions endpoint (as many local model servers do), "OpenAI Responses API" for services that offer a Responses endpoint, and "Gemini Native generateContent" for Gemini. Then click "Start routing" on the "Routing" tab of the Claude Code page. Choosing the wrong format or not starting routing usually results in a 404 or 405 error.

Conversely, choosing "Anthropic Messages" as the "Upstream Format" in Codex or Grok Build lets you use Claude-format providers; this also requires Routing.

</details>

<details>
<summary><strong>"Connectivity check" passed, so why do requests still fail?</strong></summary>

The "Connectivity check" on a provider card only checks whether the provider address is reachable; it doesn't send a real model request, so it can't verify whether the API key and model name are correct. When requests fail, check the key, model name, and upstream format; for requests that go through Routing, you can also see the exact error in the request logs under "Usage" in the sidebar.

</details>

<details>
<summary><strong>Where is my data stored?</strong></summary>

By default, everything is stored in the `.cc-switch` folder in your home directory (`C:\Users\<user>\.cc-switch` on Windows):

- **Database**: `cc-switch.db` (SQLite — providers, MCP, prompts, Skills, projects, usage records, etc.)
- **Local settings**: `settings.json` (device-level settings such as each tool's config directory, backup policy, and cloud sync connection details)
- **Backups**: `backups/` (backed up automatically every 24 hours by default, keeping the 10 most recent; adjustable in "Settings → Data → Backup & Restore"; "Other Backups" below it lists the location and size of every other backup, which you can clean up by category)
- **Skills**: `skills/` (can be changed to `~/.agents/skills` in Settings), synced to each tool via symlinks by default, falling back to copying if that fails
- **Skill Backups**: `skill-backups/` (created automatically before uninstalling or updating a skill, keeping the 20 most recent)
- **OAuth login credentials**: `copilot_auth.json`, `codex_oauth_auth.json`, `xai_oauth_auth.json`
- **Logs**: `logs/cc-switch.log` and `crash.log` — please attach them when reporting an issue
- **Device state**: `live-state.json` (whether each tool is currently in Direct or Routing mode, and what was last written), `codex-login-stash.json` (the official Codex login moved aside when you switch to a third-party provider, restored when you switch back to an official one)
- **Original config files**: `backups/live-first-write/` (each tool's config file as it was before CC Switch first rewrote it)

After you change "CC Switch Configuration Directory" in "Settings → Data → Location", all of the files above except `settings.json`, the device state, and the original config files are stored in the new directory. CC Switch doesn't move existing files automatically, so copy them over manually first. `settings.json`, the device state, and the original config files belong to this computer only: they always stay in the default directory and are not included in cloud sync.

</details>

<details>
<summary><strong>How do I manage tools inside WSL on Windows?</strong></summary>

CC Switch doesn't detect WSL automatically. In "Settings → App config", change the config directory for the corresponding tool to a path inside WSL, e.g. `\\wsl.localhost\Ubuntu\home\<user>\.claude`; after you save, CC Switch reads and writes the config inside WSL (supported for Claude Code, Codex, Gemini CLI, and Grok Build). Once this is set, the "Apps" page in the sidebar also detects and upgrades that tool in the corresponding WSL distribution.

Note: Routing writes `127.0.0.1` as the address into the config. In WSL2's default NAT networking mode, `127.0.0.1` inside WSL can't reach local routing on Windows; switch WSL to mirrored networking mode instead.

</details>

<details>
<summary><strong>Is there a command-line or headless version?</strong></summary>

CC Switch itself only ships as a desktop app that requires a graphical interface (see [Download & Installation](#download--installation) for system requirements). For servers, SSH sessions, or machines without a desktop environment, we recommend the community-maintained **[CC Switch CLI](https://github.com/SaladDay/cc-switch-cli)**: it offers both an interactive terminal UI (TUI) and a command-line mode, and can be installed via Homebrew (`brew install cc-switch-cli`) or an install script.

By default, CC Switch CLI shares the `~/.cc-switch` data directory with the desktop app and is compatible with the desktop app's WebDAV sync. The two projects are released separately, and the database version the CLI supports sometimes lags behind the desktop app; if you see a "database version is too new" message, upgrade the CLI or wait for it to catch up.

</details>

<details>
<summary><strong>Linux (Wayland + NVIDIA): clicks don't register and the window black-screens on resize</strong></summary>

The AppImage forces `GDK_BACKEND=x11` (XWayland) to avoid a historical native-Wayland crash. On newer Wayland + NVIDIA setups, this can leave the web content area unclickable (the title-bar buttons still work) and black-screen on resize. Launch with the opt-in escape hatch to switch back to native Wayland:

```bash
CC_SWITCH_GDK_BACKEND=wayland ./CC-Switch-*.AppImage
```

If you launch from a desktop icon, add it to the `.desktop` `Exec=` line (e.g. `env CC_SWITCH_GDK_BACKEND=wayland /path/to/AppImage`) or set it in your session environment. The variable is generic: on tiling Wayland compositors (sway/Hyprland) where clicks don't register, try `CC_SWITCH_GDK_BACKEND=x11` instead. Leaving it unset keeps the default behaviour.

</details>

## Contributing

Issues and suggestions are welcome! Before developing a new feature, please open an issue to discuss the approach first; feature PRs that aren't a good fit for the project may be closed.

**Tech stack**: Tauri 2 · Rust · React 18 · TypeScript · SQLite

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=originalFactor/cc-switch-lite&type=Date)](https://www.star-history.com/#originalFactor/cc-switch-lite&Date)

## License

MIT © Jason Young

This repository is modified by originalFactor. Commits of the original repository were squashed, but the code still credits them. See [original repo](https://github.com/farion1231/cc-switch) for details.
