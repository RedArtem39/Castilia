// libs:
// expect: modern 6 a,b 3 ok done
(async function(){
  async function* gen(){ for (let i=1;i<=3;i++){ await null; yield i } }
  let s = 0; for await (const v of gen()) s += v;
  const m = new Map([['a',1],['b',2]]);
  const {x = 3, ...rest} = {y: 1};
  class P { #v = 'ok'; static make(){ return new P } get v(){ return this.#v } }
  const r = await Promise.allSettled([Promise.reject(1), 2]);
  console.log('modern', s, [...m.keys()].join(), x, P.make().v, r.length === 2 ? 'done' : 'no');
})();
