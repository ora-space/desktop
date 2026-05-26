# AI Chat IDE - Design Document

## Goals

The target is an IDE-like AI Chat application mimicking the aesthetics of VSCode and Cursor.

## UI / UX Guidelines

- **Theming**: Light, dark, and system (follows OS preference). Toggled via a Sun / Monitor / Moon capsule control.
- **Color Palette**:
  - Primary Accent: `oklch(0.71 0.18 38.65)` (used for highlights and button hovers)
  - Button Base: `oklch(0.66 0.18 38.65)`
  - Backgrounds: Warm whites (`oklch(0.9748 0.009 70)`) and slight grays for depth.
  - Text: Dark grays (`oklch(0.25 0.01 70)`) for hierarchy.
  - Borders: Subtle dividing lines (`oklch(0.85 0.01 70)`).

## Technical Stack

- **Framework**: React + Vite
- **UI Primitives**: Radix UI for accessible base components.
- **Icons**: Lucide for crisp SVG components.
- **Styling**: Tailwind v4 — no hand-written CSS classes; all styling via utility classes.

## Component Library (`@ora/ui`)

Shared component library at `packages/ui`, consumed by `apps/desktop` and the showcase app `apps/ui`.

Implementation mirrors shadcn: Radix UI primitives + CVA (class-variance-authority) for variants + tailwind-merge for class merging.

### Design Tokens

Tokens live in `packages/ui/src/theme.css` as a Tailwind v4 `@theme` block. This makes tokens available as native Tailwind utilities (e.g. `bg-primary`, `text-fg`, `border-border`) in all consuming apps.

Token groups:
- **Colors**: `bg`, `bg-secondary`, `bg-subtle`, `fg`, `fg-secondary`, `border`, `border-subtle`, `primary`, `primary-transparent`, `btn-bg`, `btn-fg`
- **Radii**: `radius-sm` (4px), `radius-md` (6px), `radius-lg` (8px) — VSCode/Cursor compact aesthetic
- **Typography**: `font-sans`, `font-mono`

Dark mode overrides are applied via `[data-theme="dark"]` and `@media (prefers-color-scheme: dark) [data-theme="system"]` selectors in the same file. The `data-theme` attribute is set on `<html>` and persisted to `localStorage`.

### Consuming an App

Each consuming app's CSS entry point must include:
```css
@import "tailwindcss";
@import "@ora/ui/theme.css";
@source "../../../packages/ui/src";
```

To avoid a flash of unstyled theme on load, add an inline script at the top of `<head>`:
```html
<script>
  (function () {
    var t = localStorage.getItem("ora-theme") || "system";
    document.documentElement.setAttribute("data-theme", t);
  })();
</script>
```

### Component API Conventions

- All components accept `className` for override via tailwind-merge.
- `Button` supports `asChild` (Radix Slot pattern) for polymorphic rendering.
- Variant props follow CVA: `variant` and `size`.
- No component defines its own CSS classes.

### Components

| Component | Primitives |
|---|---|
| Alert | — |
| AlertDialog | `@radix-ui/react-alert-dialog` |
| Avatar | `@radix-ui/react-avatar` |
| Badge | — |
| Breadcrumb | `@radix-ui/react-slot` |
| Button | `@radix-ui/react-slot` |
| Card | — |
| Checkbox | `@radix-ui/react-checkbox` |
| Collapsible | `@radix-ui/react-collapsible` |
| Dialog | `@radix-ui/react-dialog` |
| Input | — |
| ThemeToggle | — |

### Showcase App (`apps/ui`)

Layout: fixed header with theme toggle + left sidebar with ghost button navigation + scrollable main content area. Each component has a dedicated page under `apps/ui/src/pages/`. See `docs/showcase-task.md` for the workflow when adding new components.
