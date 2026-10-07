// tight numeric loops (JIT fast paths)
function sum(n) { let s = 0; for (let i = 0; i < n; i++) { s += i * 2 - (i % 7); } return s; }
function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function sieve(n) { const a = new Uint8Array(n + 1); let c = 0; for (let i = 2; i <= n; i++) { if (!a[i]) { c++; for (let j = i * i; j <= n; j += i) a[j] = 1; } } return c; }
const t0 = Date.now(); const r = [sum(3000000), fib(24), sieve(300000)];
console.log('numeric', r.join(','));
