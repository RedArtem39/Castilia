// property access, method calls, allocation (typical app code)
class Vec { constructor(x, y) { this.x = x; this.y = y; } add(o) { return new Vec(this.x + o.x, this.y + o.y); } len() { return Math.sqrt(this.x * this.x + this.y * this.y); } }
function run(n) { let v = new Vec(0, 0); const d = new Vec(1, 2); for (let i = 0; i < n; i++) v = v.add(d); return Math.round(v.len()); }
function strs(n) { const parts = []; for (let i = 0; i < n; i++) parts.push('item' + i); return parts.join(',').length; }
function maps(n) { const m = new Map(); for (let i = 0; i < n; i++) m.set('k' + (i % 1000), i); let s = 0; for (const [, v] of m) s += v; return s; }
function arrays(n) { const a = []; for (let i = 0; i < n; i++) a.push({ id: i, v: i % 13 }); return a.filter(o => o.v > 5).map(o => o.id).reduce((x, y) => x + y, 0); }
console.log('objects', [run(300000), strs(100000), maps(200000), arrays(100000)].join(','));
