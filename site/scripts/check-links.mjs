// Fails when an internal link or asset in dist/ does not resolve.
import { readdirSync, readFileSync, existsSync, statSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
const dist = resolve('dist'), base = '/fluxwire';
const walk = (d) => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : [join(d, f)]));
let bad = 0;
for (const f of walk(dist).filter((x) => x.endsWith('.html'))) {
  const html = readFileSync(f, 'utf8');
  for (const m of html.matchAll(/(?:href|src)="([^"#?]+)[^"]*"/g)) {
    let u = m[1];
    if (/^(https?:|mailto:|data:|\/\/)/.test(u)) continue;
    let p = u.startsWith('/') ? (u.startsWith(base) ? u.slice(base.length) : u) : resolve(dirname(f.replace(dist, '')), u);
    p = join(dist, p);
    if (!(existsSync(p) && (!statSync(p).isDirectory() || existsSync(join(p, 'index.html'))))) { console.error(`${f.replace(dist, '')}: broken ${u}`); bad++; }
  }
}
if (bad) process.exit(1);
console.log('links ok');
