// libs: vue.js
// expect: <p>count 2</p>
const el = document.createElement('div'); document.body.appendChild(el);
const app = Vue.createApp({ data(){ return {n:1} }, template: '<p>count {{n}}</p>', mounted(){ this.n++; } });
app.mount(el);
Vue.nextTick().then(()=>console.log(el.innerHTML));
