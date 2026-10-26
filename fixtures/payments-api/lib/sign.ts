export function sign(body: string, key: string): string {
  return `sha256=${body.length}:${key.length}`;
}
