# AGENTS.md

Guidance for AI coding agents working in this repository. Human-facing docs live in
[README.md](./README.md) and [CONTRIBUTING.md](./CONTRIBUTING.md).

## Project overview

CC Switch is a cross-platform desktop app (Tauri 2) that manages and switches providers for AI
coding CLIs and editors: Claude Code, Claude Desktop, Codex, Gemini CLI, and Grok Build. It also
manages MCP servers, prompts, Skills, project profiles, session history, and an optional local
routing proxy.

- **Frontend**: React 18 + TypeScript, Vite 7, TailwindCSS 3.4, shadcn/ui (Radix), TanStack Query v5
- **Backend**: Rust, Tauri 2, rusqlite (bundled SQLite), tokio, axum/hyper/reqwest for the proxy
- **UI languages**: Simplified Chinese (`zh`) and English (`en`)

```
src/                 Frontend (React + TypeScript)
src/components/      One directory per feature (providers, mcp, prompts, skills, sessions, proxy, ...)
src/lib/api/         Tauri invoke wrappers; src/lib/query/ holds TanStack Query config
src/hooks/           Business logic hooks
src/i18n/locales/    UI strings: zh.json, en.json
src-tauri/src/       Backend (Rust): commands/ -> services/ -> database/
src-tauri/src/proxy/ Local routing: forwarding, format conversion, failover, billing
tests/               Frontend tests (vitest); tests/msw/ holds the Tauri mocks
```

## Setup

Requires Node.js 20.19+ or 22.12+ (`.node-version` pins 22.12.0), pnpm 10 (pinned by
`packageManager`; run `corepack enable` once), and the pinned Rust toolchain from
`rust-toolchain.toml` (rustup installs it automatically). See [CONTRIBUTING.md](./CONTRIBUTING.md)
for per-platform Tauri prerequisites.

```bash
pnpm install
pnpm dev          # Vite on http://localhost:3000, then the Tauri window; fails if 3000 is taken
```

On a fresh clone, create the frontend output directory before running any cargo command under
`src-tauri/`, or compilation fails on the missing path:

```bash
mkdir -p dist     # or: pnpm build:renderer
```

## Commands

Frontend (repository root):

| Command | Purpose |
| --- | --- |
| `pnpm typecheck` | TypeScript check, strict; unused imports and parameters are errors |
| `pnpm format:check` / `pnpm format` | Prettier check / write, `src/` only, default config |
| `pnpm test:unit` / `pnpm test:unit:watch` | vitest run / watch |
| `pnpm build:renderer` | Frontend-only build into `dist/` |
| `pnpm build` | Production bundle; needs `TAURI_SIGNING_PRIVATE_KEY` for updater signing |

Backend (run inside `src-tauri/`):

| Command | Purpose |
| --- | --- |
| `cargo fmt` / `cargo fmt --check` | Format / verify Rust formatting |
| `cargo clippy -- -D warnings` | Lint; CI treats warnings as errors |
| `cargo test` | Run backend tests |
| `cargo nextest run --profile ci` | Same tests the way CI runs them (per-test process, 180s timeout) |

There is no ESLint in this project. Run the full pre-submit gate:

```bash
pnpm typecheck && pnpm format:check && pnpm test:unit && pnpm build:renderer
cd src-tauri && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

## Architecture rules

- **Layered backend**: Tauri commands validate parameters and forward to services; services hold
  business logic; DAOs touch the database. Keep the command layer thin — no business logic there.
- **SQLite is the single source of truth** for providers, MCP, prompts, Skills, projects, and usage
  data (`~/.cc-switch/cc-switch.db`). Machine-only settings (directory overrides, backup policy)
  live in `~/.cc-switch/settings.json`.
- **Write live configs atomically** (temp file + rename) through the existing `*_config.rs` /
  `live/` writers. Never write `~/.claude`, `~/.codex`, `~/.gemini`, or similar paths directly.
- **Switch-mode writes**: backfill the current live config into the current provider before writing
  the new one. Preserve the model the target tool already uses.
- **New Tauri commands**: name the Rust function in `snake_case`, register it in the
  `tauri::generate_handler!` list in `src-tauri/src/lib.rs`, and add a typed wrapper in
  `src/lib/api/`. The frontend calls `invoke` with **camelCase** argument names (`provider_id` in
  Rust becomes `providerId` on the frontend).
- **Proxy code is separate** from live-config writing. Do not couple routing changes into the
  config writers.

## Code style

- Formatting is enforced by Prettier (`src/`) and `cargo fmt`; do not hand-format or restate style
  rules in reviews — run the tools.
- TypeScript is strict with `noUnusedLocals` / `noUnusedParameters`; remove dead imports rather
  than leaving them.
- Use the `@/` path alias for imports from `src/`.
- Prefer the existing shadcn/ui components in `src/components/ui/` over new ad-hoc primitives.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(provider): ...`, `fix(tray): ...`, `docs(readme): ...`, `ci: ...`, `chore(deps): ...`.

## Testing

- **Frontend**: vitest runs `*.test.ts(x)` under `src/` and `tests/` in a jsdom environment. Tauri
  calls are mocked with MSW (`tests/msw/`); component tests use `@testing-library/react`. Add
  component tests under `tests/components/`, config/preset tests under `tests/config/`.
- **Backend**: unit tests live next to the source in `#[cfg(test)]` modules; integration tests live
  in `src-tauri/tests/`. Prefer `cargo nextest run --profile ci` to match CI behavior.
- **Isolate the real user config**: some backend tests read and write `~/.cc-switch` and
  `~/.codex`. Point `CC_SWITCH_TEST_HOME` at a temporary directory instead of changing `HOME`
  (changing `HOME` also moves the Cargo and rustup caches and forces a full rebuild):

  ```bash
  CC_SWITCH_TEST_HOME="$(mktemp -d)" cargo test
  ```

- Any user-visible change needs tests that fail before the fix. Do not weaken or delete an existing
  assertion to make a change pass.

## Internationalization

Every user-facing string goes through i18next's `t()` and must be added to **both** locale
files: `src/i18n/locales/{zh,en}.json`. Never hardcode user-visible text. Missing keys fall
back to English at runtime instead of failing the build, so verify the locale files yourself —
`tests/config/localeCoverage.test.ts` guards key parity.

## Boundaries

- Do not edit generated output: `dist/`, `src-tauri/target/`, `pnpm-lock.yaml` (unless dependencies
  actually changed), or `src-tauri/Cargo.lock` by hand.
- Do not commit credentials, API keys, or real provider configs. Preset files under `src/config/`
  must contain placeholders, never live secrets.
- Do not add a provider preset for a relay or aggregator service that is not a project sponsor; see
  the Provider Presets section of [CONTRIBUTING.md](./CONTRIBUTING.md).
- Keep changes scoped to the task. Do not reformat unrelated files, rename public APIs, or bump
  dependencies as a side effect.
- Keep `README.md` consistent when behavior described there changes.

## Pull requests

Open an issue before starting new features. Keep each PR to one feature or fix, fill in the PR
template, and confirm before submitting:

- [ ] `pnpm typecheck`, `pnpm format:check`, `pnpm test:unit`, `pnpm build:renderer` pass
- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` pass when Rust changed
- [ ] Both locale files updated when user-facing text changed

AI-assisted contributions are welcome, but you own the result: every line must be understood and
verified locally. See the AI-Assisted Contributions section of [CONTRIBUTING.md](./CONTRIBUTING.md).
