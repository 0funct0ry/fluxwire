---
name: Retry with backoff
---

# Retry with backoff

Retries a flaky endpoint through another request, doubling the wait each time.

```http
GET {{BASE_URL}}/status/200,503
```

```ts post
let delay = 250;
for (let attempt = 1; fw.response.status >= 500 && attempt <= 3; attempt++) {
  console.warn(`attempt ${attempt} failed, waiting ${delay}ms`);
  await fw.sleep(delay);
  delay *= 2;
  const res = await fw.sendRequest('14-retry-with-backoff.flux.md');
  if (res.status < 500) break;
}
```
