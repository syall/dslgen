//! Mid-level IR types for calc-lang (spec.md §8.1): a small three-address-code IR
//! with a structured `If` node instead of jump-based basic blocks. See
//! `calc-lang/docs/a4-mid-level-ir-and-lowering.md` for the full design rationale
//! and `DECISIONS.md`'s A4 entry for what's deliberately deferred (`Loop`/
//! `Break`/`Continue`/`Return` — no AST construct needs them yet).

pub use calc_syntax::BinOp;

/// A compiler-introduced intermediate value ("temporary" in three-address-code
/// terminology) — distinct from a source-level variable name. Every instruction
/// names the `Temp` it writes to; operands reference other instructions' `Temp`s
/// rather than nested sub-expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Temp(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub enum Instr {
    Const {
        dst: Temp,
        value: f64,
    },
    BinOp {
        dst: Temp,
        op: BinOp,
        lhs: Temp,
        rhs: Temp,
    },
    /// Copies `src` into `dst`. Used to funnel an `If`'s two branches into one
    /// shared result temp ("phi via copies") — see the module docs.
    Copy {
        dst: Temp,
        src: Temp,
    },
    If {
        dst: Temp,
        cond: Temp,
        then_block: Block,
        else_block: Block,
    },
}

/// A straight-line instruction sequence: no internal branches. Control flow
/// between blocks is expressed by nesting them inside a structured node like
/// `Instr::If`, not by jump instructions connecting a flat list of blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block(pub Vec<Instr>);

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub body: Block,
    pub result: Temp,
}
