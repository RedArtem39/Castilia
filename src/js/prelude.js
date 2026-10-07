// Standard web platform pieces implemented on top of the native DOM/BOM bridges.
(function (G) {
  'use strict';
  const hide = (o, k, v) => Object.defineProperty(o, k, { value: v, writable: true, configurable: true, enumerable: false });
  const expose = (name, v) => hide(G, name, v);

  class DOMException extends Error {
    constructor(message = '', name = 'Error') {
      super(message);
      hide(this, 'name', name);
      const codes = { IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4, InvalidCharacterError: 5, NotFoundError: 8, NotSupportedError: 9, InvalidStateError: 11, SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14, InvalidAccessError: 15, SecurityError: 18, NetworkError: 19, AbortError: 20, TimeoutError: 23, DataCloneError: 25 };
      hide(this, 'code', codes[name] || 0);
    }
  }
  expose('DOMException', DOMException);

  // ---- URL -------------------------------------------------------------
  const blobUrls = new Map();
  const enc = (s) => encodeURIComponent(s).replace(/%20/g, '+').replace(/[!'()~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
  const dec = (s) => { try { return decodeURIComponent(s.replace(/\+/g, ' ')); } catch (e) { return s; } };

  class URLSearchParams {
    #list = [];
    #url = null;
    constructor(init) {
      if (init === undefined || init === null) return;
      if (typeof init === 'string') this.#parse(init);
      else if (init instanceof URLSearchParams) this.#list = init.#list.map((p) => [p[0], p[1]]);
      else if (typeof init[Symbol.iterator] === 'function') { for (const p of init) this.#list.push([String(p[0]), String(p[1])]); }
      else for (const k of Object.keys(init)) this.#list.push([k, String(init[k])]);
    }
    #parse(s) {
      this.#list = [];
      if (s[0] === '?') s = s.slice(1);
      if (!s) return;
      for (const part of s.split('&')) {
        if (!part) continue;
        const i = part.indexOf('=');
        this.#list.push(i < 0 ? [dec(part), ''] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
      }
    }
    _attach(url) { this.#url = url; }
    _reset(s) { this.#parse(s); }
    #update() { if (this.#url) this.#url._setSearch(this.toString()); }
    get size() { return this.#list.length; }
    append(k, v) { this.#list.push([String(k), String(v)]); this.#update(); }
    delete(k, v) { this.#list = this.#list.filter((p) => !(p[0] === String(k) && (v === undefined || p[1] === String(v)))); this.#update(); }
    get(k) { const p = this.#list.find((p) => p[0] === String(k)); return p ? p[1] : null; }
    getAll(k) { return this.#list.filter((p) => p[0] === String(k)).map((p) => p[1]); }
    has(k, v) { return this.#list.some((p) => p[0] === String(k) && (v === undefined || p[1] === String(v))); }
    set(k, v) {
      k = String(k); v = String(v);
      const i = this.#list.findIndex((p) => p[0] === k);
      if (i < 0) this.#list.push([k, v]);
      else { this.#list[i][1] = v; this.#list = this.#list.filter((p, j) => j <= i || p[0] !== k); }
      this.#update();
    }
    sort() { this.#list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); this.#update(); }
    forEach(cb, thisArg) { for (const [k, v] of this.#list) cb.call(thisArg, v, k, this); }
    keys() { return this.#list.map((p) => p[0])[Symbol.iterator](); }
    values() { return this.#list.map((p) => p[1])[Symbol.iterator](); }
    entries() { return this.#list.map((p) => [p[0], p[1]])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    toString() { return this.#list.map((p) => enc(p[0]) + '=' + enc(p[1])).join('&'); }
  }
  expose('URLSearchParams', URLSearchParams);

  class URL {
    #p;
    #sp = null;
    constructor(url, base) {
      const p = __parseURL(String(url), base === undefined ? null : String(base instanceof URL ? base.href : base));
      if (!p) throw new TypeError("Failed to construct 'URL': Invalid URL");
      this.#p = p;
    }
    static canParse(url, base) { try { new URL(url, base); return true; } catch (e) { return false; } }
    static parse(url, base) { try { return new URL(url, base); } catch (e) { return null; } }
    static createObjectURL(blob) { const u = 'blob:null/' + Math.random().toString(16).slice(2) + Math.random().toString(16).slice(2); blobUrls.set(u, blob); return u; }
    static revokeObjectURL(u) { blobUrls.delete(u); }
    _setSearch(s) { this.#set('search', s ? '?' + s : ''); }
    #set(part, v) { const r = __setURLPart(this.#p.href, part, String(v)); if (r) this.#p = r; }
    get href() { return this.#p.href; }
    set href(v) { const p = __parseURL(String(v), null); if (!p) throw new TypeError('Invalid URL'); this.#p = p; if (this.#sp) this.#sp._reset(p.search); }
    get origin() { return this.#p.origin; }
    get protocol() { return this.#p.protocol; } set protocol(v) { this.#set('protocol', v); }
    get username() { return this.#p.username; } set username(v) { this.#set('username', v); }
    get password() { return this.#p.password; } set password(v) { this.#set('password', v); }
    get host() { return this.#p.host; } set host(v) { this.#set('host', v); }
    get hostname() { return this.#p.hostname; } set hostname(v) { this.#set('hostname', v); }
    get port() { return this.#p.port; } set port(v) { this.#set('port', v); }
    get pathname() { return this.#p.pathname; } set pathname(v) { this.#set('pathname', v); }
    get search() { return this.#p.search; } set search(v) { this.#set('search', v); if (this.#sp) this.#sp._reset(this.#p.search); }
    get hash() { return this.#p.hash; } set hash(v) { this.#set('hash', v); }
    get searchParams() {
      if (!this.#sp) { this.#sp = new URLSearchParams(this.#p.search); this.#sp._attach(this); }
      return this.#sp;
    }
    toString() { return this.#p.href; }
    toJSON() { return this.#p.href; }
  }
  expose('URL', URL);
  expose('webkitURL', URL);

  // ---- Blob / File / FileReader / FormData ------------------------------
  const utf8 = (s) => unescape(encodeURIComponent(s));
  class Blob {
    #text; #type;
    constructor(parts = [], opts = {}) {
      let t = '';
      for (const p of parts) t += (p instanceof Blob) ? p._text() : (p && p.length !== undefined && typeof p !== 'string' ? String.fromCharCode.apply(null, Array.from(p)) : String(p));
      this.#text = t; this.#type = opts.type ? String(opts.type).toLowerCase() : '';
    }
    _text() { return this.#text; }
    get size() { return utf8(this.#text).length; }
    get type() { return this.#type; }
    slice(s, e, t) { return new Blob([this.#text.slice(s, e)], { type: t }); }
    text() { return Promise.resolve(this.#text); }
    arrayBuffer() { return Promise.resolve(new TextEncoder().encode(this.#text)); }
    stream() { throw new DOMException('Not supported', 'NotSupportedError'); }
  }
  expose('Blob', Blob);
  class File extends Blob {
    constructor(parts, name, opts = {}) { super(parts, opts); hide(this, 'name', String(name)); hide(this, 'lastModified', opts.lastModified || Date.now()); }
  }
  expose('File', File);
  class FileReader extends EventTarget {
    constructor() { super(); this.readyState = 0; this.result = null; this.error = null; this.onload = null; this.onloadend = null; this.onerror = null; this.onprogress = null; }
    #done(result) {
      setTimeout(() => {
        this.result = result; this.readyState = 2;
        for (const t of ['load', 'loadend']) { const ev = new ProgressEvent(t); if (typeof this['on' + t] === 'function') this['on' + t](ev); this.dispatchEvent(ev); }
      }, 0);
    }
    readAsText(b) { this.#done(b._text()); }
    readAsDataURL(b) { this.#done('data:' + (b.type || 'application/octet-stream') + ';base64,' + btoa(utf8(b._text()))); }
    readAsArrayBuffer(b) { this.#done(new TextEncoder().encode(b._text())); }
    abort() {}
  }
  expose('FileReader', FileReader);
  class FormData {
    #l = [];
    constructor(form) {
      if (form) for (const p of __formData(form)) this.#l.push([p[0], p[1]]);
    }
    append(k, v) { this.#l.push([String(k), v instanceof Blob ? v : String(v)]); }
    delete(k) { this.#l = this.#l.filter((p) => p[0] !== String(k)); }
    get(k) { const p = this.#l.find((p) => p[0] === String(k)); return p ? p[1] : null; }
    getAll(k) { return this.#l.filter((p) => p[0] === String(k)).map((p) => p[1]); }
    has(k) { return this.#l.some((p) => p[0] === String(k)); }
    set(k, v) { k = String(k); const i = this.#l.findIndex((p) => p[0] === k); const nv = v instanceof Blob ? v : String(v); if (i < 0) this.#l.push([k, nv]); else { this.#l[i][1] = nv; this.#l = this.#l.filter((p, j) => j <= i || p[0] !== k); } }
    forEach(cb, t) { for (const [k, v] of this.#l) cb.call(t, v, k, this); }
    keys() { return this.#l.map((p) => p[0])[Symbol.iterator](); }
    values() { return this.#l.map((p) => p[1])[Symbol.iterator](); }
    entries() { return this.#l.map((p) => [p[0], p[1]])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
  }
  expose('FormData', FormData);

  // ---- Encoding -----------------------------------------------------------
  class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(s = '') { const b = utf8(String(s)); const out = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) out[i] = b.charCodeAt(i); return out; }
    encodeInto(s, dest) { const e = this.encode(s); const n = Math.min(e.length, dest.length); for (let i = 0; i < n; i++) dest[i] = e[i]; return { read: s.length, written: n }; }
  }
  class TextDecoder {
    constructor(label = 'utf-8') { this.encoding = String(label).toLowerCase(); this.fatal = false; this.ignoreBOM = false; }
    decode(buf) {
      if (buf === undefined) return '';
      const a = Array.from(buf instanceof ArrayBuffer ? [] : buf);
      let s = '';
      for (let i = 0; i < a.length; i += 8192) s += String.fromCharCode.apply(null, a.slice(i, i + 8192));
      if (this.encoding === 'utf-8' || this.encoding === 'utf8') { try { return decodeURIComponent(escape(s)); } catch (e) { return s; } }
      return s;
    }
  }
  expose('TextEncoder', TextEncoder);
  expose('TextDecoder', TextDecoder);

  // ---- Fetch API ----------------------------------------------------------
  class Headers {
    #m = new Map();
    constructor(init) {
      if (!init) return;
      if (init instanceof Headers) init.forEach((v, k) => this.append(k, v));
      else if (Array.isArray(init) || typeof init[Symbol.iterator] === 'function') { for (const p of init) this.append(p[0], p[1]); }
      else for (const k of Object.keys(init)) this.append(k, init[k]);
    }
    append(k, v) { k = String(k).toLowerCase(); v = String(v); this.#m.set(k, this.#m.has(k) ? this.#m.get(k) + ', ' + v : v); }
    set(k, v) { this.#m.set(String(k).toLowerCase(), String(v)); }
    get(k) { const v = this.#m.get(String(k).toLowerCase()); return v === undefined ? null : v; }
    has(k) { return this.#m.has(String(k).toLowerCase()); }
    delete(k) { this.#m.delete(String(k).toLowerCase()); }
    forEach(cb, t) { for (const [k, v] of [...this.#m].sort()) cb.call(t, v, k, this); }
    keys() { return [...this.#m.keys()].sort()[Symbol.iterator](); }
    values() { return [...this.#m].sort().map((p) => p[1])[Symbol.iterator](); }
    entries() { return [...this.#m].sort()[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
  }
  expose('Headers', Headers);

  function bodyToText(body) {
    if (body === undefined || body === null) return null;
    if (typeof body === 'string') return body;
    if (body instanceof URLSearchParams) return body.toString();
    if (body instanceof Blob) return body._text();
    if (body instanceof FormData) { const sp = new URLSearchParams(); body.forEach((v, k) => sp.append(k, typeof v === 'string' ? v : v.name || '')); return sp.toString(); }
    if (body.length !== undefined) return new TextDecoder().decode(body);
    return String(body);
  }

  class Body {
    #text; #used = false;
    _init(text) { this.#text = text; }
    get bodyUsed() { return this.#used; }
    _bodyText() { return this.#text === undefined ? null : this.#text; }
    #take() { if (this.#used) return Promise.reject(new TypeError('Body has already been consumed.')); this.#used = true; return Promise.resolve(this.#text === undefined || this.#text === null ? '' : this.#text); }
    text() { return this.#take(); }
    json() { return this.#take().then((t) => JSON.parse(t)); }
    blob() { return this.#take().then((t) => new Blob([t], { type: this.headers.get('content-type') || '' })); }
    arrayBuffer() { return this.#take().then((t) => new TextEncoder().encode(t)); }
    formData() { return this.#take().then((t) => { const fd = new FormData(); new URLSearchParams(t).forEach((v, k) => fd.append(k, v)); return fd; }); }
  }

  class Request extends Body {
    constructor(input, init = {}) {
      super();
      const r = input instanceof Request ? input : null;
      this.url = r ? r.url : (G.location && G.location.href !== 'about:blank' ? new URL(String(input), G.location.href).href : String(input));
      this.method = String(init.method || (r && r.method) || 'GET').toUpperCase();
      this.headers = new Headers(init.headers || (r && r.headers));
      this.signal = init.signal || (r && r.signal) || null;
      this.credentials = init.credentials || 'same-origin';
      this.mode = init.mode || 'cors';
      this.cache = init.cache || 'default';
      this.redirect = init.redirect || 'follow';
      this.referrer = init.referrer || 'about:client';
      this._init(init.body !== undefined ? bodyToText(init.body) : r ? r._bodyText() : null);
    }
    clone() { return new Request(this); }
  }
  expose('Request', Request);

  class Response extends Body {
    constructor(body = null, init = {}) {
      super();
      this.status = init.status === undefined ? 200 : init.status;
      this.statusText = init.statusText === undefined ? '' : String(init.statusText);
      this.headers = new Headers(init.headers);
      this.url = init.url || '';
      this.redirected = false;
      this.type = 'basic';
      this._init(bodyToText(body));
    }
    get ok() { return this.status >= 200 && this.status < 300; }
    clone() { const r = new Response(this._bodyText(), { status: this.status, statusText: this.statusText, headers: this.headers, url: this.url }); return r; }
    static error() { const r = new Response(null, { status: 0 }); r.type = 'error'; return r; }
    static json(data, init = {}) { const r = new Response(JSON.stringify(data), init); r.headers.set('content-type', 'application/json'); return r; }
    static redirect(url, status = 302) { return new Response(null, { status, headers: { location: url } }); }
  }
  expose('Response', Response);

  expose('fetch', function fetch(input, init) {
    return new Promise((resolve, reject) => {
      let req;
      try { req = new Request(input, init); } catch (e) { reject(e); return; }
      if (req.signal && req.signal.aborted) { reject(req.signal.reason || new DOMException('The operation was aborted.', 'AbortError')); return; }
      const hdrs = [];
      req.headers.forEach((v, k) => hdrs.push([k, v]));
      const r = __hostFetch(req.method, req.url, hdrs, req._bodyText());
      if (!r) { reject(new TypeError('Failed to fetch')); return; }
      const res = new Response(r.body, { status: r.status, statusText: r.statusText, headers: r.headers, url: r.url });
      resolve(res);
    });
  });

  class XMLHttpRequest extends EventTarget {
    constructor() {
      super();
      this.readyState = 0; this.status = 0; this.statusText = ''; this.responseText = ''; this.response = ''; this.responseType = ''; this.responseURL = ''; this.timeout = 0; this.withCredentials = false;
      this.onreadystatechange = null; this.onload = null; this.onerror = null; this.onloadend = null; this.onabort = null; this.ontimeout = null; this.onprogress = null; this.onloadstart = null;
      this._h = []; this._rh = []; this._async = true; this.upload = new EventTarget();
    }
    open(method, url, async = true) { this._method = String(method).toUpperCase(); this._url = String(url); this._async = async !== false; this.readyState = 1; this._fire('readystatechange'); }
    setRequestHeader(k, v) { this._h.push([String(k), String(v)]); }
    overrideMimeType() {}
    getResponseHeader(k) { const h = this._rh.find((p) => p[0].toLowerCase() === String(k).toLowerCase()); return h ? h[1] : null; }
    getAllResponseHeaders() { return this._rh.map((p) => p[0] + ': ' + p[1]).join('\r\n') + (this._rh.length ? '\r\n' : ''); }
    abort() { this._aborted = true; this.readyState = 0; this._fire('abort'); }
    _fire(type) {
      const ev = new ProgressEvent(type);
      if (typeof this['on' + type] === 'function') { try { this['on' + type](ev); } catch (e) { reportError(e); } }
      this.dispatchEvent(ev);
    }
    send(body) {
      const run = () => {
        if (this._aborted) return;
        const r = __hostFetch(this._method, this._url, this._h, body === undefined || body === null ? null : bodyToText(body));
        if (!r) { this.readyState = 4; this._fire('readystatechange'); this._fire('error'); this._fire('loadend'); return; }
        this.status = r.status; this.statusText = r.statusText; this.responseURL = r.url; this._rh = r.headers; this.responseText = r.body;
        if (this.responseType === 'json') { try { this.response = JSON.parse(r.body); } catch (e) { this.response = null; } }
        else if (this.responseType === 'arraybuffer') this.response = new TextEncoder().encode(r.body);
        else if (this.responseType === 'blob') this.response = new Blob([r.body]);
        else this.response = r.body;
        this.readyState = 2; this._fire('readystatechange');
        this.readyState = 3; this._fire('readystatechange');
        this.readyState = 4; this._fire('readystatechange');
        this._fire('load'); this._fire('loadend');
      };
      this._fire('loadstart');
      if (this._async) setTimeout(run, 0); else run();
    }
  }
  XMLHttpRequest.UNSENT = 0; XMLHttpRequest.OPENED = 1; XMLHttpRequest.HEADERS_RECEIVED = 2; XMLHttpRequest.LOADING = 3; XMLHttpRequest.DONE = 4;
  expose('XMLHttpRequest', XMLHttpRequest);

  // ---- Abort ---------------------------------------------------------------
  class AbortSignal extends EventTarget {
    constructor() { super(); this.aborted = false; this.reason = undefined; this.onabort = null; }
    throwIfAborted() { if (this.aborted) throw this.reason; }
    static abort(reason) { const s = new AbortSignal(); s.aborted = true; s.reason = reason === undefined ? new DOMException('This operation was aborted', 'AbortError') : reason; return s; }
    static timeout(ms) { const s = new AbortSignal(); setTimeout(() => s._abort(new DOMException('The operation timed out.', 'TimeoutError')), ms); return s; }
    static any(signals) { const s = new AbortSignal(); for (const x of signals) { if (x.aborted) { s._abort(x.reason); break; } x.addEventListener('abort', () => s._abort(x.reason)); } return s; }
    _abort(reason) {
      if (this.aborted) return;
      this.aborted = true; this.reason = reason === undefined ? new DOMException('This operation was aborted', 'AbortError') : reason;
      const ev = new Event('abort');
      if (typeof this.onabort === 'function') this.onabort(ev);
      this.dispatchEvent(ev);
    }
  }
  class AbortController {
    constructor() { this.signal = new AbortSignal(); }
    abort(reason) { this.signal._abort(reason); }
  }
  expose('AbortSignal', AbortSignal);
  expose('AbortController', AbortController);

  // ---- Intl ----------------------------------------------------------------
  const Intl = {};
  Intl.NumberFormat = class NumberFormat {
    constructor(locale, opts) { this._o = opts || {}; this._l = locale; }
    format(n) { return Number(n).toLocaleString(this._l, this._o); }
    formatToParts(n) { return [{ type: 'literal', value: this.format(n) }]; }
    resolvedOptions() { return Object.assign({ locale: 'en-US', numberingSystem: 'latn', style: 'decimal', minimumFractionDigits: 0, maximumFractionDigits: 3 }, this._o); }
    static supportedLocalesOf(l) { return [].concat(l || []); }
  };
  Intl.DateTimeFormat = class DateTimeFormat {
    constructor(locale, opts) { this._o = opts; this._l = locale; }
    format(d) { d = d === undefined ? new Date() : new Date(d); const o = this._o; return (o && (o.hour || o.minute) && !(o.year || o.month || o.day || o.weekday)) ? d.toLocaleTimeString(this._l, o) : (o && (o.hour || o.minute)) ? d.toLocaleString(this._l, o) : d.toLocaleDateString(this._l, o); }
    formatToParts(d) { return [{ type: 'literal', value: this.format(d) }]; }
    resolvedOptions() { return Object.assign({ locale: 'en-US', calendar: 'gregory', numberingSystem: 'latn', timeZone: 'UTC' }, this._o); }
    static supportedLocalesOf(l) { return [].concat(l || []); }
  };
  Intl.Collator = class Collator {
    constructor(locale, opts) { this._o = opts || {}; const self = this; this.compare = (a, b) => String(a).localeCompare(String(b), locale, self._o); }
    resolvedOptions() { return { locale: 'en-US', usage: 'sort', sensitivity: 'variant' }; }
  };
  Intl.PluralRules = class PluralRules {
    constructor(l, o) { this._ord = o && o.type === 'ordinal'; }
    select(n) {
      if (this._ord) { const a = n % 10, b = n % 100; return a === 1 && b !== 11 ? 'one' : a === 2 && b !== 12 ? 'two' : a === 3 && b !== 13 ? 'few' : 'other'; }
      return n === 1 ? 'one' : 'other';
    }
    resolvedOptions() { return { locale: 'en-US' }; }
  };
  Intl.RelativeTimeFormat = class RelativeTimeFormat {
    constructor(l, o) { this._o = o || {}; }
    format(v, unit) {
      unit = String(unit).replace(/s$/, '');
      if (this._o.numeric === 'auto') { if (v === 0) return unit === 'day' ? 'today' : 'this ' + unit; if (v === 1 && unit === 'day') return 'tomorrow'; if (v === -1 && unit === 'day') return 'yesterday'; }
      const a = Math.abs(v); const u = a === 1 ? unit : unit + 's';
      return v < 0 || Object.is(v, -0) ? a + ' ' + u + ' ago' : 'in ' + a + ' ' + u;
    }
  };
  Intl.ListFormat = class ListFormat {
    constructor(l, o) { this._o = o || {}; }
    format(list) {
      const a = Array.from(list); const w = this._o.type === 'disjunction' ? 'or' : 'and';
      if (a.length <= 1) return a.join('');
      if (a.length === 2) return a[0] + ' ' + w + ' ' + a[1];
      return a.slice(0, -1).join(', ') + ', ' + w + ' ' + a[a.length - 1];
    }
  };
  Intl.Segmenter = class Segmenter {
    constructor(l, o) { this._g = (o && o.granularity) || 'grapheme'; }
    segment(s) {
      const g = this._g; const out = [];
      if (g === 'word') { let m; const re = /\w+|\W/g; while ((m = re.exec(s))) out.push({ segment: m[0], index: m.index, input: s, isWordLike: /\w/.test(m[0]) }); }
      else { let i = 0; for (const ch of s) { out.push({ segment: ch, index: i, input: s }); i += ch.length; } }
      return out;
    }
  };
  Intl.DisplayNames = class DisplayNames { constructor() {} of(c) { return c; } };
  Intl.getCanonicalLocales = (l) => [].concat(l || []);
  Intl.supportedValuesOf = () => [];
  expose('Intl', Intl);

  // ---- DOM helpers -----------------------------------------------------------
  class DOMParser {
    parseFromString(str, type) {
      const doc = document.implementation.createHTMLDocument('');
      doc.body.innerHTML = String(str);
      doc.documentElement = doc.body;
      return doc;
    }
  }
  expose('DOMParser', DOMParser);
  class XMLSerializer { serializeToString(n) { return n.outerHTML !== undefined ? n.outerHTML : n.textContent; } }
  expose('XMLSerializer', XMLSerializer);
  expose('Image', function Image(w, h) { const i = document.createElement('img'); if (w !== undefined) i.setAttribute('width', w); if (h !== undefined) i.setAttribute('height', h); return i; });
  expose('Option', function Option(text, value, defaultSelected, selected) { const o = document.createElement('option'); if (text !== undefined) o.textContent = text; if (value !== undefined) o.setAttribute('value', value); if (defaultSelected || selected) o.setAttribute('selected', ''); return o; });
  expose('Audio', function Audio(src) { const a = document.createElement('audio'); if (src) a.setAttribute('src', src); return a; });

  const NodeFilter = { FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3, SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 1, SHOW_ATTRIBUTE: 2, SHOW_TEXT: 4, SHOW_CDATA_SECTION: 8, SHOW_COMMENT: 128, SHOW_DOCUMENT: 256, SHOW_DOCUMENT_TYPE: 512, SHOW_DOCUMENT_FRAGMENT: 1024 };
  expose('NodeFilter', NodeFilter);
  function* walk(root) { for (const c of Array.from(root.childNodes)) { yield c; yield* walk(c); } }
  function makeWalker(root, what, filter) {
    what = what === undefined ? 0xFFFFFFFF : what >>> 0;
    const all = [];
    for (const n of walk(root)) {
      if (!(what & (1 << (n.nodeType - 1)))) continue;
      let r = 1;
      if (filter) r = typeof filter === 'function' ? filter(n) : filter.acceptNode(n);
      if (r === 1) all.push(n);
    }
    let i = -1;
    return {
      root, whatToShow: what, filter, currentNode: root,
      nextNode() { i++; if (i < all.length) { this.currentNode = all[i]; return all[i]; } return null; },
      previousNode() { if (i > 0) { i--; this.currentNode = all[i]; return all[i]; } return null; },
      firstChild() { return root.firstChild; },
      parentNode() { return this.currentNode.parentNode; },
      nextSibling() { return this.currentNode.nextSibling; },
      previousSibling() { return this.currentNode.previousSibling; },
    };
  }
  hide(Document.prototype, 'createTreeWalker', function (root, what, filter) { return makeWalker(root, what, filter); });
  hide(Document.prototype, 'createNodeIterator', function (root, what, filter) { const w = makeWalker(root, what, filter); w.nextNode = w.nextNode; return w; });
  class CSSStyleSheetImpl { constructor() { this.cssRules = []; this.rules = this.cssRules; this.disabled = false; } replaceSync(t) { this._t = t; } replace(t) { this._t = t; return Promise.resolve(this); } insertRule(r, i = 0) { this.cssRules.splice(i, 0, { cssText: r }); return i; } deleteRule(i) { this.cssRules.splice(i, 1); } }
  expose('CSSStyleSheet', CSSStyleSheetImpl);
  hide(Document.prototype, 'adoptedStyleSheets', []);

  // ---- XPath (minimal; evaluates nothing) ----------------------------------------------
  const XPathResult = { ANY_TYPE: 0, NUMBER_TYPE: 1, STRING_TYPE: 2, BOOLEAN_TYPE: 3, UNORDERED_NODE_ITERATOR_TYPE: 4, ORDERED_NODE_ITERATOR_TYPE: 5, UNORDERED_NODE_SNAPSHOT_TYPE: 6, ORDERED_NODE_SNAPSHOT_TYPE: 7, ANY_UNORDERED_NODE_TYPE: 8, FIRST_ORDERED_NODE_TYPE: 9 };
  expose('XPathResult', XPathResult);
  const emptyXPath = () => ({ snapshotLength: 0, singleNodeValue: null, booleanValue: false, numberValue: NaN, stringValue: '', iterateNext() { return null; }, snapshotItem() { return null; } });
  expose('XPathEvaluator', class XPathEvaluator { evaluate() { return emptyXPath(); } createExpression() { return { evaluate: emptyXPath }; } });
  hide(Document.prototype, 'evaluate', function () { return emptyXPath(); });

  // ---- Observers ----------------------------------------------------------------
  class IntersectionObserver {
    constructor(cb, opts) { this._cb = cb; this.root = (opts && opts.root) || null; this.rootMargin = (opts && opts.rootMargin) || '0px'; this.thresholds = [0]; this._t = new Set(); }
    observe(el) {
      if (this._t.has(el)) return;
      this._t.add(el);
      setTimeout(() => {
        if (!this._t.has(el)) return;
        const r = el.getBoundingClientRect();
        this._cb([{ target: el, isIntersecting: true, intersectionRatio: 1, boundingClientRect: r, intersectionRect: r, rootBounds: null, time: performance.now() }], this);
      }, 0);
    }
    unobserve(el) { this._t.delete(el); }
    disconnect() { this._t.clear(); }
    takeRecords() { return []; }
  }
  expose('IntersectionObserver', IntersectionObserver);
  class ResizeObserver {
    constructor(cb) { this._cb = cb; this._t = new Set(); }
    observe(el) {
      this._t.add(el);
      setTimeout(() => {
        if (!this._t.has(el)) return;
        const r = el.getBoundingClientRect();
        const size = [{ inlineSize: r.width, blockSize: r.height }];
        this._cb([{ target: el, contentRect: r, borderBoxSize: size, contentBoxSize: size, devicePixelContentBoxSize: size }], this);
      }, 0);
    }
    unobserve(el) { this._t.delete(el); }
    disconnect() { this._t.clear(); }
  }
  expose('ResizeObserver', ResizeObserver);
  expose('PerformanceObserver', class PerformanceObserver { constructor(cb) {} observe() {} disconnect() {} takeRecords() { return []; } static get supportedEntryTypes() { return []; } });

  // ---- Misc platform objects -------------------------------------------------------
  const crypto = {
    getRandomValues(a) { const b = __randomBytes(a.length); for (let i = 0; i < a.length; i++) a[i] = b[i]; return a; },
    randomUUID() { const b = __randomBytes(16); b[6] = (b[6] & 15) | 64; b[8] = (b[8] & 63) | 128; const h = Array.from(b, (x) => x.toString(16).padStart(2, '0')).join(''); return h.slice(0, 8) + '-' + h.slice(8, 12) + '-' + h.slice(12, 16) + '-' + h.slice(16, 20) + '-' + h.slice(20); },
    subtle: {},
  };
  expose('crypto', crypto);
  expose('CSS', {
    supports(a, b) { return true; },
    escape(s) { return String(s).replace(/[^a-zA-Z0-9_ -￿-]/g, (c) => '\\' + c).replace(/^(\d)/, '\\3$1 '); },
  });
  expose('Notification', class Notification { constructor() {} static get permission() { return 'denied'; } static requestPermission() { return Promise.resolve('denied'); } close() {} });
  expose('WebSocket', class WebSocket extends EventTarget {
    constructor(url) { super(); this.url = url; this.readyState = 3; this.onerror = null; this.onclose = null; this.onopen = null; this.onmessage = null; setTimeout(() => { const e = new Event('error'); if (typeof this.onerror === 'function') this.onerror(e); this.dispatchEvent(e); const c = new Event('close'); if (typeof this.onclose === 'function') this.onclose(c); this.dispatchEvent(c); }, 0); }
    send() { throw new DOMException('WebSocket is not open', 'InvalidStateError'); }
    close() {}
  });
  expose('Worker', class Worker extends EventTarget { constructor() { super(); throw new DOMException('Workers are not supported', 'NotSupportedError'); } });
  class MessagePort extends EventTarget {
    constructor() { super(); this.onmessage = null; this._o = null; }
    postMessage(data) { const o = this._o; setTimeout(() => { const ev = new MessageEvent('message', { data }); if (typeof o.onmessage === 'function') o.onmessage(ev); o.dispatchEvent(ev); }, 0); }
    start() {} close() {}
  }
  expose('MessageChannel', class MessageChannel { constructor() { this.port1 = new MessagePort(); this.port2 = new MessagePort(); this.port1._o = this.port2; this.port2._o = this.port1; } });
  expose('BroadcastChannel', class BroadcastChannel extends EventTarget { constructor(n) { super(); this.name = n; this.onmessage = null; } postMessage() {} close() {} });
  expose('visualViewport', { width: 1024, height: 768, scale: 1, offsetLeft: 0, offsetTop: 0, pageLeft: 0, pageTop: 0, addEventListener() {}, removeEventListener() {} });
  expose('webkitRequestAnimationFrame', G.requestAnimationFrame);
  expose('webkitCancelAnimationFrame', G.cancelAnimationFrame);
  expose('MediaQueryListEvent', class MediaQueryListEvent extends Event {});

  // ---- Typed array algorithms that build new arrays ----------------------------
  const TAP = Object.getPrototypeOf(Uint8Array.prototype);
  const taNew = (ta, n) => new ta.constructor(n);
  const def = (o, k, v) => Object.defineProperty(o, k, { value: v, writable: true, configurable: true, enumerable: false });
  def(TAP, 'map', function (f, t) { const r = taNew(this, this.length); for (let i = 0; i < this.length; i++) r[i] = f.call(t, this[i], i, this); return r; });
  def(TAP, 'filter', function (f, t) { const out = []; for (let i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) out.push(this[i]); return new this.constructor(out); });
  def(TAP, 'reverse', function () { for (let i = 0, j = this.length - 1; i < j; i++, j--) { const x = this[i]; this[i] = this[j]; this[j] = x; } return this; });
  def(TAP, 'sort', function (cmp) { const a = Array.from(this).sort(cmp || ((x, y) => (x < y ? -1 : x > y ? 1 : 0))); for (let i = 0; i < a.length; i++) this[i] = a[i]; return this; });
  def(TAP, 'copyWithin', function (t, s, e) { const a = Array.prototype.copyWithin.call(Array.from(this), t, s, e); for (let i = 0; i < a.length; i++) this[i] = a[i]; return this; });
  def(TAP, 'toReversed', function () { return this.slice().reverse(); });
  def(TAP, 'toSorted', function (c) { return this.slice().sort(c); });
  def(TAP, 'with', function (i, v) { const r = this.slice(); r[i < 0 ? this.length + i : i] = v; return r; });

  // ---- Encoding (bytes) ----------------------------------------------------------
  const bytesOf = (buf) => {
    if (buf === undefined || buf === null) return new Uint8Array(0);
    if (buf instanceof ArrayBuffer) return new Uint8Array(buf);
    if (ArrayBuffer.isView(buf)) return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    return new Uint8Array(buf);
  };
  const utf8Decode = (b, fatal) => {
    let i = 0;
    if (b.length >= 3 && b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf) i = 3;
    const parts = [];
    let codes = [];
    for (; i < b.length;) {
      const c = b[i];
      let cp = -1, n = 0;
      if (c < 0x80) { cp = c; n = 1; }
      else if (c >= 0xc2 && c < 0xe0 && i + 1 < b.length && (b[i + 1] & 0xc0) === 0x80) { cp = ((c & 31) << 6) | (b[i + 1] & 63); n = 2; }
      else if (c >= 0xe0 && c < 0xf0 && i + 2 < b.length && (b[i + 1] & 0xc0) === 0x80 && (b[i + 2] & 0xc0) === 0x80) { cp = ((c & 15) << 12) | ((b[i + 1] & 63) << 6) | (b[i + 2] & 63); n = 3; if (cp < 0x800 || (cp >= 0xd800 && cp < 0xe000)) cp = -1; }
      else if (c >= 0xf0 && c < 0xf5 && i + 3 < b.length && (b[i + 1] & 0xc0) === 0x80 && (b[i + 2] & 0xc0) === 0x80 && (b[i + 3] & 0xc0) === 0x80) { cp = ((c & 7) << 18) | ((b[i + 1] & 63) << 12) | ((b[i + 2] & 63) << 6) | (b[i + 3] & 63); n = 4; if (cp < 0x10000 || cp > 0x10ffff) cp = -1; }
      if (cp < 0) { if (fatal) throw new TypeError('The encoded data was not valid for encoding utf-8'); cp = 0xfffd; n = 1; }
      if (cp > 0xffff) { cp -= 0x10000; codes.push(0xd800 + (cp >> 10), 0xdc00 + (cp & 1023)); } else codes.push(cp);
      i += n;
      if (codes.length > 8000) { parts.push(String.fromCharCode.apply(null, codes)); codes = []; }
    }
    parts.push(String.fromCharCode.apply(null, codes));
    return parts.join('');
  };
  def(TextDecoder.prototype, 'decode', function (buf) {
    const b = bytesOf(buf);
    const enc = this.encoding;
    if (enc === 'utf-8' || enc === 'utf8' || enc === 'unicode-1-1-utf-8') return utf8Decode(b, this.fatal);
    if (enc === 'utf-16le' || enc === 'utf-16') { let s = ''; for (let i = 0; i + 1 < b.length; i += 2) s += String.fromCharCode(b[i] | (b[i + 1] << 8)); return s; }
    let s = ''; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]); return s;
  });
  def(TextEncoder.prototype, 'encode', function (str = '') {
    str = String(str);
    const out = new Uint8Array(str.length * 3);
    let n = 0;
    for (let i = 0; i < str.length; i++) {
      let c = str.charCodeAt(i);
      if (c >= 0xd800 && c < 0xdc00 && i + 1 < str.length) { const d = str.charCodeAt(i + 1); if (d >= 0xdc00 && d < 0xe000) { c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00); i++; } else c = 0xfffd; }
      else if (c >= 0xd800 && c < 0xe000) c = 0xfffd;
      if (c < 0x80) out[n++] = c;
      else if (c < 0x800) { out[n++] = 0xc0 | (c >> 6); out[n++] = 0x80 | (c & 63); }
      else if (c < 0x10000) { out[n++] = 0xe0 | (c >> 12); out[n++] = 0x80 | ((c >> 6) & 63); out[n++] = 0x80 | (c & 63); }
      else { out[n++] = 0xf0 | (c >> 18); out[n++] = 0x80 | ((c >> 12) & 63); out[n++] = 0x80 | ((c >> 6) & 63); out[n++] = 0x80 | (c & 63); }
    }
    return out.slice(0, n);
  });
  def(TextEncoder.prototype, 'encodeInto', function (s, dest) {
    const e = this.encode(s); const n = Math.min(e.length, dest.length); dest.set(e.subarray(0, n)); return { read: s.length, written: n };
  });
  def(Blob.prototype, 'arrayBuffer', function () { return Promise.resolve(new TextEncoder().encode(this._text()).buffer); });
  def(Blob.prototype, 'bytes', function () { return Promise.resolve(new TextEncoder().encode(this._text())); });
  def(Blob.prototype, 'stream', function () { const t = this._text(); return new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode(t)); c.close(); } }); });

  // ---- Streams ---------------------------------------------------------------------
  class ReadableStreamDefaultController {
    constructor(s) { this._s = s; }
    get desiredSize() { return this._s._hwm - this._s._queue.length; }
    enqueue(chunk) { const s = this._s; if (s._state !== 'readable' || s._closeRequested) throw new TypeError('Cannot enqueue into a closed stream'); if (s._readers.length) s._readers.shift().resolve({ value: chunk, done: false }); else s._queue.push(chunk); }
    close() { const s = this._s; if (s._closeRequested) return; s._closeRequested = true; if (!s._queue.length) s._finish(); }
    error(e) { this._s._fail(e); }
  }
  class ReadableStreamDefaultReader {
    constructor(stream) { if (stream._reader) throw new TypeError('ReadableStream is locked'); this._s = stream; stream._reader = this; this.closed = new Promise((r, j) => { this._close = r; this._err = j; }); this.closed.catch(() => {}); if (stream._state === 'closed') this._close(); }
    read() {
      const s = this._s;
      if (!s) return Promise.reject(new TypeError('Reader released'));
      if (s._queue.length) { const v = s._queue.shift(); if (!s._queue.length && s._closeRequested) s._finish(); else s._pull(); return Promise.resolve({ value: v, done: false }); }
      if (s._state === 'closed') return Promise.resolve({ value: undefined, done: true });
      if (s._state === 'errored') return Promise.reject(s._error);
      const p = new Promise((resolve, reject) => s._readers.push({ resolve, reject }));
      s._pull();
      return p;
    }
    releaseLock() { if (this._s) { this._s._reader = null; this._s = null; } }
    cancel(r) { return this._s ? this._s.cancel(r) : Promise.resolve(); }
  }
  class ReadableStream {
    constructor(src = {}, strategy = {}) {
      this._src = src; this._queue = []; this._readers = []; this._state = 'readable'; this._reader = null; this._closeRequested = false; this._pulling = false;
      this._hwm = strategy.highWaterMark === undefined ? 1 : strategy.highWaterMark;
      this._ctrl = new ReadableStreamDefaultController(this);
      let started;
      try { started = src.start ? src.start(this._ctrl) : undefined; } catch (e) { this._fail(e); return; }
      Promise.resolve(started).then(() => { this._started = true; this._pull(); }, (e) => this._fail(e));
    }
    _pull() {
      if (!this._started || this._pulling || this._state !== 'readable' || this._closeRequested || !this._src.pull) return;
      if (this._queue.length >= this._hwm && !this._readers.length) return;
      this._pulling = true;
      let r; try { r = this._src.pull(this._ctrl); } catch (e) { this._fail(e); return; }
      Promise.resolve(r).then(() => { this._pulling = false; if (this._readers.length) this._pull(); }, (e) => this._fail(e));
    }
    _finish() { this._state = 'closed'; for (const r of this._readers.splice(0)) r.resolve({ value: undefined, done: true }); if (this._reader) this._reader._close(); }
    _fail(e) { if (this._state !== 'readable') return; this._state = 'errored'; this._error = e; this._queue = []; for (const r of this._readers.splice(0)) r.reject(e); if (this._reader) this._reader._err(e); }
    get locked() { return !!this._reader; }
    getReader() { return new ReadableStreamDefaultReader(this); }
    cancel(reason) { this._queue = []; this._state = 'closed'; for (const r of this._readers.splice(0)) r.resolve({ value: undefined, done: true }); return Promise.resolve(this._src.cancel ? this._src.cancel(reason) : undefined); }
    async pipeTo(dest, opts = {}) {
      const r = this.getReader(); const w = dest.getWriter();
      try { for (;;) { const { value, done } = await r.read(); if (done) break; await w.write(value); } if (!opts.preventClose) await w.close(); }
      catch (e) { if (!opts.preventAbort) await w.abort(e); throw e; }
      finally { r.releaseLock(); w.releaseLock(); }
    }
    pipeThrough(t, opts) { this.pipeTo(t.writable, opts).catch(() => {}); return t.readable; }
    tee() {
      const r = this.getReader(); const ctrls = [];
      const pull = () => r.read().then(({ value, done }) => { for (const c of ctrls) { if (done) c.close(); else c.enqueue(value); } });
      const a = new ReadableStream({ start(c) { ctrls.push(c); }, pull });
      const b = new ReadableStream({ start(c) { ctrls.push(c); }, pull });
      return [a, b];
    }
    values(opts = {}) {
      const r = this.getReader();
      return { next: () => r.read(), return: (v) => { if (!opts.preventCancel) r.cancel(v); r.releaseLock(); return Promise.resolve({ value: v, done: true }); }, [Symbol.asyncIterator]() { return this; } };
    }
    [Symbol.asyncIterator](o) { return this.values(o); }
    static from(it) {
      if (it instanceof ReadableStream) return it;
      const i = it[Symbol.asyncIterator] ? it[Symbol.asyncIterator]() : it[Symbol.iterator]();
      return new ReadableStream({ async pull(c) { const { value, done } = await i.next(); if (done) c.close(); else c.enqueue(value); } }, { highWaterMark: 0 });
    }
  }
  class WritableStreamDefaultWriter {
    constructor(s) { if (s._writer) throw new TypeError('WritableStream is locked'); this._s = s; s._writer = this; this.closed = s._closedP; this.ready = Promise.resolve(); }
    get desiredSize() { return 1; }
    write(chunk) { const s = this._s; return s._enqueue(() => (s._sink.write ? s._sink.write(chunk, s._ctrl) : undefined)); }
    close() { const s = this._s; return s._enqueue(() => (s._sink.close ? s._sink.close() : undefined)).then(() => { s._state = 'closed'; s._resolveClosed(); }); }
    abort(r) { const s = this._s; s._state = 'errored'; return Promise.resolve(s._sink.abort ? s._sink.abort(r) : undefined); }
    releaseLock() { if (this._s) { this._s._writer = null; this._s = null; } }
  }
  class WritableStream {
    constructor(sink = {}) {
      this._sink = sink; this._writer = null; this._state = 'writable';
      this._ctrl = { error: (e) => { this._state = 'errored'; this._error = e; }, signal: new AbortController().signal };
      this._closedP = new Promise((r) => { this._resolveClosed = r; });
      this._chain = Promise.resolve(sink.start ? sink.start(this._ctrl) : undefined);
    }
    _enqueue(fn) { const p = this._chain.then(() => { if (this._state === 'errored') throw this._error; return fn(); }); this._chain = p.catch(() => {}); return p; }
    get locked() { return !!this._writer; }
    getWriter() { return new WritableStreamDefaultWriter(this); }
    close() { return this.getWriter().close(); }
    abort(r) { return this.getWriter().abort(r); }
  }
  class TransformStream {
    constructor(t = {}) {
      let rc;
      this.readable = new ReadableStream({ start(c) { rc = c; } });
      const ctrl = { enqueue: (c) => rc.enqueue(c), error: (e) => rc.error(e), terminate: () => rc.close(), get desiredSize() { return 1; } };
      const started = t.start ? t.start(ctrl) : undefined;
      this.writable = new WritableStream({
        start: () => started,
        write: (chunk) => (t.transform ? t.transform(chunk, ctrl) : ctrl.enqueue(chunk)),
        close: () => Promise.resolve(t.flush ? t.flush(ctrl) : undefined).then(() => rc.close()),
        abort: (e) => rc.error(e),
      });
    }
  }
  class TextEncoderStream extends TransformStream { constructor() { const e = new TextEncoder(); super({ transform(c, ctl) { ctl.enqueue(e.encode(c)); } }); } }
  class TextDecoderStream extends TransformStream { constructor(l) { const d = new TextDecoder(l); super({ transform(c, ctl) { ctl.enqueue(d.decode(c)); } }); } }
  for (const [n, c] of Object.entries({ ReadableStream, ReadableStreamDefaultReader, ReadableStreamDefaultController, WritableStream, WritableStreamDefaultWriter, TransformStream, TextEncoderStream, TextDecoderStream })) expose(n, c);
  expose('ByteLengthQueuingStrategy', class ByteLengthQueuingStrategy { constructor(o) { this.highWaterMark = o.highWaterMark; } size(c) { return c.byteLength; } });
  expose('CountQueuingStrategy', class CountQueuingStrategy { constructor(o) { this.highWaterMark = o.highWaterMark; } size() { return 1; } });
  // Response/Request bodies as streams
  for (const C of [G.Response, G.Request]) {
    if (!C) continue;
    Object.defineProperty(C.prototype, 'body', { configurable: true, get() { const t = this._bodyText(); if (t === null) return null; const bytes = new TextEncoder().encode(t); return new ReadableStream({ start(c) { c.enqueue(bytes); c.close(); } }); } });
    def(C.prototype, 'bytes', function () { return this.text().then((t) => new TextEncoder().encode(t)); });
    def(C.prototype, 'arrayBuffer', function () { return this.text().then((t) => new TextEncoder().encode(t).buffer); });
  }

  // ---- Interfaces scripts feature-detect or extend --------------------------------
  const iface = (name, proto, base) => {
    const C = { [name]: function () { throw new TypeError('Illegal constructor'); } }[name];
    const p = proto || Object.create(base ? base.prototype : Object.prototype);
    Object.defineProperty(C, 'prototype', { value: p, writable: false }); def(p, 'constructor', C);
    Object.defineProperty(p, Symbol.toStringTag, { value: name, configurable: true });
    if (base) Object.setPrototypeOf(C, base);
    expose(name, C);
    return C;
  };
  const adopt = (name, obj, base) => {
    if (!obj || G[name]) return;
    const p = Object.create(base ? base.prototype : Object.getPrototypeOf(obj) || Object.prototype);
    iface(name, p);
    try { Object.setPrototypeOf(obj, p); } catch (e) {}
  };
  adopt('Location', G.location);
  adopt('History', G.history);
  adopt('Navigator', G.navigator);
  adopt('Screen', G.screen);
  adopt('Performance', G.performance);
  adopt('Storage', G.localStorage);
  if (G.sessionStorage && G.Storage) try { Object.setPrototypeOf(G.sessionStorage, G.Storage.prototype); } catch (e) {}
  if (!G.Window) iface('Window', Object.create(G.EventTarget ? G.EventTarget.prototype : Object.prototype));
  // window → Window.prototype → EventTarget.prototype (polyfills capture natives from the prototypes)
  try { if (Object.getPrototypeOf(G) !== G.Window.prototype) Object.setPrototypeOf(G, G.Window.prototype); } catch (e) {}
  for (const n of ['CDATASection', 'ProcessingInstruction']) if (!G[n]) iface(n, null, G.Text || G.Node);
  if (!G.StaticRange) expose('StaticRange', class StaticRange { constructor(i) { Object.assign(this, { startContainer: i.startContainer, startOffset: i.startOffset, endContainer: i.endContainer, endOffset: i.endOffset }); this.collapsed = this.startContainer === this.endContainer && this.startOffset === this.endOffset; } });
  if (!G.Crypto) adopt('Crypto', G.crypto);
  if (!G.SubtleCrypto) iface('SubtleCrypto');
  if (!G.MessagePort) expose('MessagePort', MessagePort);
  if (!G.ElementInternals) expose('ElementInternals', class ElementInternals { constructor() { this.states = new Set(); this.form = null; this.validity = { valid: true }; } setFormValue() {} setValidity() {} checkValidity() { return true; } reportValidity() { return true; } });
  if (G.HTMLElement && !G.HTMLElement.prototype.attachInternals) def(G.HTMLElement.prototype, 'attachInternals', function () { return new G.ElementInternals(); });
  if (!G.DOMMatrix) {
    class DOMMatrixReadOnly {
      constructor(init) { const m = Array.isArray(init) && init.length === 6 ? init : [1, 0, 0, 1, 0, 0]; [this.a, this.b, this.c, this.d, this.e, this.f] = m; this.is2D = true; }
      get m11() { return this.a; } get m12() { return this.b; } get m21() { return this.c; } get m22() { return this.d; } get m41() { return this.e; } get m42() { return this.f; }
      get isIdentity() { return this.a === 1 && this.b === 0 && this.c === 0 && this.d === 1 && this.e === 0 && this.f === 0; }
      multiply(o) { const a = this; return new DOMMatrix([a.a * o.a + a.c * o.b, a.b * o.a + a.d * o.b, a.a * o.c + a.c * o.d, a.b * o.c + a.d * o.d, a.a * o.e + a.c * o.f + a.e, a.b * o.e + a.d * o.f + a.f]); }
      translate(x = 0, y = 0) { return this.multiply(new DOMMatrix([1, 0, 0, 1, x, y])); }
      scale(s = 1, sy = s) { return this.multiply(new DOMMatrix([s, 0, 0, sy, 0, 0])); }
      inverse() { const { a, b, c, d, e, f } = this; const det = a * d - b * c; return new DOMMatrix([d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det]); }
      transformPoint(p = {}) { const x = p.x || 0, y = p.y || 0; return { x: this.a * x + this.c * y + this.e, y: this.b * x + this.d * y + this.f, z: 0, w: 1 }; }
      toString() { return 'matrix(' + [this.a, this.b, this.c, this.d, this.e, this.f].join(', ') + ')'; }
    }
    class DOMMatrix extends DOMMatrixReadOnly {}
    expose('DOMMatrixReadOnly', DOMMatrixReadOnly); expose('DOMMatrix', DOMMatrix); expose('WebKitCSSMatrix', DOMMatrix);
    expose('DOMPoint', class DOMPoint { constructor(x = 0, y = 0, z = 0, w = 1) { Object.assign(this, { x, y, z, w }); } });
  }
  if (!G.Path2D) expose('Path2D', class Path2D { addPath() {} closePath() {} moveTo() {} lineTo() {} bezierCurveTo() {} quadraticCurveTo() {} arc() {} arcTo() {} ellipse() {} rect() {} roundRect() {} });
  if (!G.EventSource) expose('EventSource', class EventSource extends EventTarget { constructor(url) { super(); this.url = String(url); this.readyState = 2; this.onerror = null; this.onmessage = null; this.onopen = null; setTimeout(() => { const e = new Event('error'); if (typeof this.onerror === 'function') this.onerror(e); this.dispatchEvent(e); }, 0); } close() { this.readyState = 2; } });
  // IndexedDB is not persisted: opening fails asynchronously so pages take their fallback path
  if (!G.indexedDB) {
    class IDBRequest extends EventTarget { constructor() { super(); this.result = undefined; this.error = null; this.readyState = 'pending'; this.onsuccess = null; this.onerror = null; this.onupgradeneeded = null; this.onblocked = null; } }
    const fail = (req) => setTimeout(() => { req.readyState = 'done'; req.error = new DOMException('IndexedDB is not available', 'UnknownError'); const e = new Event('error'); if (typeof req.onerror === 'function') req.onerror(e); req.dispatchEvent(e); }, 0);
    class IDBFactory { open() { const r = new IDBRequest(); fail(r); return r; } deleteDatabase() { const r = new IDBRequest(); fail(r); return r; } databases() { return Promise.resolve([]); } cmp(a, b) { return a < b ? -1 : a > b ? 1 : 0; } }
    expose('IDBRequest', IDBRequest); expose('IDBOpenDBRequest', IDBRequest); expose('IDBFactory', IDBFactory);
    expose('indexedDB', new IDBFactory());
  }
  if (!G.caches) expose('caches', { open: () => Promise.reject(new DOMException('CacheStorage is not available', 'SecurityError')), match: () => Promise.resolve(undefined), has: () => Promise.resolve(false), keys: () => Promise.resolve([]), delete: () => Promise.resolve(false) });
  // Navigation timing for `performance.getEntriesByType('navigation')`
  if (G.performance) {
    const nav = { name: String(G.location && G.location.href), entryType: 'navigation', startTime: 0, duration: 120, initiatorType: 'navigation', nextHopProtocol: 'h2', workerStart: 0, redirectStart: 0, redirectEnd: 0, fetchStart: 1, domainLookupStart: 1, domainLookupEnd: 1, connectStart: 1, connectEnd: 1, secureConnectionStart: 1, requestStart: 2, responseStart: 40, responseEnd: 50, transferSize: 0, encodedBodySize: 0, decodedBodySize: 0, serverTiming: [], unloadEventStart: 0, unloadEventEnd: 0, domInteractive: 80, domContentLoadedEventStart: 90, domContentLoadedEventEnd: 91, domComplete: 110, loadEventStart: 115, loadEventEnd: 120, type: 'navigate', redirectCount: 0, activationStart: 0, toJSON() { return Object.assign({}, this); } };
    const byType = (t) => (t === 'navigation' ? [nav] : t === 'paint' ? [{ name: 'first-contentful-paint', entryType: 'paint', startTime: 100, duration: 0 }] : []);
    def(G.performance, 'getEntriesByType', byType);
    def(G.performance, 'getEntries', () => [nav]);
    if (!G.performance.timing) def(G.performance, 'timing', { navigationStart: 0, fetchStart: 1, responseStart: 40, responseEnd: 50, domLoading: 60, domInteractive: 80, domContentLoadedEventStart: 90, domContentLoadedEventEnd: 91, domComplete: 110, loadEventStart: 115, loadEventEnd: 120 });
    if (!G.performance.navigation) def(G.performance, 'navigation', { type: 0, redirectCount: 0 });
  }
  if (!G.PerformanceNavigationTiming) iface('PerformanceNavigationTiming');
  if (G.performance && !G.performance.mark) {
    const marks = [];
    def(G.performance, 'mark', (name, o = {}) => { const e = { name: String(name), entryType: 'mark', startTime: o.startTime !== undefined ? o.startTime : G.performance.now(), duration: 0, detail: o.detail === undefined ? null : o.detail }; marks.push(e); return e; });
    def(G.performance, 'measure', (name, a, b) => { const find = (n) => { const m = marks.filter((x) => x.name === n).pop(); return m ? m.startTime : 0; }; const s = typeof a === 'string' ? find(a) : (a && a.start) || 0; const e = typeof b === 'string' ? find(b) : G.performance.now(); const m = { name: String(name), entryType: 'measure', startTime: s, duration: e - s, detail: null }; marks.push(m); return m; });
    def(G.performance, 'clearMarks', (n) => { for (let i = marks.length - 1; i >= 0; i--) if (marks[i].entryType === 'mark' && (n === undefined || marks[i].name === n)) marks.splice(i, 1); });
    def(G.performance, 'clearMeasures', (n) => { for (let i = marks.length - 1; i >= 0; i--) if (marks[i].entryType === 'measure' && (n === undefined || marks[i].name === n)) marks.splice(i, 1); });
    def(G.performance, 'clearResourceTimings', () => {});
    def(G.performance, 'setResourceTimingBufferSize', () => {});
    def(G.performance, 'getEntriesByName', (n, t) => marks.filter((m) => m.name === n && (!t || m.entryType === t)));
    const byType = G.performance.getEntriesByType;
    def(G.performance, 'getEntriesByType', (t) => (t === 'mark' || t === 'measure' ? marks.filter((m) => m.entryType === t) : byType(t)));
    if (G.performance.timeOrigin === undefined) def(G.performance, 'timeOrigin', Date.now() - G.performance.now());
    def(G.performance, 'toJSON', () => ({ timeOrigin: G.performance.timeOrigin }));
  }
  if (!G.PerformanceObserver) expose('PerformanceObserver', class PerformanceObserver { constructor(cb) { this._cb = cb; } observe() {} disconnect() {} takeRecords() { return []; } static get supportedEntryTypes() { return ['mark', 'measure', 'navigation', 'paint']; } });
  // the prelude runs before a document exists: patch the prototype, never the instance
  if (G.Document && !Object.getOwnPropertyDescriptor(G.Document.prototype, 'location')) {
    Object.defineProperty(G.Document.prototype, 'location', { configurable: true, enumerable: true, get() { return G.location; }, set(v) { G.location.href = v; } });
  }
})(globalThis);
