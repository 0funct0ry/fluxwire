---
name: Store token
---

# Store token

Simulates a login. The token is saved as a secret and persisted to the environment's local file.

```http
POST {{BASE_URL}}/anything/login
Content-Type: application/json

{ "user": "demo" }
```

```ts post
fw.vars.setSecret('token', 'demo-token-' + Date.now());
fw.env.persist('token');
```
