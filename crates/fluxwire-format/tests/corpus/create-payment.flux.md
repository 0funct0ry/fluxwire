---
name: Create payment
tags: [smoke, payments]
operation: createPayment
auth: inherit
capture:
  paymentId: $.id
  location: header Location
---

# Create payment

Creates a payment. `Idempotency-Key` is generated on every send, so
re-sending creates a new payment. Pin it with `fw.vars` to replay.

```http
POST {{baseUrl}}/v1/payments
    ?expand=customer
    # &debug=true
Content-Type: application/json
Idempotency-Key: {{idemKey}}
# X-Debug: 1

{
  "amount": 4200,
  "currency": "INR",
  "customer": "{{customerId}}"
}
```

The response must be a 201 with the payment in `requires_capture`:

```http expect
HTTP 201
Content-Type: application/json

{ "status": "requires_capture", "amount": 4200 }
```

```ts pre
import { sign } from "lib/sign.ts";
fw.vars.set("idemKey", crypto.randomUUID());
```

Every payment gets an id we can use in later requests:

```ts test
fw.test("has an id", () => fw.expect(fw.response.json()).toHaveProperty("id"));
```
