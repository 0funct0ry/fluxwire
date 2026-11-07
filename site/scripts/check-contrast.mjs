// WCAG AA (4.5:1) contrast check of the text/background token pairs in both themes.
import { readFileSync } from 'node:fs';
const lum = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const vars = (block) => Object.fromEntries([...block.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)].map((m) => [m[1], m[2]]));
const mk = readFileSync('src/styles/marketing.css', 'utf8');
const docs = readFileSync('src/styles/docs.css', 'utf8');
const light = vars(mk.slice(mk.indexOf(':root {'), mk.indexOf('@media')));
const dark = vars(mk.slice(mk.indexOf(':root[data-theme="dark"]'), mk.indexOf('* {')));
const dl = vars(docs.slice(0, docs.indexOf(":root[data-theme='dark']")));
const dd = vars(docs.slice(docs.indexOf(":root[data-theme='dark']")));
const sets = [
  ['marketing light', light, [['ink', 'bg'], ['muted', 'bg'], ['faint', 'bg'], ['blue', 'bg'], ['green', 'panel'], ['amber', 'panel'], ['red', 'panel'], ['muted', 'panel'], ['faint', 'code']]],
  ['marketing dark', dark, [['ink', 'bg'], ['muted', 'bg'], ['faint', 'bg'], ['blue', 'bg'], ['green', 'panel'], ['amber', 'panel'], ['red', 'panel'], ['muted', 'panel'], ['faint', 'code']]],
  ['docs light', dl, [['sl-color-white', 'sl-color-black'], ['sl-color-gray-2', 'sl-color-black'], ['sl-color-gray-3', 'sl-color-black'], ['sl-color-accent', 'sl-color-black'], ['sl-color-gray-3', 'sl-color-gray-6']]],
  ['docs dark', dd, [['sl-color-white', 'sl-color-black'], ['sl-color-gray-2', 'sl-color-black'], ['sl-color-gray-3', 'sl-color-black'], ['sl-color-accent', 'sl-color-black'], ['sl-color-gray-3', 'sl-color-gray-6']]],
];
let bad = 0;
for (const [name, v, pairs] of sets) for (const [f, b] of pairs) {
  if (!v[f] || !v[b]) { console.error(`${name}: missing ${f}/${b}`); bad++; continue; }
  const r = ratio(v[f], v[b]);
  if (r < 4.5) { console.error(`${name}: ${f} on ${b} is ${r.toFixed(2)}:1`); bad++; }
}
if (bad) process.exit(1);
console.log('contrast ok (AA, both themes)');
