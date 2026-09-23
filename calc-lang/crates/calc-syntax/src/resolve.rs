//! Name resolution over calc-lang's AST (spec.md §6.2's `#[scope_*]`/`#[binding]`
//! roles, made concrete): a hand-written pass that walks `Expr`, builds nested
//! lexical scopes at each `Expr::Block`, and reports unresolved identifiers and
//! duplicate bindings within a single scope. This is deliberately hand-rolled logic
//! for now, not routed through a built-in mechanism — §7.3's `scope_enter`/
//! `scope_exit`/`symbol_declare`/`symbol_lookup` built-in refactor is session A18's
//! job, once there's a generic role-driven lowering pass (Part B) to plug into.

use std::collections::HashMap;

use crate::ast::{Expr, Stmt};

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveError {
    UnresolvedIdentifier { name: String },
    DuplicateBinding { name: String },
}

/// Resolves every identifier reference in `expr` against the lexical scopes formed
/// by nested `Expr::Block`s, accumulating every error found rather than stopping at
/// the first one (mirroring `ParserFrontend::parse`'s `Result<T, Vec<Diagnostic>>`
/// shape).
pub fn resolve(expr: &Expr) -> Result<(), Vec<ResolveError>> {
    let mut scopes: Vec<HashMap<String, ()>> = Vec::new();
    let mut errors = Vec::new();
    resolve_expr(expr, &mut scopes, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn resolve_expr(
    expr: &Expr,
    scopes: &mut Vec<HashMap<String, ()>>,
    errors: &mut Vec<ResolveError>,
) {
    match expr {
        Expr::Number(_) => {}
        Expr::Var(name) => {
            if !scopes.iter().rev().any(|scope| scope.contains_key(name)) {
                errors.push(ResolveError::UnresolvedIdentifier { name: name.clone() });
            }
        }
        Expr::BinOp(l, _, r) => {
            resolve_expr(l, scopes, errors);
            resolve_expr(r, scopes, errors);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            resolve_expr(cond, scopes, errors);
            resolve_expr(then_branch, scopes, errors);
            resolve_expr(else_branch, scopes, errors);
        }
        Expr::Block { stmts, result } => {
            scopes.push(HashMap::new());
            for stmt in stmts {
                match stmt {
                    Stmt::Let { name, value } => {
                        resolve_expr(value, scopes, errors);
                        let scope = scopes.last_mut().expect("scope just pushed");
                        if scope.contains_key(name) {
                            errors.push(ResolveError::DuplicateBinding { name: name.clone() });
                        } else {
                            scope.insert(name.clone(), ());
                        }
                    }
                    Stmt::Print(inner) => resolve_expr(inner, scopes, errors),
                }
            }
            resolve_expr(result, scopes, errors);
            scopes.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::ParserFrontend;
    use crate::lalrpop_frontend::LalrpopFrontend;

    fn parse(src: &str) -> Expr {
        LalrpopFrontend.parse(src).expect("should parse")
    }

    #[test]
    fn reports_an_unresolved_identifier() {
        let ast = parse("x + 1");
        assert_eq!(
            resolve(&ast),
            Err(vec![ResolveError::UnresolvedIdentifier {
                name: "x".to_string()
            }])
        );
    }

    #[test]
    fn reports_a_duplicate_binding_in_the_same_scope() {
        let ast = parse("{ let x = 1; let x = 2; x }");
        assert_eq!(
            resolve(&ast),
            Err(vec![ResolveError::DuplicateBinding {
                name: "x".to_string()
            }])
        );
    }

    #[test]
    fn resolves_a_binding_used_in_its_own_blocks_result() {
        let ast = parse("{ let x = 1; x + 2 }");
        assert_eq!(resolve(&ast), Ok(()));
    }

    #[test]
    fn allows_shadowing_across_nested_scopes() {
        let ast = parse("{ let x = 1; if x { let x = 2; x } else { 0 } }");
        assert_eq!(resolve(&ast), Ok(()));
    }

    /// `Stmt::Print` (session A11) resolves its inner expression like any other
    /// sub-expression, so an unresolved identifier inside `print(...)` is still
    /// caught — and, unlike `let`, doesn't add anything to the enclosing scope.
    #[test]
    fn resolves_into_a_print_statements_argument() {
        let ast = parse("{ print(x); 0 }");
        assert_eq!(
            resolve(&ast),
            Err(vec![ResolveError::UnresolvedIdentifier {
                name: "x".to_string()
            }])
        );
    }
}
