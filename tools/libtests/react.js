// libs: react.js react-dom.js
// expect: <div><h1>Hello 3</h1><button>inc</button></div> after-effect
const e = React.createElement;
function App(){ const [n,setN] = React.useState(3); React.useEffect(()=>{ setTimeout(()=>console.log('after-effect'),0) },[]); return e('div',null,e('h1',null,'Hello ',n),e('button',null,'inc')); }
const root = document.createElement('div'); document.body.appendChild(root);
ReactDOM.flushSync(()=>ReactDOM.createRoot(root).render(e(App)));
console.log(root.innerHTML);
