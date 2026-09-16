# ScT identity

The approved mark is **C04-05 — Plain planes**, selected from round two of
`icon-concepts.html`. It is a perspective chest made from three softly rounded
planes, with no clasp. Preserve the selected geometry, orientation and color order.

`web/public/brand.svg` is the canonical transparent mark. `web/public/favicon.svg`
places the same mark on a white rounded tile, matching the treatment used by
Crumbles and Simple Agents. `web/public/apple-touch-icon.png` is its 180 px export.

The exact shared palette is light `#bfdbfe`, medium `#60a5fa`, and strong `#2563eb`.
The top plane is light, the left plane medium, and the right plane strong blue.
Keep the seams transparent in the standalone mark.

The web scaffold uses blue-600 branding, gray-50 page backgrounds, white panels,
gray-200 borders and gray-900 main text, consistent with the other applications.
The gallery remains a design reference; its other concepts are not approved marks.

## Palette comparison

Checked against Crumbles' `crumbles-web/src/style.css`,
`crumbles-web/public/favicon.svg`, and Simple Agents' `web/style.css`,
`web/brand.svg`, and `docs/branding.md` on 2026-09-16. Crumbles uses Tailwind
blue/gray utility families for its UI; Simple Agents specifies the corresponding
hex colors directly. ScT uses the same palette through variables in
`web/src/style.css`.

| Role | Color |
| --- | --- |
| Icon light / medium / strong | `#bfdbfe` / `#60a5fa` / `#2563eb` |
| Primary action / hover / focus | `#2563eb` / `#1d4ed8` / `#3b82f6` |
| Page / panel / border | `#f9fafb` / `#ffffff` / `#e5e7eb` |
| Main / secondary / muted text | `#111827` / `#4b5563` / `#6b7280` |
| Pale blue surface / text | `#dbeafe` / `#1d4ed8` |
| Success surface / text | `#dcfce7` / `#15803d` |
| Warning surface / text | `#fef3c7` / `#92400e` |
| Error surface / text | `#fee2e2` / `#b91c1c` |

Use the documented action and status colors as those controls are implemented.
Crumbles also supports dark and high-contrast themes; this scaffold currently
implements the shared light palette only.

## Asset usage

- App identity and repository README: transparent `brand.svg`.
- Browser tab: `favicon.svg`; browser theme color: `#2563eb`.
- Apple home-screen bookmark: `apple-touch-icon.png`.
- Standalone concept gallery: an embedded copy of the approved favicon, so the
  HTML file remains self-contained.

Reuse these assets for future login and navigation surfaces. Do not redraw the
mark separately in individual components or use an exploration variant.

Vite copies these public assets into the web build. Run `npm --prefix web run build`
to include them in the assets served by the Rust server.
