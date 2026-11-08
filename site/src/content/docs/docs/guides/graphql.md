---
title: GraphQL
description: Send GraphQL operations, explore the schema and build queries visually.
---

A GraphQL request is an ordinary `http` request plus a `graphql query` block and, optionally, a `json variables` block.
Fluxwire adds `requires: [graphql]` to the front matter when you save, so older versions open the file but refuse to run
it.

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

## How the request is sent

A `POST` request sends a JSON body with `query`, `variables` and `operationName`. A `GET` request sends the same three
values in the query string. Headers, authentication, environments and scripts work as they do for HTTP. When a file holds
several operations, set `operation: Name` in the front matter; a single named operation is sent by name automatically.

## Checking the response

`http expect` matches partially. An expected JSON object that names neither `data` nor `errors` is compared with the
`data` member of the response, so you only write the fields you care about:

````markdown
```http expect
200

{ "payment": { "status": "PAID" } }
```
````

## Schema, completion and the builder

Run `graphql: introspect` from the command palette to fetch the schema. It is cached under `.fluxwire/graphql/` for the
URL and the active environment, so completion and validation work offline afterwards. With a schema the query editor
suggests fields, arguments, enum values and variables, and underlines unknown fields, missing required arguments and
undefined variables.

The query builder shows the schema as a field tree with checkboxes. Arrows open an object field without selecting it, and
the filter box finds a field several levels down. Checking a field fills its required arguments at once, so the query stays valid. A selected field lists its arguments
(sent with the request, required ones marked) apart from its fields (returned): type a value in the argument input
(bare words are quoted for `String` and `ID` arguments), or press **$ variable**, which declares the variable and creates
the `json variables` block with a placeholder value. The **Variables** pane under the query shows the `json variables` block as a key/value
table or as JSON (use the **Table** and **JSON** buttons), and its top edge can be dragged to change its height. Checking a field
writes it into the `graphql query` block, and editing the text updates the tree whenever the query parses. Unchecking
every field leaves an empty `query { }` skeleton. The builder works on documents with one operation and no fragments.

## From the command line

`fluxwire run` and `fluxwire send` need no GraphQL-specific options. A build of Fluxwire without GraphQL support exits
with code `66` for these files.
