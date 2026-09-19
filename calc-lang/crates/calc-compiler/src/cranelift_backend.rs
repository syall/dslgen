//! Cranelift codegen backend (spec.md §8.1, session A6): lowers `calc-ir`'s IR to a
//! native object file, the first of v1's two planned `Backend`-trait
//! implementations (the trait itself, unifying this and A7's LLVM backend, doesn't
//! exist yet — see `DECISIONS.md`'s A6 entry for why that's deliberately deferred to
//! A8, not built speculatively here). See
//! `calc-lang/docs/a6-cranelift-codegen-backend.md` for the full walkthrough.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use calc_ir::{Block as IrBlock, Instr, Program, Temp};

/// Maps every `Temp` used by a program to the Cranelift `Variable` standing in for
/// it — `declare_var` mints `Variable`s itself (it doesn't accept caller-chosen
/// ids), so this table is how `lower_instr` finds the right one back given a
/// `Temp`.
type VarMap = HashMap<Temp, Variable>;

/// Compiles an entire `calc-ir` program into a finished object file's bytes: a
/// `calc_main() -> f64` function holding the actual compiled program (mirroring
/// `calc_ir::interp`'s `Value::Number(f64)` result), plus a small C-ABI `main() ->
/// i32` entry point that calls it and returns the result as the process's exit code
/// — see `DECISIONS.md`'s A6 entry for why an exit code stands in for real output
/// before A9's built-ins (and A12's link driver) exist.
pub fn compile_to_object(program: &Program) -> Vec<u8> {
    let mut module = new_object_module();

    let calc_main_id = declare_calc_main(&mut module);
    define_calc_main(&mut module, calc_main_id, program);

    let main_id = declare_c_main(&mut module);
    define_c_main(&mut module, main_id, calc_main_id);

    let product = module.finish();
    product.object.write().expect("valid object file")
}

/// An empty `ObjectModule` targeting the host CPU — the fixed opening sequence of
/// every compile (walked through in the docs' "Recipe 4").
fn new_object_module() -> ObjectModule {
    let isa_builder =
        cranelift_native::builder().expect("host architecture is supported by cranelift-native");
    let mut flag_builder = settings::builder();
    flag_builder.set("is_pic", "false").expect("valid setting");
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .expect("host ISA settings are valid");

    let object_builder = ObjectBuilder::new(isa, "calc_main", default_libcall_names())
        .expect("object builder configuration is valid");
    ObjectModule::new(object_builder)
}

fn declare_calc_main(module: &mut ObjectModule) -> FuncId {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::F64));
    module
        .declare_function("calc_main", Linkage::Export, &sig)
        .expect("calc_main is declared exactly once")
}

fn define_calc_main(module: &mut ObjectModule, func_id: FuncId, program: &Program) {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::F64));

    let mut ctx = Context::new();
    ctx.func.signature = sig;

    let mut builder_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);

    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);

    // Every `Temp` in the program gets an `F64` Cranelift `Variable`. Using
    // `Variable`/`def_var`/`use_var` instead of a hand-rolled `Temp -> Value` table
    // hands the "phi at merge points" problem to Cranelift's own SSA-construction
    // machinery (Braun et al.), which is exactly the trick that dissolves
    // `Instr::Copy`'s "two static definition sites" (per `DECISIONS.md`'s A4 "phi
    // via copies" entry) into ordinary variable writes.
    //
    // `collect_temps` can list the same `Temp` more than once — e.g. `If`'s `dst`
    // is collected once for the `If` itself and again via each branch's `Copy`
    // sharing that `dst` — so `entry().or_insert_with(..)` declares each `Temp`'s
    // `Variable` exactly once rather than minting (and immediately orphaning) a
    // fresh one on every repeat.
    let mut vars = VarMap::new();
    for temp in collect_temps(program) {
        vars.entry(temp)
            .or_insert_with(|| builder.declare_var(types::F64));
    }

    lower_block(&program.body, &mut builder, &vars);
    let result = builder.use_var(vars[&program.result]);
    builder.ins().return_(&[result]);

    builder.finalize(module.isa().frontend_config());

    module
        .define_function(func_id, &mut ctx)
        .expect("calc_main body is well-formed");
}

fn declare_c_main(module: &mut ObjectModule) -> FuncId {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::I32));
    module
        .declare_function("main", Linkage::Export, &sig)
        .expect("main is declared exactly once")
}

/// `main`'s only job is to make the object file linkable into a runnable executable
/// (see the module doc comment) — it calls `calc_main` and converts its `f64`
/// result to `i32` via a *saturating* conversion (`fcvt_to_sint_sat`), which is
/// always well-defined even for results outside `i32`'s range, unlike a raw
/// truncating cast.
fn define_c_main(module: &mut ObjectModule, main_id: FuncId, calc_main_id: FuncId) {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::I32));

    let mut ctx = Context::new();
    ctx.func.signature = sig;

    let mut builder_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);

    let calc_main_ref = module.declare_func_in_func(calc_main_id, builder.func);

    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);

    let call = builder.ins().call(calc_main_ref, &[]);
    let result = builder.inst_results(call)[0];
    let exit_code = builder.ins().fcvt_to_sint_sat(types::I32, result);
    builder.ins().return_(&[exit_code]);

    builder.finalize(module.isa().frontend_config());

    module
        .define_function(main_id, &mut ctx)
        .expect("main body is well-formed");
}

/// Every `Temp` a program uses must get a `Variable` before any block references it
/// (Cranelift requires declaration before use), so this walks the whole IR up front
/// collecting each `Temp` that's ever written to — `program.result` plus every
/// instruction's `dst` across both `If` branches.
fn collect_temps(program: &Program) -> Vec<Temp> {
    let mut temps = vec![program.result];
    collect_block_temps(&program.body, &mut temps);
    temps
}

fn collect_block_temps(block: &IrBlock, temps: &mut Vec<Temp>) {
    for instr in &block.0 {
        match instr {
            Instr::Const { dst, .. } => temps.push(*dst),
            Instr::BinOp { dst, .. } => temps.push(*dst),
            Instr::Copy { dst, .. } => temps.push(*dst),
            Instr::If {
                dst,
                then_block,
                else_block,
                ..
            } => {
                temps.push(*dst);
                collect_block_temps(then_block, temps);
                collect_block_temps(else_block, temps);
            }
        }
    }
}

fn lower_block(block: &IrBlock, builder: &mut FunctionBuilder, vars: &VarMap) {
    for instr in &block.0 {
        lower_instr(instr, builder, vars);
    }
}

fn lower_instr(instr: &Instr, builder: &mut FunctionBuilder, vars: &VarMap) {
    match instr {
        Instr::Const { dst, value } => {
            let v = builder.ins().f64const(*value);
            builder.def_var(vars[dst], v);
        }
        Instr::BinOp { dst, op, lhs, rhs } => {
            let lhs = builder.use_var(vars[lhs]);
            let rhs = builder.use_var(vars[rhs]);
            let result = match op {
                calc_ir::BinOp::Add => builder.ins().fadd(lhs, rhs),
                calc_ir::BinOp::Sub => builder.ins().fsub(lhs, rhs),
                calc_ir::BinOp::Mul => builder.ins().fmul(lhs, rhs),
                calc_ir::BinOp::Div => builder.ins().fdiv(lhs, rhs),
            };
            builder.def_var(vars[dst], result);
        }
        Instr::Copy { dst, src } => {
            let v = builder.use_var(vars[src]);
            builder.def_var(vars[dst], v);
        }
        Instr::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            // calc-lang has no boolean type, so `if`'s condition is truthy when
            // nonzero — the same rule `calc_ir::interp` uses. `FloatCC::NotEqual` is
            // IEEE 754's "unordered or not equal", which agrees with Rust's `f64 !=
            // 0.0` on every input including NaN, so this is bit-for-bit the same
            // semantics as the interpreter's `as_number() != 0.0` check.
            let cond_val = builder.use_var(vars[cond]);
            let zero = builder.ins().f64const(0.0);
            let is_truthy = builder.ins().fcmp(FloatCC::NotEqual, cond_val, zero);

            let then_blk = builder.create_block();
            let else_blk = builder.create_block();
            let merge_blk = builder.create_block();

            builder.ins().brif(is_truthy, then_blk, &[], else_blk, &[]);

            builder.switch_to_block(then_blk);
            builder.seal_block(then_blk);
            lower_block(then_block, builder, vars);
            builder.ins().jump(merge_blk, &[]);

            builder.switch_to_block(else_blk);
            builder.seal_block(else_blk);
            lower_block(else_block, builder, vars);
            builder.ins().jump(merge_blk, &[]);

            builder.switch_to_block(merge_blk);
            builder.seal_block(merge_blk);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::{resolve, ParserFrontend};

    use super::compile_to_object;
    use crate::link_stub;

    /// Compiles `src` all the way to a linked, runnable executable via the exact
    /// same parse → resolve → lower → `compile_to_object` → `link_stub::link`
    /// pipeline `calcc build --backend=cranelift` uses, runs it, and returns its
    /// exit code — the compiled program's answer, per `define_c_main`'s doc
    /// comment.
    fn compile_and_run(src: &str, out_name: &str) -> i32 {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        resolve(&ast).expect("should resolve");
        let program = calc_ir::lower(&ast);

        let object_bytes = compile_to_object(&program);

        let out_path = std::env::temp_dir().join(format!("calc_a6_test_{out_name}"));
        let exe_path = link_stub::link(&object_bytes, &out_path).expect("link should succeed");

        let status = Command::new(&exe_path)
            .status()
            .expect("built executable should run");
        let _ = std::fs::remove_file(&exe_path);

        status.code().expect("process should exit normally")
    }

    /// The interpreter (A5) is this session's correctness oracle, per spec.md
    /// §8.1/§9.1's "interpreter as semantics reference for codegen backends" —
    /// every case here asserts the Cranelift-compiled executable's exit code
    /// matches what `calc_ir::interpret` computes for the same source.
    fn assert_matches_interpreter(src: &str, out_name: &str) {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        resolve(&ast).expect("should resolve");
        let calc_ir::Value::Number(expected) = calc_ir::interpret(&calc_ir::lower(&ast));

        assert_eq!(compile_and_run(src, out_name), expected as i32);
    }

    #[test]
    fn compiles_and_runs_straight_line_arithmetic() {
        assert_matches_interpreter("2 + 3 * 4", "straight_line");
    }

    #[test]
    fn compiles_and_runs_the_if_branch() {
        assert_matches_interpreter("if 1 { 10 } else { 20 }", "if_branch");
    }

    #[test]
    fn compiles_and_runs_the_else_branch() {
        assert_matches_interpreter("if 0 { 10 } else { 20 }", "else_branch");
    }

    #[test]
    fn compiles_and_runs_a_let_bound_if_expression() {
        assert_matches_interpreter("{ let x = 1; if x { x + 1 } else { 2 } }", "let_bound_if");
    }
}
