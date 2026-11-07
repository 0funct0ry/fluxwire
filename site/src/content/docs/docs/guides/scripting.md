---
title: Scripting
description: Write pre-request scripts, post-response scripts, and tests in JavaScript or TypeScript.
---

Scripts are fenced blocks with the role `pre`, `post`, or `test`, in language `ts` or `js`. They run in an embedded,
sandboxed QuickJS engine. TypeScript is type-stripped, not type-checked.

````markdown
```ts pre title="Sign the request"
import { sign } from "lib/sign.ts";
fw.vars.set("idemKey", crypto.randomUUID());
fw.request.headers.set("X-Signature", sign(fw.request.body.text()));
```

```ts test
fw.test("has an id", () => fw.expect(fw.response.json()).toHaveProperty("id"));
```
````

## Order of execution

`pre` scripts run before the send, in file order, Collection first, then Folder, then Request. Then variables are
interpolated, auth is applied, and the request executes. Captures run next, followed by `post` scripts, the
`http expect` block, and `test` blocks. All results appear together in the Tests tab; script output appears in Console.

## The fw API

See the generated [fw script API](../../reference/fw-api/) for every declaration. Highlights:

- `fw.request` and `fw.response` for reading and changing messages.
- `fw.vars`, `fw.env`, and `fw.globals` for variables.
- `fw.test` and `fw.expect` for assertions, including `toMatchSchema`.
- `fw.sendRequest(path)` to call another Request, `fw.sleep(ms)`, and `fw.skip(reason)`.
- `fw.crypto` helpers and `fw.jsonpath`.

Share helpers by placing modules in the Collection's `lib/` folder and importing them as `lib/name.ts`.

## Trust

Scripts run only in a trusted Workspace. Fluxwire asks before running scripts from a Collection it has not seen, and
remembers the answer per Collection. In CI, scripts run without a prompt because you chose to run the command.

## Postman scripts

Imported Postman scripts keep working through the `pm` compatibility layer. Anything that depends on npm packages is
reported by the import so you can port it.
