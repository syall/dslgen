//! Lowering calc-lang's typed AST (`calc_syntax::Expr`/`Stmt`) into this crate's
//! mid-level IR (spec.md §8.1). `lower()` assumes its input has already passed
//! `calc_syntax::resolve()` — an unresolved identifier here is a caller bug, not a
//! user-facing error, so it panics rather than returning a `Result`.

use std::collections::HashMap;

use calc_syntax::{BinOp, Expr, Stmt};

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

/// calc-lang's only "call syntax" is its operators: `+`, `*`, and `-` are implemented
/// by the `add`/`mul` (spec.md §7 kind 1, session A9) and `sub` (kind 2, session A10)
/// built-ins, so they lower to `CallBuiltin`. `/` has no built-in yet and stays an
/// inline `BinOp`.
fn builtin_for(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("add"),
        BinOp::Mul => Some("mul"),
        BinOp::Sub => Some("sub"),
        BinOp::Div => None,
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
            instrs.push(match builtin_for(*op) {
                Some(name) => Instr::CallBuiltin {
                    dst,
                    name: name.to_string(),
                    args: vec![lhs, rhs],
                },
                None => Instr::BinOp {
                    dst,
                    op: *op,
                    lhs,
                    rhs,
                },
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
    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::ParserFrontend;

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
                            Instr::CallBuiltin {
                                dst: Temp(3),
                                name: "add".to_string(),
                                args: vec![Temp(0), Temp(2)],
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

    #[test]
    fn plus_times_and_minus_lower_to_builtin_calls_but_divide_stays_inline() {
        let ast = LalrpopFrontend
            .parse("(1 + 2) * 3 - 4 / 5")
            .expect("should parse");
        let Program { body, .. } = lower(&ast);

        let call_names: Vec<&str> = body
            .0
            .iter()
            .filter_map(|i| match i {
                Instr::CallBuiltin { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let inline_ops: Vec<BinOp> = body
            .0
            .iter()
            .filter_map(|i| match i {
                Instr::BinOp { op, .. } => Some(*op),
                _ => None,
            })
            .collect();
        assert_eq!(call_names, ["add", "mul", "sub"]);
        assert_eq!(inline_ops, [BinOp::Div]);
    }

    /// Names an instruction's variant. The `match` has no wildcard arm, so adding a new
    /// `Instr` variant breaks the build here until it's listed — and therefore until
    /// `lowering_emits_every_kind_of_instruction`'s program is extended to emit it.
    fn kind(instr: &Instr) -> &'static str {
        match instr {
            Instr::Const { .. } => "Const",
            Instr::BinOp { .. } => "BinOp",
            Instr::CallBuiltin { .. } => "CallBuiltin",
            Instr::Copy { .. } => "Copy",
            Instr::If { .. } => "If",
        }
    }

    fn collect_kinds(block: &Block, kinds: &mut std::collections::BTreeSet<&'static str>) {
        for instr in &block.0 {
            kinds.insert(kind(instr));
            if let Instr::If {
                then_block,
                else_block,
                ..
            } = instr
            {
                collect_kinds(then_block, kinds);
                collect_kinds(else_block, kinds);
            }
        }
    }

    /// One program that lowers to every `Instr` variant: `Const` (the literals), `If` with
    /// its two `Copy`s (the branches' shared result), `CallBuiltin` (`+`) and inline
    /// `BinOp` (`/`, the only operator with no built-in as of session A10).
    #[test]
    fn lowering_emits_every_kind_of_instruction() {
        let ast = LalrpopFrontend
            .parse("if 1 { 2 + 3 } else { 4 / 5 }")
            .expect("should parse");
        let mut kinds = std::collections::BTreeSet::new();
        collect_kinds(&lower(&ast).body, &mut kinds);

        assert_eq!(
            kinds.into_iter().collect::<Vec<_>>(),
            ["BinOp", "CallBuiltin", "Const", "Copy", "If"]
        );
    }
}
