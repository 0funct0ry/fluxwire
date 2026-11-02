# Log in
POST https://api.example.com/login
Content-Type: application/json
{
  "user": "demo",
  "password": "{{password}}"
}

HTTP 200
[Captures]
token: jsonpath "$.token"
session: header "Set-Cookie"
[Asserts]
header "Content-Type" == "application/json"
jsonpath "$.token" exists
jsonpath "$.user.name" == "demo"
jsonpath "$.roles" count > 0
duration < 1000
xpath "string(//a)" == "x"

# Fetch profile
GET https://api.example.com/profile
Authorization: Bearer {{token}}
[QueryStringParams]
verbose: true
fields: name,email

HTTP 200
[Asserts]
status == 200
jsonpath "$.name" startsWith "De"
