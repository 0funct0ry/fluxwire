(function (g) {
  "use strict";
  const N = (op, arg) => {
    const r = JSON.parse(__native(op, JSON.stringify(arg === undefined ? null : arg)));
    if (r.err !== undefined) throw new Error(r.err);
    return r.ok;
  };
  const init = JSON.parse(__init);

  // ---- helpers ----------------------------------------------------------------
  const show = (v) => {
    if (v === undefined) return "undefined";
    if (typeof v === "function") return "[Function]";
    try {
      const s = JSON.stringify(v);
      return s === undefined ? String(v) : s;
    } catch (e) {
      return String(v);
    }
  };
  const str = (v) =>
    typeof v === "string" ? v : v === undefined || v === null ? "" : typeof v === "object" ? JSON.stringify(v) : String(v);
  function deepEqual(a, b) {
    if (Object.is(a, b)) return true;
    if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    const ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (const k of ka) if (!Object.prototype.hasOwnProperty.call(b, k) || !deepEqual(a[k], b[k])) return false;
    return true;
  }
  function getPath(obj, path) {
    const parts = Array.isArray(path) ? path : String(path).replace(/\[(\d+)\]/g, ".$1").split(".").filter((p) => p !== "");
    let cur = obj;
    for (const p of parts) {
      if (cur === null || cur === undefined || !(p in Object(cur))) return { found: false };
      cur = cur[p];
    }
    return { found: true, value: cur };
  }

  class AssertionError extends Error {
    constructor(message, expected, actual) {
      super(message);
      this.name = "AssertionError";
      this.expected = expected;
      this.actual = actual;
    }
  }

  // ---- console ----------------------------------------------------------------
  const fmtArgs = (args) => args.map((a) => (typeof a === "string" ? a : a instanceof Error ? a.stack || a.message : show(a))).join(" ");
  g.console = {};
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    g.console[level] = (...args) => { N("log", { level, text: fmtArgs(args) }); };
  }

  // ---- headers / request / response -------------------------------------------
  const findIdx = (list, name) => list.findIndex((h) => h[0].toLowerCase() === String(name).toLowerCase());
  function readHeaders(list) {
    return {
      get: (n) => { const i = findIdx(list, n); return i < 0 ? undefined : list[i][1]; },
      has: (n) => findIdx(list, n) >= 0,
      all: () => { const o = {}; for (const [k, v] of list) o[k] = v; return o; },
    };
  }
  function makeRequest(r) {
    const list = r.headers;
    const op = (o) => N("req.op", o);
    const hdr = readHeaders(list);
    hdr.set = (n, v) => {
      v = String(v);
      const i = findIdx(list, n);
      if (i < 0) list.push([String(n), v]); else list[i][1] = v;
      op({ op: "set_header", name: String(n), value: v });
    };
    hdr.remove = (n) => {
      const i = findIdx(list, n);
      if (i >= 0) list.splice(i, 1);
      op({ op: "remove_header", name: String(n) });
    };
    return {
      get method() { return r.method; },
      set method(v) { r.method = String(v); op({ op: "method", value: r.method }); },
      get url() { return r.url; },
      set url(v) { r.url = String(v); op({ op: "url", value: r.url }); },
      name: r.name,
      path: r.path,
      headers: hdr,
      body: {
        text: () => (r.body === null || r.body === undefined ? "" : r.body),
        json: () => JSON.parse(r.body),
        set: (v) => { r.body = str(v); op({ op: "body", value: r.body }); },
      },
    };
  }
  function makeResponse(r) {
    return {
      status: r.status,
      statusText: r.statusText,
      headers: readHeaders(r.headers),
      text: () => r.text,
      json: () => JSON.parse(r.text),
      time: r.time,
      size: r.size,
      timings: r.timings,
    };
  }

  // ---- variables ----------------------------------------------------------------
  const vars = new Map(Object.entries(init.vars));
  const envVars = new Map(Object.entries(init.env));
  const setVar = (name, value, secret) => {
    value = str(value);
    vars.set(name, value);
    N("vars.set", { name, value, secret });
  };

  // ---- expect -----------------------------------------------------------------
  class Expectation {
    constructor(actual, negate) { this.actual = actual; this.negate = negate; }
    get not() { return new Expectation(this.actual, !this.negate); }
    _check(pass, desc, expected) {
      if (pass === this.negate) {
        throw new AssertionError(`expected ${show(this.actual)} ${this.negate ? "not " : ""}${desc}`, expected, this.actual);
      }
    }
    toBe(e) { this._check(Object.is(this.actual, e), `to be ${show(e)}`, e); }
    toEqual(e) { this._check(deepEqual(this.actual, e), `to equal ${show(e)}`, e); }
    toBeTruthy() { this._check(!!this.actual, "to be truthy", true); }
    toBeFalsy() { this._check(!this.actual, "to be falsy", false); }
    toContain(e) {
      const a = this.actual;
      const pass = typeof a === "string" ? a.includes(e) : Array.isArray(a) ? a.some((x) => deepEqual(x, e)) : false;
      this._check(pass, `to contain ${show(e)}`, e);
    }
    toMatch(re) {
      const rx = re instanceof RegExp ? re : new RegExp(re);
      this._check(typeof this.actual === "string" && rx.test(this.actual), `to match ${String(rx)}`, String(rx));
    }
    toHaveProperty(path, ...rest) {
      const r = getPath(this.actual, path);
      const hasValue = rest.length > 0;
      this._check(r.found && (!hasValue || deepEqual(r.value, rest[0])), `to have property ${show(path)}${hasValue ? " = " + show(rest[0]) : ""}`, hasValue ? rest[0] : path);
    }
    toHaveLength(n) {
      const len = this.actual === null || this.actual === undefined ? undefined : this.actual.length;
      this._check(len === n, `to have length ${n} (got ${len})`, n);
    }
    toBeGreaterThan(n) { this._check(this.actual > n, `to be greater than ${n}`, n); }
    toBeLessThan(n) { this._check(this.actual < n, `to be less than ${n}`, n); }
    toBeOneOf(list) { this._check(list.some((x) => deepEqual(x, this.actual)), `to be one of ${show(list)}`, list); }
    toMatchSchema(schema) {
      const r = N("schema", { value: this.actual, schema });
      this._check(r.valid, `to match schema${r.valid ? "" : ": " + r.errors.join("; ")}`, schema);
    }
  }

  // ---- tests ------------------------------------------------------------------
  const record = (name, passed, err) => {
    const e = err || {};
    N("test.record", {
      name,
      passed,
      message: passed ? null : String((err && err.message) || err),
      expected: !passed && "expected" in e ? show(e.expected) : null,
      actual: !passed && "actual" in e ? show(e.actual) : null,
    });
  };
  const test = (name, fn) => {
    try {
      const r = fn();
      if (r && typeof r.then === "function") r.then(() => record(name, true), (e) => record(name, false, e));
      else record(name, true);
    } catch (e) {
      record(name, false, e);
    }
  };

  // ---- sending ----------------------------------------------------------------
  const sendRequest = (target, opts) =>
    new Promise((resolve) => resolve(makeResponse(N("send", { target, vars: (opts && opts.vars) || {} }))));

  // ---- crypto / encoding ------------------------------------------------------
  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  g.btoa = (s) => {
    s = String(s);
    let out = "";
    for (let i = 0; i < s.length; i += 3) {
      const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error("btoa: character out of range");
      out += B64[a >> 2] + B64[((a & 3) << 4) | ((b || 0) >> 4)];
      out += i + 1 < s.length ? B64[((b & 15) << 2) | ((c || 0) >> 6)] : "=";
      out += i + 2 < s.length ? B64[c & 63] : "=";
    }
    return out;
  };
  g.atob = (s) => {
    s = String(s).replace(/[\s=]+$/g, "");
    let out = "", bits = 0, acc = 0;
    for (const ch of s) {
      const v = B64.indexOf(ch);
      if (v < 0) throw new Error("atob: invalid character");
      acc = (acc << 6) | v;
      bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); }
    }
    return out;
  };
  g.TextEncoder = class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(s = "") {
      const u = unescape(encodeURIComponent(String(s)));
      const out = new Uint8Array(u.length);
      for (let i = 0; i < u.length; i++) out[i] = u.charCodeAt(i);
      return out;
    }
  };
  g.TextDecoder = class TextDecoder {
    get encoding() { return "utf-8"; }
    decode(b) {
      if (!b) return "";
      const bytes = b instanceof ArrayBuffer ? new Uint8Array(b) : b;
      let s = "";
      for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
      return decodeURIComponent(escape(s));
    }
  };
  g.crypto = { randomUUID: () => N("uuid") };
  const hash = (alg) => (data, enc) => N("crypto", { alg, data: str(data), enc });
  const fwCrypto = {
    sha256: hash("sha256"),
    sha512: hash("sha512"),
    hmac: (alg, key, data, enc) => N("crypto", { alg: "hmac-" + alg, key: str(key), data: str(data), enc }),
  };

  // ---- URL / URLSearchParams --------------------------------------------------
  class URLSearchParams {
    constructor(init) {
      this._l = [];
      if (typeof init === "string") {
        for (const part of init.replace(/^\?/, "").split("&")) {
          if (!part) continue;
          const i = part.indexOf("=");
          const dec = (x) => decodeURIComponent(x.replace(/\+/g, " "));
          this._l.push(i < 0 ? [dec(part), ""] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
        }
      } else if (Array.isArray(init)) {
        for (const [k, v] of init) this._l.push([String(k), String(v)]);
      } else if (init && typeof init === "object") {
        for (const k of Object.keys(init)) this._l.push([k, String(init[k])]);
      }
    }
    append(k, v) { this._l.push([String(k), String(v)]); }
    delete(k) { this._l = this._l.filter((p) => p[0] !== k); }
    get(k) { const p = this._l.find((x) => x[0] === k); return p ? p[1] : null; }
    getAll(k) { return this._l.filter((x) => x[0] === k).map((x) => x[1]); }
    has(k) { return this._l.some((x) => x[0] === k); }
    set(k, v) {
      const i = this._l.findIndex((x) => x[0] === k);
      if (i < 0) this._l.push([String(k), String(v)]);
      else { this._l[i][1] = String(v); this._l = this._l.filter((x, j) => x[0] !== k || j === i); }
    }
    forEach(cb) { for (const [k, v] of this._l) cb(v, k, this); }
    entries() { return this._l.map((p) => [p[0], p[1]])[Symbol.iterator](); }
    keys() { return this._l.map((p) => p[0])[Symbol.iterator](); }
    values() { return this._l.map((p) => p[1])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    toString() { return this._l.map(([k, v]) => encodeURIComponent(k) + "=" + encodeURIComponent(v)).join("&"); }
  }
  class URL {
    constructor(url, base) {
      let s = String(url);
      if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(s)) {
        if (base === undefined) throw new TypeError("Invalid URL: " + s);
        const b = new URL(base);
        s = s.startsWith("/") ? b.origin + s : b.origin + b.pathname.replace(/[^/]*$/, "") + s;
      }
      const m = /^([a-zA-Z][a-zA-Z0-9+.-]*:)\/\/(?:([^:@/]*)(?::([^@/]*))?@)?([^:/?#]*)(?::(\d+))?([^?#]*)(\?[^#]*)?(#.*)?$/.exec(s);
      if (!m) throw new TypeError("Invalid URL: " + s);
      this.protocol = m[1].toLowerCase();
      this.username = m[2] || "";
      this.password = m[3] || "";
      this.hostname = m[4].toLowerCase();
      this.port = m[5] || "";
      this.pathname = m[6] || "/";
      this.search = m[7] && m[7] !== "?" ? m[7] : "";
      this.hash = m[8] && m[8] !== "#" ? m[8] : "";
      this.searchParams = new URLSearchParams(this.search);
    }
    get host() { return this.hostname + (this.port ? ":" + this.port : ""); }
    get origin() { return this.protocol + "//" + this.host; }
    get href() {
      const q = this.searchParams.toString();
      const auth = this.username ? this.username + (this.password ? ":" + this.password : "") + "@" : "";
      return this.protocol + "//" + auth + this.host + this.pathname + (q ? "?" + q : "") + this.hash;
    }
    toString() { return this.href; }
    toJSON() { return this.href; }
  }
  g.URL = URL;
  g.URLSearchParams = URLSearchParams;

  // ---- fw -----------------------------------------------------------------------
  const fw = {
    request: init.request ? makeRequest(init.request) : undefined,
    response: init.response ? makeResponse(init.response) : undefined,
    vars: {
      get: (n) => vars.get(n),
      set: (n, v) => setVar(n, v, false),
      setSecret: (n, v) => setVar(n, v, true),
      unset: (n) => { vars.delete(n); N("vars.unset", { name: n }); },
      has: (n) => vars.has(n),
    },
    env: {
      get: (n) => envVars.get(n),
      set: (n, v) => { v = str(v); envVars.set(n, v); N("env.set", { name: n, value: v }); },
      persist: (n) => { N("env.persist", { name: n }); },
      name: init.envName,
    },
    globals: { get: (n) => init.globals[n] },
    test,
    expect: (v) => new Expectation(v, false),
    sendRequest,
    sleep: (ms) => new Promise((resolve) => { N("sleep", ms); resolve(); }),
    skip: (reason) => { N("skip", { reason: reason === undefined ? "" : String(reason) }); },
    crypto: fwCrypto,
    jsonpath: (obj, expr) => N("jsonpath", { value: obj, expr }),
    AssertionError,
  };
  g.fw = fw;
  g.__fwInternals = { N, makeResponse, deepEqual, show, getPath, Expectation, AssertionError };
})(globalThis);
