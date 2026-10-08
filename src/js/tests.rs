use super::*;
use alloc::{string::ToString, vec::Vec};

fn run(src: &str) -> Vec<String> {
    run_tier(src, vm::Tier::Bytecode)
}

fn run_tier(src: &str, tier: vm::Tier) -> Vec<String> {
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.interp.tier = tier;
    #[cfg(feature = "jit")]
    if tier == vm::Tier::Native {
        rt.interp.jit.threshold = 1;
        rt.set_code_memory(alloc::boxed::Box::new(test_exec_mem::Mem));
    }
    // collect very often so a wrongly freed object shows up as a test failure
    rt.interp.heap.threshold = 50;
    if let Err(e) = rt.execute(src, &mut doc) {
        let mut l = rt.logs();
        l.push(alloc::format!("ERR {e}"));
        return l;
    }
    rt.interp.run_timers_until(1e9, 10000);
    rt.logs().into_iter().map(|l| l.strip_prefix("log: ").map(|s| s.to_string()).unwrap_or(l)).collect()
}

/// Both the AST walker and the bytecode VM must agree with the expectation.
fn check(src: &str, expect: &[&str]) {
    let got = run_tier(src, vm::Tier::Ast);
    assert_eq!(got, expect, "AST tier, script: {src}");
    let got = run(src);
    assert_eq!(got, expect, "bytecode tier, script: {src}");
    check_native(src, expect);
}

/// Semantics only the bytecode tier implements (real suspension, deep recursion).
fn check_vm(src: &str, expect: &[&str]) {
    let got = run(src);
    assert_eq!(got, expect, "bytecode tier, script: {src}");
    check_native(src, expect);
}

/// The native tier (every function compiled immediately) must agree too.
fn check_native(src: &str, expect: &[&str]) {
    #[cfg(all(feature = "jit", target_arch = "x86_64", any(windows, unix)))]
    {
        let got = run_tier(src, vm::Tier::Native);
        assert_eq!(got, expect, "native tier, script: {src}");
    }
    let _ = (src, expect);
}

#[cfg(all(feature = "jit", target_arch = "x86_64", any(windows, unix)))]
mod test_exec_mem {
    extern crate std;
    use core::ptr::NonNull;
    pub struct Mem;
    #[cfg(windows)]
    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
        fn VirtualProtect(addr: *mut u8, size: usize, protect: u32, old: *mut u32) -> i32;
    }
    #[cfg(unix)]
    unsafe extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
    }
    impl crate::platform::CodeMemory for Mem {
        fn install(&mut self, code: &[u8]) -> Option<NonNull<u8>> {
            unsafe {
                #[cfg(windows)]
                let p = {
                    let p = VirtualAlloc(core::ptr::null_mut(), code.len(), 0x3000, 0x04);
                    core::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len());
                    let mut old = 0;
                    VirtualProtect(p, code.len(), 0x20, &mut old);
                    p
                };
                #[cfg(unix)]
                let p = {
                    let flags = if cfg!(target_os = "macos") { 0x1002 } else { 0x22 };
                    let p = mmap(core::ptr::null_mut(), code.len(), 3, flags, -1, 0);
                    core::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len());
                    mprotect(p, code.len(), 5);
                    p
                };
                NonNull::new(p)
            }
        }
    }
}

#[test]
fn vm_compiles_common_code() {
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.execute(
        "function a(x,{y,z=2}=[],...r){for(const k in {p:1}){} for(let i=0;i<2;i++){try{x+=i}catch(e){}finally{}} switch(x){case 1:break;default:} return x?.toString()+`${y}`} a(1); class C{m(){return [...arguments]}} new C().m(); (async()=>{await 1})(); (function*(){yield* [1]})().next();",
        &mut doc,
    )
    .unwrap();
    assert!(rt.interp.vm_stats.compiled >= 5, "{:?}", rt.interp.vm_stats);
    assert_eq!(rt.interp.vm_stats.fallbacks, 0, "{:?}", rt.interp.vm_stats);
}

#[test]
fn vm_generators_suspend() {
    check_vm(
        "function* g(){ const x = yield 1; console.log('got', x); try { yield 2 } finally { console.log('cleanup') } }
         const it=g(); console.log(it.next().value); console.log(it.next('A').value); console.log(it.return(5).value, it.next().done)",
        &["1", "got A", "2", "cleanup", "5 true"],
    );
    check_vm("function* nat(){let i=0; while(true) yield i++} const it=nat(); it.next(); it.next(); console.log(it.next().value)", &["2"]);
    check_vm(
        "function* inner(){ const r = yield 'a'; return r + '!' } function* outer(){ const v = yield* inner(); yield v }
         const o = outer(); console.log(o.next().value, o.next('x').value, o.next().done)",
        &["a x! true"],
    );
    check_vm("function* g(){ try { yield 1 } catch(e) { yield 'caught ' + e } } const it=g(); it.next(); console.log(it.throw('E').value)", &["caught E"]);
}

#[test]
fn vm_async_ordering() {
    check_vm(
        "async function a(){ console.log('a1'); await null; console.log('a2') } a(); console.log('sync'); Promise.resolve().then(()=>console.log('p'))",
        &["a1", "sync", "a2", "p"],
    );
    check_vm("async function w(){ await new Promise(()=>{}); console.log('never') } w(); console.log('after')", &["after"]);
    check_vm(
        "async function f(){ try { await Promise.reject(new Error('boom')) } catch(e) { return e.message } } f().then(v=>console.log('caught', v))",
        &["caught boom"],
    );
    check_vm(
        "async function* ag(){ yield 1; await null; yield 2 } (async()=>{ for await (const v of ag()) console.log(v); console.log('done') })()",
        &["1", "2", "done"],
    );
    check_vm(
        "const sleep=ms=>new Promise(r=>setTimeout(r,ms)); (async()=>{ const t0=Date.now(); await sleep(50); console.log('slept', Date.now()-t0>=50) })(); console.log('first')",
        &["first", "slept true"],
    );
}

#[test]
fn vm_semantics() {
    check_vm("function f(n){return n?f(n-1)+1:0} console.log(f(5000))", &["5000"]);
    check_vm("var x = 1; function f(){ var x = 2; return eval('x') } console.log(f(), eval('x'), (0,eval)('typeof f'))", &["2 1 function"]);
    check_vm("function t(){ if (true) { function inner(){ return 'i' } } return inner() } console.log(t())", &["i"]);
    check_vm("function f(){ try { return 1 } finally { console.log('fin') } } console.log(f())", &["fin", "1"]);
    check_vm("function f(){ for (const x of [1,2,3]) { try { if (x==2) break } finally { console.log('f'+x) } } return 'end' } console.log(f())", &["f1", "f2", "end"]);
    check_vm(
        "function f(){ const it={i:0,[Symbol.iterator](){return this},next(){return {value:this.i++,done:false}},return(){console.log('closed');return {}}}; for(const v of it){ if(v==1) break } } f()",
        &["closed"],
    );
    check_vm("function f(){ const fs=[]; for(let i=0;i<3;i++) fs.push(()=>i); return fs.map(f=>f()).join() } console.log(f())", &["0,1,2"]);
    check_vm("function f(){ const o={a:{b(){return this.c}, c:5}}; return [o?.a?.b(), o.x?.y.z, o.a.zz?.(), o.x?.[1]] } console.log(f().join('|'))", &["5|||"]);
    check_vm("function f(){ let s=''; a: for (const i of [1,2]) { try { for (const j of [1,2]) { if (j==2) continue a; s+=i+''+j } } finally { s+='F' } } return s } console.log(f())", &["11F21F"]);
    check_vm("function f(a, b = a + 1, {c} = {c: b * 2}){ return [a,b,c] } console.log(f(1).join(), f(1,5).join())", &["1,2,4 1,5,10"]);
    check_vm("function f(){ 'use strict'; const c = 1; try { c = 2 } catch(e) { return e instanceof TypeError } } console.log(f())", &["true"]);
    check_vm("function f(){ let x = 0; x ||= 5; x &&= x + 1; let y; y ??= 'd'; const o = {n:null}; o.n ??= 3; o['m'] ||= 4; return [x,y,o.n,o.m].join() } console.log(f())", &["6,d,3,4"]);
    check_vm("function f(){ return typeof undeclared + typeof f + (void 0) } console.log(f())", &["undefinedfunctionundefined"]);
    check_vm("class A { static #c = 0; #v; constructor(v){ this.#v = v; A.#c++ } get v(){ return this.#v } static count(){ return A.#c } } function f(){ new A(1); return new A(7).v + ':' + A.count() } console.log(f())", &["7:2"]);
}

#[test]
fn arithmetic_and_strings() {
    check("console.log(1+2, 'a'+1, 0.1+0.2, 7%3, 2**10, -5 % 3, 1/3)", &["3 a1 0.30000000000000004 1 1024 -2 0.3333333333333333"]);
    check("console.log('abc'.toUpperCase(), 'a-b-c'.split('-'), ' x '.trim()+'|', 'abc'.at(-1))", &["ABC [ 'a', 'b', 'c' ] x| c"]);
    check("console.log(`t ${1+1} ${'x'}`)", &["t 2 x"]);
}

#[test]
fn functions_and_closures() {
    check("function mk(){let n=0;return()=>++n} const c=mk();c();c();console.log(c())", &["3"]);
    check("const f=(a,b=2,...r)=>a+b+r.length;console.log(f(1),f(1,3,4,5))", &["3 6"]);
    check("console.log((function(){return arguments.length})(1,2,3))", &["3"]);
    check("var o={a:1,get b(){return this.a+1},m(){return this.b*2}};console.log(o.m())", &["4"]);
}

#[test]
fn classes() {
    check(
        "class A{constructor(x){this.x=x} get dbl(){return this.x*2} static s(){return 's'} #p=5; getP(){return this.#p}}
         class B extends A{constructor(){super(4)} dbl2(){return super.dbl+1}}
         const b=new B();console.log(b.dbl,b.dbl2(),A.s(),b.getP(),b instanceof A)",
        &["8 9 s 5 true"],
    );
}

#[test]
fn destructuring_spread() {
    check("const {a,b:{c=3}={},...r}={a:1,x:2,y:3};const [p,,q=9,...z]=[1,2,undefined,4,5];console.log(a,c,r,p,q,z)", &["1 3 { x: 2, y: 3 } 1 9 [ 4, 5 ]"]);
    check("console.log([...'ab',...[1,2]], Math.max(...[1,5,3]), {...{a:1},b:2})", &["[ 'a', 'b', 1, 2 ] 5 { a: 1, b: 2 }"]);
}

#[test]
fn control_flow() {
    check("let s=0;outer:for(let i=0;i<5;i++){for(let j=0;j<5;j++){if(j==2)continue outer;if(i==3)break outer;s+=1}}console.log(s)", &["6"]);
    check("try{null.x}catch(e){console.log(e instanceof TypeError, e.message)}finally{console.log('f')}", &["true Cannot read properties of null (reading 'x')", "f"]);
    check("switch(3){case 1:console.log(1);case 3:console.log(3);case 4:console.log(4);break;default:console.log('d')}", &["3", "4"]);
}

#[test]
fn generators_iterators() {
    check("function*g(){yield 1;yield 2;return 3}const it=g();console.log(it.next().value,it.next().value,it.next().value,it.next().done)", &["1 2 3 true"]);
    check("const m=new Map([[1,'a'],[2,'b']]);console.log([...m.keys()],[...m],m.size,new Set([1,1,2]).size)", &["[ 1, 2 ] [ [ 1, 'a' ], [ 2, 'b' ] ] 2 2"]);
    check("const o={*[Symbol.iterator](){yield 1;yield 2}};console.log(Array.from(o),[...o])", &["[ 1, 2 ] [ 1, 2 ]"]);
}

#[test]
fn promises_async() {
    check("Promise.resolve(1).then(v=>console.log('then',v));console.log('sync')", &["sync", "then 1"]);
    check("async function f(){await null;return 5}f().then(v=>console.log(v));console.log('a')", &["a", "5"]);
    check("(async()=>{try{await Promise.reject(new Error('x'))}catch(e){console.log('caught',e.message)}})()", &["caught x"]);
    check("Promise.all([1,Promise.resolve(2)]).then(v=>console.log(v));setTimeout(()=>console.log('t'),10)", &["[ 1, 2 ]", "t"]);
    check("Promise.allSettled([Promise.reject(1),2]).then(r=>console.log(JSON.stringify(r)))", &[r#"[{"status":"rejected","reason":1},{"status":"fulfilled","value":2}]"#]);
}

#[test]
fn json_regex_date() {
    check("console.log(JSON.stringify({a:[1,{b:2}],c:'x',d:undefined}),JSON.parse('{\"a\":[1,2,{\"b\":null}]}').a[2].b)", &[r#"{"a":[1,{"b":2}],"c":"x"} null"#]);
    check("console.log(JSON.stringify({a:1,b:[1,2]},null,2))", &["{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}"]);
    check("console.log('a1b22c'.replace(/\\d+/g,m=>'['+m+']'),/(\\d+)-(\\d+)/.exec('x 12-34')[2],'a,b;c'.split(/[,;]/))", &["a[1]b[22]c 34 [ 'a', 'b', 'c' ]"]);
    check("console.log(new Date(0).toISOString(),new Date('2020-02-29T12:00:00Z').getUTCDay(),new Date(2020,0,31).getMonth())", &["1970-01-01T00:00:00.000Z 6 0"]);
}

#[test]
fn proxies_symbols_misc() {
    check("const p=new Proxy({}, {get:(t,k)=>'k:'+String(k)});console.log(p.foo)", &["k:foo"]);
    check("const s=Symbol('d');const o={[s]:1};console.log(o[s],s.toString(),typeof s)", &["1 Symbol(d) symbol"]);
    check("console.log([3,1,2].sort(),[3,1,10].sort((a,b)=>a-b),[1,[2,[3]]].flat(Infinity),Object.entries({a:1}))", &["[ 1, 2, 3 ] [ 1, 3, 10 ] [ 1, 2, 3 ] [ [ 'a', 1 ] ]"]);
    check("console.log(typeof undefinedVar, (()=>{try{undefinedVar}catch(e){return e.name}})())", &["undefined ReferenceError"]);
    check("console.log(parseInt('42px'),parseFloat('3.5e1x'),(1234.5678).toFixed(2),(255).toString(16),Number('0x10'))", &["42 35 1234.57 ff 16"]);
    check("console.log(a?.b?.c, null ?? 'd', 0 || 'e', (x=>x?.y)(null)); var a;", &["undefined d e undefined"]);
}

#[test]
fn getters_labels_tagged() {
    check("function t(s,...v){return s.raw.join('|')+v.join(',')}console.log(t`a${1}b${2}c`)", &["a|b|c1,2"]);
    check("const o={};Object.defineProperty(o,'x',{get(){return 7},enumerable:false});console.log(o.x,Object.keys(o).length)", &["7 0"]);
    check("console.log([1,2,3].map(x=>x*2).filter(x=>x>2).reduce((a,b)=>a+b,0), Object.assign({},{a:1},{b:2}))", &["10 { a: 1, b: 2 }"]);
}

fn run_html(html: &str, src: &str) -> Vec<String> {
    let mut doc = crate::html::parse(html);
    let mut rt = Runtime::new();
    rt.set_url("http://example.com/a/b?q=1#h");
    if let Err(e) = rt.execute(src, &mut doc) {
        let mut l = rt.logs();
        l.push(alloc::format!("ERR {e}"));
        return l;
    }
    rt.interp.run_timers_until(1e9, 10000);
    rt.logs().into_iter().map(|l| l.strip_prefix("log: ").map(|s| s.to_string()).unwrap_or(l)).collect()
}

fn check_html(html: &str, src: &str, expect: &[&str]) {
    let got = run_html(html, src);
    assert_eq!(got, expect, "script: {src}");
}

#[test]
fn prelude_loads_cleanly() {
    let rt = Runtime::new();
    assert_eq!(rt.logs(), Vec::<String>::new());
}

#[test]
fn dom_basics() {
    check_html("<div id=a class='x y'><p>hi</p><p>there</p></div>", "const a=document.getElementById('a');console.log(a.tagName,a.children.length,a.firstElementChild.textContent,a.className,a.classList.contains('y'))", &["DIV 2 hi x y true"]);
    check_html("<ul id=u></ul>", "const u=document.getElementById('u');for(let i=0;i<3;i++){const li=document.createElement('li');li.textContent='n'+i;u.appendChild(li)}console.log(u.innerHTML,u.querySelectorAll('li').length)", &["<li>n0</li><li>n1</li><li>n2</li> 3"]);
    check_html("<div id=a></div>", "const a=document.getElementById('a');a.innerHTML='<span class=k>x</span><b>y</b>';a.classList.add('p','q');a.classList.toggle('p');a.setAttribute('data-foo-bar','1');console.log(a.outerHTML,a.dataset.fooBar,Object.keys(a.dataset))", &["<div id=\"a\" class=\"q\" data-foo-bar=\"1\"><span class=\"k\">x</span><b>y</b></div> 1 [ 'fooBar' ]"]);
    check_html("<p id=p>x</p>", "const p=document.getElementById('p');p.style.color='red';p.style.backgroundColor='blue';console.log(p.getAttribute('style'),p.style.color,p.style.cssText)", &["color: red; background-color: blue; red color: red; background-color: blue;"]);
}

#[test]
fn dom_traversal_and_selectors() {
    check_html("<div><span id=a>1</span><span id=b>2</span><i>3</i></div>", "const a=document.getElementById('a');console.log(a.nextElementSibling.id,a.nextSibling.id,a.parentNode.tagName,a.closest('div').tagName,a.matches('span#a'),document.querySelector('i').previousElementSibling.id,a.parentNode.lastChild.tagName)", &["b b DIV DIV true b I"]);
    check_html("<p class=a>1</p><p class='a b'>2</p>", "console.log(document.getElementsByClassName('a').length,document.querySelectorAll('.a.b').length,document.getElementsByTagName('p').length,[...document.querySelectorAll('p')].map(p=>p.textContent).join())", &["2 1 2 1,2"]);
    check_html("<div id=x></div>", "const x=document.getElementById('x');const f=document.createDocumentFragment();f.append('a',document.createElement('b'),'c');x.appendChild(f);console.log(x.innerHTML,x.childNodes.length,f.childNodes.length)", &["a<b></b>c 3 0"]);
}

#[test]
fn events_flow() {
    check_html(
        "<div id=o><button id=b>x</button></div>",
        "const o=document.getElementById('o'),b=document.getElementById('b');const log=[];o.addEventListener('click',e=>log.push('cap:'+e.target.id),true);o.addEventListener('click',e=>log.push('bub:'+e.currentTarget.id));b.addEventListener('click',e=>{log.push('t');e.stopPropagation()});b.click();console.log(log.join())",
        &["cap:b,t"],
    );
    check_html("<button id=b>x</button>", "const b=document.getElementById('b');let n=0;b.onclick=()=>n++;b.addEventListener('click',()=>n+=10,{once:true});b.click();b.click();console.log(n)", &["12"]);
    check_html("<input id=c type=checkbox>", "const c=document.getElementById('c');const ev=[];c.addEventListener('change',()=>ev.push('change:'+c.checked));c.click();c.click();console.log(ev.join(),c.checked)", &["change:true,change:false false"]);
    check_html("<div id=d></div>", "const d=document.getElementById('d');d.addEventListener('custom',e=>console.log('got',e.detail.v));d.dispatchEvent(new CustomEvent('custom',{detail:{v:5},bubbles:true}))", &["got 5"]);
}

#[test]
fn timers_and_observers() {
    check_html("<div id=a></div>", "const a=document.getElementById('a');new MutationObserver(r=>console.log('mut',r.length,r[0].type)).observe(a,{childList:true});a.appendChild(document.createElement('i'));setTimeout(()=>console.log('t2'),20);setTimeout(()=>console.log('t1'),10);requestAnimationFrame(()=>console.log('raf'))", &["mut 1 childList", "t1", "raf", "t2"]);
    check_html("", "let n=0;const id=setInterval(()=>{n++;if(n==3){clearInterval(id);console.log('done',n)}},5)", &["done 3"]);
}

#[test]
fn bom_apis() {
    check_html("", "console.log(location.href,location.pathname,location.search,location.hash,location.host)", &["http://example.com/a/b?q=1#h /a/b ?q=1 #h example.com"]);
    check_html("", "const u=new URL('../x?y=1#z','http://a.com/p/q/r');u.searchParams.append('k','v w');console.log(u.href,u.searchParams.get('y'),u.origin)", &["http://a.com/p/x?y=1&k=v+w#z 1 http://a.com"]);
    check_html("", "localStorage.setItem('a','1');localStorage.b='2';console.log(localStorage.getItem('b'),localStorage.length,Object.keys(localStorage).join(),sessionStorage.getItem('a'))", &["2 2 a,b null"]);
    check_html("", "fetch('http://x.test/').catch(e=>console.log('fail',e.name))", &["fail TypeError"]);
    check_html("", "console.log(btoa('hello'),atob('aGVsbG8='),encodeURIComponent('a b&c'),JSON.stringify(new URLSearchParams('a=1&b=2').get('b')))", &["aGVsbG8= hello a%20b%26c \"2\""]);
    check_html("", "console.log(matchMedia('(min-width: 500px)').matches, innerWidth, typeof navigator.userAgent, typeof Intl.NumberFormat)", &["true 1024 string function"]);
}

#[test]
fn forms_and_values() {
    check_html("<form id=f><input name=a value=1><input name=b type=checkbox checked><select name=s><option value=x>X</option><option value=y selected>Y</option></select><textarea name=t>hello</textarea></form>", "const f=document.getElementById('f');console.log(f.elements.length,new FormData(f).get('s'),new FormData(f).get('t'),f.querySelector('select').value,f.querySelector('[name=a]').value)", &["4 y hello y 1"]);
    check_html("<input id=i>", "const i=document.getElementById('i');i.value='abc';console.log(i.value,i.getAttribute('value'),i.type)", &["abc abc text"]);
}

#[test]
fn gc_collects_cycles_and_keeps_live_objects() {
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.interp.heap.threshold = 0;
    rt.execute(
        "globalThis.keep = []; for (let i = 0; i < 2000; i++) { const a = {i}; const b = {a}; a.b = b; function f(){ return a } a.f = f; if (i % 100 == 0) keep.push(a) }
         class K { constructor(){ this.self = this } } for (let i = 0; i < 500; i++) new K();",
        &mut doc,
    )
    .unwrap();
    let freed = rt.collect_garbage();
    assert!(freed > 5000, "freed {freed}");
    let out = rt.eval_in_page(&mut doc, "keep.length + ':' + keep[3].b.a.i + ':' + keep[3].f().i + ':' + new K().self.constructor.name").unwrap();
    assert_eq!(out, "20:300:300:K");
    // a second pass finds nothing new
    assert!(rt.collect_garbage() < 50);
}

#[test]
fn gc_keeps_suspended_generators_and_pending_async() {
    check_vm(
        "function* g(){ const o = {v: 1}; o.me = o; yield 1; yield o.me.v + 1 } globalThis.it = g(); it.next();
         for (let i = 0; i < 3000; i++) { const x = {}; x.x = x }
         async function a(){ const o = {n: 5}; o.o = o; await new Promise(r => setTimeout(r, 10)); console.log('async', o.o.n) } a();
         for (let i = 0; i < 3000; i++) { const x = {}; x.x = x }
         console.log('gen', it.next().value)",
        &["gen 2", "async 5"],
    );
}

#[test]
fn binary_data_and_streams() {
    check_vm(
        "const b = new ArrayBuffer(8); const u = new Uint8Array(b); const d = new DataView(b); d.setUint16(0, 0x1234); d.setFloat32(4, 1.5, true);
         const f = new Float32Array(b, 4, 1); const i16 = new Int16Array([70000, -1]);
         console.log(u[0], u[1], f[0], i16.join(), u.length, b.byteLength, new Uint8Array([1,2,3]).map(x => x * 2).join(), Uint8Array.from('12', Number).join(), u.subarray(0, 2).buffer === b, Object.prototype.toString.call(u))",
        &["18 52 1.5 4464,-1 8 8 2,4,6 1,2 true [object Uint8Array]"],
    );
    check_vm(
        "const t = new TextEncoder().encode('Привет €'); console.log(t.length, new TextDecoder().decode(t), new TextDecoder().decode(t.buffer))",
        &["16 Привет € Привет €"],
    );
    check_vm(
        "(async () => { const rs = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close() } }); let s = ''; for await (const x of rs) s += x;
          const ts = new TransformStream({ transform(ch, c) { c.enqueue(ch.toUpperCase()) } }); const w = ts.writable.getWriter(); w.write('x'); w.close();
          const r = await ts.readable.getReader().read(); const body = await new Response('hi').body.getReader().read();
          console.log(s, r.value, new TextDecoder().decode(body.value)) })()",
        &["ab X hi"],
    );
}

#[test]
fn array_holes() {
    check_vm(
        "const a = new Array(3); a[1] = 'x'; const b = [1, , 3]; delete b[0]; const seen = []; a.forEach((v, i) => seen.push(i));
         console.log(a.length, seen.join(), a.map(v => v + '!').join('|'), 1 in a, 0 in a, b.join('-'), Object.keys(b).join(), [...a].length, a[0] === undefined, JSON.stringify(a), a.filter(() => true).length, a.indexOf(undefined), a.includes(undefined))",
        &["3 1 |x!| true false --3 2 3 true [null,\"x\",null] 1 -1 true"],
    );
}

#[test]
#[ignore]
fn dump_bytecode() {
    let src = "function sum(n) { let s = 0; for (let i = 0; i < n; i++) { s += i * 2 - (i % 7); } return s; } sum(3)";
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.execute(src, &mut doc).unwrap();
    let name = rt.interp.interner.intern("sum");
    let genv = rt.interp.global_env.clone();
    let f = rt.interp.lookup(name, &genv).ok().unwrap();
    if let value::Value::Obj(o) = f {
        if let value::Class::Function(value::Func::Closure(c)) = &o.borrow().class {
            let code = c.def.code.get().unwrap().clone().unwrap();
            let mut out = String::new();
            for (i, op) in code.ops.iter().enumerate() {
                out.push_str(&alloc::format!("{i:3} {op:?}
"));
            }
            panic!("BYTECODE
{out}");
        }
    }
}

#[test]
fn value_layout_is_fixed() {
    use value::Value;
    assert_eq!(Value::SIZE, 24);
    let raw = |v: &Value| -> [u8; 24] { unsafe { core::mem::transmute_copy(v) } };
    let n = raw(&Value::Num(1.5));
    assert_eq!(n[0], Value::TAG_NUM);
    assert_eq!(f64::from_le_bytes(n[8..16].try_into().unwrap()), 1.5);
    let b = raw(&Value::Bool(true));
    assert_eq!((b[0], b[8]), (Value::TAG_BOOL, 1));
    assert_eq!(raw(&Value::Undefined)[0], Value::TAG_UNDEFINED);
    assert_eq!(raw(&Value::Null)[0], Value::TAG_NULL);
    assert!(raw(&Value::str("x"))[0] > Value::TAG_LAST_PLAIN);
}

#[test]
#[cfg(all(feature = "jit", target_arch = "x86_64", any(windows, unix)))]
fn native_tier_compiles_and_matches() {
    let src = "function sum(n) { let s = 0; for (let i = 0; i < n; i++) { s += i * 2 - (i % 7); if (s > 1e15) break; } return s; }
               function cmp(a, b) { let c = 0; for (let i = 0; i < 100; i++) { if (a < b) c++; if (a >= b) c--; if (a === b) c += 10; if (!(a != b)) c += 100; a += 0.5; } return c; }
               function nan() { let x = 0 / 0, k = 0; while (k < 3) { if (x) k += 100; if (!x) k++; } return k; }
               console.log(sum(100000), cmp(1, 30), nan(), -sum(3))";
    let expect = run_tier(src, vm::Tier::Bytecode);
    assert_eq!(expect, ["9999600005 126 3 -3"]);
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.interp.tier = vm::Tier::Native;
    rt.interp.jit.threshold = 1;
    rt.set_code_memory(alloc::boxed::Box::new(test_exec_mem::Mem));
    rt.execute(src, &mut doc).unwrap();
    let logs: Vec<String> = rt.logs().into_iter().map(|l| l.strip_prefix("log: ").map(|s| s.to_string()).unwrap_or(l)).collect();
    assert_eq!(logs, expect);
    let st = rt.vm_stats();
    assert!(st.native_compiled >= 3, "{st:?}");
}

#[test]
#[ignore]
#[cfg(all(feature = "jit", target_arch = "x86_64", any(windows, unix)))]
fn native_debug() {
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    rt.interp.tier = vm::Tier::Native;
    rt.interp.jit.threshold = 1;
    rt.set_code_memory(alloc::boxed::Box::new(test_exec_mem::Mem));
    let mut out = String::new();
    for s in ["function a(){ let k=0; for (let i=0;i<10;i++) k+=i; return k } console.log(a())", "function b(){ let k=0; for (let i=0;i<10;i++) k+=i; return k } console.log(b())", "function c(x){ let k=0; while (k < 3) { k++ } return k } console.log(c())"] {
        rt.execute(s, &mut doc).unwrap();
        out.push_str(&alloc::format!("{:?} {:?}\n", rt.logs().last(), rt.vm_stats()));
    }
    panic!("{out}");
}

#[test]
fn bigint() {
    check(
        "const a = 2n ** 64n, b = BigInt('0xff'), c = -7n; let d = 5n; d++;
         console.log(typeof a, String(a), b.toString(16), c / 2n, c % 2n, a > 1e19, 10n == 10, 10n === 10, d, -c, ~1n, 1n << 70n, BigInt.asIntN(8, 255n), BigInt.asUintN(64, -1n), [3n, 1n, 2n].sort().join())",
        &["bigint 18446744073709551616 ff -3n -1n true true false 6n 7n -2n 1180591620717411303424n -1n 18446744073709551615n 1,2,3"],
    );
    check("try { 1n + 1 } catch (e) { console.log(e instanceof TypeError) } try { JSON.stringify({a: 1n}) } catch (e) { console.log(e.message) }", &["true", "Do not know how to serialize a BigInt"]);
    check_vm("function f(){ let x = 0n; for (let i = 0n; i < 10n; i++) x += i * i; return x } console.log(f(), typeof f())", &["285n bigint"]);
}

#[test]
fn iframe_context() {
    check(
        "const f = document.createElement('iframe'); console.log(f.contentWindow); document.body.appendChild(f); const w = f.contentWindow; const d = f.contentDocument;
         d.open(); d.write('<p id=x>hi</p>'); d.close();
         console.log(typeof w.Array, w.parent === window, d.getElementById('x').textContent, document.getElementById('x'), w.document === d)",
        &["null", "function true hi null true"],
    );
}

#[test]
fn intl_callable_without_new() {
    check(
        "const f = Intl.DateTimeFormat(); const n = new Intl.NumberFormat('en'); console.log(typeof f.format, f instanceof Intl.DateTimeFormat, n instanceof Intl.NumberFormat, Intl.Collator().compare('a', 'b'), new Intl.Locale('en-US').region, typeof performance.clearMarks)",
        &["function true true -1 US function"],
    );
}

#[test]
fn bogus_comments_and_live_tree_walker() {
    check(
        "const t = document.createElement('template'); t.innerHTML = '<p>a<?m$1$>b<!x>c</p><i></i>';
         const w = document.createTreeWalker(document, 129); const frag = document.importNode(t.content, true); w.currentNode = frag;
         const seen = []; let n; while ((n = w.nextNode())) seen.push(n.nodeType === 8 ? 'c:' + n.data : n.tagName);
         const it = document.createNodeIterator(frag, NodeFilter.SHOW_TEXT); const txt = []; while ((n = it.nextNode())) txt.push(n.data);
         console.log(seen.join(','), txt.join(''))",
        &["P,c:?m$1$,c:x,I abc"],
    );
}

#[test]
fn custom_elements_upgrade_in_shadow_and_when_defined() {
    check(
        "customElements.whenDefined('x-inner').then(c => console.log('defined', c.name));
         class Outer extends HTMLElement { connectedCallback() { const r = this.attachShadow({mode: 'open'}); const t = document.createElement('template'); t.innerHTML = '<x-inner a=1></x-inner>'; r.appendChild(document.importNode(t.content, true)); } }
         customElements.define('x-outer', Outer); document.body.appendChild(document.createElement('x-outer'));
         class Inner extends HTMLElement { static observedAttributes = ['a']; attributeChangedCallback(n, o, v) { console.log('attr', n, v) } connectedCallback() { console.log('inner connected', typeof this.hello) } hello() {} }
         customElements.define('x-inner', Inner);
         const later = document.createElement('x-late'); document.body.appendChild(later);
         customElements.define('x-late', class XLate extends HTMLElement { connectedCallback() { console.log('late', this instanceof XLate) } });",
        &["attr a 1", "inner connected function", "late true", "defined Inner"],
    );
}

#[test]
fn detached_dom_is_collected_and_reused() {
    let mut doc = crate::html::parse("<div id=box></div><div id=other>keep</div>");
    let mut rt = Runtime::new();
    let src = "var box = document.getElementById('box'); var n = 0; var kept;
        function step() {
            box.innerHTML = '<p>x</p>'.repeat(1000);
            if (n === 3) kept = box.firstChild;
            if (++n < 80) setTimeout(step, 1);
            else console.log(kept.textContent, kept.parentNode === null, box.children.length, document.getElementById('other').textContent, document.querySelectorAll('p').length);
        }
        step();";
    rt.execute(src, &mut doc).unwrap();
    rt.advance(&mut doc, 1000.0, 10000);
    let logs: Vec<String> = rt.logs();
    assert_eq!(logs, alloc::vec!["log: x true 1000 keep 1000".to_string()]);
    // 80 rounds of 1000 nodes would be 80k without collection
    assert!(doc.nodes.len() < 50_000, "nodes: {}", doc.nodes.len());
}
