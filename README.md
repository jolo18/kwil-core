# Kwil

A multi-tenant tool that writes SEO and AI-search content for B2B SaaS and micro-SaaS companies, and hosts the blogs it writes for.

Each **site** has a **knowledge library**: short, atomic entries holding what the company knows and believes, captured through an interview, its website, documents and sales call transcripts. Kwil plans a rolling **calendar** from an approved **topic map**, drafts each post only from approved entries (every claim cites one), puts drafts in a review queue, and publishes them to a hosted blog on the site's own domain.

When the library has too little to say about a planned piece, Kwil holds the piece and asks you two or three questions instead of writing filler. Opinions, positioning and anything said on your behalf always need a person's approval; checkable product facts from your own site can be approved by an AI verifier against a quoted passage.

The full spec is [jolo18/kwil-core#1](https://github.com/jolo18/kwil-core/issues/1).

## Layout

| Path | What it is |
|---|---|
| `backend/` | Rust (Axum, sqlx, Postgres). One binary with four roles: `api`, `worker`, `blog`, `dev`. |
| `backend/migrations/` | The database schema. Applied automatically at start-up. |
| `backend/tests/` | API-level integration tests against a real Postgres database. |
| `web/` | The app: Vite, React, TypeScript, TanStack Router and Query, Tailwind, shadcn-style components, Tiptap. |
| `web/e2e/` | The Playwright browser test for the main path. |
| `deploy/gcp.sh` | Google Cloud deployment (Cloud Run, Cloud SQL, Cloud Tasks, Cloud Scheduler, Cloud Storage, Secret Manager). |

## Running locally

Needs Rust, Node 22 and Postgres 16.

```bash
createdb kwil
```

```bash
cd backend && set -a && . ./.env.example && set +a && cargo run -- dev
```

```bash
cd web && npm install && npm run dev
```

Open http://localhost:5173. Hosted blogs are served at `http://<site-slug>.localhost:8081`.

With the defaults, Kwil runs with:

- **The development login.** No sign-in screen; you are `dev-user` in `Dev Organization`. It refuses to start when `APP_ENV=production`.
- **The demo AI.** A deterministic stand-in that works from the same structured input the real models get, so every feature works offline. It is not a writer: set `AI_PROVIDER=openrouter` and `OPENROUTER_API_KEY` for real drafts.
- **The in-process job worker**, which also runs the scheduled tick every minute.

### Switching on real services

| Service | Backend settings | Frontend settings |
|---|---|---|
| Clerk sign-in | `AUTH_MODE=clerk`, `CLERK_JWKS_URL`, `CLERK_ISSUER`, `CLERK_WEBHOOK_SECRET` | `VITE_CLERK_PUBLISHABLE_KEY` |
| OpenRouter | `AI_PROVIDER=openrouter`, `OPENROUTER_API_KEY` | |
| Cloudflare for SaaS (custom domains, cache purging) | `CDN=cloudflare`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ZONE_ID`, `CLOUDFLARE_CNAME_TARGET` | |
| Email for the weekly digest (Resend) | `MAILER=resend`, `RESEND_API_KEY`, `MAIL_FROM` | |
| Cloud Tasks | `QUEUE=cloudtasks`, `CLOUD_TASKS_QUEUE`, `WORKER_URL`, `CLOUD_TASKS_SERVICE_ACCOUNT` | |
| Cloud Storage | `STORAGE=gcs`, `GCS_BUCKET` | |

All settings are listed in `backend/src/config.rs`. Model names per tier are set per site in the app.

## Tests

```bash
cd backend && cargo test
```

Each integration test creates its own database on the Postgres at `TEST_DATABASE_URL` (default `postgres://postgres@localhost:5432/postgres`). Outside services are faked at the edge: a scripted AI provider, the in-process queue, test-signed Clerk tokens, in-memory storage, and fake Cloudflare, GitHub and web fetching. The clock is set by each test.

```bash
cd web && npx playwright test
```

The browser test starts the backend and web app if they aren't running, then goes from creating a site through the interview, library, topic map, calendar, drafting and review to reading the published post on the hosted blog. Set `PW_CHROMIUM` to use a pre-installed Chromium.

## Deploying

`deploy/gcp.sh` sets up Google Cloud end to end; read it first, as it has not been run against a real project. The web app goes to Firebase Hosting (`web/firebase.json`). Cloudflare for SaaS sits in front of the blog service for custom domains.
