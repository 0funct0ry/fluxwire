---
name: Feature flag header
---

# Feature flag header

Enables a beta flag outside production and removes any stale override header.

```http
GET {{BASE_URL}}/headers
x-feature-beta: stale
```

```ts pre
fw.request.headers.remove('x-feature-beta');
if (fw.env.name !== 'production') fw.request.headers.set('x-feature-beta', 'on');
```

```ts post
console.log('beta header seen:', fw.response.json().headers['X-Feature-Beta'] ?? 'none');
```
