---
name: Response contract tests
---

# Response contract tests

Prepares a request, then asserts the response contract in a `test` block and logs a one-line summary.

```http
GET {{BASE_URL}}/json
accept: application/json
```

```ts pre
fw.vars.set('startedAt', new Date().toISOString());
```

```ts post
console.log(`[${fw.vars.get('startedAt')}] ${fw.request.method} ${fw.request.path} -> ${fw.response.status} (${fw.response.size} bytes)`);
```

```ts test
fw.test('status is 200', () => fw.expect(fw.response.status).toBe(200));
fw.test('is JSON', () => fw.expect(fw.response.headers.get('content-type')).toMatch(/json/));
fw.test('has slideshow', () => fw.expect(fw.response.json()).toHaveProperty('slideshow.title'));
```
