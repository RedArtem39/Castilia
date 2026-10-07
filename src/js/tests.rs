use super::*;
use alloc::{string::ToString, vec::Vec};

fn run(src: &str) -> Vec<String> {
    let mut doc = crate::html::parse("<html><body></body></html>");
    let mut rt = Runtime::new();
    if let Err(e) = rt.execute(src, &mut doc) {
        let mut l = rt.logs();
        l.push(alloc::format!("ERR {e}"));
        return l;
    }
    rt.interp.run_timers_until(1e9, 10000);
    rt.logs().into_iter().map(|l| l.strip_prefix("log: ").map(|s| s.to_string()).unwrap_or(l)).collect()
}

fn check(src: &str, expect: &[&str]) {
    let got = run(src);
    assert_eq!(got, expect, "script: {src}");
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
