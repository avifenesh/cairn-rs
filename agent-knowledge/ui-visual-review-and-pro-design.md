# UI Visual Review from CLI & Professional Dashboard Design

## Part 1: Taking Screenshots from CLI (No Browser Required)

### shot-scraper (Recommended -- Python, Playwright-based)
```bash
pip install shot-scraper
shot-scraper install                          # installs browser deps once

# Basic screenshot
shot-scraper http://localhost:3000 -o dashboard.png

# With viewport size
shot-scraper http://localhost:3000 -o dashboard.png --width 1440 --height 900

# Capture specific element
shot-scraper http://localhost:3000 --selector ".dashboard-container" -o widget.png

# Inject JS before capture (e.g., wait for data, toggle dark mode)
shot-scraper http://localhost:3000 --javascript "document.documentElement.classList.add('dark')" -o dark.png

# Multi-URL batch capture via YAML
shot-scraper multi shots.yml
```

### Playwright (Node.js -- scriptable)
```bash
npx playwright install --with-deps chromium

# No CLI screenshot subcommand exists. Use a one-liner script:
node -e "
  const { chromium } = require('playwright');
  (async () => {
    const b = await chromium.launch();
    const p = await b.newPage();
    await p.setViewportSize({ width: 1440, height: 900 });
    await p.emulateMedia({ colorScheme: 'dark' });
    await p.goto('http://localhost:3000', { waitUntil: 'networkidle' });
    await p.screenshot({ path: 'dashboard.png', fullPage: true });
    await b.close();
  })();
"
```

### Chrome DevTools MCP (Available in This Environment)
The `chrome-devtools-mcp` plugin provides `take_screenshot` and `take_snapshot` tools
that work directly from the Claude Code agent -- iterative visual review without
leaving the terminal.

### Visual Regression Testing
- **Percy** (percy.io), **Chromatic** (chromatic.com), **BackstopJS**, **reg-suit**

---

## Part 2: The 10 Most Impactful Professional Dashboard Design Fixes

### 1. Restrained Color Palette (Single Accent + Neutrals)
**Amateur:** Rainbow of colors, every section a different hue.
**Pro:** One primary accent (blue or indigo), one semantic set (red/amber/green for status), rest is neutrals.

### 2. Consistent Spacing Scale (4px/8px Base)
**Amateur:** Random padding values (13px, 22px, 37px).
**Pro:** Use Tailwind's spacing scale strictly: `p-2` (8px), `p-3` (12px), `p-4` (16px), `p-6` (24px), `gap-4`, `gap-6`. Never custom values.

### 3. Typography Hierarchy (3 Sizes Max Per View)
**Amateur:** 6+ font sizes, inconsistent weights.
**Pro:** Page title (`text-lg font-semibold`), section label (`text-sm font-medium`), body/data (`text-sm`), caption (`text-xs text-gray-400`). That is the entire scale.

### 4. Muted Borders, No Heavy Shadows
**Amateur:** `shadow-lg`, thick colored borders, `border-2`.
**Pro:** `border border-white/5` or `border-white/10`, `divide-white/5`. Shadows only for floating elements like dropdowns (`shadow-lg`). Cards use border, not shadow.

### 5. Low-Contrast Card Surfaces (Layered Grays)
**Amateur:** Cards same color as background, or jarring contrast.
**Pro:** Background `bg-gray-950`, cards `bg-gray-900`, elevated panels `bg-gray-800/50`. Subtle 1-step difference creates depth without visual noise.

### 6. Uniform Border Radius
**Amateur:** Mix of `rounded`, `rounded-lg`, `rounded-xl`, `rounded-full` on similar elements.
**Pro:** Pick ONE radius for cards/panels (`rounded-lg`) and ONE for buttons/inputs (`rounded-md`). Apply universally.

### 7. Deliberate White Space
**Amateur:** Dense, cramped layouts filling every pixel.
**Pro:** Generous `p-6` inside cards, `gap-6` between cards, `py-8` between sections. Enterprise dashboards breathe.

### 8. Muted Text Colors (Not Pure White)
**Amateur:** `text-white` everywhere on dark backgrounds.
**Pro:** Primary text `text-gray-100`, secondary `text-gray-400`, disabled `text-gray-500`. Reserve `text-white` for headings only.

### 9. Status Colors With Opacity
**Amateur:** Bright `bg-red-500` badges, `bg-green-500` indicators.
**Pro:** `bg-red-500/10 text-red-400`, `bg-green-500/10 text-green-400`. Muted background tint + slightly desaturated text.

### 10. Consistent Icon + Text Alignment
**Amateur:** Icons randomly sized, misaligned with text.
**Pro:** Icons always `size-4` (16px) with `gap-2` from text. Vertically centered with `items-center`. Use a single icon set (Lucide, Heroicons).

---

## Part 3: Recommended Dark Dashboard Color Palette

### Background Layers (use Zinc or Gray)
```
Page background:     bg-gray-950    (#0a0a0f)
Card/panel:          bg-gray-900    (#141419)
Elevated surface:    bg-gray-800    (#1e1e26)  or bg-gray-800/50
Hover state:         bg-white/5
Active state:        bg-white/10
```

### Text Hierarchy
```
Heading:             text-white  or  text-gray-50
Primary text:        text-gray-200
Secondary text:      text-gray-400
Disabled/caption:    text-gray-500
```

### Borders & Dividers
```
Subtle border:       border-white/5     (barely visible)
Standard border:     border-white/10    (default for cards)
Emphasized border:   border-white/15
Divider:             divide-white/5
```

### Single Primary Accent
```
Primary action:      bg-indigo-500  hover:bg-indigo-400
Primary text:        text-indigo-400
Primary badge:       bg-indigo-500/10 text-indigo-400
```

### Semantic Status (always muted)
```
Success:  bg-emerald-500/10  text-emerald-400  border-emerald-500/20
Warning:  bg-amber-500/10    text-amber-400    border-amber-500/20
Error:    bg-red-500/10      text-red-400      border-red-500/20
Info:     bg-blue-500/10     text-blue-400     border-blue-500/20
```

---

## Part 4: Tailwind Classes to Use vs Avoid

### USE (Professional Patterns)
```html
<!-- Card -->
<div class="rounded-lg border border-white/10 bg-gray-900 p-6">

<!-- Stat badge -->
<span class="rounded-md bg-emerald-500/10 px-2 py-1 text-xs font-medium text-emerald-400">

<!-- Section header -->
<h2 class="text-sm font-medium text-gray-400">

<!-- Data grid -->
<div class="grid grid-cols-3 gap-6">

<!-- Table row hover -->
<tr class="hover:bg-white/5 transition-colors">

<!-- Sidebar nav item -->
<a class="flex items-center gap-2 rounded-md px-3 py-2 text-sm text-gray-400 hover:bg-white/5 hover:text-gray-200">
```

### AVOID (Amateur Tells)
```
shadow-xl shadow-2xl         -- Heavy shadows scream "template"
border-2 border-4            -- Thick borders look unrefined
rounded-3xl rounded-full     -- Overly round on cards/panels
bg-gradient-to-r             -- Gradients on cards (fine for CTAs sparingly)
text-white everywhere        -- Flattens hierarchy; use gray-200/400
ring-4 ring-offset-4         -- Excessive focus rings
animate-bounce animate-ping  -- Distracting on dashboards
bg-blue-600 bg-red-600       -- Full-saturation backgrounds for status
uppercase tracking-widest    -- Overused "design system" look
```

---

## Part 5: Design Systems to Study

| System      | Key Lesson                                              |
|-------------|---------------------------------------------------------|
| Linear      | Extreme restraint, single accent, fluid animations      |
| Vercel/Geist| Developer-first, high contrast, monospace for data      |
| Stripe      | Information hierarchy, consistent density, muted colors |
| Tailwind UI | Production patterns, well-tested dark variants          |
| Radix UI    | Accessible primitives, composable dark themes           |
| shadcn/ui   | Zinc-based dark palette, battle-tested Tailwind tokens  |

shadcn/ui's dark palette (HSL `222.2 84% 4.9%` background, `210 40% 98%` foreground,
`217.2 32.6% 17.5%` for muted/accent/border) is the gold standard in the Tailwind ecosystem.
