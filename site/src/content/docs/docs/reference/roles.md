---
title: Roles and info strings
description: Block roles, request languages and reserved names, generated from the protocol registry.
---

A fenced block's info string is `<language> [role]`. This page is generated from the `ProtocolRegistry`, so it
lists exactly what this version recognizes.

## Protocols

| Protocol | Request languages | Interaction | Feature in `requires` |
|---|---|---|---|
| `http` | `http` | Exchange | none (base format) |
| `ws` | `ws` | Session | none (base format) |
| `graphql` | `http` | Exchange | `graphql` |

## Protocol roles

| Protocol | Request language | Role | Content | How many |
|---|---|---|---|---|
| `http` | `http` | `expect` | Message | At most one |
| `http` | `http` | `form` | KvLines | At most one |
| `http` | `http` | `multipart` | KvLines | At most one |
| `http` | `http` | `defaults` | HeaderLines | At most one |
| `ws` | `ws` | `message` | Text | Any number |
| `graphql` | `graphql` | `query` | Text | At most one |
| `graphql` | `json` | `variables` | Text | At most one |

## Core roles

| Kind | Names |
|---|---|
| Script roles (languages `ts`, `js`) | `pre`, `post`, `test` |
| Other core roles | `config`, `example` |

## Reserved names

Reserved roles and request languages are not prose and not runnable. A file that uses one is editable and shows
"not supported in this version".

| Kind | Names |
|---|---|
| Reserved roles | `query`, `variables`, `arguments`, `schema`, `step`, `response`, `fixture`, `mock`, `stream`, `metadata`, `resource`, `prompt` |
| Reserved request languages | `grpc`, `mcp`, `sse`, `mqtt`, `amqp`, `kafka`, `nats` |
