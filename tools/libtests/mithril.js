// libs: mithril.js
// expect: <span class="a">m 1</span>
const root = document.createElement('div'); document.body.appendChild(root);
m.render(root, m('span.a', 'm ', 1)); console.log(root.innerHTML);
