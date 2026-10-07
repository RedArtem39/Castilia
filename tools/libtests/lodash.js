// libs: lodash.js
// expect: 1,2,3|a-b|{"a":{"b":2}}|true
const r = [_.uniq([1,2,2,3]).join(), _.kebabCase('aB'), JSON.stringify(_.merge({a:{b:1}},{a:{b:2}})), _.isEqual({x:[1]},{x:[1]})];
console.log(r.join('|'));
