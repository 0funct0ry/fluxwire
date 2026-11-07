// Generates reference/keybindings.md from the ACTIONS table in fluxwire-ui/src/actions.rs.
import { read, write } from './lib.mjs';
const src = read('crates/fluxwire-ui/src/actions.rs');
const table = src.slice(src.indexOf('pub static ACTIONS'));
const end = table.indexOf('\n];');
const re = /\b([ah])!\(\s*"([^"]+)",\s*\w+,\s*(Some\("([^"]+)"\)|None),\s*(Some\("([^"]+)"\)|None)\s*,?\s*\)/g;
const rows = [];
for (const m of table.slice(0, end).matchAll(re)) rows.push({ palette: m[1] === 'a', label: m[2], key: m[4], ctx: m[6] });
const pretty = (k) => k.split(' ').map((c) => c.split('-').map((p) => ({ secondary: 'Cmd/Ctrl', cmd: 'Cmd', ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', enter: 'Enter', escape: 'Esc' }[p] ?? (p.length === 1 ? p.toUpperCase() : p))).join('+')).join(', ');
const bound = rows.filter((r) => r.key).sort((a, b) => a.label.localeCompare(b.label));
const pal = rows.filter((r) => r.palette).map((r) => r.label).sort();
write('site/src/content/docs/docs/reference/keybindings.md', `---
title: Keybindings
description: Default keybindings and palette commands, generated from the app's action table.
---

\`Cmd/Ctrl\` means Cmd on macOS and Ctrl on Linux and Windows. Open **keymap: show** from the command palette to see
and rebind every action; **keymap: edit** opens \`keymap.toml\`. Entries that depend on focus list their context.

## Default bindings

| Action | Binding | Context |
|---|---|---|
${bound.map((r) => `| \`${r.label}\` | <kbd>${pretty(r.key)}</kbd> | ${r.ctx ?? 'Global'} |`).join('\n')}

## Palette commands

Open the palette with <kbd>Cmd/Ctrl+Shift+P</kbd>. Prefix the query with \`~\` to preview and switch themes.

${pal.map((l) => `- \`${l}\``).join('\n')}
`);
