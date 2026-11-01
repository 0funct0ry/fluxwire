---
name: Error logger
---

# Error logger

Logs the body of any failing response so the Console tab explains it.

```http
GET {{BASE_URL}}/status/418
```

```ts post
if (fw.response.status >= 400) {
  console.error(`${fw.response.status} ${fw.response.statusText}`, fw.response.text());
}
```
