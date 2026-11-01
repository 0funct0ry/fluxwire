---
name: Login token chain
---

# Login token chain

Fetches a token with another request only when none is cached, then stores it as a secret.

```http
GET {{BASE_URL}}/bearer
accept: application/json
```

```ts pre
if (!fw.vars.has('token')) {
  const res = await fw.sendRequest('06-store-token.flux.md');
  console.log('login status', res.status);
}
fw.request.headers.set('authorization', `Bearer ${fw.vars.get('token')}`);
```

```ts post
console.log('authenticated:', fw.response.json().authenticated);
```
