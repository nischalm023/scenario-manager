---
name: frontend-design-system
description: Design system documentation for Linear/Stripe-quality UI implementation. Use when creating, modifying, or reviewing frontend components.
---

# Frontend Design System

Design system for the GTP Simulator frontend implementing a Linear/Stripe-quality aesthetic.

---

## 1. Design Philosophy

**Core Principles:**
- **Clean & intentional** - Every element serves a purpose
- **Subtle sophistication** - Depth through shadows, not borders
- **Orange brand identity** - `#F97316` as the primary accent
- **Hierarchy through typography** - Size, weight, and color create clear levels
- **Breathing room** - Generous spacing creates calm, professional interfaces

**Anti-patterns to avoid:**
- Heavy borders or dividers
- Too many font sizes (stick to the 5-level scale)
- Hardcoded gray values (use semantic tokens)
- Cramped layouts with insufficient padding

---

## 2. Typography Scale

**5 levels only.** Each size has one clear job.

| Token | Size | Line Height | Weight | Tracking | Tailwind |
|-------|------|-------------|--------|----------|----------|
| **display** | 36px | 1.1 | 700 | -0.02em | `text-display` |
| **heading** | 18px | 1.35 | 600 | -0.01em | `text-heading` |
| **body** | 14px | 1.5 | 400 | 0 | `text-body` |
| **small** | 12px | 1.5 | 400 | 0 | `text-small` |
| **caption** | 11px | 1.4 | 600 | 0.05em | `text-caption` |

**Hierarchy ratios:**
```
36px ─────┐
          │ 2.0x (dramatic - metrics must pop)
18px ─────┤
          │ 1.29x (clear distinction)
14px ─────┤
          │ 1.17x (subtle but visible)
12px ─────┤
          │ 1.09x (minimal - relies on weight/case)
11px ─────┘
```

---

## 3. Text Colors

**4 semantic levels.** Never use hardcoded grays for text.

| Token | Hex | Gray | Tailwind | Purpose |
|-------|-----|------|----------|---------|
| **primary** | #111827 | 900 | `text-text-primary` | Display, headings, emphasis |
| **secondary** | #374151 | 700 | `text-text-secondary` | Body text, readable prose |
| **tertiary** | #6B7280 | 500 | `text-text-tertiary` | Labels, captions, help text |
| **disabled** | #9CA3AF | 400 | `text-text-disabled` | Disabled states, placeholders |

**Visual hierarchy:**
```
#111827 ████████████████████  text-primary   (Display, Headings)
#374151 ████████████████      text-secondary (Body text)
#6B7280 ████████████          text-tertiary  (Labels, captions)
#9CA3AF ████████              text-disabled  (Inactive)
```

---

## 4. Typography + Color Combinations

| Element | Typography | Color | Classes |
|---------|------------|-------|---------|
| **Metric value** | display | primary | `text-display text-text-primary` |
| **Section title** | heading | primary | `text-heading text-text-primary` |
| **Body text** | body | secondary | `text-body text-text-secondary` |
| **Body emphasis** | body / 500 | primary | `text-body font-medium text-text-primary` |
| **Help text** | small | tertiary | `text-small text-text-tertiary` |
| **Label** | caption / UPPER | tertiary | `text-caption uppercase text-text-tertiary` |
| **Disabled** | any | disabled | `text-text-disabled` |

---

## 5. Color System

### Surfaces
| Token | Value | Tailwind | Usage |
|-------|-------|----------|-------|
| background | #FAFAFA | `bg-background` | Page background |
| card | #FFFFFF | `bg-card` | Card, panel backgrounds |
| border | #E5E7EB | `border-border` | Subtle borders |
| input | #E5E7EB | `border-input` | Form input borders |

### Brand
| Token | Value | Tailwind | Usage |
|-------|-------|----------|-------|
| primary | #F97316 | `bg-primary` | Buttons, accents |
| primary/90 | — | `hover:bg-primary/90` | Primary hover |
| primary-foreground | #FFFFFF | `text-primary-foreground` | Text on primary |

### Semantic Status
| Status | Color | Light BG | Tailwind |
|--------|-------|----------|----------|
| Success | #10B981 | #D1FAE5 | `text-success`, `bg-success-light` |
| Warning | #F59E0B | #FEF3C7 | `text-warning`, `bg-warning/10` |
| Danger | #EF4444 | #FEE2E2 | `text-danger`, `bg-danger/10` |
| Info | #3B82F6 | #DBEAFE | `text-info`, `bg-info/10` |

**Status indicator pattern:**
```tsx
<span className="text-success bg-success-light px-2 py-0.5 rounded text-small">
  Active
</span>
```

---

## 6. Spacing Scale

**Principle: Cohesion through proximity.** Related elements tight, unrelated elements distant.

### The Scale
| Level | Value | Tailwind | Cohesion |
|-------|-------|----------|----------|
| **Micro** | 4px | `gap-1` | Atomic (icon + text) |
| **Tight** | 6-8px | `gap-1.5`, `gap-2` | High (label → input) |
| **Related** | 12-16px | `gap-3`, `gap-4` | Medium (field → field) |
| **Grouped** | 20-24px | `gap-5`, `gap-6` | Low (section → section) |
| **Separate** | 32px+ | `gap-8` | None (unrelated blocks) |

### Cohesion Rules
- **HIGH** (4-8px): Label→input, icon→text, button text→icon
- **MEDIUM** (12-16px): Field→field, list items, related actions
- **LOW** (20-24px): Section→section, card internal sections
- **NONE** (32px+): Unrelated blocks, major page divisions

### Component Spacing
| Component | Internal | External |
|-----------|----------|----------|
| Card | `p-5` (20px) | `gap-6` between cards |
| Form field | `space-y-1.5` | `space-y-4` between fields |
| Section | `space-y-4` | `mt-8` or `gap-8` between |
| Button group | `gap-2` | — |

### Touch Targets
Minimum interactive size: **44×44px** (accessibility)
```tsx
// Even small icons need adequate touch area
<button className="p-2 min-h-11 min-w-11">
  <Icon className="h-4 w-4" />
</button>
```

---

## 7. Border Radius

| Token | Value | Tailwind | Usage |
|-------|-------|----------|-------|
| sm | 2px | `rounded-sm` | Badges, small chips |
| md | 4px | `rounded-md` | Buttons, inputs |
| lg | 6px | `rounded-lg` | Cards, panels (default) |
| xl | 12px | `rounded-xl` | Modals, large containers |

**Default:** Use `rounded-lg` (6px) for cards and containers.

---

## 8. Shadows

| Class | CSS | Usage |
|-------|-----|-------|
| `shadow-card` | `0 1px 3px rgba(0,0,0,0.04), 0 1px 2px rgba(0,0,0,0.02)` | Resting state |
| `shadow-card-hover` | `0 4px 12px rgba(0,0,0,0.08)` | Hover state |

**Pattern:**
```tsx
<div className="shadow-card hover:shadow-card-hover transition-shadow duration-200">
```

---

## 9. Interactive States

### State Classes
| State | Pattern | Example |
|-------|---------|---------|
| **Hover** | `hover:bg-*/90` | `hover:bg-primary/90` |
| **Focus** | `focus-visible:ring-1 focus-visible:ring-ring` | Keyboard focus ring |
| **Active** | `active:scale-[0.98]` | Press feedback (optional) |
| **Disabled** | `disabled:opacity-50 disabled:pointer-events-none` | Inactive state |

### Transitions
| Type | Class | Duration |
|------|-------|----------|
| Colors | `transition-colors` | 200ms |
| Shadow | `transition-shadow` | 200ms |
| All | `transition-all` | 200ms |

**Standard duration:** `duration-200` (200ms)

### Button Pattern
```tsx
<button className="
  bg-primary text-primary-foreground
  hover:bg-primary/90
  focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring
  disabled:opacity-50 disabled:pointer-events-none
  transition-colors duration-200
">
```

### Input Pattern
```tsx
<input className="
  border border-input bg-transparent
  focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring
  disabled:cursor-not-allowed disabled:opacity-50
  transition-colors
" />
```

### Card Pattern
```tsx
<div className="
  bg-card rounded-lg p-5
  shadow-card hover:shadow-card-hover
  transition-shadow duration-200
">
```

---

## 10. Z-Index Scale

| Layer | Z-Index | Usage |
|-------|---------|-------|
| Base | `z-0` | Default content |
| Raised | `z-10` | Active tabs, sticky elements |
| Dropdown | `z-20` | Select menus, dropdowns |
| Fixed | `z-30` | Fixed headers, sidebars |
| Modal | `z-40` | Dialog overlays |
| Tooltip | `z-50` | Tooltips, popovers (topmost) |

**Rule:** Never use arbitrary z-index values. Stick to the scale.

---

## 11. Accessibility

### Focus Rings
```tsx
// Standard pattern - keyboard only, visible ring
className="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"

// Minimal variant (inputs, small elements)
className="focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
```
- **Always** use `focus-visible:` not `focus:` (keyboard only)
- **Never** remove focus without replacement
- Ring offset (`ring-offset-2`) improves visibility on colored backgrounds

### Contrast Ratios (WCAG AA)
| Text Token | Hex | On White | Status |
|------------|-----|----------|--------|
| text-primary | #111827 | 16:1 | ✓ AAA |
| text-secondary | #374151 | 10:1 | ✓ AAA |
| text-tertiary | #6B7280 | 4.5:1 | ✓ AA |
| text-disabled | #9CA3AF | 2.7:1 | ✗ decorative only |

**Warning:** `text-disabled` fails contrast—use only for clearly disabled/decorative elements.

### Color Independence
Never rely on color alone. Pair with icons or text:
```tsx
// ✗ Bad - color only
<span className="text-danger">Error</span>

// ✓ Good - color + icon
<span className="text-danger flex items-center gap-1.5">
  <AlertCircle className="h-4 w-4" />
  Error message
</span>
```

### Error States
```tsx
<div className="space-y-1.5">
  <Input
    aria-invalid={!!error}
    aria-describedby={error ? "field-error" : undefined}
    className={error ? "border-danger focus-visible:ring-danger" : ""}
  />
  {error && (
    <p id="field-error" className="text-danger text-small flex items-center gap-1">
      <AlertCircle className="h-3 w-3" />
      {error}
    </p>
  )}
</div>
```

### Screen Readers & Motion
```tsx
<span className="sr-only">Close dialog</span>  // Visually hidden, announced
className="transition-all motion-reduce:transition-none"  // Respect reduced motion
```

---

## 12. Component Patterns

### MetricCard
```tsx
<div className="bg-card rounded-lg p-5 shadow-card hover:shadow-card-hover transition-shadow duration-200">
  <p className="text-display text-text-primary tabular-nums">
    {value.toLocaleString()}
  </p>
  <p className="text-caption uppercase text-text-tertiary mt-2">
    {label}
  </p>
</div>
```

### Form Field
```tsx
<div className="space-y-1.5">
  <Label className="text-small text-text-tertiary font-medium">
    {label}
  </Label>
  <Input className="h-9 text-body" />
</div>
```

### Button
```tsx
<Button className="bg-primary text-primary-foreground hover:bg-primary/90 h-9 px-4">
  {children}
</Button>
```

---

## 13. Review Checklist

### Typography
- [ ] Metrics: `text-display` (36px)
- [ ] Section titles: `text-heading` or `text-caption uppercase`
- [ ] Body: `text-body`, Labels: `text-caption uppercase`
- [ ] No arbitrary font sizes

### Colors
- [ ] No hardcoded hex (use semantic tokens)
- [ ] Status colors paired with icons (color independence)
- [ ] Disabled text only for truly disabled elements

### Spacing (Cohesion)
- [ ] Related elements tight (4-8px)
- [ ] Sections separated (24-32px)
- [ ] Touch targets ≥44px

### Interactive States
- [ ] All buttons have hover + focus-visible + disabled states
- [ ] Transitions use `duration-200`
- [ ] Focus uses `focus-visible:` not `focus:`

### Accessibility
- [ ] Focus rings visible (ring-2 or ring-1)
- [ ] Errors have icons, not just color
- [ ] `aria-invalid` + `aria-describedby` on error inputs

---

## Quick Reference

```
TYPOGRAPHY                          COLORS
text-display  36px  bold            text-text-primary   #111827
text-heading  18px  semibold        text-text-secondary #374151
text-body     14px  regular         text-text-tertiary  #6B7280
text-small    12px  regular         text-text-disabled  #9CA3AF
text-caption  11px  semibold+upper  bg-primary          #F97316

SPACING (cohesion)                  INTERACTIVE
gap-1   4px  micro (icon+text)      hover:bg-primary/90
gap-2   8px  tight (label→input)    focus-visible:ring-1 focus-visible:ring-ring
gap-4  16px  related (fields)       disabled:opacity-50
gap-6  24px  grouped (sections)     transition-colors duration-200

Z-INDEX: z-0 base | z-10 raised | z-20 dropdown | z-40 modal | z-50 tooltip
RADIUS:  rounded-sm 2px | rounded-md 4px | rounded-lg 6px (default) | rounded-xl 12px
```
