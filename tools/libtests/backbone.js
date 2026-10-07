// libs: underscore.js jquery.js backbone.js
// expect: bb 5 changed
const M = Backbone.Model.extend({defaults:{v:1}}); const m = new M(); let ch=''; m.on('change:v', ()=>ch='changed'); m.set('v',5); console.log('bb', m.get('v'), ch);
