---
name: Get a pet
seq: 3
tags: [pets]
operation: GET /pets/{petId}
deprecated: true
path:
  petId: "{{petId}}"
---

# Get a pet

```http
GET {{baseUrl}}/pets/:petId
```

```http expect
HTTP 200
```
