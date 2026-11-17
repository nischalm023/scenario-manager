# Remove Python Backend - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove Python backend and rename Java backend to be the primary backend.

**Architecture:** Two-phase migration. Phase 1 removes Python backend entirely. Phase 2 renames `apps/backend` to `apps/backend`. Each phase is a separate PR for easier review.

**Tech Stack:** NX monorepo, Java/Spring Boot, Git

---

## Phase 1: Remove Python Backend

### Task 1: Delete Python Backend Directory

**Files:**
- Delete: `apps/backend/` (entire directory)

**Step 1: Verify Java backend works before changes**

Run: `npx nx serve backend`
Expected: Server starts on port 8080

Stop the server (Ctrl+C).

**Step 2: Delete the Python backend directory**

```bash
rm -rf apps/backend
```

**Step 3: Verify deletion**

Run: `ls apps/`
Expected: Only `backend` and `frontend` directories remain

**Step 4: Commit**

```bash
git add -A
git commit -m "chore: remove Python backend directory

Python backend is no longer used. Java backend is feature-complete
and serves as the primary backend."
```

---

### Task 2: Delete Python Root Config Files

**Files:**
- Delete: `pyrightconfig.json`
- Delete: `.python-version`

**Step 1: Delete Python config files**

```bash
rm pyrightconfig.json .python-version
```

**Step 2: Verify deletion**

Run: `ls -la | grep -E "pyright|python-version"`
Expected: No output (files deleted)

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: remove Python-specific config files

Remove pyrightconfig.json and .python-version as Python backend
is no longer part of the project."
```

---

### Task 3: Update .gitignore

**Files:**
- Modify: `.gitignore`

**Step 1: Remove Python backend scenario ignores (lines 199-207)**

Remove these lines from `.gitignore`:

```
apps/backend/data/scenarios/scenario_b/
apps/backend/data/scenarios/scenario_c/
apps/backend/data/scenarios/scenario_d/
apps/backend/data/scenarios/scenario_e/
apps/backend/data/scenarios/scenario_f/
apps/backend/data/scenarios/scenario_g/
apps/backend/data/scenarios/scenario_h/
apps/backend/data/scenarios/scenario_i/
apps/backend/data/scenarios/scenario_j/
```

**Step 2: Verify the change**

Run: `grep -n "apps/backend/" .gitignore`
Expected: Only `apps/backend/` entries remain

**Step 3: Commit**

```bash
git add .gitignore
git commit -m "chore: remove Python backend entries from .gitignore"
```

---

### Task 4: Update .dockerignore

**Files:**
- Modify: `.dockerignore`

**Step 1: Remove Python backend line**

Remove line 49:
```
apps/backend/
```

Also remove pyproject.toml and uv.lock references (lines 50-51):
```
pyproject.toml
uv.lock
```

**Step 2: Verify the change**

Run: `grep -n "apps/backend/" .dockerignore`
Expected: Only `apps/backend/` entries remain

**Step 3: Commit**

```bash
git add .dockerignore
git commit -m "chore: remove Python backend entries from .dockerignore"
```

---

### Task 5: Update .gcloudignore

**Files:**
- Modify: `.gcloudignore`

**Step 1: Remove Python backend line**

Remove line 50:
```
apps/backend/
```

Also remove pyproject.toml and uv.lock references (lines 51-52):
```
pyproject.toml
uv.lock
```

**Step 2: Verify the change**

Run: `grep -n "apps/backend/" .gcloudignore`
Expected: Only `apps/backend/` entries remain

**Step 3: Commit**

```bash
git add .gcloudignore
git commit -m "chore: remove Python backend entries from .gcloudignore"
```

---

### Task 6: Update .pre-commit-config.yaml

**Files:**
- Modify: `.pre-commit-config.yaml`

**Step 1: Remove Python-specific hooks**

Remove the black, isort, and flake8 repo blocks (lines 12-29). Keep only the pre-commit-hooks repo:

```yaml
repos:
  - repo: https://github.com/pre-commit/pre-commit-hooks
    rev: v4.5.0
    hooks:
      - id: trailing-whitespace
      - id: end-of-file-fixer
      - id: check-yaml
      - id: check-added-large-files
      - id: check-merge-conflict
      - id: check-toml
```

**Step 2: Verify the change**

Run: `cat .pre-commit-config.yaml`
Expected: Only pre-commit-hooks repo remains, no black/isort/flake8

**Step 3: Commit**

```bash
git add .pre-commit-config.yaml
git commit -m "chore: remove Python linting hooks from pre-commit config

Remove black, isort, and flake8 hooks as Python backend is removed."
```

---

### Task 7: Verify Java Backend Still Works

**Step 1: Start Java backend**

Run: `npx nx serve backend`
Expected: Server starts on port 8080

**Step 2: Test health endpoint**

Run (in another terminal): `curl -s http://localhost:8080/health`
Expected: `{"status":"UP"}` or similar healthy response

**Step 3: Run tests**

Run: `npx nx test backend`
Expected: All tests pass

Stop the server.

---

### Task 8: Create Phase 1 Summary Commit

**Step 1: Verify all Phase 1 changes**

Run: `git log --oneline -10`
Expected: See commits for tasks 1-6

**Step 2: Tag the Phase 1 completion (optional)**

```bash
git tag phase1-python-backend-removed
```

---

## Phase 2: Rename Java Backend

### Task 9: Rename Directory

**Files:**
- Rename: `apps/backend/` → `apps/backend/`

**Step 1: Rename using git mv**

```bash
git mv apps/backend apps/backend
```

**Step 2: Verify rename**

Run: `ls apps/`
Expected: `backend` and `frontend` directories

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: rename backend to backend

Java backend is now the primary and only backend."
```

---

### Task 10: Update project.json

**Files:**
- Modify: `apps/backend/project.json`

**Step 1: Update project name and paths**

Change `name` from `backend` to `backend`.

Update all `cd apps/backend` to `cd apps/backend` in command strings.

The file should have:
- Line 2: `"name": "backend",`
- All commands: `cd apps/backend && mvn ...`

**Step 2: Verify NX recognizes the project**

Run: `npx nx show project backend`
Expected: Shows project configuration (no errors)

**Step 3: Commit**

```bash
git add apps/backend/project.json
git commit -m "chore: update project.json for renamed backend"
```

---

### Task 11: Update Dockerfile

**Files:**
- Modify: `Dockerfile`

**Step 1: Replace all backend references**

Replace all occurrences of `apps/backend` with `apps/backend`:
- Line 30: `COPY apps/backend/pom.xml ./apps/backend/`
- Line 31: `COPY apps/backend/libs ./apps/backend/libs`
- Line 34: `COPY apps/backend/src ./apps/backend/src`
- Line 37: `COPY --from=frontend-build /app/dist/apps/frontend ./apps/backend/src/main/resources/static`
- Line 40: `RUN cd apps/backend && mvn clean package -DskipTests -B`
- Line 48: `COPY --from=backend-build /app/apps/backend/target/dessimulator-1.0.0-SNAPSHOT.jar app.jar`
- Line 54: `COPY --from=backend-build /app/apps/backend/src/main/resources/scenarios /tmp/scenarios`

**Step 2: Verify no backend references remain**

Run: `grep -n "backend" Dockerfile`
Expected: No output

**Step 3: Commit**

```bash
git add Dockerfile
git commit -m "chore: update Dockerfile paths for renamed backend"
```

---

### Task 12: Update .gitignore

**Files:**
- Modify: `.gitignore`

**Step 1: Replace backend references**

Replace all `apps/backend/` with `apps/backend/`:
- Line 42: `apps/backend/target/`
- Lines 63-65: Java Desmo-J experiments paths
- Line 208+: Java scenario ignores

**Step 2: Verify changes**

Run: `grep -n "backend" .gitignore`
Expected: No output

**Step 3: Commit**

```bash
git add .gitignore
git commit -m "chore: update .gitignore paths for renamed backend"
```

---

### Task 13: Update .dockerignore

**Files:**
- Modify: `.dockerignore`

**Step 1: Replace backend references**

Replace all `apps/backend` with `apps/backend`:
- Line 9: `!apps/backend/README.md`
- Line 32: `!apps/backend/libs/*.jar`
- Line 60: `apps/backend/src/test/`

**Step 2: Verify changes**

Run: `grep -n "backend" .dockerignore`
Expected: No output

**Step 3: Commit**

```bash
git add .dockerignore
git commit -m "chore: update .dockerignore paths for renamed backend"
```

---

### Task 14: Update .gcloudignore

**Files:**
- Modify: `.gcloudignore`

**Step 1: Replace backend references**

Replace all `apps/backend` with `apps/backend`:
- Line 12: `!apps/backend/README.md`
- Line 33: `!apps/backend/libs/*.jar`
- Line 63: `apps/backend/src/test/`

**Step 2: Verify changes**

Run: `grep -n "backend" .gcloudignore`
Expected: No output

**Step 3: Commit**

```bash
git add .gcloudignore
git commit -m "chore: update .gcloudignore paths for renamed backend"
```

---

### Task 15: Update VS Code Configuration

**Files:**
- Modify: `.vscode/tasks.json`
- Modify: `.vscode/launch.json`

**Step 1: Update tasks.json**

Replace all `apps/backend` with `apps/backend` (lines 10, 36, 62).

**Step 2: Update launch.json**

Replace all `apps/backend` with `apps/backend` (lines 8, 19, 32).

**Step 3: Verify changes**

Run: `grep -rn "backend" .vscode/`
Expected: No output

**Step 4: Commit**

```bash
git add .vscode/
git commit -m "chore: update VS Code config paths for renamed backend"
```

---

### Task 16: Bulk Update Documentation

**Files:**
- Modify: Multiple files in `docs/` and `specs/`

**Step 1: Find all files with backend references**

Run: `grep -rl "backend" docs/ specs/ CLAUDE.md README.md`

**Step 2: Bulk replace using sed**

```bash
# macOS sed syntax
find docs specs -name "*.md" -exec sed -i '' 's/backend/backend/g' {} +
sed -i '' 's/backend/backend/g' CLAUDE.md README.md
```

**Step 3: Verify no references remain**

Run: `grep -rn "backend" docs/ specs/ CLAUDE.md README.md`
Expected: No output

**Step 4: Commit**

```bash
git add docs/ specs/ CLAUDE.md README.md
git commit -m "docs: update all documentation for renamed backend

Bulk replace backend references with backend across all
markdown files in docs/, specs/, CLAUDE.md, and README.md."
```

---

### Task 17: Reset NX Cache and Verify

**Step 1: Clear NX cache**

```bash
npx nx reset
```

**Step 2: Verify NX graph**

Run: `npx nx graph`
Expected: Opens browser showing `backend` and `frontend` projects (no `backend`)

**Step 3: Test backend with new name**

Run: `npx nx serve backend`
Expected: Server starts on port 8080

Run: `npx nx test backend`
Expected: All tests pass

Run: `npx nx build backend`
Expected: JAR built successfully

---

### Task 18: Final Verification

**Step 1: Verify no backend references in entire repo**

```bash
grep -rn "backend" --include="*.json" --include="*.yml" --include="*.yaml" --include="*.md" --include="*.sh" . | grep -v node_modules | grep -v target | grep -v ".git/"
```

Expected: No output (or only historical git references)

**Step 2: Test Docker build**

```bash
docker build -t queue-simulator-test .
```

Expected: Build succeeds

**Step 3: Run Docker container**

```bash
docker run -p 8080:8080 queue-simulator-test
```

Expected: Application starts, accessible at http://localhost:8080

**Step 4: Tag completion**

```bash
git tag phase2-backend-renamed
```

---

## Summary

**Phase 1 Commits (7):**
1. Remove Python backend directory
2. Remove Python config files
3. Update .gitignore
4. Update .dockerignore
5. Update .gcloudignore
6. Update .pre-commit-config.yaml
7. (Verification - no commit)

**Phase 2 Commits (9):**
1. Rename directory
2. Update project.json
3. Update Dockerfile
4. Update .gitignore
5. Update .dockerignore
6. Update .gcloudignore
7. Update VS Code config
8. Bulk update documentation
9. (Final verification - no commit)

**Post-Migration:**
- Run `npx nx reset` to clear cache
- Verify `npx nx serve backend` works
- Update any team wikis or external documentation
