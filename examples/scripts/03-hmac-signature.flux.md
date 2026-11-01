---
name: HMAC signature
---

# HMAC signature

Signs the method, path, and timestamp with a shared secret.

```http
POST {{BASE_URL}}/post
Content-Type: application/json

{ "event": "ping" }
```

```ts pre
const ts = String(Date.now());
const payload = `${fw.request.method}${fw.request.path}${ts}${fw.request.body.text()}`;
fw.request.headers.set('x-timestamp', ts);
fw.request.headers.set('x-signature', fw.crypto.hmac('sha256', fw.env.get('hmacKey') ?? '', payload, 'hex'));
```

```ts post
console.log('server saw signature', fw.response.json().headers['X-Signature']);
```
