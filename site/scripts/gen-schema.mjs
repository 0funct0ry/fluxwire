// Publishes the JSON Schema and generates reference/front-matter.md from it.
import { read, write } from './lib.mjs';
const raw = read('docs/schema/front-matter.schema.json');
write('site/public/schema/front-matter.schema.json', raw);
const s = JSON.parse(raw);
const ty = (p) => {
  if (p.$ref) return `\`${p.$ref.split('/').pop()}\``;
  if (p.anyOf) return [...new Set(p.anyOf.map(ty))].join(' or ');
  const t = Array.isArray(p.type) ? p.type.filter((x) => x !== 'null').join(' or ') : p.type;
  if (t === 'array' && p.items) return `list of ${ty(p.items)}`;
  if (t === 'object' && p.additionalProperties) return `map of ${ty(p.additionalProperties)}`;
  return `\`${t ?? 'any'}\``;
};
const esc = (t = '') => t.replace(/\|/g, '\\|').replace(/\n/g, ' ');
let body = '| Key | Type | Description |\n|---|---|---|\n';
for (const [k, p] of Object.entries(s.properties).sort()) body += `| \`${k}\` | ${ty(p)} | ${esc(p.description)} |\n`;
let defs = '';
for (const [n, d] of Object.entries(s.$defs ?? {})) {
  if (!d.properties) continue;
  defs += `\n### ${n.replace(/Schema$/, '')}\n\n| Key | Type | Description |\n|---|---|---|\n`;
  for (const [k, p] of Object.entries(d.properties).sort()) defs += `| \`${k}\` | ${ty(p)} | ${esc(p.description)} |\n`;
}
write('site/src/content/docs/docs/reference/front-matter.md', `---
title: Front matter reference
description: Every front matter key of a .flux.md file, generated from the published JSON Schema.
---

${s.description}

The schema is published at [\`/schema/front-matter.schema.json\`](../../../schema/front-matter.schema.json).
Point your editor or agent at it with a YAML language-server modeline or a \`yaml.schemas\` setting. This page is
generated from \`docs/schema/front-matter.schema.json\`.

## Keys

${body}
${defs}`);
