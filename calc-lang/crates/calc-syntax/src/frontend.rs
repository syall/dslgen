//! The `ParserFrontend` contract (spec.md §5): any parsing strategy — a generated
//! grammar-file frontend (LALRPOP, pest, ...) or a hand-written one — turns DSL source
//! text into a typed AST plus a role model, so nothing downstream (lowering, the LSP)
//! needs to know which frontend produced either.

/// A parse error, located by byte offset into the source text.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseDiagnostic {
    pub message: String,
    pub offset: usize,
}

/// The `#[control_flow(...)]`-tagged rules a frontend reports (spec.md §6.2).
#[derive(Debug, Clone, PartialEq)]
pub struct ControlFlowRole {
    pub kind: String,
    pub rule: String,
}

/// The structural role metadata a `ParserFrontend` reports about its grammar,
/// independent of the AST shape its actions produce (spec.md §6.2). Only the
/// categories calc-lang's grammar actually uses so far are populated; `scopes` and
/// `bindings` fill in once block scoping exists (session A3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleModel {
    pub identifiers: Vec<String>,
    pub keywords: Vec<String>,
    pub control_flow: Vec<ControlFlowRole>,
    pub scopes: Vec<String>,
    pub bindings: Vec<String>,
}

pub trait ParserFrontend {
    type Ast;

    fn parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>>;

    fn role_model(&self) -> RoleModel;
}
