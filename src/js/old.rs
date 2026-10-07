//! A deliberately small, native ECMAScript interpreter. Unsupported syntax is
//! reported as an error; it is never silently treated as successful execution.
use crate::dom::{Document, NodeId};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

const MAX_PARSE_DEPTH: usize = 32;
const MAX_EVAL_DEPTH: usize = 64;
const MAX_TOKENS: usize = 2048;

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    String(String),
    Number(f64),
    Symbol(char),
    Operator(String),
    End,
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        if tokens.len() >= MAX_TOKENS {
            return Err(String::from("JavaScript token limit reached"));
        }
        let rest = &source[pos..];
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() {
            pos += c.len_utf8();
        } else if rest.starts_with("//") {
            pos += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").ok_or("unclosed comment")?;
            pos += end + 2;
        } else if c == '\'' || c == '"' {
            pos += 1;
            let mut value = String::new();
            let mut closed = false;
            while pos < source.len() {
                let ch = source[pos..].chars().next().unwrap();
                pos += ch.len_utf8();
                if ch == c {
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    let escaped = source[pos..]
                        .chars()
                        .next()
                        .ok_or("unfinished string escape")?;
                    pos += escaped.len_utf8();
                    value.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    value.push(ch);
                }
            }
            if !closed {
                return Err(String::from("unclosed string"));
            }
            tokens.push(Token::String(value));
        } else if c.is_ascii_digit() {
            let start = pos;
            pos += 1;
            while pos < source.len()
                && (source.as_bytes()[pos].is_ascii_digit() || source.as_bytes()[pos] == b'.')
            {
                pos += 1;
            }
            let number = source[start..pos]
                .parse()
                .map_err(|_| String::from("invalid number"))?;
            tokens.push(Token::Number(number));
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = pos;
            pos += c.len_utf8();
            while pos < source.len() {
                let ch = source[pos..].chars().next().unwrap();
                if !(ch.is_alphanumeric() || ch == '_' || ch == '$') {
                    break;
                }
                pos += ch.len_utf8();
            }
            tokens.push(Token::Name(source[start..pos].to_string()));
        } else if let Some(operator) = ["===", "!==", "&&", "||", "==", "!=", "<=", ">=", "=>"]
            .into_iter()
            .find(|operator| rest.starts_with(operator))
        {
            tokens.push(Token::Operator(operator.to_string()));
            pos += operator.len();
        } else if "(){}.,;=+-*/%!<>".contains(c) {
            tokens.push(Token::Symbol(c));
            pos += c.len_utf8();
        } else {
            return Err(format!("unsupported JavaScript token: {c}"));
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}

#[derive(Debug, Clone)]
enum Expr {
    Literal(Value),
    Name(String),
    Member(Box<Expr>, String),
    Call(Box<Expr>, Vec<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Binary(Box<Expr>, String, Box<Expr>),
    Unary(char, Box<Expr>),
    Function(Vec<String>, Vec<Stmt>),
}

#[derive(Debug, Clone)]
enum Stmt {
    Declare(String, Expr),
    Expression(Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    Function(String, Vec<String>, Vec<Stmt>),
    Return(Option<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn new(source: &str) -> Result<Self, String> {
        Ok(Self {
            tokens: tokenize(source)?,
            pos: 0,
            depth: 0,
        })
    }
    fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::End)
    }
    fn take(&mut self) -> Token {
        let token = self.current().clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        token
    }
    fn symbol(&mut self, symbol: char) -> bool {
        if self.current() == &Token::Symbol(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn operator(&mut self, operator: &str) -> bool {
        if matches!(self.current(), Token::Operator(value) if value == operator) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn keyword(&mut self, keyword: &str) -> bool {
        if matches!(self.current(), Token::Name(value) if value == keyword) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, symbol: char) -> Result<(), String> {
        if self.symbol(symbol) {
            Ok(())
        } else {
            Err(format!("expected '{symbol}'"))
        }
    }
    fn name(&mut self) -> Result<String, String> {
        match self.take() {
            Token::Name(name) => Ok(name),
            _ => Err(String::from("expected identifier")),
        }
    }
    fn statements(&mut self, block: bool) -> Result<Vec<Stmt>, String> {
        let mut result = Vec::new();
        while self.current() != &Token::End && (!block || self.current() != &Token::Symbol('}')) {
            if self.symbol(';') {
                continue;
            }
            result.push(self.statement()?);
        }
        if block {
            self.expect('}')?;
        }
        Ok(result)
    }
    fn statement(&mut self) -> Result<Stmt, String> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(String::from("JavaScript parser nesting limit reached"));
        }
        self.depth += 1;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn statement_inner(&mut self) -> Result<Stmt, String> {
        if self.keyword("if") {
            self.expect('(')?;
            let condition = self.expression()?;
            self.expect(')')?;
            let then = self.statement_body()?;
            let otherwise = if self.keyword("else") {
                self.statement_body()?
            } else {
                Vec::new()
            };
            return Ok(Stmt::If(condition, then, otherwise));
        }
        if self.keyword("function") {
            let name = self.name()?;
            self.expect('(')?;
            let params = self.parameters()?;
            self.expect('{')?;
            return Ok(Stmt::Function(name, params, self.statements(true)?));
        }
        if self.keyword("return") {
            let value = if matches!(
                self.current(),
                Token::Symbol(';') | Token::Symbol('}') | Token::End
            ) {
                None
            } else {
                Some(self.expression()?)
            };
            self.terminator()?;
            return Ok(Stmt::Return(value));
        }
        if self.keyword("let") || self.keyword("const") || self.keyword("var") {
            let name = self.name()?;
            self.expect('=')?;
            let value = self.expression()?;
            self.terminator()?;
            return Ok(Stmt::Declare(name, value));
        }
        let expr = self.expression()?;
        self.terminator()?;
        Ok(Stmt::Expression(expr))
    }
    fn statement_body(&mut self) -> Result<Vec<Stmt>, String> {
        if self.symbol('{') {
            self.statements(true)
        } else {
            Ok(Vec::from([self.statement()?]))
        }
    }
    fn terminator(&mut self) -> Result<(), String> {
        if self.symbol(';') || matches!(self.current(), Token::Symbol('}') | Token::End) {
            Ok(())
        } else {
            Err(String::from("expected semicolon"))
        }
    }
    fn parameters(&mut self) -> Result<Vec<String>, String> {
        let mut params = Vec::new();
        if self.symbol(')') {
            return Ok(params);
        }
        loop {
            params.push(self.name()?);
            if self.symbol(')') {
                return Ok(params);
            }
            self.expect(',')?;
        }
    }
    fn expression(&mut self) -> Result<Expr, String> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(String::from("JavaScript parser nesting limit reached"));
        }
        self.depth += 1;
        let result = (|| {
            let left = self.logical_or()?;
            if self.symbol('=') {
                Ok(Expr::Assign(Box::new(left), Box::new(self.expression()?)))
            } else {
                Ok(left)
            }
        })();
        self.depth -= 1;
        result
    }
    fn logical_or(&mut self) -> Result<Expr, String> {
        let mut left = self.logical_and()?;
        while self.operator("||") {
            left = Expr::Binary(
                Box::new(left),
                String::from("||"),
                Box::new(self.logical_and()?),
            );
        }
        Ok(left)
    }
    fn logical_and(&mut self) -> Result<Expr, String> {
        let mut left = self.equality()?;
        while self.operator("&&") {
            left = Expr::Binary(
                Box::new(left),
                String::from("&&"),
                Box::new(self.equality()?),
            );
        }
        Ok(left)
    }
    fn equality(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        loop {
            let operator = ["===", "!==", "==", "!="]
                .into_iter()
                .find(|op| self.operator(op));
            let Some(operator) = operator else {
                break;
            };
            left = Expr::Binary(
                Box::new(left),
                operator.to_string(),
                Box::new(self.comparison()?),
            );
        }
        Ok(left)
    }
    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.addition()?;
        loop {
            let operator = if self.operator("<=") {
                Some("<=")
            } else if self.operator(">=") {
                Some(">=")
            } else if self.symbol('<') {
                Some("<")
            } else if self.symbol('>') {
                Some(">")
            } else {
                None
            };
            let Some(operator) = operator else {
                break;
            };
            left = Expr::Binary(
                Box::new(left),
                operator.to_string(),
                Box::new(self.addition()?),
            );
        }
        Ok(left)
    }
    fn addition(&mut self) -> Result<Expr, String> {
        let mut left = self.multiplication()?;
        loop {
            let operator = if self.symbol('+') {
                Some("+")
            } else if self.symbol('-') {
                Some("-")
            } else {
                None
            };
            let Some(operator) = operator else {
                break;
            };
            left = Expr::Binary(
                Box::new(left),
                operator.to_string(),
                Box::new(self.multiplication()?),
            );
        }
        Ok(left)
    }
    fn multiplication(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            let operator = if self.symbol('*') {
                Some("*")
            } else if self.symbol('/') {
                Some("/")
            } else if self.symbol('%') {
                Some("%")
            } else {
                None
            };
            let Some(operator) = operator else {
                break;
            };
            left = Expr::Binary(
                Box::new(left),
                operator.to_string(),
                Box::new(self.unary()?),
            );
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        for operator in ['!', '-', '+'] {
            if self.symbol(operator) {
                return Ok(Expr::Unary(operator, Box::new(self.unary()?)));
            }
        }
        self.postfix()
    }
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.primary()?;
        loop {
            if self.symbol('.') {
                expr = Expr::Member(Box::new(expr), self.name()?);
            } else if self.symbol('(') {
                let mut args = Vec::new();
                if !self.symbol(')') {
                    loop {
                        args.push(self.expression()?);
                        if self.symbol(')') {
                            break;
                        }
                        self.expect(',')?;
                    }
                }
                expr = Expr::Call(Box::new(expr), args);
            } else {
                break;
            }
        }
        Ok(expr)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        if self.symbol('(') {
            let start = self.pos;
            if self.symbol(')') && self.operator("=>") {
                self.expect('{')?;
                return Ok(Expr::Function(Vec::new(), self.statements(true)?));
            }
            self.pos = start;
            if matches!(self.current(), Token::Name(_)) {
                let name = self.name()?;
                if self.symbol(')') && self.operator("=>") {
                    self.expect('{')?;
                    return Ok(Expr::Function(Vec::from([name]), self.statements(true)?));
                }
            }
            self.pos = start;
            let expr = self.expression()?;
            self.expect(')')?;
            return Ok(expr);
        }
        match self.take() {
            Token::String(s) => Ok(Expr::Literal(Value::String(s))),
            Token::Number(n) => Ok(Expr::Literal(Value::Number(n))),
            Token::Name(s) if s == "true" => Ok(Expr::Literal(Value::Boolean(true))),
            Token::Name(s) if s == "false" => Ok(Expr::Literal(Value::Boolean(false))),
            Token::Name(s) if s == "null" => Ok(Expr::Literal(Value::Null)),
            Token::Name(s) if s == "undefined" => Ok(Expr::Literal(Value::Undefined)),
            Token::Name(s) => {
                if self.operator("=>") {
                    self.expect('{')?;
                    Ok(Expr::Function(Vec::from([s]), self.statements(true)?))
                } else {
                    Ok(Expr::Name(s))
                }
            }
            _ => Err(String::from("expected expression")),
        }
    }
}

#[derive(Debug, Clone)]
enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
    Window,
    Document,
    Node(NodeId),
    Event(NodeId, NodeId, String),
    Function(Vec<String>, Vec<Stmt>),
}

impl Value {
    fn js_string(&self) -> String {
        match self {
            Self::Undefined => String::from("undefined"),
            Self::Null => String::from("null"),
            Self::Boolean(b) => b.to_string(),
            Self::Number(n) => n.to_string(),
            Self::String(s) => s.clone(),
            Self::Window => String::from("[object Window]"),
            Self::Document => String::from("[object Document]"),
            Self::Node(_) => String::from("[object Element]"),
            Self::Event(_, _, _) => String::from("[object Event]"),
            Self::Function(_, _) => String::from("function"),
        }
    }
    fn truthy(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Boolean(value) => *value,
            Self::Number(value) => *value != 0.0 && !value.is_nan(),
            Self::String(value) => !value.is_empty(),
            _ => true,
        }
    }
    fn number(&self) -> f64 {
        match self {
            Self::Undefined => f64::NAN,
            Self::Null => 0.0,
            Self::Boolean(value) => {
                if *value {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Number(value) => *value,
            Self::String(value) => value.trim().parse().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
    fn strict_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Undefined, Self::Undefined) | (Self::Null, Self::Null) => true,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Node(a), Self::Node(b)) => a == b,
            (Self::Window, Self::Window) | (Self::Document, Self::Document) => true,
            _ => false,
        }
    }
    fn loose_eq(&self, other: &Self) -> bool {
        if self.strict_eq(other) {
            return true;
        }
        if matches!(
            (self, other),
            (Self::Null, Self::Undefined) | (Self::Undefined, Self::Null)
        ) {
            return true;
        }
        if matches!(
            (self, other),
            (Self::Number(_), Self::String(_))
                | (Self::String(_), Self::Number(_))
                | (Self::Boolean(_), _)
                | (_, Self::Boolean(_))
        ) {
            return self.number() == other.number();
        }
        false
    }
}

#[derive(Debug, Clone)]
struct Listener {
    node: NodeId,
    event: String,
    callback: Value,
}

/// Persistent JS state for a single document. No OS calls or external JS runtime.
#[derive(Debug, Default)]
pub struct Runtime {
    globals: BTreeMap<String, Value>,
    window: BTreeMap<String, Value>,
    listeners: Vec<Listener>,
    operations: usize,
    call_depth: usize,
    eval_depth: usize,
}

impl Runtime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, source: &str, document: &mut Document) -> Result<(), String> {
        let statements = Parser::new(source)?.statements(false)?;
        self.operations = 0;
        self.call_depth = 0;
        self.eval_depth = 0;
        self.run(&statements, document, &mut BTreeMap::new(), true)
            .map(|_| ())
    }

    pub fn dispatch(
        &mut self,
        node: NodeId,
        event: &str,
        document: &mut Document,
    ) -> Result<usize, String> {
        self.begin_event();
        self.dispatch_to(node, node, event, document)
    }

    pub(crate) fn begin_event(&mut self) {
        self.operations = 0;
        self.call_depth = 0;
        self.eval_depth = 0;
    }

    pub(crate) fn dispatch_to(
        &mut self,
        current_target: NodeId,
        target: NodeId,
        event: &str,
        document: &mut Document,
    ) -> Result<usize, String> {
        let callbacks: Vec<Value> = self
            .listeners
            .iter()
            .filter(|listener| listener.node == current_target && listener.event == event)
            .map(|listener| listener.callback.clone())
            .collect();
        for callback in &callbacks {
            if let Value::Function(params, body) = callback {
                self.call_function(
                    params,
                    body,
                    &[Value::Event(target, current_target, event.to_string())],
                    document,
                )?;
            }
        }
        Ok(callbacks.len())
    }

    fn run(
        &mut self,
        statements: &[Stmt],
        document: &mut Document,
        locals: &mut BTreeMap<String, Value>,
        global: bool,
    ) -> Result<Option<Value>, String> {
        for statement in statements {
            self.tick()?;
            match statement {
                Stmt::Declare(name, expr) => {
                    let value = self.eval(expr, document, locals)?;
                    if global {
                        self.globals.insert(name.clone(), value);
                    } else {
                        locals.insert(name.clone(), value);
                    }
                }
                Stmt::Expression(expr) => {
                    self.eval(expr, document, locals)?;
                }
                Stmt::If(condition, then, otherwise) => {
                    let branch = if self.eval(condition, document, locals)?.truthy() {
                        then
                    } else {
                        otherwise
                    };
                    if let Some(value) = self.run(branch, document, locals, global)? {
                        return Ok(Some(value));
                    }
                }
                Stmt::Function(name, params, body) => {
                    let value = Value::Function(params.clone(), body.clone());
                    if global {
                        self.globals.insert(name.clone(), value);
                    } else {
                        locals.insert(name.clone(), value);
                    }
                }
                Stmt::Return(value) => {
                    let value = match value {
                        Some(expr) => self.eval(expr, document, locals)?,
                        None => Value::Undefined,
                    };
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }

    fn call_function(
        &mut self,
        params: &[String],
        body: &[Stmt],
        args: &[Value],
        document: &mut Document,
    ) -> Result<Value, String> {
        if self.call_depth >= 64 {
            return Err(String::from("JavaScript call depth limit reached"));
        }
        self.call_depth += 1;
        let mut locals = BTreeMap::new();
        for (index, name) in params.iter().enumerate() {
            locals.insert(
                name.clone(),
                args.get(index).cloned().unwrap_or(Value::Undefined),
            );
        }
        let result = self.run(body, document, &mut locals, false);
        self.call_depth -= 1;
        Ok(result?.unwrap_or(Value::Undefined))
    }

    fn tick(&mut self) -> Result<(), String> {
        self.operations += 1;
        if self.operations > 100_000 {
            Err(String::from("JavaScript operation limit reached"))
        } else {
            Ok(())
        }
    }

    fn eval(
        &mut self,
        expr: &Expr,
        document: &mut Document,
        locals: &mut BTreeMap<String, Value>,
    ) -> Result<Value, String> {
        if self.eval_depth >= MAX_EVAL_DEPTH {
            return Err(String::from("JavaScript expression depth limit reached"));
        }
        self.eval_depth += 1;
        let result = self.eval_inner(expr, document, locals);
        self.eval_depth -= 1;
        result
    }

    fn eval_inner(
        &mut self,
        expr: &Expr,
        document: &mut Document,
        locals: &mut BTreeMap<String, Value>,
    ) -> Result<Value, String> {
        self.tick()?;
        match expr {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::Name(name) if name == "document" => Ok(Value::Document),
            Expr::Name(name) if name == "window" || name == "globalThis" => Ok(Value::Window),
            Expr::Name(name) => locals
                .get(name)
                .or_else(|| self.globals.get(name))
                .cloned()
                .ok_or_else(|| format!("unknown identifier: {name}")),
            Expr::Member(object, name) => {
                let object = self.eval(object, document, locals)?;
                match (object, name.as_str()) {
                    (Value::Window, "document") => Ok(Value::Document),
                    (Value::Window, property) => Ok(self
                        .window
                        .get(property)
                        .cloned()
                        .unwrap_or(Value::Undefined)),
                    (Value::Document, "body") => Ok(Value::Node(document.body)),
                    (Value::Node(id), "textContent") => {
                        Ok(Value::String(document.text_content(id).unwrap_or_default()))
                    }
                    (Value::Node(id), "id") => Ok(Value::String(
                        document
                            .nodes
                            .get(id)
                            .and_then(|n| n.attribute("id"))
                            .unwrap_or("")
                            .to_string(),
                    )),
                    (Value::Event(id, _, _), "target") => Ok(Value::Node(id)),
                    (Value::Event(_, id, _), "currentTarget") => Ok(Value::Node(id)),
                    (Value::Event(_, _, event), "type") => Ok(Value::String(event)),
                    _ => Err(format!("unsupported property: {name}")),
                }
            }
            Expr::Assign(target, value) => {
                let value = self.eval(value, document, locals)?;
                match target.as_ref() {
                    Expr::Name(name) => {
                        if locals.contains_key(name) {
                            locals.insert(name.clone(), value.clone());
                        } else {
                            self.globals.insert(name.clone(), value.clone());
                        }
                    }
                    Expr::Member(object, name) => {
                        let object = self.eval(object, document, locals)?;
                        if matches!(object, Value::Window) {
                            self.window.insert(name.clone(), value.clone());
                            return Ok(value);
                        }
                        let Value::Node(id) = object else {
                            return Err(String::from("assignment target is not an element"));
                        };
                        let changed = match name.as_str() {
                            "textContent" => document.set_text_content(id, &value.js_string()),
                            "id" => document.set_attribute(id, "id", &value.js_string()),
                            _ => return Err(format!("unsupported writable property: {name}")),
                        };
                        if !changed {
                            return Err(String::from("invalid DOM node"));
                        }
                    }
                    _ => return Err(String::from("invalid assignment target")),
                }
                Ok(value)
            }
            Expr::Unary(operator, expr) => {
                let value = self.eval(expr, document, locals)?;
                match operator {
                    '!' => Ok(Value::Boolean(!value.truthy())),
                    '-' => Ok(Value::Number(-value.number())),
                    '+' => Ok(Value::Number(value.number())),
                    _ => Err(String::from("unsupported unary operator")),
                }
            }
            Expr::Binary(left, operator, right) => {
                let left = self.eval(left, document, locals)?;
                if operator == "&&" && !left.truthy() {
                    return Ok(left);
                }
                if operator == "||" && left.truthy() {
                    return Ok(left);
                }
                let right = self.eval(right, document, locals)?;
                match operator.as_str() {
                    "&&" | "||" => Ok(right),
                    "+" => {
                        if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                            Ok(Value::String(left.js_string() + &right.js_string()))
                        } else {
                            Ok(Value::Number(left.number() + right.number()))
                        }
                    }
                    "-" => Ok(Value::Number(left.number() - right.number())),
                    "*" => Ok(Value::Number(left.number() * right.number())),
                    "/" => Ok(Value::Number(left.number() / right.number())),
                    "%" => Ok(Value::Number(left.number() % right.number())),
                    "===" => Ok(Value::Boolean(left.strict_eq(&right))),
                    "!==" => Ok(Value::Boolean(!left.strict_eq(&right))),
                    "==" => Ok(Value::Boolean(left.loose_eq(&right))),
                    "!=" => Ok(Value::Boolean(!left.loose_eq(&right))),
                    "<" | "<=" | ">" | ">=" => {
                        let compare = if let (Value::String(a), Value::String(b)) = (&left, &right)
                        {
                            Some(a.cmp(b))
                        } else {
                            left.number().partial_cmp(&right.number())
                        };
                        let matched = match operator.as_str() {
                            "<" => compare.is_some_and(|v| v.is_lt()),
                            "<=" => compare.is_some_and(|v| v.is_le()),
                            ">" => compare.is_some_and(|v| v.is_gt()),
                            _ => compare.is_some_and(|v| v.is_ge()),
                        };
                        Ok(Value::Boolean(matched))
                    }
                    _ => Err(format!("unsupported binary operator: {operator}")),
                }
            }
            Expr::Function(params, body) => Ok(Value::Function(params.clone(), body.clone())),
            Expr::Call(callee, args) => {
                if !matches!(callee.as_ref(), Expr::Member(_, _)) {
                    let target = self.eval(callee, document, locals)?;
                    let mut values = Vec::new();
                    for arg in args {
                        values.push(self.eval(arg, document, locals)?);
                    }
                    let Value::Function(params, body) = target else {
                        return Err(String::from("value is not callable"));
                    };
                    return self.call_function(&params, &body, &values, document);
                }
                let Expr::Member(object, method) = callee.as_ref() else {
                    return Err(String::from("unsupported function call"));
                };
                let receiver = self.eval(object, document, locals)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.eval(arg, document, locals)?);
                }
                match (receiver, method.as_str()) {
                    (Value::Document, "createElement") => {
                        let tag = values.first().ok_or("missing tag name")?.js_string();
                        Ok(Value::Node(document.create_element(&tag)))
                    }
                    (Value::Document, "getElementById") => Ok(values
                        .first()
                        .and_then(|v| document.get_element_by_id(&v.js_string()))
                        .map(Value::Node)
                        .unwrap_or(Value::Null)),
                    (Value::Document, "querySelector") => {
                        let id = values
                            .first()
                            .and_then(|v| v.js_string().strip_prefix('#').map(str::to_string))
                            .and_then(|id| document.get_element_by_id(&id));
                        Ok(id.map(Value::Node).unwrap_or(Value::Null))
                    }
                    (Value::Node(id), "setAttribute") => {
                        let name = values.first().ok_or("missing attribute name")?.js_string();
                        let value = values.get(1).ok_or("missing attribute value")?.js_string();
                        if !document.set_attribute(id, &name, &value) {
                            return Err(String::from("invalid DOM node"));
                        }
                        Ok(Value::Undefined)
                    }
                    (Value::Node(id), "appendChild") => {
                        let Some(Value::Node(child)) = values.first() else {
                            return Err(String::from("appendChild expects a node"));
                        };
                        if !document.append_child(id, *child) {
                            return Err(String::from("invalid appendChild operation"));
                        }
                        Ok(Value::Node(*child))
                    }
                    (Value::Node(id), "remove") => {
                        if !document.remove(id) {
                            return Err(String::from("cannot remove node"));
                        }
                        Ok(Value::Undefined)
                    }
                    (Value::Node(id), "addEventListener") => {
                        let event = values.first().ok_or("missing event name")?.js_string();
                        let callback = values.get(1).ok_or("missing callback")?.clone();
                        if !matches!(callback, Value::Function(_, _)) {
                            return Err(String::from("callback must be a function"));
                        }
                        self.listeners.push(Listener {
                            node: id,
                            event,
                            callback,
                        });
                        Ok(Value::Undefined)
                    }
                    (Value::Window, name) => {
                        let Some(Value::Function(params, body)) = self.window.get(name).cloned()
                        else {
                            return Err(format!("unsupported window method: {name}"));
                        };
                        self.call_function(&params, &body, &values, document)
                    }
                    _ => Err(format!("unsupported DOM method: {method}")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    #[test]
    fn changes_dom_and_dispatches_click() {
        let mut doc = html::parse("<p id='result'>old</p><button id='go'>Go</button>");
        let mut js = Runtime::new();
        js.execute("const result = document.getElementById('result'); result.textContent = 'ready'; document.getElementById('go').addEventListener('click', () => { result.textContent = 'clicked'; });", &mut doc).unwrap();
        let result = doc.get_element_by_id("result").unwrap();
        assert_eq!(doc.text_content(result).as_deref(), Some("ready"));
        let go = doc.get_element_by_id("go").unwrap();
        assert_eq!(js.dispatch(go, "click", &mut doc).unwrap(), 1);
        assert_eq!(doc.text_content(result).as_deref(), Some("clicked"));
    }

    #[test]
    fn rejects_unsupported_syntax() {
        let mut doc = html::parse("<p>test</p>");
        assert!(Runtime::new().execute("for (;;) {}", &mut doc).is_err());
        for broken in ["if (", "document.", "function f(", "const x = ("] {
            assert!(Runtime::new().execute(broken, &mut doc).is_err());
        }
        let nested = format!("{}true{};", "(".repeat(300), ")".repeat(300));
        assert!(Runtime::new().execute(&nested, &mut doc).is_err());
        let flat = format!("{}1;", "1+".repeat(3000));
        assert!(Runtime::new().execute(&flat, &mut doc).is_err());
        let bounded_flat = format!("{}1;", "1+".repeat(900));
        assert!(Runtime::new().execute(&bounded_flat, &mut doc).is_err());
    }

    #[test]
    fn window_properties_share_the_global_object() {
        let mut doc = html::parse("<p>old</p>");
        Runtime::new()
            .execute(
                "window.message = 'ready'; document.body.textContent = globalThis.message;",
                &mut doc,
            )
            .unwrap();
        assert_eq!(doc.text_content(doc.body).as_deref(), Some("ready"));
    }

    #[test]
    fn function_if_precedence_and_event_parameter_work_together() {
        let mut doc = html::parse("<button id='go'>Go</button><p id='result'>old</p>");
        let mut js = Runtime::new();
        js.execute("function choose(n) { if (n >= 2 && n < 4) { return 'yes'; } else { return 'no'; } } function clicked(e) { document.getElementById('result').textContent = choose(1 + 2 * 1) + e.type; } document.getElementById('go').addEventListener('click', clicked);", &mut doc).unwrap();
        let go = doc.get_element_by_id("go").unwrap();
        js.dispatch(go, "click", &mut doc).unwrap();
        let result = doc.get_element_by_id("result").unwrap();
        assert_eq!(doc.text_content(result).as_deref(), Some("yesclick"));
    }

    #[test]
    fn logical_operators_skip_unused_side_effects() {
        let mut doc = html::parse("<p>safe</p>");
        Runtime::new().execute("false && (document.body.textContent = 'wrong'); true || (document.body.textContent = 'wrong');", &mut doc).unwrap();
        assert_eq!(doc.text_content(doc.body).as_deref(), Some("safe"));
    }
}
