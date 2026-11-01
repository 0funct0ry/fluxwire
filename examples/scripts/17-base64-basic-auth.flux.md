---
name: Manual basic auth
---

# Manual basic auth

Builds a Basic header from environment values instead of the `auth` front matter.

```http
GET {{BASE_URL}}/basic-auth/demo/secret
```

```ts pre
const pair = `demo:${fw.env.get('apiKey') ?? 'secret'}`;
fw.request.headers.set('authorization', 'Basic ' + btoa(pair));
```

```ts post
console.log('status', fw.response.status);
```
