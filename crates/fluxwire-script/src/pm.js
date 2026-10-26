(function (g) {
  "use strict";
  const { deepEqual, show, getPath, AssertionError } = g.__fwInternals;
  const fw = g.fw;

  const unsupported = (path) =>
    new Proxy(function () {}, {
      get: (_t, p) => (typeof p === "symbol" ? undefined : unsupported(path + "." + p)),
      apply: () => { throw new Error("Unsupported in Fluxwire: " + path); },
      construct: () => { throw new Error("Unsupported in Fluxwire: " + path); },
    });
  const strict = (obj, path) =>
    new Proxy(obj, {
      get: (t, p, r) => (typeof p === "symbol" || p in t ? Reflect.get(t, p, r) : unsupported(path + "." + p)),
    });

  // ---- chai (BDD subset) ----------------------------------------------------------
  const typeOf = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v);
  class Chai {
    constructor(actual, neg = false, deep = false, label = undefined) {
      this.actual = actual; this.neg = neg; this._deep = deep;
    }
    _chk(pass, desc, expected) {
      if (pass === this.neg) {
        throw new AssertionError(`expected ${show(this.actual)} ${this.neg ? "not " : ""}${desc}`, expected, this.actual);
      }
    }
    get not() { return new Chai(this.actual, !this.neg, this._deep); }
    get ok() { this._chk(!!this.actual, "to be truthy", true); return this; }
    get true() { this._chk(this.actual === true, "to be true", true); return this; }
    get false() { this._chk(this.actual === false, "to be false", false); return this; }
    get null() { this._chk(this.actual === null, "to be null", null); return this; }
    get undefined() { this._chk(this.actual === undefined, "to be undefined", undefined); return this; }
    get exist() { this._chk(this.actual !== null && this.actual !== undefined, "to exist", "exists"); return this; }
    get empty() {
      const a = this.actual;
      const n = typeof a === "string" || Array.isArray(a) ? a.length : a && typeof a === "object" ? Object.keys(a).length : 1;
      this._chk(n === 0, "to be empty", "empty"); return this;
    }
    a(type) { this._chk(typeOf(this.actual) === type, `to be a ${type}`, type); return this; }
    equal(v) { this._chk(this._deep ? deepEqual(this.actual, v) : Object.is(this.actual, v), `to equal ${show(v)}`, v); return this; }
    eql(v) { this._chk(deepEqual(this.actual, v), `to deeply equal ${show(v)}`, v); return this; }
    include(v) {
      const a = this.actual;
      const pass = typeof a === "string" ? a.includes(v) : Array.isArray(a) ? a.some((x) => deepEqual(x, v)) :
        a && typeof a === "object" && v && typeof v === "object" ? Object.keys(v).every((k) => deepEqual(a[k], v[k])) : false;
      this._chk(pass, `to include ${show(v)}`, v); return this;
    }
    property(name, ...rest) {
      const r = getPath(this.actual, [name]);
      this._chk(r.found && (rest.length === 0 || deepEqual(r.value, rest[0])), `to have property ${show(name)}`, rest[0]);
      return rest.length ? this : new Chai(r.value, false, false);
    }
    lengthOf(n) { this._chk(this.actual && this.actual.length === n, `to have length ${n}`, n); return this; }
    above(n) { this._chk(this.actual > n, `to be above ${n}`, n); return this; }
    below(n) { this._chk(this.actual < n, `to be below ${n}`, n); return this; }
    least(n) { this._chk(this.actual >= n, `to be at least ${n}`, n); return this; }
    most(n) { this._chk(this.actual <= n, `to be at most ${n}`, n); return this; }
    match(re) { this._chk(re.test(String(this.actual)), `to match ${String(re)}`, String(re)); return this; }
    string(s) { this._chk(typeof this.actual === "string" && this.actual.includes(s), `to contain string ${show(s)}`, s); return this; }
    oneOf(list) { this._chk(list.some((x) => deepEqual(x, this.actual)), `to be one of ${show(list)}`, list); return this; }
    keys(...ks) {
      const flat = ks.length === 1 && Array.isArray(ks[0]) ? ks[0] : ks;
      const have = Object.keys(this.actual || {});
      this._chk(flat.every((k) => have.includes(k)) && have.length === flat.length, `to have keys ${show(flat)}`, flat); return this;
    }
    status(code) {
      const r = this.actual;
      const pass = typeof code === "number" ? r.code === code : r.status === code;
      this._chk(pass, `to have status ${show(code)}`, code); return this;
    }
  }
  for (const w of ["to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same", "does", "but", "still"]) {
    Object.defineProperty(Chai.prototype, w, { get() { return this; } });
  }
  for (const [alias, name] of [["an", "a"], ["equals", "equal"], ["eq", "equal"], ["includes", "include"], ["contain", "include"], ["contains", "include"], ["gt", "above"], ["greaterThan", "above"], ["lt", "below"], ["lessThan", "below"], ["lengthOf", "lengthOf"], ["length", "lengthOf"]]) {
    Chai.prototype[alias] = Chai.prototype[name];
  }
  Object.defineProperty(Chai.prototype, "deep", { get() { return new Chai(this.actual, this.neg, true); } });

  // ---- pm -------------------------------------------------------------------------
  const lookup = (n) => { const v = fw.vars.get(n); return v !== undefined ? v : fw.env.get(n); };
  const varScope = (extra) => strict({
    get: (n) => lookup(n),
    set: (n, v) => fw.vars.set(n, v),
    unset: (n) => fw.vars.unset(n),
    has: (n) => lookup(n) !== undefined,
    toObject: () => ({}),
    ...extra,
  }, "pm.variables");

  const req = fw.request;
  const res = fw.response;
  const pm = strict({
    environment: varScope({ name: fw.env.name }),
    variables: varScope(),
    collectionVariables: varScope(),
    globals: strict({
      get: (n) => { const v = fw.globals.get(n); return v !== undefined ? v : fw.vars.get(n); },
      set: (n, v) => fw.vars.set(n, v),
      unset: (n) => fw.vars.unset(n),
      has: (n) => fw.globals.get(n) !== undefined,
    }, "pm.globals"),
    info: { requestName: req ? req.name : "", requestId: req ? req.path : "", eventName: res ? "test" : "prerequest" },
    test: (name, fn) => fw.test(name, fn),
    expect: (v) => new Chai(v),
    sendRequest: (target, cb) => {
      const t = typeof target === "string" ? { url: target, method: "GET" } : target;
      const spec = {
        method: t.method || "GET",
        url: typeof t.url === "object" && t.url !== null ? t.url.raw || String(t.url) : t.url,
        headers: Array.isArray(t.header) ? t.header.map((h) => [h.key, h.value]) : Object.entries(t.header || {}),
        body: t.body && t.body.raw !== undefined ? t.body.raw : null,
      };
      fw.sendRequest(spec).then((r) => cb && cb(null, pmResponse(r)), (e) => cb && cb(e));
    },
  }, "pm");
  function pmResponse(r) {
    const base = {
      code: r.status,
      status: r.statusText,
      responseTime: r.time,
      responseSize: r.size,
      headers: { get: (n) => r.headers.get(n), has: (n) => r.headers.has(n) },
      json: () => r.json(),
      text: () => r.text(),
      to: {
        have: {
          status: (c) => new Chai(base).status(c),
          header: (n, v) => { const x = r.headers.get(n); new Chai(x).exist; if (v !== undefined) new Chai(x).equal(v); },
          jsonBody: (path) => { if (path !== undefined) new Chai(r.json()).property(path); },
        },
        be: {
          get ok() { new Chai(r.status >= 200 && r.status < 300).true; return true; },
          get success() { new Chai(r.status >= 200 && r.status < 300).true; return true; },
          get error() { new Chai(r.status >= 400).true; return true; },
        },
      },
    };
    return base;
  }
  if (req) {
    pm.request = {
      get url() { return { toString: () => req.url, getHost: () => new URL(req.url).hostname, getPath: () => new URL(req.url).pathname, getQueryString: () => new URL(req.url).search.replace(/^\?/, "") }; },
      method: req.method,
      headers: {
        get: (n) => req.headers.get(n),
        add: (h) => req.headers.set(h.key, h.value),
        upsert: (h) => req.headers.set(h.key, h.value),
        remove: (n) => req.headers.remove(n),
        has: (n) => req.headers.get(n) !== undefined,
      },
      body: { get raw() { return req.body.text(); } },
    };
  }
  if (res) pm.response = pmResponse(res);
  g.pm = pm;
  g.require = (name) => { throw new Error(`Unsupported in Fluxwire: require('${name}') (npm packages are not available; import from lib/ instead)`); };
})(globalThis);
