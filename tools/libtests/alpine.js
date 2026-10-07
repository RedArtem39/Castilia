// libs: alpine.js
// expect: <span x-text="n">5</span>
document.body.innerHTML = '<div x-data="{n:5}"><span x-text="n"></span></div>';
setTimeout(()=>console.log(document.querySelector('span').outerHTML), 50);
