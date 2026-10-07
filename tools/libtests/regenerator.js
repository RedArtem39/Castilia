// libs: regenerator.js
// expect: regen 1 2 done
var _marked = regeneratorRuntime.mark(function g(){ return regeneratorRuntime.wrap(function(ctx){ while(1) switch(ctx.prev = ctx.next){ case 0: ctx.next = 2; return 1; case 2: ctx.next = 4; return 2; case 4: case 'end': return ctx.stop(); } }, g); });
const it = _marked(); console.log('regen', it.next().value, it.next().value, it.next().done ? 'done' : 'no');
