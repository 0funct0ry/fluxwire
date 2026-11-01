---
name: API key from environment
---

# API key from environment

Adds the API key only when it is defined, and fails fast otherwise.

```http
GET {{BASE_URL}}/headers
```

```ts pre
const key = fw.env.get('apiKey');
if (!key) throw new Error('apiKey is not set in the active environment');
fw.request.headers.set('x-api-key', key);
```

```ts post
console.log('server received a key:', 'X-Api-Key' in fw.response.json().headers);
```
