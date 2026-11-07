// Generates reference/fw-api.md from the shipped fw.d.ts declarations.
import { read, write } from './lib.mjs';
const src = read('crates/fluxwire-script/src/fw.d.ts');
const page = `---
title: The fw script API
description: Every declaration of the fw scripting API, generated from fw.d.ts.
---

The \`fw\` global is available in \`ts pre\`, \`ts post\` and \`ts test\` blocks. The declarations below are the
exact file Fluxwire writes to \`.fluxwire/types/fw.d.ts\`, so editors complete against the same text. This page is
generated from \`crates/fluxwire-script/src/fw.d.ts\` during the site build and cannot drift from the app.

TypeScript is only type-stripped, not type-checked. The \`pm\` namespace is a compatibility layer for imported
Postman scripts and is described in the [Importing guide](../../guides/importing/).

\`\`\`ts title=".fluxwire/types/fw.d.ts"
${src.trimEnd()}
\`\`\`
`;
write('site/src/content/docs/docs/reference/fw-api.md', page);
