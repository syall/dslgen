//! LLVM codegen backend (spec.md §8.1, session A7): lowers the same `calc-ir` IR
//! `cranelift_backend` consumes to a native object file, via the `inkwell` crate's
//! safe wrappers around LLVM's C API. It's the second `Backend` implementation
//! ([`LlvmBackend`], added in A8 — see `backend.rs`); only compiled with
//! `--features backend-llvm` (see `Cargo.toml` and `DECISIONS.md`'s A7 entry). See
//! `calc-lang/docs/a7-llvm-codegen-backend.md` for the full walkthrough.

use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::intrinsics::Intrinsic;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::values::{FloatValue, FunctionValue};
use inkwell::{FloatPredicate, OptimizationLevel};

use calc_ir::{Block as IrBlock, Instr, Program, Temp};

use crate::backend::{Backend, BackendError};

/// The value each `Temp` currently holds. Unlike A6's Cranelift `Variable`s, this
/// is a plain SSA-value table: `Const`/`BinOp`/`Copy` each define one LLVM value,
/// and `If` builds its merge `phi` by hand (see `lower_instr`).
type Values<'ctx> = HashMap<Temp, FloatValue<'ctx>>;

/// Compiles a program into a finished object file's bytes, with the same shape as
/// `cranelift_backend::compile_to_object`: `calc_main() -> double` holds the
/// program, and a C-ABI `main() -> i32` returns its saturating-truncated result as
/// the exit code (see `DECISIONS.md`'s A6 entry for why).
pub fn compile_to_object(program: &Program) -> Vec<u8> {
    let context = Context::create();
    let module = build_module(&context, program);
    let machine = host_machine();
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    machine
        .write_to_memory_buffer(&module, FileType::Object)
        .expect("LLVM can emit an object file")
        .as_slice()
        .to_vec()
}

/// The LLVM [`Backend`]: a unit struct, since it carries no configuration yet.
pub struct LlvmBackend;

impl Backend for LlvmBackend {
    fn name(&self) -> &'static str {
        "llvm"
    }

    fn compile(&self, program: &Program) -> Result<Vec<u8>, BackendError> {
        Ok(compile_to_object(program))
    }
}

/// Builds the module and returns its textual LLVM IR, optionally after LLVM's
/// standard `-O2` pipeline — used by tests and the teaching doc to show what
/// LLVM's optimizer does with the same IR the Cranelift backend compiles as-is.
#[cfg(test)]
fn llvm_ir(program: &Program, optimize: bool) -> String {
    let context = Context::create();
    let module = build_module(&context, program);
    if optimize {
        let machine = host_machine();
        module
            .run_passes(
                "default<O2>",
                &machine,
                inkwell::passes::PassBuilderOptions::create(),
            )
            .expect("standard pipeline is valid");
    }
    module.print_to_string().to_string()
}

fn host_machine() -> TargetMachine {
    // `target-x86` (Cargo.toml) links only x86's LLVM backend, so this initializes
    // exactly that one; other host architectures would need their `target-*` feature.
    Target::initialize_native(&InitializationConfig::default())
        .expect("host target is compiled into LLVM");
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).expect("LLVM knows the host triple");
    target
        .create_target_machine(
            &triple,
            &TargetMachine::get_host_cpu_name().to_string_lossy(),
            &TargetMachine::get_host_cpu_features().to_string_lossy(),
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        )
        .expect("LLVM can create a target machine for the host")
}

fn build_module<'ctx>(context: &'ctx Context, program: &Program) -> Module<'ctx> {
    let module = context.create_module("calc_main");
    let builder = context.create_builder();

    let f64_ty = context.f64_type();
    let calc_main = module.add_function("calc_main", f64_ty.fn_type(&[], false), None);
    define_calc_main(context, &module, &builder, calc_main, program);

    let main = module.add_function("main", context.i32_type().fn_type(&[], false), None);
    define_c_main(context, &module, &builder, main, calc_main);

    // Malformed IR is a bug in this file, not in the user's program; catching it here
    // turns what would otherwise be a crash (or silent miscompile) inside LLVM's
    // backend into a readable message.
    module.verify().expect("generated LLVM IR is well-formed");
    module
}

fn define_calc_main<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    func: FunctionValue<'ctx>,
    program: &Program,
) {
    let entry = context.append_basic_block(func, "entry");
    builder.position_at_end(entry);

    let mut values = Values::new();
    lower_block(context, module, builder, func, &program.body, &mut values);
    builder
        .build_return(Some(&values[&program.result]))
        .expect("builder is positioned in a block");
}

fn lower_block<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    func: FunctionValue<'ctx>,
    block: &IrBlock,
    values: &mut Values<'ctx>,
) {
    for instr in &block.0 {
        lower_instr(context, module, builder, func, instr, values);
    }
}

fn lower_instr<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    func: FunctionValue<'ctx>,
    instr: &Instr,
    values: &mut Values<'ctx>,
) {
    let f64_ty = context.f64_type();
    match instr {
        Instr::Const { dst, value } => {
            values.insert(*dst, f64_ty.const_float(*value));
        }
        Instr::BinOp { dst, op, lhs, rhs } => {
            let (lhs, rhs) = (values[lhs], values[rhs]);
            let result = match op {
                calc_ir::BinOp::Add => builder.build_float_add(lhs, rhs, "add"),
                calc_ir::BinOp::Sub => builder.build_float_sub(lhs, rhs, "sub"),
                calc_ir::BinOp::Mul => builder.build_float_mul(lhs, rhs, "mul"),
                calc_ir::BinOp::Div => builder.build_float_div(lhs, rhs, "div"),
            }
            .expect("builder is positioned in a block");
            values.insert(*dst, result);
        }
        Instr::CallBuiltin { dst, name, args } => {
            // Declaring (not defining) the function leaves an undefined symbol for the
            // linker to resolve from the runtime library (spec.md §7).
            let builtin =
                calc_runtime::lookup(name).unwrap_or_else(|| panic!("unknown built-in `{name}`"));
            let callee = module.get_function(builtin.symbol).unwrap_or_else(|| {
                let params = vec![f64_ty.into(); builtin.arity];
                module.add_function(builtin.symbol, f64_ty.fn_type(&params, false), None)
            });
            let call_args: Vec<_> = args.iter().map(|a| values[a].into()).collect();
            let result = builder
                .build_call(callee, &call_args, builtin.name)
                .expect("builder is positioned in a block")
                .try_as_basic_value()
                .unwrap_basic()
                .into_float_value();
            values.insert(*dst, result);
        }
        Instr::Copy { dst, src } => {
            values.insert(*dst, values[src]);
        }
        Instr::If {
            dst,
            cond,
            then_block,
            else_block,
        } => {
            // `une` ("unordered or not equal") is LLVM's spelling of A6's
            // `FloatCC::NotEqual`: true for NaN, matching Rust's `x != 0.0`.
            let is_truthy = builder
                .build_float_compare(
                    FloatPredicate::UNE,
                    values[cond],
                    f64_ty.const_zero(),
                    "truthy",
                )
                .expect("builder is positioned in a block");

            let then_blk = context.append_basic_block(func, "then");
            let else_blk = context.append_basic_block(func, "else");
            let merge_blk = context.append_basic_block(func, "merge");
            builder
                .build_conditional_branch(is_truthy, then_blk, else_blk)
                .expect("builder is positioned in a block");

            // Each branch lowers against its own copy of the value table: temps
            // defined inside one branch don't dominate the other, so they must not
            // leak across. Only `dst` escapes, via the phi below.
            let branch_value = |blk, ir_block: &IrBlock| {
                builder.position_at_end(blk);
                let mut branch_values = values.clone();
                lower_block(context, module, builder, func, ir_block, &mut branch_values);
                builder
                    .build_unconditional_branch(merge_blk)
                    .expect("builder is positioned in a block");
                // Nested `if`s move the insertion point into their own merge block,
                // so the phi's predecessor is wherever lowering *ended*, not `blk`.
                (
                    branch_values[dst],
                    builder.get_insert_block().expect("builder has a block"),
                )
            };
            let (then_val, then_end) = branch_value(then_blk, then_block);
            let (else_val, else_end) = branch_value(else_blk, else_block);

            builder.position_at_end(merge_blk);
            let phi = builder
                .build_phi(f64_ty, "if_result")
                .expect("builder is positioned in a block");
            phi.add_incoming(&[(&then_val, then_end), (&else_val, else_end)]);
            values.insert(*dst, phi.as_basic_value().into_float_value());
        }
    }
}

/// `main() -> i32`: calls `calc_main` and converts its `double` to `i32` with the
/// `llvm.fptosi.sat` intrinsic — the saturating conversion A6 gets from Cranelift's
/// `fcvt_to_sint_sat` (a plain `fptosi` is poison on out-of-range input).
fn define_c_main<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    main: FunctionValue<'ctx>,
    calc_main: FunctionValue<'ctx>,
) {
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);

    let result = builder
        .build_call(calc_main, &[], "result")
        .expect("builder is positioned in a block")
        .try_as_basic_value()
        .unwrap_basic();

    let (i32_ty, f64_ty) = (context.i32_type(), context.f64_type());
    let fptosi_sat = Intrinsic::find("llvm.fptosi.sat").expect("intrinsic exists");
    let convert = fptosi_sat
        .get_declaration(module, &[i32_ty.into(), f64_ty.into()])
        .expect("intrinsic is overloaded on (i32, f64)");
    let exit_code = builder
        .build_call(convert, &[result.into()], "exit_code")
        .expect("builder is positioned in a block")
        .try_as_basic_value()
        .unwrap_basic();
    builder
        .build_return(Some(&exit_code))
        .expect("builder is positioned in a block");
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::{resolve, ParserFrontend};

    use super::{compile_to_object, llvm_ir};
    #[cfg(feature = "backend-cranelift")]
    use crate::cranelift_backend;
    use crate::link_stub;

    fn lower_source(src: &str) -> calc_ir::Program {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        resolve(&ast).expect("should resolve");
        calc_ir::lower(&ast)
    }

    /// Links `object_bytes` and runs the result, returning its exit code.
    fn link_and_run(object_bytes: &[u8], out_name: &str) -> i32 {
        let out_path = std::env::temp_dir().join(format!("calc_a7_test_{out_name}"));
        let exe_path = link_stub::link(object_bytes, &out_path).expect("link should succeed");
        let status = Command::new(&exe_path)
            .status()
            .expect("built executable should run");
        let _ = std::fs::remove_file(&exe_path);
        status.code().expect("process should exit normally")
    }

    /// Same oracle as A6: the interpreter's answer — and, when the Cranelift backend
    /// is also compiled in, its exit code for the same program — must equal LLVM's.
    fn assert_matches_interpreter_and_cranelift(src: &str, out_name: &str) {
        let program = lower_source(src);
        let calc_ir::Value::Number(expected) = calc_ir::interpret(&program);

        let llvm = link_and_run(&compile_to_object(&program), &format!("llvm_{out_name}"));
        assert_eq!(llvm, expected as i32);

        #[cfg(feature = "backend-cranelift")]
        {
            let clif = link_and_run(
                &cranelift_backend::compile_to_object(&program),
                &format!("clif_{out_name}"),
            );
            assert_eq!(llvm, clif);
        }
    }

    #[test]
    fn compiles_and_runs_straight_line_arithmetic() {
        assert_matches_interpreter_and_cranelift("2 + 3 * 4", "straight_line");
    }

    #[test]
    fn compiles_and_runs_the_if_branch() {
        assert_matches_interpreter_and_cranelift("if 1 { 10 } else { 20 }", "if_branch");
    }

    #[test]
    fn compiles_and_runs_the_else_branch() {
        assert_matches_interpreter_and_cranelift("if 0 { 10 } else { 20 }", "else_branch");
    }

    #[test]
    fn compiles_and_runs_a_let_bound_if_expression() {
        assert_matches_interpreter_and_cranelift(
            "{ let x = 1; if x { x + 1 } else { 2 } }",
            "let_bound_if",
        );
    }

    /// `+` and `*` compile to calls of the `add`/`mul` built-ins (A9); `-` stays inline.
    #[test]
    fn compiles_and_runs_builtin_calls_mixed_with_inline_ops() {
        assert_matches_interpreter_and_cranelift("(1 + 2) * 4 - 3", "builtins_mixed");
    }

    /// The nested `if` moves the insertion point into an inner merge block, so the
    /// outer phi must name that block (not `then`) as its predecessor — LLVM's
    /// verifier rejects the module otherwise.
    #[test]
    fn compiles_and_runs_a_nested_if() {
        assert_matches_interpreter_and_cranelift(
            "if 1 { if 0 { 1 } else { 2 } } else { 3 }",
            "nested_if",
        );
    }

    #[test]
    fn if_lowers_to_a_phi_node() {
        let ir = llvm_ir(&lower_source("if 1 { 10 } else { 20 }"), false);
        assert!(ir.contains("phi double"), "no phi in:\n{ir}");
    }

    /// What `default<O2>` buys: at `-O0` the constant-condition `if` is still a
    /// branch feeding a `phi`; the optimizer folds all of it into a plain `ret`.
    /// (Straight-line arithmetic isn't a good demo: `inkwell`'s builder already
    /// constant-folds `2 + 3 * 4` while emitting it.)
    #[test]
    fn optimizer_collapses_a_constant_if() {
        let program = lower_source("if 1 { 10 } else { 20 }");
        let unoptimized = llvm_ir(&program, false);
        assert!(unoptimized.contains("phi") && unoptimized.contains("br i1"));

        let optimized = llvm_ir(&program, true);
        assert!(
            !optimized.contains("phi") && !optimized.contains("br "),
            "{optimized}"
        );
        assert!(optimized.contains("ret double 1.000000e+01"), "{optimized}");
    }
}
