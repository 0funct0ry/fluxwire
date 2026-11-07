// Generates the CLI, exit-code and protocol tables from the `fluxwire reference` command.
import { reference, write } from './lib.mjs';
const base = 'site/src/content/docs/docs/reference';

write(`${base}/cli.md`, `---
title: Command-line reference
description: Every fluxwire command and flag, generated from the clap command tree.
---

The \`fluxwire\` binary opens the app; \`fluxwire-cli\` is the static, GUI-free build for CI. Both accept the same
commands. This page is generated from the clap definitions, so it matches the shipped binary.

Exit codes are listed on the [Exit codes](../exit-codes/) page. Shell completions: \`fluxwire completions <SHELL>\`.

${reference('cli')}`);

const codes = JSON.parse(reference('exit-codes'));
write(`${base}/exit-codes.md`, `---
title: Exit codes
description: The process exit codes of the Fluxwire command line.
---

When several conditions apply, the highest-precedence code wins: a script that threw (3) over a network failure
(2) over a test or request failure (1).

| Code | Name | Meaning |
|---|---|---|
${codes.map((c) => `| \`${c.code}\` | \`${c.name}\` | ${c.meaning} |`).join('\n')}

## Using exit codes in CI

\`\`\`sh
fluxwire run api/ --env ci --reporter junit -o report.xml
case $? in
  0) echo "all passed" ;;
  1) echo "tests failed" ;;
  2) echo "network failure" ;;
  3) echo "a script threw" ;;
  *) echo "usage or file error" ;;
esac
\`\`\`
`);

const p = JSON.parse(reference('protocols'));
const li = (a) => a.map((x) => `\`${x}\``).join(', ');
let roles = '| Protocol | Request language | Role | Content | How many |\n|---|---|---|---|---|\n';
for (const pr of p.protocols)
  for (const r of pr.roles) roles += `| \`${pr.id}\` | \`${r.language ?? pr.request_languages[0]}\` | \`${r.role}\` | ${r.content} | ${r.cardinality === 'AtMostOne' ? 'At most one' : 'Any number'} |\n`;
write(`${base}/roles.md`, `---
title: Roles and info strings
description: Block roles, request languages and reserved names, generated from the protocol registry.
---

A fenced block's info string is \`<language> [role]\`. This page is generated from the \`ProtocolRegistry\`, so it
lists exactly what this version recognizes.

## Protocols

| Protocol | Request languages | Interaction | Feature in \`requires\` |
|---|---|---|---|
${p.protocols.map((x) => `| \`${x.id}\` | ${li(x.request_languages)} | ${x.interaction === 'Session' ? 'Session' : 'Exchange'} | ${x.feature ? `\`${x.feature}\`` : 'none (base format)'} |`).join('\n')}

## Protocol roles

${roles}
## Core roles

| Kind | Names |
|---|---|
| Script roles (languages ${li(p.script_languages)}) | ${li(p.script_roles)} |
| Other core roles | ${li(p.core_roles)} |

## Reserved names

Reserved roles and request languages are not prose and not runnable. A file that uses one is editable and shows
"not supported in this version".

| Kind | Names |
|---|---|
| Reserved roles | ${li(p.reserved_roles)} |
| Reserved request languages | ${li(p.reserved_request_languages)} |
`);
