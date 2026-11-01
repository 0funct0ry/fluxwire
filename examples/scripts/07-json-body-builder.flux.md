---
name: JSON body builder
---

# JSON body builder

Generates a fresh order in the pre script and exposes its ID afterwards.

```http
POST {{BASE_URL}}/anything/orders
Content-Type: application/json

{}
```

```ts pre
const order = {
  id: `ord_${Math.floor(Math.random() * 1e6)}`,
  createdAt: new Date().toISOString(),
  items: [{ sku: 'A-1', qty: 2 }],
};
fw.vars.set('orderId', order.id);
fw.request.body.set(order);
```

```ts post
fw.test('echoed order id', () => fw.expect(fw.response.json().json.id).toBe(fw.vars.get('orderId')));
```
