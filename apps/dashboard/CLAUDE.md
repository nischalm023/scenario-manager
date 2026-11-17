# CLAUDE.md - Admin Dashboard

This is the admin dashboard for the GSB platform. It lets admins manage all the library data (agents, processes, technologies, scenarios, advisory rules, functional areas) and monitor organizations, users, and projects.

---

## Tech Stack

- **Next.js 15** (App Router)
- **Tailwind CSS 4**
- **Untitled UI React** for all UI components (https://www.untitledui.com/react/components)
- **TypeScript** (strict)
- **GraphQL** for API communication with `apps/api`

---

## Core Rules

1. **Keep it simple.** Pick the most straightforward approach. If a junior dev would struggle to follow it, simplify.
2. **No over-engineering.** No factories, no abstract base classes, no premature abstractions. Three similar lines are better than a clever helper used once.
3. **Readability first.** Code should read like plain English. Short functions, clear names, flat structure.
4. **Use Untitled UI components for everything.** Buttons, inputs, tables, modals, dropdowns, badges, alerts - always reach for Untitled UI first. Only build custom components when Untitled UI genuinely does not have what you need.
5. **No dead code.** No unused imports, no commented-out blocks, no leftover variables.

---

## Folder Structure

```
apps/dashboard/
├── app/                          # Next.js App Router pages and layouts
│   ├── layout.tsx                # Root layout (sidebar, topbar)
│   ├── page.tsx                  # Dashboard home / overview
│   ├── (auth)/                   # Login, auth pages (no sidebar)
│   │   └── login/page.tsx
│   └── (dashboard)/              # All admin pages (with sidebar)
│       ├── agents/
│       │   ├── page.tsx          # List page
│       │   └── [slug]/page.tsx   # Detail/edit page
│       ├── processes/
│       ├── technologies/
│       ├── scenarios/
│       ├── advisory-rules/
│       ├── functional-areas/
│       ├── users/
│       ├── organizations/
│       └── projects/
├── components/
│   ├── layout/                   # Sidebar, Topbar, Nav items
│   └── features/                 # Feature-specific components (AgentForm, ProcessTable, etc.)
├── lib/
│   ├── graphql/                  # GraphQL queries and mutations
│   ├── api.ts                    # GraphQL client setup
│   └── utils.ts                  # Small utility functions (formatDate, slugify, etc.)
├── hooks/                        # Custom React hooks
├── types/                        # Shared TypeScript types
└── public/                       # Static assets
```

---

## Page Rules

- **Default to Server Components.** Only add `"use client"` when you need interactivity, state, or browser APIs.
- **One page = one concern.** List pages list things. Detail pages show/edit one thing. Do not combine them.
- **Data fetching lives in `lib/graphql/`.** Pages call functions from `lib/`, never write raw fetch/GraphQL inside page components.
- **Use route groups** `(auth)` and `(dashboard)` to separate layouts. Auth pages have no sidebar. Dashboard pages have the full admin shell.

---

## Component Rules

- **Always check Untitled UI first.** Before building any UI element, look at https://www.untitledui.com/react/components. Use their Button, Input, Table, Modal, Badge, Alert, Dropdown, Tabs, etc.
- **Small, focused components.** If a component goes beyond ~80 lines, split it.
- **Name components clearly.** `AgentForm.tsx` not `Form.tsx`. `ProcessTable.tsx` not `DataTable.tsx`.
- **Props over context.** Pass data through props. Only use React context for truly global state (auth, theme).
- **No wrapper components around Untitled UI.** Use their components directly. Do not create a `<MyButton>` that just wraps their `<Button>`.

---

## GraphQL Rules

- **All queries and mutations go in `lib/graphql/`.** Organized by feature: `lib/graphql/agents.ts`, `lib/graphql/processes.ts`, etc.
- **One file per feature.** Each file exports the queries and mutations for that feature.
- **Keep query fields minimal.** Only request fields you actually display. Do not fetch everything "just in case".
- **Type the responses.** Define return types in `types/` and use them.

---

## TypeScript Rules

- **No `any`.** Use `unknown` and narrow, or define a proper type.
- **Types go in `types/` folder.** One file per feature: `types/agents.ts`, `types/processes.ts`, etc.
- **Always type function params and return values.**
- **Use interfaces for object shapes, types for unions/intersections.**

---

## Styling Rules

- **Tailwind only.** No CSS files, no CSS modules, no styled-components.
- **Use Tailwind 4 conventions.** CSS-first configuration, `@theme` directive, no `tailwind.config.js` unless absolutely necessary.
- **Stick to the Untitled UI design system.** Use their color tokens, spacing, and typography when available. Do not invent your own design system.
- **No inline `style` props.** Everything goes through Tailwind classes.
- **Keep class lists readable.** If a class string gets very long, break it across lines or extract repeated patterns into Tailwind `@apply` in a global CSS file.

---

## State Management

- **Start with URL state.** Filters, search terms, pagination, active tabs - put them in the URL using `searchParams`. This makes pages shareable and bookmarkable.
- **Local state for forms.** Use `useState` for form inputs, modal open/close, and other ephemeral UI state.
- **No global state library** unless a clear need arises. React context + URL params cover 99% of admin dashboard needs.

---

## Forms

- **Keep forms simple.** Use controlled inputs with `useState`. No form libraries unless forms get very complex (10+ fields with cross-field validation).
- **Validate on submit, not on every keystroke.** Show errors after the user tries to submit.
- **Use Untitled UI form components.** Their Input, Select, Checkbox, Toggle, etc.
- **Show loading state on submit buttons.** Disable the button and show a spinner while the mutation runs.

---

## Error Handling

- **Show errors to the user.** Use Untitled UI Alert or Toast components.
- **Catch GraphQL errors in the API layer** (`lib/api.ts`). Transform them into user-friendly messages.
- **Do not silently swallow errors.** If something fails, the user should know.

---

## What NOT to Do

- Do not add dependencies without a clear reason
- Do not create abstract base components or generic wrappers
- Do not use `console.log` in committed code
- Do not build your own design system - use Untitled UI
- Do not fetch data inside components - put it in `lib/graphql/`
- Do not add a global state library (Redux, Zustand, Jotai) unless you hit a real wall with context + URL state
- Do not write CSS files - use Tailwind
- Do not over-type things - keep TypeScript helpful, not bureaucratic

---

## Docs

Before building features, read the docs in the root `docs/` folder:
- `docs/graphql-api.md` - Full API reference with example queries and mutations
- `docs/data-flow-and-collections.md` - How all collections relate to each other

<!-- BEGIN:nextjs-agent-rules -->

# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->
