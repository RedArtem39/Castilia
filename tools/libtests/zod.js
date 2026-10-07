// libs: zod.js
// expect: zod true false
const s = Zod.object({a: Zod.string(), n: Zod.number().min(2)}); console.log('zod', s.safeParse({a:'x', n:3}).success, s.safeParse({a:1, n:1}).success);
