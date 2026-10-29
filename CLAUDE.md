# CLAUDE.md

@AGENTS.md

## Overview

Scenario Manager monorepo (pnpm workspaces + Turborepo; pnpm 12.3.4, Node 20 in CI). Lets users configure and run scenarios against maps, bots and PPS data.

| Path | What it is |
| --- | --- |
| `apps/web` | Next.js 16 / React 19 frontend (Tailwind 4, shadcn/radix/base-ui, react-query, react-hook-form, zod 3). **This Next.js has breaking changes — read `node_modules/next/dist/docs/` before writing web code** (see [apps/web/AGENTS.md](apps/web/AGENTS.md)). |
| `apps/api/core-service` | NestJS 12 backend (ESM, zod 4 via `nestjs-zod`, Drizzle ORM + Postgres, OpenTelemetry). |
| `packages/shared` | Backend utilities (request logger, circuit breaker via `opossum`, safe listeners). Compiled to `dist/`; consumers use the built output, hence Turbo's `^build` dependency. |
| `packages/shared-types` | TS-source-only types shared with web (no build step). |

## Commands

Run from the repo root. Env lives in the root `.env` (`cp .env.example .env`); `docker compose up -d` starts Postgres 16 and Redis.

```bash
pnpm dev                 # web + core-service (tsc --watch + node --watch) + shared watch
pnpm build | lint | typecheck | test      # all packages via turbo
pnpm build-service       # build + typecheck + lint + test, core-service only

pnpm --filter core-service exec vitest run <path-to.spec.ts>
pnpm --filter web exec vitest run <path-to.test.ts>      # add -t "name" for one test
pnpm --filter web format                                 # prettier --write

# Drizzle, from apps/api/core-service
pnpm exec drizzle-kit generate    # after editing src/db/schema/*
pnpm exec drizzle-kit migrate
```

Hooks (husky): `commit-msg` runs commitlint (Angular style, `feat(scope): subject`); `pre-commit` runs prettier on `apps/web` staged files; `pre-push` runs web lint, typecheck, test and `format:check`. CI (`.github/workflows/build.yml`) runs only `pnpm build` — it does not run tests or lint, so run them yourself.

## Backend (core-service)

Hexagonal modules, one per bounded context in `src/` (`scenarios`, `runs`, `maps`, `bots`, `pps`, `projects`), each with `domain/`, `application/` (`use-cases/`, `ports/`), `infrastructure/`, `presentation/` (controller + `dto/`) and `*.module.ts`. Shared pieces are in `src/common/`. Copy the closest existing module when adding one.

- **Dependency direction**: presentation → application → domain; infrastructure implements application ports. Use cases depend on port interfaces bound to injection tokens (e.g. `PPS_REPOSITORY`). Check a module's providers before assuming persistence: `pps` and `runs` still bind **in-memory** repositories; `scenarios` uses Drizzle plus a GCS config reader.
- **Errors**: throw `DomainError` subclasses (`common/domain/domain-error.ts`) with `kind` (`invalid | not_found | forbidden | conflict | unavailable | bad_gateway`) and optional `code`/`details`. `DomainErrorFilter` maps `kind` to HTTP status. Never throw Nest `HttpException` from domain/application code.
- **Validation**: DTOs are zod schemas (`nestjs-zod`); `ZodValidationPipe` is global.
- **Config**: `src/config.ts` is the only place env is read and validated; import `appConfig`, don't touch `process.env`. Blank env vars count as unset. `load-env.ts` must be the first import (`main.ts`, `drizzle.config.ts`). Add new vars to both `config.ts` and `.env.example`.
- **HTTP**: prefix `/api`, URI versioning default `v1`, Swagger via `setupSwagger`. CORS and CSRF both use `allowedOrigins` (`FRONTEND_ORIGIN`/`DEV_ORIGIN`). Default port 3002. Update Swagger/DTO metadata with any endpoint change.
- **Auth**: `Roles` decorator and `RolesGuard` exist in `common/` but no controller uses `@Roles` yet — don't assume endpoints are protected. `UserEmail` decorator supplies the caller (falls back to `system`).
- **Imports use `.js` extensions** (ESM + plain `tsc -p tsconfig.build.json` to `dist/`).
- **DB**: schema in `src/db/schema/`; SQL migrations + snapshots in `drizzle/` are committed. Generate with drizzle-kit; never hand-edit snapshots or `_journal.json`.
- **Telemetry**: `main.ts` imports `telemetry/bootstrap.js` before everything else so auto-instrumentation can patch modules. Keep that order.
- **Upstreams**: `MAP_API`, `RUN_ENGINE_URL`, `BFF_SERVICE_URL` are reached through HTTP adapters in `infrastructure/` (e.g. `runs/infrastructure/http-run-trigger.adapter.ts`) wrapped with the circuit-breaker helpers from `packages/shared`. Follow that pattern for new outbound calls.
- **Tests**: vitest, colocated `*.spec.ts` — use-case specs plus `*.controller.http.spec.ts` for HTTP-level tests.

## Frontend (web)

- Feature-sliced: `features/{scenarios,scenario-setup,scenario-config}` own their `api.ts`, react-query `hooks.ts`, zod `schema.ts` (zod 3, unlike the backend), `validation.ts` and components. `app/` holds routes only. Shared pieces: `components/`, `lib/`.
- `lib/api` is the HTTP client (interceptors, retries, timeout, typed errors). `API_BASE_URL` (`lib/env.ts`) defaults to `/api`, served by **mock route handlers** in `app/api/**` over in-memory `lib/mock` stores. `NEXT_PUBLIC_API_BASE_URL` points it at the real core-service (e.g. `http://localhost:3002/api/v1`). Keep mock and backend contracts in sync when changing either.
- Tests are colocated `*.test.ts` (vitest). Prettier is enforced.

## Working rules

- Read the neighbouring code and the closest existing implementation before adding files, utilities, abstractions or dependencies; reuse `common/`, `packages/shared` and `lib/` helpers.
- Smallest complete change. No speculative abstractions, drive-by refactors, or edits to unrelated files. Preserve API/backward compatibility unless the task says otherwise.
- Don't invent endpoints, env vars, DB columns, scripts or config options — confirm they exist in code first. New env vars/columns need the matching `config.ts`/`.env.example`/migration updates.
- Never commit secrets; `.env` is local. Don't weaken CSRF/CORS/validation.
- Validate with commands that exist, scoped to what you touched: backend → `pnpm build-service`; web → `pnpm --filter web lint`, `typecheck`, `test`, `format:check`. Say explicitly which checks you didn't run and why.
- Update this file or the relevant doc when a change alters architecture, commands, config or workflow.
- Commits: Angular-style conventional messages (`feat(scope): subject`); keep PRs focused. Work is branch-per-feature (`feat/...`) off `main`.

## Gotchas

- Changing `packages/shared` requires a rebuild before core-service sees it (`pnpm build`, or `pnpm dev` which watches).
- `.env.example` comments mention `NEXT_PUBLIC_API_URL`, but the code reads `NEXT_PUBLIC_API_BASE_URL`.
- `AGENTS.md` is regenerated by `next dev`; commit it with your work rather than stripping it from diffs.
