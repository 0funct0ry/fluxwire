---
name: Body checksum
---

# Body checksum

Sends a SHA-256 of the body so the server can verify integrity.

```http
POST {{BASE_URL}}/post
Content-Type: application/json

{ "file": "report.csv", "rows": 120 }
```

```ts pre
const digest = fw.crypto.sha256(fw.request.body.text(), 'hex');
fw.request.headers.set('x-content-sha256', digest);
fw.vars.set('bodyDigest', digest);
```

```ts post
console.log('digest echoed:', fw.response.json().headers['X-Content-Sha256'] === fw.vars.get('bodyDigest'));
```
