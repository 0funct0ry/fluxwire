---
name: Skip on production
---

# Skip on production

A safety guard: ends the run without sending when the active environment is named `production`.

```http
DELETE {{BASE_URL}}/delete
```

```ts pre
if (fw.env.name === 'production') {
  fw.skip('Destructive request blocked on production');
}
```

```ts post
console.log('deleted on', fw.env.name, 'status', fw.response.status);
```
