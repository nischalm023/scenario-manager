# Remove Python Backend - Design Document

**Date:** 2025-12-22
**Status:** Approved
**Scope:** Remove `apps/backend` (Python) and rename `apps/backend` to `apps/backend`

## Overview

Consolidate from dual backends to Java-only. The Java backend is feature-complete and the Python backend has never been deployed.

## Phased Approach

### Phase 1: Remove Python Backend

**Delete entirely:**
- `apps/backend/` directory (entire Python backend)
- `pyrightconfig.json` (Python type checker)
- `.python-version`

**Modify:**

| File | Change |
|------|--------|
| `.gitignore` | Remove lines 199-207 (Python backend scenario ignores) |
| `.dockerignore` | Remove `apps/backend` reference |
| `.gcloudignore` | Remove `apps/backend` reference |
| `.pre-commit-config.yaml` | Remove Python hooks (ruff, black) |
| `CLAUDE.md` | Remove Python backend section, update architecture |
| `README.md` | Rewrite for Java-only (remove Python/FastAPI/SimPy refs) |

**Keep unchanged:**
- Historical docs in `specs/` and `docs/plans/` that reference Python backend (archival)

**Verification:**
- `npx nx serve backend` starts successfully
- `npx nx test backend` passes
- Frontend connects to backend on port 8080

### Phase 2: Rename Java Backend to Backend

**Directory rename:**
```bash
git mv apps/backend apps/backend
```

**Update references (77+ files):**

| Category | Files | Change |
|----------|-------|--------|
| NX Config | `apps/backend/project.json` | Update `name` to `backend`, update all paths |
| Build | `Dockerfile` | `apps/backend` → `apps/backend` |
| Git | `.gitignore` | Update Java paths |
| IDE | `.vscode/tasks.json`, `.vscode/launch.json` | Update paths |
| Docs | `CLAUDE.md`, `docs/JAVA_STYLE_GUIDE.md` | Update all references |
| Plans | `docs/plans/*.md` (40+ files) | Bulk find/replace |
| Specs | `specs/**/*.md` (15+ files) | Bulk find/replace |

**Verification:**
- `npx nx serve backend` works with new name
- `npx nx test backend` passes
- `npx nx build backend` produces JAR
- `docker build .` succeeds

## Files Summary

### Phase 1 Deletions
```
apps/backend/           # Entire directory (~6,477 files with cache)
pyrightconfig.json
.python-version
```

### Phase 1 Modifications
```
.gitignore
.dockerignore
.gcloudignore
.pre-commit-config.yaml
CLAUDE.md
README.md
```

### Phase 2 Modifications (after rename)
```
apps/backend/project.json
Dockerfile
.gitignore
.vscode/tasks.json
.vscode/launch.json
CLAUDE.md
docs/JAVA_STYLE_GUIDE.md
docs/plans/*.md (bulk)
specs/**/*.md (bulk)
```

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Breaking NX cache | Run `npx nx reset` after Phase 2 |
| IDE indexing issues | Close and reopen project after rename |
| Git history loss | Using `git mv` preserves history |
| Missing references | Grep for `backend` after Phase 2 to catch stragglers |

## Post-Migration Cleanup

After both phases complete:
1. Run `npx nx reset` to clear NX cache
2. Run `npm install` to update workspace references
3. Verify `npx nx graph` shows correct project structure
4. Update any team documentation or wikis

## Testing Strategy

**Phase 1:**
- Run Java backend tests: `npx nx test backend`
- Start server: `npx nx serve backend`
- Run frontend and verify API connectivity

**Phase 2:**
- Same tests with new name: `npx nx test backend`, `npx nx serve backend`
- Full Docker build: `docker build -t test .`
- Integration test: Start container, verify frontend + backend work
