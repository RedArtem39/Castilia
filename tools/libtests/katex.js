// libs: katex.js highlight.js
// expect: katex true hljs true
console.log('katex', katex.renderToString('x^2').includes('katex'), 'hljs', hljs.highlight('const x = 1', {language:'javascript'}).value.includes('hljs-keyword'));
