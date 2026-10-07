//! Syntax tree for the JS engine.
use super::value::JsStr;
use alloc::{boxed::Box, rc::Rc, vec::Vec};

pub type Name = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FnKind {
    Normal,
    Method,
    Getter,
    Setter,
    Ctor,
    DerivedCtor,
}

pub struct Param {
    pub pat: Pat,
    pub default: Option<Expr>,
    pub rest: bool,
}

pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Lexical declarations (let/const/class) hoisted to the top of the block.
    pub lets: Vec<(Name, VarKind)>,
    /// Function declarations hoisted to the top of the block.
    pub funcs: Vec<Rc<FuncDef>>,
}

pub enum FuncBody {
    Block(Block),
    Expr(Box<Expr>),
}

pub struct FuncDef {
    pub name: Option<Name>,
    pub params: Vec<Param>,
    pub body: FuncBody,
    pub is_arrow: bool,
    pub is_async: bool,
    pub is_generator: bool,
    pub kind: FnKind,
    pub var_names: Vec<Name>,
    pub uses_args: bool,
    pub simple_params: bool,
    pub strict: bool,
    pub source: Rc<str>,
    pub span: (usize, usize),
    /// Number of formal parameters before the first default/rest (the `length` property).
    pub length: usize,
    /// Bytecode compiled on first call (`None` inside: not compilable, use the AST walker).
    pub code: core::cell::OnceCell<Option<Rc<super::vm::Code>>>,
}

pub enum PropKey {
    Name(JsStr),
    Computed(Expr),
    Private(JsStr),
}

pub enum PropValue {
    Value(Expr),
    Getter(Rc<FuncDef>),
    Setter(Rc<FuncDef>),
    Method(Rc<FuncDef>),
    Spread(Expr),
    /// `{ a = 1 }` — only valid when reinterpreted as a pattern.
    ShorthandDefault(Name, Expr),
}

pub struct PropDef {
    pub key: PropKey,
    pub value: PropValue,
}

pub enum ClassMemberKind {
    Method(Rc<FuncDef>),
    Getter(Rc<FuncDef>),
    Setter(Rc<FuncDef>),
    Field(Option<Expr>),
    StaticBlock(Block),
}

pub struct ClassMember {
    pub key: PropKey,
    pub is_static: bool,
    pub kind: ClassMemberKind,
}

pub struct ClassDef {
    pub name: Option<Name>,
    pub superclass: Option<Expr>,
    pub ctor: Option<Rc<FuncDef>>,
    pub members: Vec<ClassMember>,
    pub source: Rc<str>,
    pub span: (usize, usize),
}

pub enum Pat {
    Ident(Name),
    /// Member expression target of an assignment pattern.
    Expr(Box<Expr>),
    Array(Vec<Option<PatElem>>, Option<Box<Pat>>),
    Object(Vec<PatProp>, Option<Box<Pat>>),
}

pub struct PatElem {
    pub pat: Pat,
    pub default: Option<Expr>,
}

pub struct PatProp {
    pub key: PropKey,
    pub pat: Pat,
    pub default: Option<Expr>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    In,
    InstanceOf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogOp {
    And,
    Or,
    Nullish,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssignOp {
    Assign,
    Bin(BinOp),
    Logical(LogOp),
}

pub enum ArgElem {
    Expr(Expr),
    Spread(Expr),
}

pub enum Expr {
    Num(f64),
    Str(JsStr),
    Bool(bool),
    Null,
    This,
    Ident(Name),
    Template(Vec<JsStr>, Vec<Expr>),
    Tagged(Box<Expr>, Rc<TemplateObj>, Vec<Expr>),
    Regex(JsStr, JsStr),
    Array(Vec<Option<ArgElem>>),
    Object(Vec<PropDef>),
    Function(Rc<FuncDef>),
    Class(Rc<ClassDef>),
    Unary(UnOp, Box<Expr>),
    Update { inc: bool, prefix: bool, target: Box<Expr> },
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(LogOp, Box<Expr>, Box<Expr>),
    Assign(AssignOp, Box<Pat>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call { callee: Box<Expr>, args: Vec<ArgElem>, optional: bool },
    New(Box<Expr>, Vec<ArgElem>),
    Member { obj: Box<Expr>, prop: Box<Expr>, optional: bool },
    Dot { obj: Box<Expr>, name: JsStr, optional: bool },
    PrivateMember { obj: Box<Expr>, name: JsStr, optional: bool },
    OptChain(Box<Expr>),
    Seq(Vec<Expr>),
    Yield(Option<Box<Expr>>, bool),
    Await(Box<Expr>),
    NewTarget,
    SuperCall(Vec<ArgElem>),
    SuperMember(Box<Expr>),
    SuperDot(JsStr),
    ImportCall(Box<Expr>),
    ImportMeta,
    /// Parenthesised expression (needed to tell `(a, b) => x` and `(a) = 1` apart).
    Paren(Box<Expr>),
}

pub struct TemplateObj {
    pub cooked: Vec<Option<JsStr>>,
    pub raw: Vec<JsStr>,
}

pub enum ForInit {
    Var(VarKind, Vec<(Pat, Option<Expr>)>),
    Expr(Expr),
}

pub enum ForHead {
    Var(VarKind, Pat),
    Pat(Pat),
}

pub struct SwitchCase {
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
}

pub enum ImportSpec {
    Default(Name),
    Namespace(Name),
    Named(JsStr, Name),
}

pub enum Stmt {
    Expr(Expr),
    Var(VarKind, Vec<(Pat, Option<Expr>)>),
    Func(Rc<FuncDef>),
    Class(Rc<ClassDef>),
    Return(Option<Expr>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    Block(Block),
    For { init: Option<ForInit>, test: Option<Expr>, update: Option<Expr>, body: Box<Stmt>, lets: Vec<Name> },
    ForIn { head: ForHead, obj: Expr, body: Box<Stmt> },
    ForOf { head: ForHead, iter: Expr, body: Box<Stmt>, is_await: bool },
    While(Expr, Box<Stmt>),
    With(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Break(Option<Name>),
    Continue(Option<Name>),
    Throw(Expr),
    Try { block: Block, param: Option<Pat>, handler: Option<Block>, finalizer: Option<Block> },
    Switch { disc: Expr, cases: Vec<SwitchCase>, lets: Vec<(Name, VarKind)>, funcs: Vec<Rc<FuncDef>> },
    Labeled(Name, Box<Stmt>),
    Empty,
    Import { source: JsStr, specs: Vec<ImportSpec> },
    ExportNamed { specs: Vec<(Name, JsStr)>, from: Option<JsStr> },
    /// `export <declaration>`: the declaration plus the exported names.
    ExportDecl(Box<Stmt>, Vec<(Name, JsStr)>),
    ExportDefault(Expr),
    ExportAll(JsStr, Option<JsStr>),
}

pub struct Program {
    pub body: Block,
    pub var_names: Vec<Name>,
    pub is_module: bool,
}
