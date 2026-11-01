---
name: Pagination collector
---

# Pagination collector

Fetches several pages after the first response and stores the combined count.

```http
GET {{BASE_URL}}/anything?page=1
```

```ts post
let total = 0;
for (let page = 2; page <= 3; page++) {
  const res = await fw.sendRequest('16-pagination-collector.flux.md', { vars: { page } });
  total += Object.keys(res.json().args).length;
}
fw.vars.set('pagesFetched', 3);
console.log('extra args seen', total);
```
