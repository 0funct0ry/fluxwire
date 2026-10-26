/** HMAC-SHA256 signature of a request body, as `sha256=<hex>`. */
export function sign(body: string, key: string): string {
  return "sha256=" + fw.crypto.hmac("sha256", key, body);
}
