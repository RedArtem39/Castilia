//! Recursive-descent parser for ES2022 (scripts and modules).
use super::{
    ast::*,
    lexer::{Lexer, Tok, Token},
    value::JsStr,
};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[derive(Default)]
pub struct Interner {
    names: Vec<JsStr>,
    map: BTreeMap<JsStr, u32>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Name {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let rc: JsStr = Rc::from(s);
        let id = self.names.len() as u32;
        self.names.push(rc.clone());
        self.map.insert(rc, id);
        id
    }
    pub fn get(&self, id: Name) -> &JsStr {
        &self.names[id as usize]
    }
}

struct FnCtx {
    vars: Vec<Name>,
    uses_args: bool,
    is_async: bool,
    is_gen: bool,
    strict: bool,
}

struct BlockCtx {
    lets: Vec<(Name, VarKind)>,
    funcs: Vec<Rc<FuncDef>>,
}

pub struct Parser<'i> {
    lex: Lexer,
    tok: Token,
    prev_end: usize,
    interner: &'i mut Interner,
    fns: Vec<FnCtx>,
    blocks: Vec<BlockCtx>,
    src: Rc<str>,
    is_module: bool,
    no_in: bool,
    depth: usize,
}

type PResult<T> = Result<T, String>;

const MAX_DEPTH: usize = 160;

pub fn parse_program(src: &str, interner: &mut Interner, is_module: bool) -> PResult<Program> {
    let src: Rc<str> = Rc::from(src);
    let mut p = Parser {
        lex: Lexer::new(src.clone()),
        tok: Token { tok: Tok::Eof, nl: false, start: 0, end: 0 },
        prev_end: 0,
        interner,
        fns: vec![FnCtx { vars: Vec::new(), uses_args: false, is_async: is_module, is_gen: false, strict: is_module }],
        blocks: vec![BlockCtx { lets: Vec::new(), funcs: Vec::new() }],
        src,
        is_module,
        no_in: false,
        depth: 0,
    };
    p.advance()?;
    let mut stmts = Vec::new();
    p.directives();
    while p.tok.tok != Tok::Eof {
        stmts.push(p.statement()?);
    }
    let b = p.blocks.pop().expect("block");
    let f = p.fns.pop().expect("fn");
    Ok(Program { body: Block { stmts, lets: b.lets, funcs: b.funcs }, var_names: f.vars, is_module })
}

/// Parse a function body or expression text used by `new Function` and attribute handlers.
pub fn parse_function_source(params: &str, body: &str, interner: &mut Interner) -> PResult<Rc<FuncDef>> {
    let src = format!("(function anonymous({params}\n) {{\n{body}\n}})");
    let prog = parse_program(&src, interner, false)?;
    for s in prog.body.stmts {
        if let Stmt::Expr(Expr::Paren(e)) = s {
            if let Expr::Function(f) = *e {
                return Ok(f);
            }
        }
    }
    Err("SyntaxError: invalid function source".to_string())
}

fn is_reserved(s: &str) -> bool {
    matches!(
        s,
        "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger" | "default" | "delete" | "do" | "else" | "export" | "extends"
            | "finally" | "for" | "function" | "if" | "import" | "in" | "instanceof" | "new" | "return" | "super" | "switch" | "this" | "throw"
            | "try" | "typeof" | "var" | "void" | "while" | "with" | "null" | "true" | "false" | "enum"
    )
}

impl<'i> Parser<'i> {
    fn advance(&mut self) -> PResult<()> {
        self.prev_end = self.tok.end;
        self.tok = self.lex.next()?;
        Ok(())
    }

    fn peek(&self) -> PResult<Token> {
        let mut l = self.lex.clone();
        l.next()
    }

    fn err<T>(&self, msg: &str) -> PResult<T> {
        let line = self.lex.line_of(self.tok.start);
        Err(format!("SyntaxError: {msg} (line {line}, near {})", describe(&self.tok.tok)))
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(&self.tok.tok, Tok::Punct(q) if *q == p)
    }

    fn eat_punct(&mut self, p: &str) -> PResult<bool> {
        if self.is_punct(p) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_punct(&mut self, p: &str) -> PResult<()> {
        if self.eat_punct(p)? { Ok(()) } else { self.err(&format!("expected '{p}'")) }
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(&self.tok.tok, Tok::Ident(s) if &**s == k)
    }

    fn eat_kw(&mut self, k: &str) -> PResult<bool> {
        if self.is_kw(k) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_kw(&mut self, k: &str) -> PResult<()> {
        if self.eat_kw(k)? { Ok(()) } else { self.err(&format!("expected '{k}'")) }
    }

    fn name(&mut self, s: &str) -> Name {
        self.interner.intern(s)
    }

    fn semicolon(&mut self) -> PResult<()> {
        if self.eat_punct(";")? || self.is_punct("}") || self.tok.tok == Tok::Eof || self.tok.nl {
            Ok(())
        } else {
            self.err("expected ';'")
        }
    }

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn in_async(&self) -> bool {
        self.fns.last().is_some_and(|f| f.is_async)
    }
    fn in_gen(&self) -> bool {
        self.fns.last().is_some_and(|f| f.is_gen)
    }
    fn strict(&self) -> bool {
        self.fns.last().is_some_and(|f| f.strict)
    }

    fn directives(&mut self) {
        if let Tok::Str(s) = &self.tok.tok {
            if &**s == "use strict" {
                if let Some(f) = self.fns.last_mut() {
                    f.strict = true;
                }
            }
        }
    }

    fn ident_name(&mut self) -> PResult<JsStr> {
        match &self.tok.tok {
            Tok::Ident(s) => {
                let s = s.clone();
                self.advance()?;
                Ok(s)
            }
            _ => self.err("expected identifier"),
        }
    }

    fn binding_ident(&mut self) -> PResult<Name> {
        match &self.tok.tok {
            Tok::Ident(s) if !is_reserved(s) => {
                let s = s.clone();
                self.advance()?;
                Ok(self.name(&s))
            }
            _ => self.err("expected identifier"),
        }
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn declare_let(&mut self, n: Name, k: VarKind) {
        if let Some(b) = self.blocks.last_mut() {
            if !b.lets.iter().any(|(m, _)| *m == n) {
                b.lets.push((n, k));
            }
        }
    }

    fn declare_var(&mut self, n: Name) {
        if let Some(f) = self.fns.last_mut() {
            if !f.vars.contains(&n) {
                f.vars.push(n);
            }
        }
    }

    fn declare_pat(&mut self, pat: &Pat, kind: VarKind) {
        let mut names = Vec::new();
        pat_names(pat, &mut names);
        for n in names {
            match kind {
                VarKind::Var => self.declare_var(n),
                k => self.declare_let(n, k),
            }
        }
    }

    fn statement(&mut self) -> PResult<Stmt> {
        self.enter()?;
        let r = self.statement_inner();
        self.leave();
        r
    }

    fn block_body(&mut self) -> PResult<Block> {
        self.expect_punct("{")?;
        self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
        let mut stmts = Vec::new();
        while !self.is_punct("}") {
            if self.tok.tok == Tok::Eof {
                return self.err("unexpected end of input");
            }
            stmts.push(self.statement()?);
        }
        self.advance()?;
        let b = self.blocks.pop().expect("block");
        Ok(Block { stmts, lets: b.lets, funcs: b.funcs })
    }

    fn statement_inner(&mut self) -> PResult<Stmt> {
        let tok = self.tok.tok.clone();
        match &tok {
            Tok::Punct("{") => Ok(Stmt::Block(self.block_body()?)),
            Tok::Punct(";") => {
                self.advance()?;
                Ok(Stmt::Empty)
            }
            Tok::Ident(kw) => match &**kw {
                "var" | "const" => {
                    let kind = if &**kw == "var" { VarKind::Var } else { VarKind::Const };
                    self.advance()?;
                    let decls = self.var_decls(kind)?;
                    self.semicolon()?;
                    Ok(Stmt::Var(kind, decls))
                }
                "let" => {
                    let next = self.peek()?;
                    let is_decl = match &next.tok {
                        Tok::Ident(n) => !matches!(&**n, "in" | "of" | "instanceof") || true,
                        Tok::Punct("[") | Tok::Punct("{") => true,
                        _ => false,
                    };
                    if is_decl {
                        self.advance()?;
                        let decls = self.var_decls(VarKind::Let)?;
                        self.semicolon()?;
                        Ok(Stmt::Var(VarKind::Let, decls))
                    } else {
                        self.expression_statement()
                    }
                }
                "function" => {
                    let f = self.function_decl(false)?;
                    Ok(Stmt::Func(f))
                }
                "async" => {
                    let next = self.peek()?;
                    if matches!(&next.tok, Tok::Ident(f) if &**f == "function") && !next.nl {
                        self.advance()?;
                        let f = self.function_decl(true)?;
                        Ok(Stmt::Func(f))
                    } else {
                        self.expression_statement()
                    }
                }
                "class" => {
                    let c = self.class(true)?;
                    if let Some(n) = c.name {
                        self.declare_let(n, VarKind::Let);
                    }
                    Ok(Stmt::Class(c))
                }
                "if" => {
                    self.advance()?;
                    self.expect_punct("(")?;
                    let test = self.expression()?;
                    self.expect_punct(")")?;
                    let cons = self.sub_statement()?;
                    let alt = if self.eat_kw("else")? { Some(Box::new(self.sub_statement()?)) } else { None };
                    Ok(Stmt::If(test, Box::new(cons), alt))
                }
                "for" => self.for_statement(),
                "while" => {
                    self.advance()?;
                    self.expect_punct("(")?;
                    let test = self.expression()?;
                    self.expect_punct(")")?;
                    let body = self.sub_statement()?;
                    Ok(Stmt::While(test, Box::new(body)))
                }
                "do" => {
                    self.advance()?;
                    let body = self.sub_statement()?;
                    self.expect_kw("while")?;
                    self.expect_punct("(")?;
                    let test = self.expression()?;
                    self.expect_punct(")")?;
                    self.eat_punct(";")?;
                    Ok(Stmt::DoWhile(Box::new(body), test))
                }
                "return" => {
                    self.advance()?;
                    let v = if self.is_punct(";") || self.is_punct("}") || self.tok.tok == Tok::Eof || self.tok.nl { None } else { Some(self.expression()?) };
                    self.semicolon()?;
                    Ok(Stmt::Return(v))
                }
                "break" | "continue" => {
                    let is_break = &**kw == "break";
                    self.advance()?;
                    let label = match &self.tok.tok {
                        Tok::Ident(l) if !self.tok.nl && !is_reserved(l) => {
                            let l = l.clone();
                            self.advance()?;
                            Some(self.name(&l))
                        }
                        _ => None,
                    };
                    self.semicolon()?;
                    Ok(if is_break { Stmt::Break(label) } else { Stmt::Continue(label) })
                }
                "throw" => {
                    self.advance()?;
                    let e = self.expression()?;
                    self.semicolon()?;
                    Ok(Stmt::Throw(e))
                }
                "try" => self.try_statement(),
                "switch" => self.switch_statement(),
                "debugger" => {
                    self.advance()?;
                    self.semicolon()?;
                    Ok(Stmt::Empty)
                }
                "with" => self.err("'with' is not supported"),
                "import" => {
                    let next = self.peek()?;
                    if matches!(next.tok, Tok::Punct("(") | Tok::Punct(".")) {
                        self.expression_statement()
                    } else {
                        self.import_decl()
                    }
                }
                "export" => self.export_decl(),
                _ => {
                    // labeled statement?
                    if !is_reserved(kw) {
                        let next = self.peek()?;
                        if matches!(next.tok, Tok::Punct(":")) {
                            let label = self.name(kw);
                            self.advance()?;
                            self.advance()?;
                            let body = self.sub_statement()?;
                            return Ok(Stmt::Labeled(label, Box::new(body)));
                        }
                    }
                    self.expression_statement()
                }
            },
            _ => self.expression_statement(),
        }
    }

    /// Statement in a position where declarations create their own implicit block.
    fn sub_statement(&mut self) -> PResult<Stmt> {
        if self.is_kw("function") || self.is_kw("class") || self.is_kw("const") || (self.is_kw("let") && matches!(self.peek()?.tok, Tok::Ident(_) | Tok::Punct("[") | Tok::Punct("{"))) {
            self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
            let s = self.statement()?;
            let b = self.blocks.pop().expect("block");
            return Ok(Stmt::Block(Block { stmts: vec![s], lets: b.lets, funcs: b.funcs }));
        }
        self.statement()
    }

    fn expression_statement(&mut self) -> PResult<Stmt> {
        let e = self.expression()?;
        self.semicolon()?;
        Ok(Stmt::Expr(e))
    }

    fn var_decls(&mut self, kind: VarKind) -> PResult<Vec<(Pat, Option<Expr>)>> {
        let mut out = Vec::new();
        loop {
            let pat = self.binding_pattern()?;
            self.declare_pat(&pat, kind);
            let init = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
            out.push((pat, init));
            if !self.eat_punct(",")? {
                break;
            }
        }
        Ok(out)
    }

    fn for_statement(&mut self) -> PResult<Stmt> {
        self.advance()?;
        let is_await = self.eat_kw("await")?;
        self.expect_punct("(")?;
        // loop-scoped lexical declarations
        self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
        let result = self.for_rest(is_await);
        let b = self.blocks.pop().expect("block");
        let mut stmt = result?;
        if let Stmt::For { lets, .. } = &mut stmt {
            *lets = b.lets.iter().map(|(n, _)| *n).collect();
        }
        match stmt {
            Stmt::For { .. } => Ok(stmt),
            other => {
                if b.lets.is_empty() {
                    Ok(other)
                } else {
                    // for-in/of heads declare per-iteration bindings handled by the interpreter
                    Ok(other)
                }
            }
        }
    }

    fn for_rest(&mut self, is_await: bool) -> PResult<Stmt> {
        let mut init: Option<ForInit> = None;
        if self.is_punct(";") {
            // no init
        } else if self.is_kw("var") || self.is_kw("const") || (self.is_kw("let") && matches!(self.peek()?.tok, Tok::Ident(_) | Tok::Punct("[") | Tok::Punct("{"))) {
            let kind = match &self.tok.tok {
                Tok::Ident(s) if &**s == "var" => VarKind::Var,
                Tok::Ident(s) if &**s == "const" => VarKind::Const,
                _ => VarKind::Let,
            };
            self.advance()?;
            let save = self.no_in;
            self.no_in = true;
            let pat = self.binding_pattern()?;
            self.declare_pat(&pat, kind);
            if self.is_kw("of") || self.is_kw("in") {
                self.no_in = save;
                let is_of = self.is_kw("of");
                self.advance()?;
                let rhs = if is_of { self.assignment()? } else { self.expression()? };
                self.expect_punct(")")?;
                let body = Box::new(self.sub_statement()?);
                let head = ForHead::Var(kind, pat);
                return Ok(if is_of { Stmt::ForOf { head, iter: rhs, body, is_await } } else { Stmt::ForIn { head, obj: rhs, body } });
            }
            let mut decls = Vec::new();
            let first_init = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
            decls.push((pat, first_init));
            while self.eat_punct(",")? {
                let p = self.binding_pattern()?;
                self.declare_pat(&p, kind);
                let i = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
                decls.push((p, i));
            }
            self.no_in = save;
            init = Some(ForInit::Var(kind, decls));
        } else {
            let save = self.no_in;
            self.no_in = true;
            let e = self.expression()?;
            self.no_in = save;
            if self.is_kw("of") || self.is_kw("in") {
                let is_of = self.is_kw("of");
                self.advance()?;
                let pat = self.expr_to_pat(e)?;
                let rhs = if is_of { self.assignment()? } else { self.expression()? };
                self.expect_punct(")")?;
                let body = Box::new(self.sub_statement()?);
                let head = ForHead::Pat(pat);
                return Ok(if is_of { Stmt::ForOf { head, iter: rhs, body, is_await } } else { Stmt::ForIn { head, obj: rhs, body } });
            }
            init = Some(ForInit::Expr(e));
        }
        self.expect_punct(";")?;
        let test = if self.is_punct(";") { None } else { Some(self.expression()?) };
        self.expect_punct(";")?;
        let update = if self.is_punct(")") { None } else { Some(self.expression()?) };
        self.expect_punct(")")?;
        let body = Box::new(self.sub_statement()?);
        Ok(Stmt::For { init, test, update, body, lets: Vec::new() })
    }

    fn try_statement(&mut self) -> PResult<Stmt> {
        self.advance()?;
        let block = self.block_body()?;
        let mut param = None;
        let mut handler = None;
        if self.eat_kw("catch")? {
            self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
            if self.eat_punct("(")? {
                let p = self.binding_pattern()?;
                self.declare_pat(&p, VarKind::Let);
                param = Some(p);
                self.expect_punct(")")?;
            }
            let hb = self.block_body();
            self.blocks.pop().expect("block");
            handler = Some(hb?);
        }
        let finalizer = if self.eat_kw("finally")? { Some(self.block_body()?) } else { None };
        if handler.is_none() && finalizer.is_none() {
            return self.err("missing catch or finally");
        }
        Ok(Stmt::Try { block, param, handler, finalizer })
    }

    fn switch_statement(&mut self) -> PResult<Stmt> {
        self.advance()?;
        self.expect_punct("(")?;
        let disc = self.expression()?;
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
        let mut cases = Vec::new();
        while !self.is_punct("}") {
            let test = if self.eat_kw("default")? {
                None
            } else {
                self.expect_kw("case")?;
                Some(self.expression()?)
            };
            self.expect_punct(":")?;
            let mut body = Vec::new();
            while !self.is_punct("}") && !self.is_kw("case") && !self.is_kw("default") {
                if self.tok.tok == Tok::Eof {
                    return self.err("unexpected end of input");
                }
                body.push(self.statement()?);
            }
            cases.push(SwitchCase { test, body });
        }
        self.advance()?;
        let b = self.blocks.pop().expect("block");
        Ok(Stmt::Switch { disc, cases, lets: b.lets, funcs: b.funcs })
    }

    fn import_decl(&mut self) -> PResult<Stmt> {
        self.advance()?;
        if let Tok::Str(s) = &self.tok.tok {
            let source = s.clone();
            self.advance()?;
            self.semicolon()?;
            return Ok(Stmt::Import { source, specs: Vec::new() });
        }
        let mut specs = Vec::new();
        if let Tok::Ident(_) = &self.tok.tok {
            if !self.is_kw("from") || matches!(self.peek()?.tok, Tok::Ident(ref f) if &**f == "from") {
                let n = self.binding_ident()?;
                self.declare_let(n, VarKind::Const);
                specs.push(ImportSpec::Default(n));
                self.eat_punct(",")?;
            }
        }
        if self.eat_punct("*")? {
            self.expect_kw("as")?;
            let n = self.binding_ident()?;
            self.declare_let(n, VarKind::Const);
            specs.push(ImportSpec::Namespace(n));
        } else if self.eat_punct("{")? {
            while !self.is_punct("}") {
                let imported: JsStr = match &self.tok.tok {
                    Tok::Str(s) => {
                        let s = s.clone();
                        self.advance()?;
                        s
                    }
                    _ => self.ident_name()?,
                };
                let local = if self.eat_kw("as")? { self.binding_ident()? } else { self.name(&imported) };
                self.declare_let(local, VarKind::Const);
                specs.push(ImportSpec::Named(imported, local));
                if !self.eat_punct(",")? {
                    break;
                }
            }
            self.expect_punct("}")?;
        }
        self.expect_kw("from")?;
        let source = match &self.tok.tok {
            Tok::Str(s) => s.clone(),
            _ => return self.err("expected module specifier"),
        };
        self.advance()?;
        // import attributes: with { type: 'json' }
        if self.is_kw("with") || self.is_kw("assert") {
            self.advance()?;
            self.skip_braces()?;
        }
        self.semicolon()?;
        Ok(Stmt::Import { source, specs })
    }

    fn skip_braces(&mut self) -> PResult<()> {
        self.expect_punct("{")?;
        let mut depth = 1;
        while depth > 0 {
            if self.tok.tok == Tok::Eof {
                return self.err("unterminated block");
            }
            if self.is_punct("{") {
                depth += 1;
            } else if self.is_punct("}") {
                depth -= 1;
            }
            self.advance()?;
        }
        Ok(())
    }

    fn export_decl(&mut self) -> PResult<Stmt> {
        self.advance()?;
        if self.eat_kw("default")? {
            if self.is_kw("function") || (self.is_kw("async") && matches!(self.peek()?.tok, Tok::Ident(ref f) if &**f == "function")) {
                let is_async = self.eat_kw("async")?;
                // function declaration or anonymous function expression
                let start = self.tok.start;
                self.advance()?;
                let is_gen = self.eat_punct("*")?;
                let name = if let Tok::Ident(_) = &self.tok.tok { Some(self.binding_ident()?) } else { None };
                let f = self.function_rest(name, is_async, is_gen, FnKind::Normal, start)?;
                if let Some(n) = name {
                    self.declare_let(n, VarKind::Let);
                    let df = self.name("default");
                    return Ok(Stmt::ExportDecl(Box::new(Stmt::Func(f.clone())), vec![(n, Rc::from("default"))]).with_alias(df));
                }
                return Ok(Stmt::ExportDefault(Expr::Function(f)));
            }
            if self.is_kw("class") {
                let c = self.class(false)?;
                if let Some(n) = c.name {
                    self.declare_let(n, VarKind::Let);
                    return Ok(Stmt::ExportDecl(Box::new(Stmt::Class(c)), vec![(n, Rc::from("default"))]));
                }
                return Ok(Stmt::ExportDefault(Expr::Class(c)));
            }
            let e = self.assignment()?;
            self.semicolon()?;
            return Ok(Stmt::ExportDefault(e));
        }
        if self.eat_punct("*")? {
            let alias = if self.eat_kw("as")? { Some(self.ident_name()?) } else { None };
            self.expect_kw("from")?;
            let source = match &self.tok.tok {
                Tok::Str(s) => s.clone(),
                _ => return self.err("expected module specifier"),
            };
            self.advance()?;
            self.semicolon()?;
            return Ok(Stmt::ExportAll(source, alias));
        }
        if self.eat_punct("{")? {
            let mut specs = Vec::new();
            while !self.is_punct("}") {
                let local = match &self.tok.tok {
                    Tok::Str(s) => {
                        let s = s.clone();
                        self.advance()?;
                        s
                    }
                    _ => self.ident_name()?,
                };
                let exported = if self.eat_kw("as")? {
                    match &self.tok.tok {
                        Tok::Str(s) => {
                            let s = s.clone();
                            self.advance()?;
                            s
                        }
                        _ => self.ident_name()?,
                    }
                } else {
                    local.clone()
                };
                let n = self.name(&local);
                specs.push((n, exported));
                if !self.eat_punct(",")? {
                    break;
                }
            }
            self.expect_punct("}")?;
            let from = if self.eat_kw("from")? {
                match &self.tok.tok {
                    Tok::Str(s) => {
                        let s = s.clone();
                        self.advance()?;
                        Some(s)
                    }
                    _ => return self.err("expected module specifier"),
                }
            } else {
                None
            };
            self.semicolon()?;
            return Ok(Stmt::ExportNamed { specs, from });
        }
        // export <declaration>
        let decl = self.statement()?;
        let mut names = Vec::new();
        match &decl {
            Stmt::Var(_, decls) => {
                for (p, _) in decls {
                    pat_names(p, &mut names);
                }
            }
            Stmt::Func(f) => {
                if let Some(n) = f.name {
                    names.push(n);
                }
            }
            Stmt::Class(c) => {
                if let Some(n) = c.name {
                    names.push(n);
                }
            }
            _ => {}
        }
        let list = names.into_iter().map(|n| (n, self.interner.get(n).clone())).collect();
        Ok(Stmt::ExportDecl(Box::new(decl), list))
    }

    // -----------------------------------------------------------------------
    // Functions and classes
    // -----------------------------------------------------------------------

    fn function_decl(&mut self, is_async: bool) -> PResult<Rc<FuncDef>> {
        let start = if is_async { self.prev_end.saturating_sub(5) } else { self.tok.start };
        let start = if is_async { self.src[..self.prev_end].rfind("async").unwrap_or(start) } else { start };
        self.expect_kw("function")?;
        let is_gen = self.eat_punct("*")?;
        let name = self.binding_ident()?;
        let f = self.function_rest(Some(name), is_async, is_gen, FnKind::Normal, start)?;
        if let Some(b) = self.blocks.last_mut() {
            b.funcs.push(f.clone());
        }
        Ok(f)
    }

    fn function_expr(&mut self, is_async: bool) -> PResult<Rc<FuncDef>> {
        let start = if is_async { self.src[..self.prev_end].rfind("async").unwrap_or(self.tok.start) } else { self.tok.start };
        self.expect_kw("function")?;
        let is_gen = self.eat_punct("*")?;
        let name = if let Tok::Ident(n) = &self.tok.tok { if is_reserved(n) { None } else { Some(self.binding_ident()?) } } else { None };
        self.function_rest(name, is_async, is_gen, FnKind::Normal, start)
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect_punct("(")?;
        let mut params = Vec::new();
        while !self.is_punct(")") {
            if self.eat_punct("...")? {
                let pat = self.binding_pattern()?;
                params.push(Param { pat, default: None, rest: true });
                self.eat_punct(",")?;
                break;
            }
            let pat = self.binding_pattern()?;
            let default = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
            params.push(Param { pat, default, rest: false });
            if !self.eat_punct(",")? {
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(params)
    }

    fn function_rest(&mut self, name: Option<Name>, is_async: bool, is_gen: bool, kind: FnKind, start: usize) -> PResult<Rc<FuncDef>> {
        self.enter()?;
        let strict = self.strict();
        self.fns.push(FnCtx { vars: Vec::new(), uses_args: false, is_async, is_gen, strict });
        self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
        let save_no_in = core::mem::replace(&mut self.no_in, false);
        let result = (|| -> PResult<(Vec<Param>, Vec<Stmt>)> {
            let params = self.params()?;
            self.expect_punct("{")?;
            self.directives();
            let mut stmts = Vec::new();
            while !self.is_punct("}") {
                if self.tok.tok == Tok::Eof {
                    return self.err("unexpected end of input");
                }
                stmts.push(self.statement()?);
            }
            self.advance()?;
            Ok((params, stmts))
        })();
        self.no_in = save_no_in;
        let b = self.blocks.pop().expect("block");
        let f = self.fns.pop().expect("fn");
        self.leave();
        let (params, stmts) = result?;
        let end = self.prev_end;
        let simple = params.iter().all(|p| matches!(p.pat, Pat::Ident(_)) && p.default.is_none() && !p.rest);
        let length = params.iter().take_while(|p| p.default.is_none() && !p.rest).count();
        Ok(Rc::new(FuncDef {
            name,
            params,
            body: FuncBody::Block(Block { stmts, lets: b.lets, funcs: b.funcs }),
            is_arrow: false,
            is_async,
            is_generator: is_gen,
            kind,
            var_names: f.vars,
            uses_args: f.uses_args,
            simple_params: simple,
            strict: f.strict,
            source: self.src.clone(),
            span: (start, end),
            length,
        }))
    }

    fn arrow_function(&mut self, params: Vec<Param>, is_async: bool, start: usize) -> PResult<Rc<FuncDef>> {
        self.enter()?;
        let strict = self.strict();
        // arrows see the enclosing function's `arguments`
        self.fns.push(FnCtx { vars: Vec::new(), uses_args: false, is_async, is_gen: false, strict });
        self.blocks.push(BlockCtx { lets: Vec::new(), funcs: Vec::new() });
        let save_no_in = self.no_in;
        let result = (|| -> PResult<FuncBody> {
            if self.is_punct("{") {
                self.no_in = false;
                self.advance()?;
                self.directives();
                let mut stmts = Vec::new();
                while !self.is_punct("}") {
                    if self.tok.tok == Tok::Eof {
                        return self.err("unexpected end of input");
                    }
                    stmts.push(self.statement()?);
                }
                self.advance()?;
                Ok(FuncBody::Block(Block { stmts, lets: Vec::new(), funcs: Vec::new() }))
            } else {
                let e = self.assignment()?;
                Ok(FuncBody::Expr(Box::new(e)))
            }
        })();
        self.no_in = save_no_in;
        let b = self.blocks.pop().expect("block");
        let f = self.fns.pop().expect("fn");
        self.leave();
        let mut body = result?;
        if let FuncBody::Block(blk) = &mut body {
            blk.lets = b.lets;
            blk.funcs = b.funcs;
        }
        if f.uses_args {
            if let Some(parent) = self.fns.last_mut() {
                parent.uses_args = true;
            }
        }
        let simple = params.iter().all(|p| matches!(p.pat, Pat::Ident(_)) && p.default.is_none() && !p.rest);
        let length = params.iter().take_while(|p| p.default.is_none() && !p.rest).count();
        Ok(Rc::new(FuncDef {
            name: None,
            params,
            body,
            is_arrow: true,
            is_async,
            is_generator: false,
            kind: FnKind::Normal,
            var_names: f.vars,
            uses_args: false,
            simple_params: simple,
            strict: f.strict,
            source: self.src.clone(),
            span: (start, self.prev_end),
            length,
        }))
    }

    fn method(&mut self, kind: FnKind, is_async: bool, is_gen: bool, start: usize) -> PResult<Rc<FuncDef>> {
        self.function_rest(None, is_async, is_gen, kind, start)
    }

    fn class(&mut self, require_name: bool) -> PResult<Rc<ClassDef>> {
        let start = self.tok.start;
        self.expect_kw("class")?;
        let name = match &self.tok.tok {
            Tok::Ident(n) if &**n != "extends" && !is_reserved(n) => Some(self.binding_ident()?),
            _ => {
                if require_name {
                    return self.err("class name expected");
                }
                None
            }
        };
        let superclass = if self.eat_kw("extends")? { Some(self.lhs_expression()?) } else { None };
        self.expect_punct("{")?;
        // class bodies are strict
        let prev_strict = self.fns.last().map(|f| f.strict);
        if let Some(f) = self.fns.last_mut() {
            f.strict = true;
        }
        let mut ctor = None;
        let mut members = Vec::new();
        while !self.is_punct("}") {
            if self.eat_punct(";")? {
                continue;
            }
            if self.tok.tok == Tok::Eof {
                return self.err("unexpected end of class");
            }
            let mstart = self.tok.start;
            let mut is_static = false;
            if self.is_kw("static") {
                let next = self.peek()?;
                if !matches!(next.tok, Tok::Punct("(") | Tok::Punct("=") | Tok::Punct(";") | Tok::Punct("}")) {
                    self.advance()?;
                    is_static = true;
                    if self.is_punct("{") {
                        // static initialization block
                        self.fns.push(FnCtx { vars: Vec::new(), uses_args: false, is_async: false, is_gen: false, strict: true });
                        let blk = self.block_body();
                        self.fns.pop();
                        members.push(ClassMember { key: PropKey::Name(Rc::from("")), is_static: true, kind: ClassMemberKind::StaticBlock(blk?) });
                        continue;
                    }
                }
            }
            let mut is_async = false;
            let mut is_gen = false;
            let mut accessor: Option<bool> = None; // Some(true)=get
            if self.is_kw("async") {
                let next = self.peek()?;
                if !matches!(next.tok, Tok::Punct("(") | Tok::Punct("=") | Tok::Punct(";") | Tok::Punct("}")) && !next.nl {
                    self.advance()?;
                    is_async = true;
                }
            }
            if self.eat_punct("*")? {
                is_gen = true;
            }
            if (self.is_kw("get") || self.is_kw("set")) && !is_async && !is_gen {
                let next = self.peek()?;
                if !matches!(next.tok, Tok::Punct("(") | Tok::Punct("=") | Tok::Punct(";") | Tok::Punct("}")) {
                    accessor = Some(self.is_kw("get"));
                    self.advance()?;
                }
            }
            let key = self.prop_key(true)?;
            if self.is_punct("(") {
                let is_ctor = !is_static && matches!(&key, PropKey::Name(n) if &**n == "constructor");
                let kind = if is_ctor {
                    if superclass.is_some() { FnKind::DerivedCtor } else { FnKind::Ctor }
                } else {
                    match accessor {
                        Some(true) => FnKind::Getter,
                        Some(false) => FnKind::Setter,
                        None => FnKind::Method,
                    }
                };
                let f = self.method(kind, is_async, is_gen, mstart)?;
                if is_ctor {
                    ctor = Some(f);
                } else {
                    let k = match accessor {
                        Some(true) => ClassMemberKind::Getter(f),
                        Some(false) => ClassMemberKind::Setter(f),
                        None => ClassMemberKind::Method(f),
                    };
                    members.push(ClassMember { key, is_static, kind: k });
                }
            } else {
                // field
                let init = if self.eat_punct("=")? {
                    self.fns.push(FnCtx { vars: Vec::new(), uses_args: false, is_async: false, is_gen: false, strict: true });
                    let e = self.assignment();
                    self.fns.pop();
                    Some(e?)
                } else {
                    None
                };
                self.semicolon()?;
                members.push(ClassMember { key, is_static, kind: ClassMemberKind::Field(init) });
            }
        }
        self.advance()?;
        if let (Some(f), Some(s)) = (self.fns.last_mut(), prev_strict) {
            f.strict = s;
        }
        Ok(Rc::new(ClassDef { name, superclass, ctor, members, source: self.src.clone(), span: (start, self.prev_end) }))
    }

    fn prop_key(&mut self, allow_private: bool) -> PResult<PropKey> {
        match self.tok.tok.clone() {
            Tok::Ident(s) => {
                self.advance()?;
                Ok(PropKey::Name(s))
            }
            Tok::Str(s) => {
                self.advance()?;
                Ok(PropKey::Name(s))
            }
            Tok::Num(n) => {
                self.advance()?;
                Ok(PropKey::Name(Rc::from(super::value::num_to_string(n))))
            }
            Tok::Private(s) if allow_private => {
                self.advance()?;
                Ok(PropKey::Private(s))
            }
            Tok::Punct("[") => {
                self.advance()?;
                let save = core::mem::replace(&mut self.no_in, false);
                let e = self.assignment();
                self.no_in = save;
                let e = e?;
                self.expect_punct("]")?;
                Ok(PropKey::Computed(e))
            }
            _ => self.err("expected property name"),
        }
    }

    // -----------------------------------------------------------------------
    // Patterns
    // -----------------------------------------------------------------------

    fn binding_pattern(&mut self) -> PResult<Pat> {
        self.enter()?;
        let r = self.binding_pattern_inner();
        self.leave();
        r
    }

    fn binding_pattern_inner(&mut self) -> PResult<Pat> {
        if self.eat_punct("[")? {
            let mut elems = Vec::new();
            let mut rest = None;
            while !self.is_punct("]") {
                if self.eat_punct(",")? {
                    elems.push(None);
                    continue;
                }
                if self.eat_punct("...")? {
                    rest = Some(Box::new(self.binding_pattern()?));
                    break;
                }
                let pat = self.binding_pattern()?;
                let default = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
                elems.push(Some(PatElem { pat, default }));
                if !self.eat_punct(",")? {
                    break;
                }
            }
            self.expect_punct("]")?;
            return Ok(Pat::Array(elems, rest));
        }
        if self.eat_punct("{")? {
            let mut props = Vec::new();
            let mut rest = None;
            while !self.is_punct("}") {
                if self.eat_punct("...")? {
                    rest = Some(Box::new(self.binding_pattern()?));
                    break;
                }
                let is_ident_key = matches!(&self.tok.tok, Tok::Ident(_));
                let key_name = if let Tok::Ident(s) = &self.tok.tok { Some(s.clone()) } else { None };
                let key = self.prop_key(false)?;
                let (pat, default);
                if self.eat_punct(":")? {
                    pat = self.binding_pattern()?;
                    default = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
                } else if is_ident_key {
                    let n = self.name(&key_name.unwrap_or_else(|| Rc::from("")));
                    pat = Pat::Ident(n);
                    default = if self.eat_punct("=")? { Some(self.assignment()?) } else { None };
                } else {
                    return self.err("expected ':' in object pattern");
                }
                props.push(PatProp { key, pat, default });
                if !self.eat_punct(",")? {
                    break;
                }
            }
            self.expect_punct("}")?;
            return Ok(Pat::Object(props, rest));
        }
        Ok(Pat::Ident(self.binding_ident()?))
    }

    fn expr_to_pat(&mut self, e: Expr) -> PResult<Pat> {
        match e {
            Expr::Ident(n) => Ok(Pat::Ident(n)),
            Expr::Member { .. } | Expr::Dot { .. } | Expr::PrivateMember { .. } | Expr::SuperDot(_) | Expr::SuperMember(_) => Ok(Pat::Expr(Box::new(e))),
            Expr::Paren(inner) => match *inner {
                e @ (Expr::Ident(_) | Expr::Member { .. } | Expr::Dot { .. }) => self.expr_to_pat(e),
                _ => self.err("invalid assignment target"),
            },
            Expr::Array(elems) => {
                let mut out = Vec::new();
                let mut rest = None;
                for el in elems {
                    match el {
                        None => out.push(None),
                        Some(ArgElem::Spread(e)) => rest = Some(Box::new(self.expr_to_pat(e)?)),
                        Some(ArgElem::Expr(Expr::Assign(AssignOp::Assign, target, def))) => out.push(Some(PatElem { pat: *target, default: Some(*def) })),
                        Some(ArgElem::Expr(e)) => out.push(Some(PatElem { pat: self.expr_to_pat(e)?, default: None })),
                    }
                }
                Ok(Pat::Array(out, rest))
            }
            Expr::Object(props) => {
                let mut out = Vec::new();
                let mut rest = None;
                for p in props {
                    match p.value {
                        PropValue::Spread(e) => rest = Some(Box::new(self.expr_to_pat(e)?)),
                        PropValue::ShorthandDefault(n, d) => out.push(PatProp { key: p.key, pat: Pat::Ident(n), default: Some(d) }),
                        PropValue::Value(Expr::Assign(AssignOp::Assign, target, def)) => out.push(PatProp { key: p.key, pat: *target, default: Some(*def) }),
                        PropValue::Value(e) => out.push(PatProp { key: p.key, pat: self.expr_to_pat(e)?, default: None }),
                        _ => return self.err("invalid destructuring target"),
                    }
                }
                Ok(Pat::Object(out, rest))
            }
            Expr::Assign(AssignOp::Assign, target, _) => Ok(*target),
            _ => self.err("invalid assignment target"),
        }
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    pub fn expression(&mut self) -> PResult<Expr> {
        let first = self.assignment()?;
        if self.is_punct(",") {
            let mut list = vec![first];
            while self.eat_punct(",")? {
                list.push(self.assignment()?);
            }
            return Ok(Expr::Seq(list));
        }
        Ok(first)
    }

    fn arrow_ahead(&self) -> PResult<bool> {
        // current token is '(' — scan to the matching ')' and look for '=>'
        let mut l = self.lex.clone();
        let mut depth = 1;
        loop {
            let t = l.next()?;
            match t.tok {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        let n = l.next()?;
                        return Ok(matches!(n.tok, Tok::Punct("=>")) && !n.nl);
                    }
                }
                Tok::Eof => return Ok(false),
                _ => {}
            }
        }
    }

    fn assignment(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.assignment_inner();
        self.leave();
        r
    }

    fn assignment_inner(&mut self) -> PResult<Expr> {
        let start = self.tok.start;
        // yield
        if self.in_gen() && self.is_kw("yield") {
            self.advance()?;
            if self.tok.nl || matches!(self.tok.tok, Tok::Punct(")") | Tok::Punct("]") | Tok::Punct("}") | Tok::Punct(",") | Tok::Punct(";") | Tok::Punct(":") | Tok::Eof) {
                return Ok(Expr::Yield(None, false));
            }
            let delegate = self.eat_punct("*")?;
            let arg = self.assignment()?;
            return Ok(Expr::Yield(Some(Box::new(arg)), delegate));
        }
        // arrow functions
        if let Tok::Ident(name) = &self.tok.tok.clone() {
            if &**name == "async" {
                let next = self.peek()?;
                match &next.tok {
                    Tok::Ident(p) if !next.nl && &**p != "function" => {
                        // async x => ...
                        let mut l = self.lex.clone();
                        l.next()?;
                        if matches!(l.next()?.tok, Tok::Punct("=>")) {
                            self.advance()?;
                            let n = self.binding_ident()?;
                            self.expect_punct("=>")?;
                            let params = vec![Param { pat: Pat::Ident(n), default: None, rest: false }];
                            return Ok(Expr::Function(self.arrow_function(params, true, start)?));
                        }
                    }
                    Tok::Punct("(") if !next.nl => {
                        let mut l = self.lex.clone();
                        l.next()?; // '('
                        let mut depth = 1;
                        let mut is_arrow = false;
                        loop {
                            let t = l.next()?;
                            match t.tok {
                                Tok::Punct("(") => depth += 1,
                                Tok::Punct(")") => {
                                    depth -= 1;
                                    if depth == 0 {
                                        is_arrow = matches!(l.next()?.tok, Tok::Punct("=>"));
                                        break;
                                    }
                                }
                                Tok::Eof => break,
                                _ => {}
                            }
                        }
                        if is_arrow {
                            self.advance()?;
                            let params = self.params()?;
                            self.expect_punct("=>")?;
                            return Ok(Expr::Function(self.arrow_function(params, true, start)?));
                        }
                    }
                    _ => {}
                }
            }
            if !is_reserved(name) {
                let next = self.peek()?;
                if matches!(next.tok, Tok::Punct("=>")) && !next.nl {
                    let n = self.binding_ident()?;
                    self.expect_punct("=>")?;
                    let params = vec![Param { pat: Pat::Ident(n), default: None, rest: false }];
                    return Ok(Expr::Function(self.arrow_function(params, false, start)?));
                }
            }
        }
        if self.is_punct("(") && self.arrow_ahead()? {
            let params = self.params()?;
            self.expect_punct("=>")?;
            return Ok(Expr::Function(self.arrow_function(params, false, start)?));
        }
        let lhs = self.conditional()?;
        let op = match &self.tok.tok {
            Tok::Punct(p) => match *p {
                "=" => Some(AssignOp::Assign),
                "+=" => Some(AssignOp::Bin(BinOp::Add)),
                "-=" => Some(AssignOp::Bin(BinOp::Sub)),
                "*=" => Some(AssignOp::Bin(BinOp::Mul)),
                "/=" => Some(AssignOp::Bin(BinOp::Div)),
                "%=" => Some(AssignOp::Bin(BinOp::Mod)),
                "**=" => Some(AssignOp::Bin(BinOp::Exp)),
                "<<=" => Some(AssignOp::Bin(BinOp::Shl)),
                ">>=" => Some(AssignOp::Bin(BinOp::Shr)),
                ">>>=" => Some(AssignOp::Bin(BinOp::UShr)),
                "&=" => Some(AssignOp::Bin(BinOp::BitAnd)),
                "|=" => Some(AssignOp::Bin(BinOp::BitOr)),
                "^=" => Some(AssignOp::Bin(BinOp::BitXor)),
                "&&=" => Some(AssignOp::Logical(LogOp::And)),
                "||=" => Some(AssignOp::Logical(LogOp::Or)),
                "??=" => Some(AssignOp::Logical(LogOp::Nullish)),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.advance()?;
            let target = if op == AssignOp::Assign { self.expr_to_pat(lhs)? } else {
                match lhs {
                    Expr::Ident(n) => Pat::Ident(n),
                    e @ (Expr::Member { .. } | Expr::Dot { .. } | Expr::PrivateMember { .. } | Expr::SuperDot(_) | Expr::SuperMember(_)) => Pat::Expr(Box::new(e)),
                    Expr::Paren(inner) => self.expr_to_pat(*inner)?,
                    _ => return self.err("invalid assignment target"),
                }
            };
            let value = self.assignment()?;
            return Ok(Expr::Assign(op, Box::new(target), Box::new(value)));
        }
        Ok(lhs)
    }

    fn conditional(&mut self) -> PResult<Expr> {
        let test = self.binary(0)?;
        if self.eat_punct("?")? {
            let save = core::mem::replace(&mut self.no_in, false);
            let cons = self.assignment();
            self.no_in = save;
            let cons = cons?;
            self.expect_punct(":")?;
            let alt = self.assignment()?;
            return Ok(Expr::Cond(Box::new(test), Box::new(cons), Box::new(alt)));
        }
        Ok(test)
    }

    fn bin_op(&self) -> Option<(u8, Result<BinOp, LogOp>)> {
        let t = &self.tok.tok;
        match t {
            Tok::Punct(p) => Some(match *p {
                "??" => (1, Err(LogOp::Nullish)),
                "||" => (2, Err(LogOp::Or)),
                "&&" => (3, Err(LogOp::And)),
                "|" => (4, Ok(BinOp::BitOr)),
                "^" => (5, Ok(BinOp::BitXor)),
                "&" => (6, Ok(BinOp::BitAnd)),
                "==" => (7, Ok(BinOp::Eq)),
                "!=" => (7, Ok(BinOp::Ne)),
                "===" => (7, Ok(BinOp::StrictEq)),
                "!==" => (7, Ok(BinOp::StrictNe)),
                "<" => (8, Ok(BinOp::Lt)),
                ">" => (8, Ok(BinOp::Gt)),
                "<=" => (8, Ok(BinOp::Le)),
                ">=" => (8, Ok(BinOp::Ge)),
                "<<" => (9, Ok(BinOp::Shl)),
                ">>" => (9, Ok(BinOp::Shr)),
                ">>>" => (9, Ok(BinOp::UShr)),
                "+" => (10, Ok(BinOp::Add)),
                "-" => (10, Ok(BinOp::Sub)),
                "*" => (11, Ok(BinOp::Mul)),
                "/" => (11, Ok(BinOp::Div)),
                "%" => (11, Ok(BinOp::Mod)),
                "**" => (12, Ok(BinOp::Exp)),
                _ => return None,
            }),
            Tok::Ident(s) => match &**s {
                "instanceof" => Some((8, Ok(BinOp::InstanceOf))),
                "in" if !self.no_in => Some((8, Ok(BinOp::In))),
                _ => None,
            },
            _ => None,
        }
    }

    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut left = if let Tok::Private(name) = &self.tok.tok.clone() {
            // `#x in obj`
            let name = name.clone();
            self.advance()?;
            Expr::Str(Rc::from(format!("#{name}")))
        } else {
            self.unary()?
        };
        while let Some((prec, op)) = self.bin_op() {
            if prec < min || prec == 0 {
                break;
            }
            self.advance()?;
            // `**` is right associative
            let right = if matches!(op, Ok(BinOp::Exp)) { self.binary(prec)? } else { self.binary(prec + 1)? };
            left = match op {
                Ok(b) => Expr::Binary(b, Box::new(left), Box::new(right)),
                Err(l) => Expr::Logical(l, Box::new(left), Box::new(right)),
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }

    fn unary_inner(&mut self) -> PResult<Expr> {
        let op = match &self.tok.tok {
            Tok::Punct("!") => Some(UnOp::Not),
            Tok::Punct("-") => Some(UnOp::Neg),
            Tok::Punct("+") => Some(UnOp::Plus),
            Tok::Punct("~") => Some(UnOp::BitNot),
            Tok::Ident(s) => match &**s {
                "typeof" => Some(UnOp::TypeOf),
                "void" => Some(UnOp::Void),
                "delete" => Some(UnOp::Delete),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.advance()?;
            let e = self.unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        if self.is_punct("++") || self.is_punct("--") {
            let inc = self.is_punct("++");
            self.advance()?;
            let t = self.unary()?;
            return Ok(Expr::Update { inc, prefix: true, target: Box::new(t) });
        }
        if self.is_kw("await") && (self.in_async() || (self.fns.len() == 1 && self.is_module)) {
            self.advance()?;
            let e = self.unary()?;
            return Ok(Expr::Await(Box::new(e)));
        }
        let e = self.lhs_expression()?;
        if (self.is_punct("++") || self.is_punct("--")) && !self.tok.nl {
            let inc = self.is_punct("++");
            self.advance()?;
            return Ok(Expr::Update { inc, prefix: false, target: Box::new(e) });
        }
        Ok(e)
    }

    fn arguments(&mut self) -> PResult<Vec<ArgElem>> {
        self.expect_punct("(")?;
        let save = core::mem::replace(&mut self.no_in, false);
        let mut args = Vec::new();
        while !self.is_punct(")") {
            if self.eat_punct("...")? {
                args.push(ArgElem::Spread(self.assignment()?));
            } else {
                args.push(ArgElem::Expr(self.assignment()?));
            }
            if !self.eat_punct(",")? {
                break;
            }
        }
        self.no_in = save;
        self.expect_punct(")")?;
        Ok(args)
    }

    fn lhs_expression(&mut self) -> PResult<Expr> {
        let mut e = if self.is_kw("new") { self.new_expression()? } else { self.primary()? };
        let mut in_chain = false;
        loop {
            if self.is_punct(".") {
                self.advance()?;
                if let Tok::Private(n) = &self.tok.tok.clone() {
                    self.advance()?;
                    e = Expr::PrivateMember { obj: Box::new(e), name: n.clone(), optional: false };
                } else {
                    let n = self.ident_name()?;
                    e = Expr::Dot { obj: Box::new(e), name: n, optional: false };
                }
            } else if self.is_punct("?.") {
                self.advance()?;
                in_chain = true;
                if self.is_punct("(") {
                    let args = self.arguments()?;
                    e = Expr::Call { callee: Box::new(e), args, optional: true };
                } else if self.is_punct("[") {
                    self.advance()?;
                    let save = core::mem::replace(&mut self.no_in, false);
                    let p = self.expression();
                    self.no_in = save;
                    let p = p?;
                    self.expect_punct("]")?;
                    e = Expr::Member { obj: Box::new(e), prop: Box::new(p), optional: true };
                } else if let Tok::Private(n) = &self.tok.tok.clone() {
                    self.advance()?;
                    e = Expr::PrivateMember { obj: Box::new(e), name: n.clone(), optional: true };
                } else {
                    let n = self.ident_name()?;
                    e = Expr::Dot { obj: Box::new(e), name: n, optional: true };
                }
            } else if self.is_punct("[") {
                self.advance()?;
                let save = core::mem::replace(&mut self.no_in, false);
                let p = self.expression();
                self.no_in = save;
                let p = p?;
                self.expect_punct("]")?;
                e = Expr::Member { obj: Box::new(e), prop: Box::new(p), optional: false };
            } else if self.is_punct("(") {
                let args = self.arguments()?;
                e = match e {
                    Expr::Ident(n) if self.interner.get(n).as_ref() == "super" => Expr::SuperCall(args),
                    other => Expr::Call { callee: Box::new(other), args, optional: false },
                };
            } else if let Tok::Template(strs, exprs) = self.tok.tok.clone() {
                self.advance()?;
                let obj = Rc::new(TemplateObj { cooked: strs.iter().map(|s| s.0.clone()).collect(), raw: strs.iter().map(|s| s.1.clone()).collect() });
                let mut ex = Vec::new();
                for (a, b) in exprs {
                    ex.push(self.sub_expression(a, b)?);
                }
                e = Expr::Tagged(Box::new(e), obj, ex);
            } else {
                break;
            }
        }
        if in_chain {
            e = Expr::OptChain(Box::new(e));
        }
        Ok(e)
    }

    fn new_expression(&mut self) -> PResult<Expr> {
        self.expect_kw("new")?;
        if self.is_punct(".") {
            self.advance()?;
            let t = self.ident_name()?;
            if &*t != "target" {
                return self.err("expected new.target");
            }
            return Ok(Expr::NewTarget);
        }
        let mut callee = if self.is_kw("new") { self.new_expression()? } else { self.primary()? };
        loop {
            if self.is_punct(".") {
                self.advance()?;
                let n = self.ident_name()?;
                callee = Expr::Dot { obj: Box::new(callee), name: n, optional: false };
            } else if self.is_punct("[") {
                self.advance()?;
                let p = self.expression()?;
                self.expect_punct("]")?;
                callee = Expr::Member { obj: Box::new(callee), prop: Box::new(p), optional: false };
            } else {
                break;
            }
        }
        let args = if self.is_punct("(") { self.arguments()? } else { Vec::new() };
        Ok(Expr::New(Box::new(callee), args))
    }

    fn sub_expression(&mut self, start: usize, end: usize) -> PResult<Expr> {
        let text: Rc<str> = Rc::from(&self.src[start..end]);
        let saved_lex = core::mem::replace(&mut self.lex, Lexer::new(text.clone()));
        let saved_tok = self.tok.clone();
        let saved_src = core::mem::replace(&mut self.src, text);
        let saved_prev = self.prev_end;
        let saved_no_in = core::mem::replace(&mut self.no_in, false);
        let r = (|| {
            self.advance()?;
            let e = self.expression()?;
            if self.tok.tok != Tok::Eof {
                return self.err("unexpected token in template expression");
            }
            Ok(e)
        })();
        self.lex = saved_lex;
        self.tok = saved_tok;
        self.src = saved_src;
        self.prev_end = saved_prev;
        self.no_in = saved_no_in;
        r
    }

    fn primary(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.primary_inner();
        self.leave();
        r
    }

    fn primary_inner(&mut self) -> PResult<Expr> {
        let tok = self.tok.tok.clone();
        match tok {
            Tok::Num(n) => {
                self.advance()?;
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.advance()?;
                Ok(Expr::Str(s))
            }
            Tok::Template(strs, exprs) => {
                self.advance()?;
                let mut ex = Vec::new();
                for (a, b) in exprs {
                    ex.push(self.sub_expression(a, b)?);
                }
                let cooked: Vec<JsStr> = strs.iter().map(|s| s.0.clone().unwrap_or_else(|| Rc::from(""))).collect();
                Ok(Expr::Template(cooked, ex))
            }
            Tok::Regex(p, f) => {
                self.advance()?;
                Ok(Expr::Regex(p, f))
            }
            Tok::Punct("(") => {
                self.advance()?;
                let save = core::mem::replace(&mut self.no_in, false);
                let e = self.expression();
                self.no_in = save;
                let e = e?;
                self.expect_punct(")")?;
                Ok(Expr::Paren(Box::new(e)))
            }
            Tok::Punct("[") => {
                self.advance()?;
                let save = core::mem::replace(&mut self.no_in, false);
                let mut elems = Vec::new();
                while !self.is_punct("]") {
                    if self.eat_punct(",")? {
                        elems.push(None);
                        continue;
                    }
                    if self.eat_punct("...")? {
                        elems.push(Some(ArgElem::Spread(self.assignment()?)));
                    } else {
                        elems.push(Some(ArgElem::Expr(self.assignment()?)));
                    }
                    if !self.eat_punct(",")? {
                        break;
                    }
                }
                self.no_in = save;
                self.expect_punct("]")?;
                Ok(Expr::Array(elems))
            }
            Tok::Punct("{") => self.object_literal(),
            Tok::Ident(name) => match &*name {
                "function" => Ok(Expr::Function(self.function_expr(false)?)),
                "async" => {
                    let next = self.peek()?;
                    if matches!(&next.tok, Tok::Ident(f) if &**f == "function") && !next.nl {
                        self.advance()?;
                        Ok(Expr::Function(self.function_expr(true)?))
                    } else {
                        self.advance()?;
                        let n = self.name("async");
                        Ok(Expr::Ident(n))
                    }
                }
                "class" => Ok(Expr::Class(self.class(false)?)),
                "this" => {
                    self.advance()?;
                    Ok(Expr::This)
                }
                "null" => {
                    self.advance()?;
                    Ok(Expr::Null)
                }
                "true" => {
                    self.advance()?;
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.advance()?;
                    Ok(Expr::Bool(false))
                }
                "super" => {
                    self.advance()?;
                    if self.is_punct("(") {
                        let args = self.arguments()?;
                        Ok(Expr::SuperCall(args))
                    } else if self.eat_punct(".")? {
                        let n = self.ident_name()?;
                        Ok(Expr::SuperDot(n))
                    } else if self.eat_punct("[")? {
                        let e = self.expression()?;
                        self.expect_punct("]")?;
                        Ok(Expr::SuperMember(Box::new(e)))
                    } else {
                        self.err("unexpected super")
                    }
                }
                "import" => {
                    self.advance()?;
                    if self.eat_punct(".")? {
                        self.ident_name()?;
                        return Ok(Expr::ImportMeta);
                    }
                    self.expect_punct("(")?;
                    let e = self.assignment()?;
                    if self.eat_punct(",")? && !self.is_punct(")") {
                        self.assignment()?;
                        self.eat_punct(",")?;
                    }
                    self.expect_punct(")")?;
                    Ok(Expr::ImportCall(Box::new(e)))
                }
                "new" => self.new_expression(),
                n if is_reserved(n) => self.err("unexpected keyword"),
                _ => {
                    self.advance()?;
                    if &*name == "arguments" {
                        // arrows share the enclosing function's arguments object
                        for f in self.fns.iter_mut().rev() {
                            f.uses_args = true;
                        }
                    }
                    let n = self.name(&name);
                    Ok(Expr::Ident(n))
                }
            },
            Tok::Private(_) => self.err("unexpected private name"),
            _ => self.err("unexpected token"),
        }
    }

    fn object_literal(&mut self) -> PResult<Expr> {
        self.expect_punct("{")?;
        let save = core::mem::replace(&mut self.no_in, false);
        let mut props = Vec::new();
        while !self.is_punct("}") {
            if self.eat_punct("...")? {
                let e = self.assignment()?;
                props.push(PropDef { key: PropKey::Name(Rc::from("")), value: PropValue::Spread(e) });
                if !self.eat_punct(",")? {
                    break;
                }
                continue;
            }
            let start = self.tok.start;
            let mut is_async = false;
            let mut is_gen = false;
            let mut accessor: Option<bool> = None;
            if self.is_kw("async") {
                let next = self.peek()?;
                if !matches!(next.tok, Tok::Punct("(") | Tok::Punct(":") | Tok::Punct(",") | Tok::Punct("}") | Tok::Punct("=")) && !next.nl {
                    self.advance()?;
                    is_async = true;
                }
            }
            if self.eat_punct("*")? {
                is_gen = true;
            }
            if (self.is_kw("get") || self.is_kw("set")) && !is_async && !is_gen {
                let next = self.peek()?;
                if !matches!(next.tok, Tok::Punct("(") | Tok::Punct(":") | Tok::Punct(",") | Tok::Punct("}") | Tok::Punct("=")) {
                    accessor = Some(self.is_kw("get"));
                    self.advance()?;
                }
            }
            let ident_key = if let Tok::Ident(s) = &self.tok.tok { Some(s.clone()) } else { None };
            let key = self.prop_key(false)?;
            if self.is_punct("(") {
                let kind = match accessor {
                    Some(true) => FnKind::Getter,
                    Some(false) => FnKind::Setter,
                    None => FnKind::Method,
                };
                let f = self.method(kind, is_async, is_gen, start)?;
                let value = match accessor {
                    Some(true) => PropValue::Getter(f),
                    Some(false) => PropValue::Setter(f),
                    None => PropValue::Method(f),
                };
                props.push(PropDef { key, value });
            } else if self.eat_punct(":")? {
                let v = self.assignment()?;
                props.push(PropDef { key, value: PropValue::Value(v) });
            } else if let Some(id) = ident_key {
                // shorthand
                let n = self.name(&id);
                if &*id == "arguments" {
                    for f in self.fns.iter_mut().rev() {
                        f.uses_args = true;
                    }
                }
                if self.eat_punct("=")? {
                    let d = self.assignment()?;
                    props.push(PropDef { key, value: PropValue::ShorthandDefault(n, d) });
                } else {
                    props.push(PropDef { key, value: PropValue::Value(Expr::Ident(n)) });
                }
            } else {
                return self.err("unexpected token in object literal");
            }
            if !self.eat_punct(",")? {
                break;
            }
        }
        self.no_in = save;
        self.expect_punct("}")?;
        Ok(Expr::Object(props))
    }
}

impl Stmt {
    fn with_alias(self, _alias: Name) -> Stmt {
        self
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(n) => format!("number {n}"),
        Tok::Str(s) => format!("string '{}'", s.chars().take(20).collect::<String>()),
        Tok::Template(..) => "template".to_string(),
        Tok::Ident(s) => format!("'{s}'"),
        Tok::Private(s) => format!("#{s}"),
        Tok::Punct(p) => format!("'{p}'"),
        Tok::Regex(..) => "regex".to_string(),
        Tok::Eof => "end of input".to_string(),
    }
}

pub fn pat_names(p: &Pat, out: &mut Vec<Name>) {
    match p {
        Pat::Ident(n) => out.push(*n),
        Pat::Expr(_) => {}
        Pat::Array(elems, rest) => {
            for e in elems.iter().flatten() {
                pat_names(&e.pat, out);
            }
            if let Some(r) = rest {
                pat_names(r, out);
            }
        }
        Pat::Object(props, rest) => {
            for pr in props {
                pat_names(&pr.pat, out);
            }
            if let Some(r) = rest {
                pat_names(r, out);
            }
        }
    }
}
