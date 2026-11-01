---
name: Use captured value
---

# Use captured value

Run [08](08-capture-json-value.flux.md) first. This request refuses to send without the captured value.

```http
GET {{BASE_URL}}/anything/{{lastUuid}}
```

```ts pre
if (!fw.vars.has('lastUuid')) fw.skip('Run 08-capture-json-value first');
```

```ts post
console.log('url was', fw.response.json().url);
```
