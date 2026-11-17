# Archived agent guidance (Java era)

Frozen guidance from the Java/queue-simulator era, moved aside when the Java
services began rolling off. Nothing here loads into agent sessions.

- `CLAUDE-java-era.md` — the old repo-root CLAUDE.md, verbatim (the root
  `AGENTS.md` was a symlink to CLAUDE.md and went with it).
- `skills/` — the full skill tree the old CLAUDE.md referenced. Four
  entries (agent-browser, c4-architecture, skill-creator,
  vercel-react-best-practices) are symlinks into `.agents/skills/`, which
  stays in place; they dangle while archived here and resolve again once
  moved back.
- `hooks.json` + `scripts/` — the prompt-driven skill-reminder hook and the
  mvn/tsc build-check hook.
- `agents/test-engineer.md` — the Java/TS test-engineer subagent.

To reintroduce a skill, move it back deliberately, one at a time, after
checking it still describes the current codebase:

```bash
git mv .claude/archive/skills/<name> .claude/skills/<name>
```
