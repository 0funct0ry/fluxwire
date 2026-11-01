---
name: Timestamp header
---

# Timestamp header

Stamps the request with an ISO time and records the round trip.

```http
GET {{BASE_URL}}/get
```

```ts pre
fw.vars.set('sentAt', Date.now());
fw.request.headers.set('x-sent-at', new Date().toISOString());
```

```ts post
const rtt = Date.now() - Number(fw.vars.get('sentAt'));
console.log(`wall clock ${rtt}ms, engine ${fw.response.time}ms`);
```
