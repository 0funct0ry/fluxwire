---
name: Request ID header
---

# Request ID header

Adds a unique correlation ID before sending and logs the one the server echoed back.

```http
GET {{BASE_URL}}/headers
accept: application/json
```

```ts pre
const id = crypto.randomUUID();
fw.vars.set('requestId', id);
fw.request.headers.set('x-request-id', id);
```

```ts post
const echoed = fw.response.json().headers['X-Request-Id'];
console.log('sent', fw.vars.get('requestId'), 'echoed', echoed);
```
