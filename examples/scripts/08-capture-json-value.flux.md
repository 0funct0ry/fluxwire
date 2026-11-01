---
name: Capture JSON value
---

# Capture JSON value

Pulls a value out of the response body for later requests.

```http
GET {{BASE_URL}}/uuid
```

```ts post
const uuid = fw.response.json().uuid;
fw.vars.set('lastUuid', uuid);
console.log('captured', uuid);
```
