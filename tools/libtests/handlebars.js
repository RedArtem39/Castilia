// libs: handlebars.js marked.js
// expect: Hi Bob! (2) <h1>T</h1>
const t = Handlebars.compile('Hi {{name}}! ({{items.length}})');
console.log(t({name:'Bob', items:[1,2]}), marked.parse('# T').trim());
