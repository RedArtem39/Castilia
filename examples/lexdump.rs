use castilia::js::lexer::{Lexer, Tok};
use std::rc::Rc;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src: Rc<str> = Rc::from(std::fs::read_to_string(path).unwrap());
    let mut l = Lexer::new(src.clone());
    let mut last: Vec<String> = Vec::new();
    loop {
        match l.next() {
            Ok(t) => {
                if t.tok == Tok::Eof { println!("ok"); break; }
                last.push(format!("{:?}@{}", t.tok, l.line_of(t.start)));
                if last.len() > 12 { last.remove(0); }
            }
            Err(e) => { println!("ERR {e}\n{}", last.join("\n")); break; }
        }
    }
}
