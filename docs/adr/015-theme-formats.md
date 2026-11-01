# ADR-015: Theme formats

## Status
Accepted (M9.05). Changes SPEC §17.7, which named only Zed-schema JSON.

## Context
Fluxwire shipped two themes (One Dark, One Light) as Zed JSON. microterm (same author, MIT, same stack) already has 53 licensed palettes in a terminal-first TOML format, the OKLCH derivation of the remaining UI colours, and a live-preview selector.

## Decision
- **Port microterm's TOML next to Zed JSON; do not convert.** TOML files are copied verbatim and parsed at runtime, so upstream palettes stay diffable and any microterm theme works unchanged. Both formats produce one GPUI-free `ThemeTokens` in `fluxwire-format::theme`; `fluxwire-ui/src/theme.rs` only converts tokens to `Hsla`.
- **Derivation.** UI tokens follow microterm (surface ±4 %, border ±12 %, border_variant and element_hover ±7 %, element_selected ±11 % OKLCH lightness toward the foreground; text_muted = mix(fg, bg, 0.3); text_placeholder = mix(fg, bg, 0.5); accent/success/warning/error = ansi 4/2/3/1). Fluxwire-specific: editor = background, elevated = elevated_surface, created/modified/deleted = success/warning/error, title_bar = background shifted 8 % toward the foreground, gutter = mix(fg, bg, 0.6); optional `[ui] title_bar` and `gutter` override these. microterm's `tab_color_*` tokens are dropped and ignored if present.
- **Syntax from the ANSI palette**, overridable by an optional `[syntax]` table: function = ansi 4, type = ansi 6, keyword = ansi 5, string = ansi 2, number = boolean = mix(ansi 1, ansi 3, 0.5), property = ansi 1, comment = ansi 8, punctuation = text_muted. A test pins six of these tokens of microterm's one-dark.toml to Fluxwire's One Dark JSON.
- **JSON wins name collisions.** One Dark and One Light stay JSON; microterm's copies live only as test fixtures. The bundled set is 2 JSON + 51 TOML = 53.
- **ANSI palette kept** in `ThemeTokens.terminal` for the integrated terminal (M16); nothing renders it yet.
- **Persistence.** `theme = "<name>"` in `settings.toml`, written with `toml_edit` (other bytes untouched, atomic rename, a malformed file is never overwritten). Without it the app follows the OS appearance; a configured theme ignores OS changes. `theme: toggle` stays a session-only One Dark/One Light flip.
- **One preview path.** `theme: select…` and the palette `~` prefix share one preview/restore/commit implementation (`theme_picker.rs`): the modal close path restores the original unless a theme row was run.

## Consequences
Adding a theme means dropping a file in `assets/themes/` and its line in `BUNDLED_THEMES` (a test checks both). User themes in the config dir, `{ dark, light }` pairs, hot reload and a contrast gate are deferred.
