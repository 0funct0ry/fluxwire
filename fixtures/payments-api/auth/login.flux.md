---
name: Login
seq: 1
capture:
  accessToken: $.access_token
---

# Login

Exchanges credentials for an access token.

```http
POST {{baseUrl}}/v1/auth/token
Content-Type: application/json

{
  "client_id": "{{clientId}}",
  "client_secret": "{{clientSecret}}"
}
```

```http expect
HTTP 200
Content-Type: application/json
```
