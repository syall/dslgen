//! Parser frontend, AST, and role/symbol model for calc-lang (spec.md §5, §6).

pub mod ast;
pub mod frontend;
pub mod lalrpop_frontend;
pub mod resolve;

pub use ast::{BinOp, Expr, Stmt};
pub use frontend::{ControlFlowRole, ParseDiagnostic, ParserFrontend, RoleModel};
pub use resolve::{resolve, ResolveError};
