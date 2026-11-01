---
name: JSONPath extract
---

# JSONPath extract

Extracts every title from the slideshow sample with `fw.jsonpath`.

```http
GET {{BASE_URL}}/json
```

```ts post
const titles = fw.jsonpath(fw.response.json(), '$.slideshow.slides[*].title');
fw.vars.set('firstTitle', titles[0]);
console.log('titles:', titles);
```
