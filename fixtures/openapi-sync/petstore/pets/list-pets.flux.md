---
name: List pets
seq: 1
tags: [pets]
operation: listPets
---

# List pets

Returns all pets.

```http
GET {{baseUrl}}/pets
    # &limit=20
X-Trace: abc
```

```http expect
HTTP 200
```
