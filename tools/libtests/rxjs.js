// libs: rxjs.js
// expect: rx 2,4,6
const {of, map, toArray} = rxjs; of(1,2,3).pipe(map(x=>x*2), toArray()).subscribe(a=>console.log('rx', a.join()));
