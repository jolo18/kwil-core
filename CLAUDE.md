# kwil-core

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues on `jolo18/kwil-core`, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Project

Kwil is a multi-tenant SEO content platform. The spec is GitHub issue #1; `README.md` covers running and deploying.

- `backend/`: Rust crate `kwil`. `cargo test` needs Postgres at `TEST_DATABASE_URL` (default `postgres://postgres@localhost:5432/postgres`). `cargo run -- dev` runs the API (8080), worker and blog service (8081) together.
- `web/`: Vite + React app. `npx tsc -b` typechecks; `npx playwright test` runs the browser test.
- Tests drive the system through its outer interfaces: the API router, the blog router, and the page-structure checker. Fakes exist only at the edge (`backend/src/services.rs`, `ai::llm::Scripted`, `clock::FixedClock`).
