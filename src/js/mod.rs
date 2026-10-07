//! ECMAScript engine: lexer, parser, tree-walking interpreter and the DOM bindings.
pub mod ast;
pub mod builtins;
pub mod dom;
pub mod interp;
pub mod lexer;
pub mod ops;
pub mod parser;
pub mod regex;
pub mod value;

use crate::dom::{Document, NodeId};
use alloc::{string::String, vec::Vec};

/// A script runtime bound to one page.
pub struct Runtime {
    pub interp: interp::Interp,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Runtime { interp: builtins::new_interp() }
    }

    pub fn execute(&mut self, source: &str, document: &mut Document) -> Result<(), String> {
        self.interp.doc = document as *mut Document;
        let r = self.interp.eval_script(source);
        self.interp.run_microtasks();
        self.interp.doc = core::ptr::null_mut();
        r
    }

    pub fn dispatch(&mut self, node: NodeId, event: &str, document: &mut Document) -> Result<usize, String> {
        self.begin_event();
        self.dispatch_to(node, node, event, document)
    }

    pub(crate) fn begin_event(&mut self) {
        self.interp.ticks = 0;
    }

    pub(crate) fn dispatch_to(&mut self, _current: NodeId, _target: NodeId, _event: &str, _document: &mut Document) -> Result<usize, String> {
        Ok(0)
    }

    pub fn logs(&self) -> Vec<String> {
        self.interp.logs.iter().map(|l| alloc::format!("{}: {}", l.level, l.text)).collect()
    }
}

impl interp::Interp {
    /// Parse and run a classic script in the global scope.
    pub fn eval_script(&mut self, source: &str) -> Result<(), String> {
        let prog = parser::parse_program(source, &mut self.interner, false).map_err(|e| alloc::format!("SyntaxError: {e}"))?;
        match self.run_program(&prog) {
            Ok(_) => Ok(()),
            Err(value::Ctl::Throw(v)) => Err(self.error_text(&v)),
            Err(_) => Ok(()),
        }
    }

    pub fn dynamic_import(&mut self, _spec: &value::Value) -> value::Res {
        self.type_error("dynamic import is not supported")
    }

    pub fn new_regexp(&mut self, pattern: &str, flags: &str) -> value::Res {
        builtins::string::new_regexp(self, pattern, flags)
    }
}

impl core::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Runtime")
    }
}

#[cfg(test)]
mod tests;
