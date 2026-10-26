---
name: Get payment
kind: request
seq: 2
---
## Get payment
Fetches a payment by id.

```http
GET   {{baseUrl}}/v1/payments/:paymentId   HTTP/1.1
Accept:application/json
X-Trace: 1
```

Example of a sample, not executed:

```http example
GET /v1/payments/pay_1
```

```bash
curl https://example.com
```

```http expect
HTTP 2xx
Content-Type: ~/json/
```
