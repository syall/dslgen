//! Mid-level IR and AST-to-IR lowering for calc-lang (spec.md §8.1).

pub mod ast_to_ir;
pub mod ir;

pub use ast_to_ir::lower;
pub use ir::{BinOp, Block, Instr, Program, Temp};
