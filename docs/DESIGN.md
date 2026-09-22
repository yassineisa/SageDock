# SageDock design system

A Windows desktop application built on **Metro's principles**, using **Windows 11 / Fluent**
conventions for the specifics. Implemented in
[`src/styles/theme.css`](../src/styles/theme.css) (tokens, type, base elements) and
[`src/styles/layout.css`](../src/styles/layout.css) (structure). Components carry class
names only — no inline styling and no page-specific overrides.

## Foundation

Metro, as Microsoft describes it: typography carries the hierarchy, **content comes before
chrome**, geometry stays simple, controls are _authentically digital_ (flat, crisp, no
imitation of physical materials), and motion exists to explain state changes. Its roots are
Swiss graphic design and the clarity of road signage.

Windows 11 supplies the measurements, so these are specifications rather than taste:

| Decision      | Value                                                   | Source                                                                           |
| ------------- | ------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Font          | Segoe UI Variable                                       | Windows default system font                                                      |
| Weights       | Regular 400, Semibold 600                               | Bold and Italic are **not** in the Windows type ramp — use Semibold for emphasis |
| Casing        | Sentence case, including titles                         | Windows 11 typography guidance                                                   |
| Minimum sizes | 14px Semibold, 12px Regular                             | Below this is illegible in some languages                                        |
| Alignment     | Left                                                    | Centre only for text under an icon                                               |
| Corner radius | **4px** persistent controls, **8px** transient surfaces | Windows 11 geometry: Button/TextBox/ListView vs ContentDialog/Flyout             |
| Icons         | Segoe Fluent Icons                                      | Ships with Windows; Segoe MDL2 Assets is the Windows 10 fallback                 |

## Type ramp

The Windows 11 type ramp guides the element and component rules in `theme.css`
and `layout.css`; unused typography utility classes have been removed:

```
caption       12 / 16   400
body          14 / 20   400
body strong   14 / 20   600
body large    18 / 24   400
subtitle      20 / 28   600   (Display face)
title         28 / 36   600   (Display face)
```

`h1` is Title, `h2` is Subtitle, `h3` is Body Strong. Page titles are one word where
possible — "Home", "Settings", "Diagnostics" — with a single explanatory line beneath.

## Colour

Warm iron neutrals, never blue-grey, never pure `#FFFFFF` or `#000000`. The accent is
**Concordia burgundy `#912338`**.

**Two accent tokens, because fill and foreground have opposite contrast requirements.** A
button fill must be dark enough to carry white text; an icon or selection marker must be
light enough to read _against_ the page. On light backgrounds one value does both; on dark
they diverge sharply. Using a single token is how the first dark theme ended up with a
near-black label on a light pink fill at roughly 3.8:1, under the 4.5:1 minimum.

| Token                              | Light                             | Dark                              |
| ---------------------------------- | --------------------------------- | --------------------------------- |
| `--bg`                             | `#f5f4f2`                         | `#1c1b1a`                         |
| `--layer`                          | `#fbfaf9`                         | `#252423`                         |
| `--layer-alt`                      | `#eeedea`                         | `#2d2c2b`                         |
| `--subtle` (hover fill)            | `#e8e6e2`                         | `#323130`                         |
| `--stroke`                         | `#e1dfdb`                         | `#3b3a39`                         |
| `--stroke-strong`                  | `#c9c6c0`                         | `#55534f`                         |
| `--text` / `--text-2` / `--text-3` | `#1b1a19` / `#5d5a55` / `#8a8681` | `#f3f2f1` / `#c8c6c4` / `#9a9792` |
| `--accent` (fill)                  | `#912338`                         | `#a8374d`                         |
| `--accent-text` (foreground)       | `#912338`                         | `#e2909d`                         |
| `--on-accent`                      | `#ffffff`                         | `#ffffff`                         |
| `--accent-subtle`                  | `#f5e9eb`                         | `#33232a`                         |

**Status colours are separate from the accent**: `--ok`, `--caution`, `--danger`. An accent
that also meant "healthy" could not sit next to a warning without the colour saying two
things at once. Status is **never colour alone** — every state also carries a word and an
icon, per the accessibility requirements in `app.md` §27.

**The logo is a filled tile, which is what resolves its contrast problem.** The mark is one
static asset shown on both backgrounds, so a bare burgundy glyph could not work: `#912338`
against the dark theme's `#1c1b1a` is around 2:1, well below the 3:1 minimum for graphical
objects. Making the burgundy the _background_ of a rounded tile moves the meaningful
contrast inside the mark, where it is controlled rather than inherited: the cream sigma
`#FFF7EE` on burgundy `#912338` reaches about 7.9:1, and the rose binding `#E59BA8` on the
same field about 3.8:1. Both clear their thresholds on either theme, because neither
depends on the page behind them.

The tile's own outer edge against the dark background stays near 2:1. That is accepted
deliberately: the edge is the boundary of a filled shape, not a part of the mark a reader
must resolve to identify it, and the sigma inside carries the meaning. It is recorded here
so the next person does not rediscover it as a bug.

The vector source is [`src-tauri/icons/icon.svg`](../src-tauri/icons/icon.svg): three flat
fills, no gradients and no effects. See [`BRANDING.md`](BRANDING.md) for the palette and for
how to regenerate the packaged icon set after changing it.

## Spacing and geometry

A 4px grid: `--sp-1` 4 through `--sp-12` 48. Gaps and padding come from tokens, never
arbitrary values.

Elevation is almost absent. Cards get a 1px stroke and `--elev-card` (a 1px hairline
shadow) to lift them off the page; **only transient surfaces** — dialogs — use
`--elev-flyout`. There are no glass effects, no gradients, and no blur. Translucency was
declined outright: WebView2 has no reliable Mica or Acrylic backdrop, and the brief calls
for restraint plus an opaque fallback, which means opaque is the honest choice.

## Motion

Short and explanatory. `--motion-fast` 90ms, `--motion-normal` 150ms.

**Filled controls are deliberately not transitioned.** Animating a button's background while
its text colour changes instantly means a theme switch leaves white labels on a still-white
button for the whole transition — unreadable. Motion is applied only where the default
background is transparent (navigation items, list rows), so there is nothing to animate away
from. `prefers-reduced-motion` disables all of it.

## Iconography

One family: **Segoe Fluent Icons**, accessed through
[`src/components/Icon.tsx`](../src/components/Icon.tsx). It ships with Windows, so SageDock
bundles no font, adds nothing to the installer, needs no `font-src` in the CSP, and carries
no third-party licence obligation.

Codepoints live in one verified table in that component. They are private-use characters, so
a wrong one renders as an empty box rather than failing loudly — keeping them in one place is
what stops that spreading. Every icon is `aria-hidden`, and every control carries its own
visible text or `aria-label`; that also keeps glyphs out of accessible names, which the UI
tests match on exactly.

**Verify a codepoint before adding it, and write it as an escape.** "Renders as an empty box
rather than failing loudly" means a wrong glyph will pass every test. Rasterise the candidate
in Edge and compare it against a deliberately unassigned private-use codepoint: if the two
bitmaps match, it is `.notdef` and unusable. Then look at the shortlist at 16px in both
themes, which is the size the navigation pane and tool cards use. New entries are written as
`""` rather than pasted glyphs, because a private-use character that degrades to a box
during a copy or an editor round-trip is exactly the failure this table exists to prevent.

The four tool icons were chosen this way: `code` braces for source, `numeric` a ruler and set
square for numerical work, `buildTools` a wrench and screwdriver, and `toolkit` a toolbox for
the complete kit. Four distinct silhouettes, so the cards are not four copies of one glyph.

## Layout

- **Navigation pane** on the left, 240px, following the Windows NavigationView pattern: icon
  plus label, subtle fill on the selected item, and a 3px accent bar on its leading edge.
  Below 900px it collapses to a 64px icon rail — each link keeps an explicit `aria-label`,
  because `display:none` on the label would otherwise strip the accessible name at exactly
  the minimum window size.
- **Command bar** beneath the page title for primary actions.
- **Home launch panel:** Open JupyterLab is the first and most prominent action once setup
  is ready. Its larger accent button opens JupyterLab's own landing page without making a
  file, always rooted at the default SageDock folder rather than the course opened last, so
  the general action stays general. Notebook creation and workspace launches use secondary
  buttons, and a workspace card opens that course's folder.
- **Content** capped at 1080px, centred, on the 4px grid.
- **No application footer.** Attribution is not repeated on every screen. The version, the
  MIT licensing statement, and **Created by Yassin Eisa** live on the **About** page, which
  has its own entry in the navigation pane. `AppFooter.tsx` and the `.app-footer*` rules
  were deleted rather than hidden, so no empty strip is left below the content: `.app-main`
  is a column flex container and `.app-content` has `flex: 1`, which reclaims the space.
- **Static list rows** (`.credit-row`, used by the About attribution) carry no hover fill and
  no `text-transform`. Nothing in the list is clickable, and open-source project names have
  deliberate casing that `text-transform: capitalize` on `.check-row` would silently rewrite
  (`pandas` to `Pandas`, `conda-forge` to `Conda-forge`). Reusing `.check-row` there would
  have misattributed the projects, which is why this pattern exists separately.
- **Status needs four tones, not two.** `.tool-state` and `.capability` use `is-ok`,
  `is-warn`, `is-off` and `is-unknown`, because "SageDock has not been able to check" is a
  different fact from "not installed" and a student acts differently on each. Collapsing them
  would tell somebody a compiler is missing when the truth is that nothing looked. Every
  state also carries a word, never colour alone.
- **`.component-row`** lists each program in a group separately, with its own tick and
  version. One tick for "build tools" would hide a missing `make`.
- `tauri.conf.json` declares `minWidth: 860` / `minHeight: 560`. The layout must hold there,
  and four UI tests assert no horizontal overflow at 860px.

## Language

Plain and instrumental. No slogans, no marketing headlines, no motivational copy. WSL,
terminals, ports, and package managers belong in Diagnostics, not in primary workflows.

**No em dashes in text the user reads.** Use a comma, a full stop, or a rewrite. They are
still fine in source comments, which are not part of the interface. Check with a search for
`—` across `src/` before shipping: the only matches should be in comments.

## Extending it

1. Add the token to `theme.css` first; never hard-code a colour or spacing value.
2. Persistent control → 4px radius. Transient surface → 8px. Nothing else rounds.
3. Accent as a fill → `--accent` with `--on-accent`. Accent as an icon, marker, or label →
   `--accent-text`.
4. Any new state needs a word and an icon, not only a colour.
5. Both themes, every time — and check a filled control immediately after switching theme,
   not just in a settled state.
6. New icons go in `Icon.tsx` with a codepoint verified against Microsoft's font listing.

Run `npm run test:ui` after any layout change (25 tests). It asserts no horizontal overflow at
860px on Home, Diagnostics, the About attribution list, and the Scientific tools page, and
regenerates the `docs/qa/` screenshots for Home, setup, Settings, About, and Scientific tools
in both themes plus the minimum window size.

A green suite is not sufficient on its own. Four visual defects in the 1.1.3 redesign passed
every test and were caught only by opening the screenshots: a dark accent below 4.5:1, phantom
`auto-fill` grid columns, unreadable button labels mid theme transition, and hover outranking
selection on the segmented control. Open the renders.
