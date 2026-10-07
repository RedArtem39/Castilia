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
})(globalThis);
