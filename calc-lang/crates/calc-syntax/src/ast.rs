//! The typed AST for calc-lang (spec.md §5's "each rule declares ... a result type,
//! so the AST is statically typed"). This replaces A1's throwaway `RawAst` now that
//! there's a firmer sense of what semantic actions and later lowering need from it.
//!
//! `Stmt` isn't here yet: calc-lang has no construct that isn't itself an
//! expression-with-a-value (see A1's doc: `if`/`else` evaluates like Rust's own `if`
//! expression), so there's nothing for a statement type to represent until A3 adds a
//! binding/declaration form. See `DECISIONS.md`'s A2 entry.

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}
