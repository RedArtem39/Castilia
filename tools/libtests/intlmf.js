// libs: intlmf.js
// expect: mf Hello Ann, you have 3 messages|one
const m = new IntlMessageFormat.IntlMessageFormat('Hello {name}, you have {n, plural, one {# message} other {# messages}}', 'en');
const p = new IntlMessageFormat.IntlMessageFormat('{n, plural, one {one} other {many}}', 'en');
console.log('mf', m.format({ name: 'Ann', n: 3 }) + '|' + p.format({ n: 1 }));
