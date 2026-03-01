# The Scuffed Crew — Design System

## Aesthetic Direction

Dark, utilitarian esports aesthetic. Think clan sites from the mid-2000s updated with modern type hierarchy and restrained motion. The visual language is direct and structural — no gradients, no rounded bubbly shapes, no pastel softness. Everything communicates that this is an org with real structure.

A subtle film grain overlay (`feTurbulence` SVG filter at 4% opacity) covers the entire viewport to add texture and prevent flat-screen sterility.

---

## Color Tokens

All colors are defined as CSS custom properties via `ThemeProvider` (see `crates/ui/src/theme/presets.rs`).

### Backgrounds (darkest → lightest)

| Token              | Value       | Usage                                    |
| ------------------ | ----------- | ---------------------------------------- |
| `--bg-void`        | `#08080c`   | Page background, full-bleed sections     |
| `--bg-surface`     | `#0e0e14`   | Slightly elevated surfaces               |
| `--bg-card`        | `#14141e`   | Cards, panels, content blocks            |
| `--bg-card-alt`    | `#1a1a28`   | Alternate card shade (hover/focus)       |
| `--bg-elevated`    | `#20202e`   | Table headers, dropdowns, modals         |

### Accent (Brand Purple)

| Token              | Value                          | Usage                          |
| ------------------ | ------------------------------ | ------------------------------ |
| `--accent`         | `#7c3aed`                      | Primary brand color, CTAs      |
| `--accent-soft`    | `rgba(124, 58, 237, 0.15)`    | Subtle backgrounds, hovers     |
| `--accent-glow`    | `rgba(124, 58, 237, 0.25)`    | Box shadows, glow effects      |
| `--accent-bright`  | `#a78bfa`                      | Light accent for text on dark   |

### Semantic Colors

| Token        | Value     | Usage                           |
| ------------ | --------- | ------------------------------- |
| `--danger`   | `#d63031` | Errors, destructive actions     |
| `--success`  | `#00c853` | Confirmations, positive states  |
| `--warning`  | `#f0b232` | Caution states, draft badges    |
| `--info`     | `#4a9eff` | Informational, links            |

### Additional colors (defined in `:root` in `main.css`)

| Token          | Value                         | Usage                        |
| -------------- | ----------------------------- | ---------------------------- |
| `--red`        | `#d63031`                     | Competition/OW theme         |
| `--red-soft`   | `rgba(214, 48, 49, 0.15)`    | Red card/pill backgrounds    |
| `--gold`       | `#f0b232`                     | Achievement, rank accents    |
| `--blue`       | `#4a9eff`                     | Info badges                  |
| `--green`      | `#00c853`                     | Win states, active status    |

### Text

| Token              | Value     | Usage                                   |
| ------------------ | --------- | --------------------------------------- |
| `--text-bright`    | `#f0eee8` | Headings, high-emphasis text            |
| `--text-primary`   | `#ccc8c0` | Body text, default content              |
| `--text-secondary` | `#807a70` | Descriptions, metadata, secondary info  |
| `--text-muted`     | `#504c44` | Labels, placeholders, disabled states   |

### Borders

| Token            | Value     | Usage                              |
| ---------------- | --------- | ---------------------------------- |
| `--border`       | `#2a2832` | Card borders, dividers             |
| `--border-light` | `#363440` | Hover borders, subtle separators   |

---

## Typography

Fonts are loaded from Google Fonts and defined in `crates/ui/src/theme/tokens.rs`.

### Font Stack

| Role            | Font Family         | CSS Variable          | Weight(s)   | Usage                                          |
| --------------- | ------------------- | --------------------- | ----------- | ---------------------------------------------- |
| Display Hero    | Bebas Neue          | `--font-display-hero` | 400         | Page titles (`h1`), hero text                  |
| Display         | Rajdhani            | `--font-display`      | 600, 700    | Section headings, card names, sub-headings     |
| Body            | Source Sans 3       | `--font-body`         | 300–700     | Paragraphs, descriptions, general content      |
| Monospace       | DM Mono             | `--font-mono`         | 400, 500    | Nav links, metadata, labels, code, dates       |

### Type Scale (Standalone Pages)

| Element              | Font              | Size     | Weight | Letter-Spacing | Color              |
| -------------------- | ----------------- | -------- | ------ | -------------- | ------------------ |
| Page title (h1)      | Bebas Neue        | 3rem     | 400    | 3px            | `--text-bright`    |
| Section heading      | Rajdhani          | 1.2rem   | 700    | —              | `--text-bright`    |
| Section label        | DM Mono           | 0.75rem  | 500    | 2px            | `--accent-bright`  |
| Body text            | Source Sans 3     | 1rem     | 400    | —              | `--text-primary`   |
| Metadata             | DM Mono           | 0.8rem   | 400    | 0.5px          | `--text-muted`     |
| Nav links            | DM Mono           | 0.72rem  | 400    | 1.5px          | `--text-secondary` |
| Status pill          | DM Mono           | 0.65rem  | 700    | 0.04em         | varies             |

---

## Layout & Spacing

### Page Structure

All standalone pages follow the same pattern:

```
max-width: 1000px        (exception: apply page = 600px)
margin: 0 auto
padding: 6rem 2rem 4rem  (6rem top clears the 60px fixed nav)
min-height: 100vh
```

### Centered Page Header Pattern

Every standalone page (`/members`, `/news`, `/tournaments`, `/tournaments/:id`, `/apply`) uses this consistent header:

```css
h1 {
    font-family: 'Bebas Neue', sans-serif;
    font-size: 3rem;
    color: var(--text-bright);
    letter-spacing: 3px;
    text-align: center;
    margin-bottom: 0.5rem;
}
h1::after {
    content: '';
    display: block;
    width: 60px;
    height: 3px;
    background: var(--accent);
    margin: 0.75rem auto 2.5rem;
    border-radius: 2px;
}
```

The 60px purple underline bar is the signature element across all page headers.

### Homepage Sections

Each homepage section uses `SectionHeader` with:
- **Label**: Monospace, uppercase, accent color (`// The Ethos`, `// Active Squads`)
- **Title**: Rajdhani 700, large
- **Description**: Body text, muted

All section headers are center-aligned.

### Content Centering Rules

- Page titles: always centered
- Metadata rows (flex): `justify-content: center`
- Description paragraphs: `text-align: center; margin: 0 auto`
- Section divider titles: `text-align: center`
- Card content: centered within card
- Grid/list content: fills naturally from left (CSS Grid `auto-fill`)
- News article body: left-aligned (readability)
- Form content: left-aligned within centered card

### Standard Gaps

| Context               | Gap       |
| --------------------- | --------- |
| Section vertical       | 2rem      |
| Card grid             | 1–1.25rem |
| Card internal padding | 1.25–2rem |
| Metadata items        | 0.75–1.5rem |
| Nav links             | 1.8rem    |

---

## Components

### Cards

```css
background: var(--bg-card);
border: 1px solid var(--border);
border-radius: 8–10px;
padding: 1.25–2rem;
transition: border-color 0.2s, transform 0.15s;
```

On hover: `border-color: var(--accent-soft)`, `transform: translateY(-2px)`.

### Status Pills

Small uppercase monospace labels with color-coded backgrounds:

| Status         | Background    | Text Color |
| -------------- | ------------- | ---------- |
| Registration   | `#7c3aed33`   | `#a78bfa`  |
| In Progress    | `#10b98133`   | `#34d399`  |
| Completed      | `#6b728033`   | `#9ca3af`  |
| Draft          | `#f59e0b33`   | `#fbbf24`  |

```css
padding: 0.15rem 0.5rem;
border-radius: 999px;
font-size: 0.65rem;
font-weight: 700;
text-transform: uppercase;
```

### Buttons

Primary (CTA):
```css
background: var(--accent);
color: #fff;
border-radius: 3px;
box-shadow: 0 0 20px var(--accent-glow);
font-family: 'DM Mono', monospace;
text-transform: uppercase;
letter-spacing: 1.5px;
```

Secondary:
```css
background: transparent;
border: 1px solid var(--border);
color: var(--text-secondary);
```

### Navigation

Fixed top bar, 60px height:
```css
background: rgba(8, 8, 12, 0.9);
backdrop-filter: blur(24px);
border-bottom: 1px solid var(--border);
```

Purple "SC" badge in top-left with `-2deg` rotation and glow shadow.

---

## Motion

### Scroll Reveal

Elements with `data-reveal` attribute fade in with upward translate:
```css
opacity: 0 → 1
transform: translateY(24px) → none
transition: 0.7s cubic-bezier(0.16, 1, 0.3, 1)
```

Staggered delays: `data-reveal-delay="1"` through `"4"` (80ms increments).

Variants: `data-reveal="left"`, `"right"`, `"scale"`.

### Hover Transitions

- Cards: `border-color` + `transform` over 0.2s
- Nav links: `color` over 0.2s
- CTA button: `box-shadow` + `transform` over 0.25s

### Live Indicator

Pulsing dot for live tournament matches:
```css
@keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
}
animation: pulse-dot 1.5s ease-in-out infinite;
```

---

## Responsive Breakpoints

| Breakpoint | Behavior                                          |
| ---------- | ------------------------------------------------- |
| > 960px    | Full desktop layout, horizontal brackets          |
| 600–960px  | Narrower cards, condensed spacing                 |
| < 768px    | Hamburger nav, stacked grids                      |
| < 600px    | Vertical bracket fallback, single-column layout   |

---

## File Map

| File                                          | Contains                                  |
| --------------------------------------------- | ----------------------------------------- |
| `crates/ui/src/theme/tokens.rs`               | Color + font token structs                |
| `crates/ui/src/theme/presets.rs`              | Scuffed Crew theme values                 |
| `crates/ui/src/theme/mod.rs`                  | ThemeProvider component, CSS var generator |
| `crates/site/assets/styles/main.css`          | All site-wide styles                      |
| `crates/site/src/components/bracket/styles.rs`| Tournament/bracket-specific styles        |
