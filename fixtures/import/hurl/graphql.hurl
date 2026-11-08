# Country by code
POST https://countries.trevorblades.com/graphql
```graphql
query Country($code: ID!) {
  country(code: $code) {
    name
  }
}

variables {
  "code": "NO"
}
```
HTTP 200
