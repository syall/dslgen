//! A tree-walking interpreter over `calc-ir`'s IR (spec.md §9.1) — `calcc run
//! --interpret` and a semantics oracle later codegen backends (A6/A7) get checked
//! against. By the time IR reaches here, source-level variable names no longer exist
//! anywhere in the data (`ast_to_ir::lower` already rewrote every `Var(name)` into a
//! `Temp` reference) — so the runtime environment is a dense `Temp -> Value` store,
//! not a name-keyed one. See `calc-lang/docs/a5-tree-walking-interpreter.md`.

use calc_syntax::BinOp;

use crate::ir::{Block, Instr, Program, Temp};

/// calc-lang's one runtime type today. An enum (not a bare `f64`) because every
/// instruction produces and consumes one — this isn't speculative scaffolding for a
/// hypothetical future type, just the natural shape of "the value an instruction
/// computes."
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Number(f64),
}

impl Value {
    fn as_number(self) -> f64 {
        let Value::Number(n) = self;
        n
    }
}

/// `Temp`s are assigned by a single counter threaded through all of `lower()`,
/// including into both arms of every `If` (see `ast_to_ir.rs`'s `next_temp`), so
/// every `Temp` in a program is globally unique and densely numbered from 0. A
/// growing `Vec` indexed by `Temp.0` is therefore a correct, simpler substitute for a
/// `HashMap<Temp, Value>`: no two instructions ever share a slot, and an untaken
/// `If` branch's temps are simply never read.
struct TempStore(Vec<Option<Value>>);

impl TempStore {
    fn new() -> Self {
        TempStore(Vec::new())
    }

    fn write(&mut self, temp: Temp, value: Value) {
        // `Temp` is a tuple struct (`struct Temp(pub u32)`), so `.0` unwraps it back
        // to the raw slot number this store indexes by.
        let idx = temp.0 as usize;
        if idx >= self.0.len() {
            self.0.resize(idx + 1, None);
        }
        self.0[idx] = Some(value);
    }

    fn read(&self, temp: Temp) -> Value {
        self.0[temp.0 as usize].unwrap_or_else(|| {
            panic!(
                "calc_ir::interp: read of Temp({}) before it was written — \
                 malformed IR (every Temp must be written before use)",
                temp.0
            )
        })
    }
}

/// Runs `program` to completion and returns its result value.
pub fn interpret(program: &Program) -> Value {
    let mut store = TempStore::new();
    exec_block(&program.body, &mut store);
    store.read(program.result)
}

fn exec_block(block: &Block, store: &mut TempStore) {
    for instr in &block.0 {
        exec_instr(instr, store);
    }
}

fn exec_instr(instr: &Instr, store: &mut TempStore) {
    match instr {
        Instr::Const { dst, value } => store.write(*dst, Value::Number(*value)),
        Instr::BinOp { dst, op, lhs, rhs } => {
            let lhs = store.read(*lhs).as_number();
            let rhs = store.read(*rhs).as_number();
            let result = match op {
                BinOp::Add => lhs + rhs,
                BinOp::Sub => lhs - rhs,
                BinOp::Mul => lhs * rhs,
                BinOp::Div => lhs / rhs,
            };
            store.write(*dst, Value::Number(result));
        }
        Instr::CallBuiltin { dst, name, args } => {
            let builtin = calc_builtins::lookup(name)
                .unwrap_or_else(|| panic!("calc_ir::interp: unknown built-in `{name}`"));
            let eval = calc_runtime::eval(name)
                .unwrap_or_else(|| panic!("calc_ir::interp: no implementation for `{name}`"));
            let args: Vec<f64> = args.iter().map(|a| store.read(*a).as_number()).collect();
            assert_eq!(
                args.len(),
                builtin.arity,
                "built-in `{name}` arity mismatch"
            );
            store.write(*dst, Value::Number(eval(&args)));
        }
        Instr::Copy { dst, src } => {
            let value = store.read(*src);
            store.write(*dst, value);
        }
        Instr::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            // calc-lang has no boolean type — `if`'s condition is a plain numeric
            // expression, so truthiness is defined as "nonzero", mirroring C.
            if store.read(*cond).as_number() != 0.0 {
                exec_block(then_block, store);
            } else {
                exec_block(else_block, store);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_to_ir::lower;
    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::ParserFrontend;

    fn interpret_source(src: &str) -> Value {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        calc_syntax::resolve(&ast).expect("should resolve");
        interpret(&lower(&ast))
    }

    #[test]
    fn interprets_a_let_bound_if_expression() {
        assert_eq!(
            interpret_source("{ let x = 1; if x { x + 1 } else { 2 } }"),
            Value::Number(2.0)
        );
    }

    #[test]
    fn a_nonzero_condition_takes_the_then_branch() {
        assert_eq!(
            interpret_source("if 1 { 1 } else { 2 }"),
            Value::Number(1.0)
        );
    }

    #[test]
    fn a_zero_condition_takes_the_else_branch() {
        assert_eq!(
            interpret_source("if 0 { 1 } else { 2 }"),
            Value::Number(2.0)
        );
    }

    #[test]
    fn evaluates_arithmetic_with_precedence() {
        assert_eq!(interpret_source("2 + 3 * 4"), Value::Number(14.0));
    }

    /// `print` (session A11, kind 3: subprocess/IPC) goes through the exact same
    /// generic `CallBuiltin` path as `add`/`mul`/`sub` — no interpreter changes
    /// were needed to support a new binding kind, only a new manifest entry
    /// (`calc-builtins`) and its implementation (`calc-runtime`). As a statement
    /// (not an expression — see `calc-lang/DECISIONS.md`'s A11 entry), its own
    /// result is unreachable, so this checks that evaluating it doesn't disturb the block's real result.
    #[test]
    fn a_print_statement_does_not_disturb_the_blocks_result() {
        assert_eq!(
            interpret_source("{ let x = 1; print(x + 2); x + 5 }"),
            Value::Number(6.0)
        );
    }

    /// Session A11's top-level `Program` entry point: `print(1); 2` is a complete
    /// program with no surrounding `{ }`, equivalent to `{ print(1); 2 }`, and
    /// runs through resolve/lower/interpret exactly the same way.
    #[test]
    fn a_top_level_program_runs_statements_with_no_surrounding_braces() {
        assert_eq!(interpret_source("print(1); 2"), Value::Number(2.0));
    }
}
