---
title: Extending the format
description: How new protocols, roles, keys, and file kinds extend .flux.md without breaking older versions.
---

The format is designed so that later releases, third parties, and new protocols can extend it without breaking existing
files or misleading older versions.

## Extension points

| Point | How it extends | Examples |
|---|---|---|
| Request language | A new protocol registers the language of its request block. | `grpc`, `mcp`, `sse`, `mqtt`, `amqp`, `kafka` |
| Role | A protocol or feature registers new roles from the reserved vocabulary. | `graphql query`, `json variables`, `json arguments`, `grpc expect` |
| Front matter key | Core adds keys; extensions use `x-` keys. | `retry`, `x-team-owner` |
| Variable scheme | `{{scheme:reference}}` resolvers registered by providers. | `{{vault:secret/payments#key}}`, `{{aws-sm:prod/api#token}}` |
| Config block | Deeply nested settings live in `yaml config`, never in front matter. | gRPC TLS, MCP server launch settings |
| File kind | New `kind` values change how a file is run. | `flow` (several labelled request blocks) |

## Compatibility rules

Version 1.0 implements all of these.

1. **Preserve what you do not understand.** Unknown front matter keys, unknown roles, unknown request languages, and
   `x-` content are kept byte for byte on every save. The formatter rewrites only the spans it owns.
2. **Must-understand via `requires`.** A file that uses a feature that changes how it runs lists that feature in
   `requires`, for example `requires: [graphql]`. Fluxwire adds and removes entries automatically on save. A version
   that does not support every listed feature opens, edits, and saves the file normally but refuses to run it ("This
   request needs GraphQL support, available in Fluxwire 1.1."), shows diagnostic `F401`, and the CLI exits with code
   `66`.
3. **Unknown request language without `requires`** (hand-written or written by another tool) is treated the same way:
   editable, not runnable, with a diagnostic.
4. **Everything not in `requires` must be safe to ignore.** Core features are added only if ignoring them cannot change
   what a request sends or how its result is judged; otherwise they get a feature name.
5. **Reserved vocabulary.** The reserved roles (`query`, `variables`, `arguments`, `schema`, `step`, `response`,
   `fixture`, `mock`, `stream`, `metadata`, `resource`, `prompt`) and the request languages `grpc`, `mcp`, `sse`,
   `mqtt`, `amqp`, `kafka`, and `nats` are reserved now. Using them before their release yields "not supported in this
   version" rather than being treated as prose.
6. **Format version lives in the collection.** `fluxwire.toml` declares `format = 1`. A breaking change (not expected)
   would bump it and ship `fluxwire migrate`. Individual files carry no version line; `requires` carries per-file
   needs.

Feature names in `requires` are lower-case words. Version 1.0 defines none, because everything in 1.0 is the base
format. Post-1.0 features define `graphql`, `grpc`, `sparql`, `mcp`, `sse`, `flow`, and `retry`; secret providers
define `vault`, `aws-sm`, and `azkv`.

## Your own extensions

- Add front matter keys that start with `x-`; they are preserved and never interpreted.
- Add roles that start with `x-`, such as `yaml x-acme-lint`.
- Do not use a reserved role or language for anything else, since a future release will give it a meaning.

## Future protocols, sketched

These are not version 1.0 features. They show that each protocol fits without changing the base format. The current
tables are on the [Roles and info strings](../roles/) page.

**GraphQL** is an HTTP request plus roles:

````markdown
---
name: Payment by id
requires: [graphql]
---

```http
POST {{baseUrl}}/graphql
Authorization: Bearer {{accessToken}}
```

```graphql query
query Payment($id: ID!) { payment(id: $id) { status amount } }
```

```json variables
{ "id": "{{paymentId}}" }
```
````

**gRPC** is a new request language:

````markdown
---
name: Create payment (gRPC)
requires: [grpc]
---

```grpc
payments.v1.Payments/CreatePayment {{grpcHost}}
authorization: Bearer {{accessToken}}
```

```json message
{ "amount": 4200, "currency": "INR" }
```

```grpc expect
status: OK
```

```yaml config
proto:
  files: [protos/payments.proto]
tls:
  ca: certs/ca.pem
```
````

**MCP** tests an MCP server's tools, resources, and prompts:

````markdown
---
name: Search failed payments tool
requires: [mcp]
---

```mcp
tools/call search_payments
Server: {{paymentsMcpUrl}}
Authorization: Bearer {{accessToken}}
```

```json arguments
{ "query": "status:failed", "limit": 5 }
```

```mcp expect
isError: false

{ "content": [ { "type": "text" } ] }
```
````

The MCP request line is the JSON-RPC method and its target: `tools/call <tool>`, `resources/read <uri>`,
`prompts/get <prompt>`, or `tools/list`. `Server:` is a Streamable HTTP URL or `stdio:<command>`. Server-Sent Events,
MQTT, AMQP, Kafka, and NATS would follow the session pattern of WebSocket: a request block that opens the connection or
subscription, `message` blocks, and an `expect` block over received messages.
