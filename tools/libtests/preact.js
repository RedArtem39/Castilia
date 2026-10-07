// libs: preact.js
// expect: <b>pre 7</b>
const root = document.createElement('div'); document.body.appendChild(root);
preact.render(preact.h('b', null, 'pre ', 7), root); console.log(root.innerHTML);
