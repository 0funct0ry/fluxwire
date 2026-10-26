---
name: Payments
seq: 2
auth:
  type: bearer
  token: "{{accessToken}}"
settings:
  run_mode: sequential
---

# Payments

Card and UPI payments. Amounts are in the smallest currency unit.

```http defaults
X-Merchant-Id: {{merchantId}}
```
