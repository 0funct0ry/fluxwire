# ADR-024: Marketing page and docs site

Status: accepted (M15)

## Context

SPEC §25 asks for one Astro project that serves a marketing page at `/` and Starlight docs at `/docs/`, deployed to
GitHub Pages under the `/fluxwire` base path. Reference pages must not drift from the code.

## Decisions

- **One Astro project in `site/`.** `src/pages/index.astro` renders the marketing page, whose markup and CSS are ported
  from `MARKETING-MOCKUP.html` into `src/components/marketing-body.html` and `src/styles/marketing.css`. Starlight owns
  `/docs/` through content under `src/content/docs/docs/`, themed by `src/styles/docs.css` using the same tokens.
- **Generated reference pages are committed.** `npm run gen` (`site/scripts/gen-*.mjs`) writes the `fw` API page (from
  `fw.d.ts`), the front matter page and the published JSON Schema (from `docs/schema/`), the keybindings
  (from the `ACTIONS` table), and the CLI, exit-code and role pages. The last three come from a hidden
  `fluxwire reference <cli|protocols|exit-codes>` command, so they read the clap tree, `ProtocolRegistry` and `exit.rs`
  directly. The files are committed so the Pages build needs only Node; the CI `site` job regenerates them and fails on
  `git diff`, which is the drift check.
- **No new Rust dependency.** The CLI Markdown renderer walks the clap tree with the existing `clap` crate.
- **Accessibility gates.** `scripts/check-contrast.mjs` asserts WCAG AA (4.5:1) for the text and background token pairs
  in both themes; `scripts/check-links.mjs` fails on broken internal links; Lighthouse CI asserts accessibility
  ≥ 0.95. The marketing page's muted, faint, green, amber and red tokens were darkened (and the dark red lightened)
  relative to the mockup to pass AA. `prefers-reduced-motion` disables animation and smooth scrolling.
- **Help entries are palette actions.** The app has no native menu bar, so "Help" is the `help:` group in the palette:
  open documentation, open format reference, open keybindings reference, and report an issue. URLs live in
  `fluxwire-ui/src/help_links.rs`.
- **Screenshots** come from `make screenshots`, which runs existing dev-mcp scenarios and copies their PNG checkpoints
  into `site/public/screenshots/`. It needs a GPU session, so it is not part of CI.

## Consequences

Changing a clap flag, a role, an exit code, a keybinding, `fw.d.ts` or the schema requires running `npm run gen` and
committing the result. The site depends on Google Fonts at runtime for IBM Plex, with system fallbacks.
