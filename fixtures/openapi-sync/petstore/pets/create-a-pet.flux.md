---
name: Create a pet
seq: 2
tags: [pets]
operation: createPet
---

# Create a pet

```http
POST {{baseUrl}}/pets
Content-Type: application/json

{
  "id": 0,
  "name": "Rex",
  "tags": [
    "string"
  ]
}
```

```http expect
HTTP 201
```
