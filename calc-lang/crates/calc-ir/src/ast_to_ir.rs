//! Lowering calc-lang's typed AST (`calc_syntax::Expr`/`Stmt`) into this crate's
//! mid-level IR (spec.md §8.1). `lower()` assumes its input has already passed
//! `calc_syntax::resolve()` — an unresolved identifier here is a caller bug, not a
//! user-facing error, so it panics rather than returning a `Result`.

use std::collections::HashMap;

use calc_syntax::{Expr, Stmt};

use crate::ir::{Block, Instr, Program, Temp};

pub fn lower(expr: &Expr) -> Program {
    let mut env: Vec<HashMap<String, Temp>> = Vec::new();
    let mut instrs = Vec::new();
    let mut next_temp = 0u32;
    let result = lower_expr(expr, &mut env, &mut instrs, &mut next_temp);
    Program {
        body: Block(instrs),
        result,
    }
}

fn fresh(next_temp: &mut u32) -> Temp {
    let temp = Temp(*next_temp);
    *next_temp += 1;
    temp
}

fn lookup(env: &[HashMap<String, Temp>], name: &str) -> Temp {
    env.iter()
        .rev()
        .find_map(|scope| scope.get(name))
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "ast_to_ir::lower: unresolved identifier `{name}` — \
                 input AST must be checked with calc_syntax::resolve() first"
            )
        })
}

fn lower_expr(
    expr: &Expr,
    env: &mut Vec<HashMap<String, Temp>>,
    instrs: &mut Vec<Instr>,
    next_temp: &mut u32,
) -> Temp {
    match expr {
        Expr::Number(value) => {
            let dst = fresh(next_temp);
            instrs.push(Instr::Const { dst, value: *value });
            dst
        }
        Expr::Var(name) => lookup(env, name),
        Expr::BinOp(lhs, op, rhs) => {
            let lhs = lower_expr(lhs, env, instrs, next_temp);
            let rhs = lower_expr(rhs, env, instrs, next_temp);
            let dst = fresh(next_temp);
            instrs.push(Instr::BinOp {
                dst,
                op: *op,
                lhs,
                rhs,
            });
            dst
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let cond = lower_expr(cond, env, instrs, next_temp);
            let dst = fresh(next_temp);

            let mut then_instrs = Vec::new();
            let then_result = lower_expr(then_branch, env, &mut then_instrs, next_temp);
            then_instrs.push(Instr::Copy {
                dst,
                src: then_result,
            });

            let mut else_instrs = Vec::new();
            let else_result = lower_expr(else_branch, env, &mut else_instrs, next_temp);
            else_instrs.push(Instr::Copy {
                dst,
                src: else_result,
            });

            instrs.push(Instr::If {
                dst,
                cond,
                then_block: Block(then_instrs),
                else_block: Block(else_instrs),
            });
            dst
        }
        Expr::Block { stmts, result } => {
            env.push(HashMap::new());
            for stmt in stmts {
                let Stmt::Let { name, value } = stmt;
                let value = lower_expr(value, env, instrs, next_temp);
                env.last_mut()
                    .expect("scope just pushed")
                    .insert(name.clone(), value);
            }
            let result = lower_expr(result, env, instrs, next_temp);
            env.pop();
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calc_syntax::{BinOp, ParserFrontend};
    use calc_syntax::lalrpop_frontend::LalrpopFrontend;

    #[test]
    fn lowers_a_let_bound_if_expression() {
        let ast = LalrpopFrontend
            .parse("{ let x = 1; if x { x + 1 } else { 2 } }")
            .expect("should parse");

        let program = lower(&ast);

        assert_eq!(
            program,
            Program {
                body: Block(vec![
                    Instr::Const {
                        dst: Temp(0),
                        value: 1.0
                    },
                    Instr::If {
                        dst: Temp(1),
                        cond: Temp(0),
                        then_block: Block(vec![
                            Instr::Const {
                                dst: Temp(2),
                                value: 1.0
                            },
                            Instr::BinOp {
                                dst: Temp(3),
                                op: BinOp::Add,
                                lhs: Temp(0),
                                rhs: Temp(2),
                            },
                            Instr::Copy {
                                dst: Temp(1),
                                src: Temp(3)
                            },
                        ]),
                        else_block: Block(vec![
                            Instr::Const {
                                dst: Temp(4),
                                value: 2.0
                            },
                            Instr::Copy {
                                dst: Temp(1),
                                src: Temp(4)
                            },
                        ]),
                    },
                ]),
                result: Temp(1),
            }
        );
    }
}
