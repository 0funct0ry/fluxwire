---
name: Timing report
---

# Timing report

Logs phase timings and warns when the response is slow.

```http
GET {{BASE_URL}}/delay/1
```

```ts post
console.log('timings', fw.response.timings);
if (fw.response.time > 800) console.warn(`Slow response: ${fw.response.time}ms`);
```
