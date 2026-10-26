---
name: Broken
badline
---

# Broken

```http
POST
Content-Type application/json
X-Ok: 1
```

```http
GET /second
```

```ts bogus
let x = 1;
```

```http expect
HTTP nope
```

```http
GET /unclosed
