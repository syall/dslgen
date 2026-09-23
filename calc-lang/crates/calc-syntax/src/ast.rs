//! The typed AST for calc-lang (spec.md §5's "each rule declares ... a result type,
//! so the AST is statically typed"). This replaces A1's throwaway `RawAst` now that
//! there's a firmer sense of what semantic actions and later lowering need from it.
//!
//! `Stmt` arrives in A3, alongside `Expr::Block`: calc-lang's first non-expression
//! construct, a `let` declaration that only makes sense inside a block's statement
//! list (see `DECISIONS.md`'s A3 entry for why block-scoped statements were chosen
//! over an ML-style `let ... in ...` expression).

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
    Block {
        stmts: Vec<Stmt>,
        result: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
    },
    /// `print(<expr>);` — calc-lang's one fixed built-in surface (session A11),
    /// the same "operators are the surface" principle A9 established for
    /// `+`/`*`/`-`, extended to one hardcoded keyword rather than general call
    /// syntax: there's no callee name to resolve, just this single construct. A
    /// statement, not an expression, since it's run for its side effect — see
    /// `calc-lang/DECISIONS.md`'s A11 entry for why (it briefly was an
    /// expression, then moved here).
    Print(Expr),
}
