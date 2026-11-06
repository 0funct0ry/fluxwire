---
name: deletePet
seq: 1
operation: deletePet
auth: none
path:
  petId: "{{petId}}"
---

# deletePet

```http
DELETE {{baseUrl}}/pets/:petId
```

```http expect
HTTP 204
```
