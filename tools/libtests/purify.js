// libs: purify.js
// expect: <p>Hello <b>world</b><img src="x"><a>l</a></p> | T
console.log(DOMPurify.sanitize('<p>Hello <b>world</b><img src=x onerror=alert(1)><script>bad()</script><a href="javascript:x">l</a></p>') + ' | ' + document.implementation.createHTMLDocument('T').title);
