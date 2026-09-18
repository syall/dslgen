//! `LalrpopFrontend`: the first concrete `ParserFrontend` (spec.md §5), covering
//! arithmetic expressions with variables, numeric literals, and `if`/`else`. Its
//! actions build `crate::ast::Expr` values directly (session A2) — see `ast.rs`.

use crate::ast::Expr;
use crate::frontend::{ControlFlowRole, ParseDiagnostic, ParserFrontend, RoleModel};

lalrpop_util::lalrpop_mod!(pub calc);

pub struct LalrpopFrontend;

impl ParserFrontend for LalrpopFrontend {
    type Ast = Expr;

    fn parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>> {
        calc::ExprParser::new()
            .parse(src)
            .map_err(|err| vec![convert_error(err)])
    }

    fn role_model(&self) -> RoleModel {
        RoleModel {
            identifiers: vec!["Ident".to_string()],
            keywords: vec!["if".to_string(), "else".to_string()],
            control_flow: vec![ControlFlowRole {
                kind: "if".to_string(),
                rule: "Term".to_string(),
            }],
            ..Default::default()
        }
    }
}

fn convert_error<T: std::fmt::Debug>(
    err: lalrpop_util::ParseError<usize, T, &str>,
) -> ParseDiagnostic {
    let offset = match &err {
        lalrpop_util::ParseError::InvalidToken { location } => *location,
        lalrpop_util::ParseError::UnrecognizedToken { token, .. } => token.0,
        lalrpop_util::ParseError::ExtraToken { token } => token.0,
        _ => 0,
    };
    ParseDiagnostic {
        message: format!("{err:?}"),
        offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinOp;

    /// Throwaway: exercises `parse()` only through the `ParserFrontend` trait, never
    /// the LALRPOP-generated `calc::ExprParser` type directly, so later sessions can
    /// swap the frontend out without this test caring.
    #[test]
    fn parses_arithmetic_variables_and_if_else() {
        let frontend = LalrpopFrontend;
        let ast = frontend
            .parse("if x - 1 { 2 + 3 * 4 } else { y / 2 }")
            .expect("should parse");

        let expected = Expr::If {
            cond: Box::new(Expr::BinOp(
                Box::new(Expr::Var("x".to_string())),
                BinOp::Sub,
                Box::new(Expr::Number(1.0)),
            )),
            then_branch: Box::new(Expr::BinOp(
                Box::new(Expr::Number(2.0)),
                BinOp::Add,
                Box::new(Expr::BinOp(
                    Box::new(Expr::Number(3.0)),
                    BinOp::Mul,
                    Box::new(Expr::Number(4.0)),
                )),
            )),
            else_branch: Box::new(Expr::BinOp(
                Box::new(Expr::Var("y".to_string())),
                BinOp::Div,
                Box::new(Expr::Number(2.0)),
            )),
        };
        assert_eq!(ast, expected);
    }

    #[test]
    fn reports_a_diagnostic_for_invalid_syntax() {
        let frontend = LalrpopFrontend;
        let err = frontend.parse("1 + ").unwrap_err();
        assert_eq!(err.len(), 1);
    }

    #[test]
    fn role_model_reflects_calc_langs_own_grammar() {
        let frontend = LalrpopFrontend;
        let roles = frontend.role_model();
        assert!(roles.keywords.contains(&"if".to_string()));
        assert!(roles.keywords.contains(&"else".to_string()));
        assert_eq!(roles.control_flow.len(), 1);
    }

    /// The roadmap's named A2 test case: a minimal `if`/`else` parses into the exact
    /// typed `Expr` shape, not just "parses without error".
    #[test]
    fn parses_if_else_into_typed_ast_shape() {
        let frontend = LalrpopFrontend;
        let ast = frontend.parse("if x { 1 } else { 2 }").expect("should parse");

        assert_eq!(
            ast,
            Expr::If {
                cond: Box::new(Expr::Var("x".to_string())),
                then_branch: Box::new(Expr::Number(1.0)),
                else_branch: Box::new(Expr::Number(2.0)),
            }
        );
    }
}
