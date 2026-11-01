---
name: Rate limit headers
---

# Rate limit headers

Reads response headers and warns when the remaining quota is low.

```http
GET {{BASE_URL}}/response-headers?x-ratelimit-remaining=3
```

```ts post
const left = Number(fw.response.headers.get('x-ratelimit-remaining') ?? Infinity);
if (left < 5) console.warn(`Only ${left} calls left`);
else console.log('quota ok', left);
```
