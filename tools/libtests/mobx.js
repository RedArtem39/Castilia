// libs: mobx.js redux.js
// expect: mobx 2 redux 3
const o = mobx.observable({n:1}); let seen=0; mobx.autorun(()=>{ seen = o.n }); o.n = 2;
const st = Redux.createStore((s=1,a)=> a.type==='inc'? s+2 : s); st.dispatch({type:'inc'});
console.log('mobx', seen, 'redux', st.getState());
