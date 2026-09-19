//! The worked examples from `docs/a7-llvm-codegen-backend.md`'s "recipes" section,
//! built as tests so those examples can't silently drift from working code.
//! `module.verify()` runs LLVM's verifier, so a malformed recipe fails here.
//!
//! An integration test (Cargo builds everything in `tests/` as its own test-only
//! binary), so it needs no `mod` declaration. It uses only `inkwell`, and
//! `host_machine` below is deliberately a copy of the private one in
//! `src/llvm_backend.rs`, so a reader can follow a recipe start to finish without
//! leaving this file. Only built with `--features backend-llvm`.
#![cfg(feature = "backend-llvm")]

use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::{FloatPredicate, OptimizationLevel};

fn host_machine() -> TargetMachine {
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

#[test]
fn recipe_1_function_with_a_parameter() {
    let context = Context::create();
    let module = context.create_module("recipes");
    let builder = context.create_builder();
    let f64_ty = context.f64_type();

    let fn_ty = f64_ty.fn_type(&[f64_ty.into()], false);
    let func = module.add_function("twice", fn_ty, None);

    let entry = context.append_basic_block(func, "entry");
    builder.position_at_end(entry);

    let x = func.get_nth_param(0).unwrap().into_float_value();
    let doubled = builder.build_float_add(x, x, "doubled").unwrap();
    builder.build_return(Some(&doubled)).unwrap(); // the terminator

    module.verify().unwrap();
    let ir = module.print_to_string().to_string();
    println!("{ir}");
    assert!(ir.contains("%doubled = fadd double %0, %0"));
}

#[test]
fn recipe_2_if_else_producing_a_value() {
    let context = Context::create();
    let module = context.create_module("recipes");
    let builder = context.create_builder();
    let f64_ty = context.f64_type();

    let fn_ty = f64_ty.fn_type(&[f64_ty.into(), f64_ty.into(), f64_ty.into()], false);
    let func = module.add_function("select", fn_ty, None);
    let entry = context.append_basic_block(func, "entry");
    builder.position_at_end(entry);
    let c = func.get_nth_param(0).unwrap().into_float_value();
    let x = func.get_nth_param(1).unwrap().into_float_value();
    let y = func.get_nth_param(2).unwrap().into_float_value();
    c.set_name("c");
    x.set_name("x");
    y.set_name("y");

    let truthy = builder
        .build_float_compare(FloatPredicate::UNE, c, f64_ty.const_zero(), "truthy")
        .unwrap();

    let then_blk = context.append_basic_block(func, "then");
    let else_blk = context.append_basic_block(func, "else");
    let merge_blk = context.append_basic_block(func, "merge");
    builder
        .build_conditional_branch(truthy, then_blk, else_blk)
        .unwrap();

    builder.position_at_end(then_blk);
    builder.build_unconditional_branch(merge_blk).unwrap();

    builder.position_at_end(else_blk);
    builder.build_unconditional_branch(merge_blk).unwrap();

    builder.position_at_end(merge_blk);
    let phi = builder.build_phi(f64_ty, "result").unwrap();
    phi.add_incoming(&[(&x, then_blk), (&y, else_blk)]);
    builder.build_return(Some(&phi.as_basic_value())).unwrap();

    module.verify().unwrap();
    let ir = module.print_to_string().to_string();
    println!("{ir}");
    assert!(ir.contains("%result = phi double [ %x, %then ], [ %y, %else ]"));
}

#[test]
fn recipe_3_one_function_calling_another() {
    let context = Context::create();
    let module = context.create_module("recipes");
    let builder = context.create_builder();
    let f64_ty = context.f64_type();
    let fn_ty = f64_ty.fn_type(&[], false);

    // Define `five` first so the call below has a function to refer to.
    let five = module.add_function("five", fn_ty, None);
    let entry = context.append_basic_block(five, "entry");
    builder.position_at_end(entry);
    builder
        .build_return(Some(&f64_ty.const_float(5.0)))
        .unwrap();

    let caller = module.add_function("caller", fn_ty, None);
    let entry = context.append_basic_block(caller, "entry");
    builder.position_at_end(entry);
    let result = builder
        .build_call(five, &[], "result")
        .unwrap()
        .try_as_basic_value()
        .unwrap_basic(); // a call could return `void`, so its result is an enum
    builder.build_return(Some(&result)).unwrap();

    module.verify().unwrap();
    let ir = module.print_to_string().to_string();
    println!("{ir}");
    assert!(ir.contains("%result = call double @five()"));
}

#[test]
fn recipe_4_module_to_object_file() {
    let context = Context::create();
    let module = context.create_module("recipes");
    let builder = context.create_builder();
    let f64_ty = context.f64_type();

    let func = module.add_function("five", f64_ty.fn_type(&[], false), None);
    let entry = context.append_basic_block(func, "entry");
    builder.position_at_end(entry);
    builder
        .build_return(Some(&f64_ty.const_float(5.0)))
        .unwrap();
    module.verify().unwrap();

    let machine = host_machine();
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    let object = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .unwrap();
    println!("object file: {} bytes", object.as_slice().len());
    assert!(!object.as_slice().is_empty());
}

/// Each test below makes one deliberate mistake and records how LLVM/`inkwell`
/// responds; the doc's "What LLVM enforces" table quotes these messages.
mod mistakes {
    use super::*;

    /// A `double @f(double)` with an `entry` block, ready for a body.
    fn start<'ctx>(
        context: &'ctx Context,
        module: &inkwell::module::Module<'ctx>,
    ) -> (
        inkwell::values::FunctionValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        let f64_ty = context.f64_type();
        let func = module.add_function("f", f64_ty.fn_type(&[f64_ty.into()], false), None);
        (func, context.append_basic_block(func, "entry"))
    }

    fn verify_error(module: &inkwell::module::Module<'_>) -> String {
        let msg = module
            .verify()
            .expect_err("verifier should reject")
            .to_string();
        println!("{msg}");
        msg
    }

    #[test]
    fn block_without_a_terminator() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (func, entry) = start(&context, &module);
        builder.position_at_end(entry);
        let x = func.get_nth_param(0).unwrap().into_float_value();
        builder.build_float_add(x, x, "sum").unwrap();
        assert!(verify_error(&module).contains("does not have terminator"));
    }

    #[test]
    fn instruction_after_the_terminator() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (func, entry) = start(&context, &module);
        builder.position_at_end(entry);
        let x = func.get_nth_param(0).unwrap().into_float_value();
        builder.build_return(Some(&x)).unwrap();
        builder.build_float_add(x, x, "late").unwrap();
        // The late `fadd` is now the block's last instruction, so LLVM reports the
        // same "no terminator" error rather than a distinct "instruction after".
        assert!(verify_error(&module).contains("does not have terminator"));
    }

    #[test]
    fn build_before_positioning_the_builder() {
        let context = Context::create();
        let builder = context.create_builder();
        let f64_ty = context.f64_type();
        let err = builder
            .build_float_add(f64_ty.const_zero(), f64_ty.const_zero(), "x")
            .expect_err("no insertion point yet");
        println!("{err}");
        assert!(matches!(err, inkwell::builder::BuilderError::UnsetPosition));
    }

    #[test]
    fn returning_the_wrong_type() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (_, entry) = start(&context, &module);
        builder.position_at_end(entry);
        builder
            .build_return(Some(&context.i32_type().const_zero()))
            .unwrap();
        assert!(verify_error(&module).contains("Function return type does not match"));
    }

    #[test]
    fn phi_naming_a_block_that_is_not_a_predecessor() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (func, entry) = start(&context, &module);
        let merge = context.append_basic_block(func, "merge");
        let x = func.get_nth_param(0).unwrap().into_float_value();
        builder.position_at_end(entry);
        builder.build_unconditional_branch(merge).unwrap();
        builder.position_at_end(merge);
        let phi = builder.build_phi(context.f64_type(), "p").unwrap();
        // `merge`'s only predecessor is `entry`, but the phi names `merge` itself.
        phi.add_incoming(&[(&x, merge)]);
        builder.build_return(Some(&phi.as_basic_value())).unwrap();
        assert!(verify_error(&module).contains("PHI node entries do not match predecessors"));
    }

    #[test]
    fn phi_after_a_non_phi_instruction() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (func, entry) = start(&context, &module);
        let merge = context.append_basic_block(func, "merge");
        let x = func.get_nth_param(0).unwrap().into_float_value();
        builder.position_at_end(entry);
        builder.build_unconditional_branch(merge).unwrap();
        builder.position_at_end(merge);
        builder.build_float_add(x, x, "sum").unwrap();
        let phi = builder.build_phi(context.f64_type(), "p").unwrap();
        phi.add_incoming(&[(&x, entry)]);
        builder.build_return(Some(&phi.as_basic_value())).unwrap();
        assert!(verify_error(&module).contains("PHI nodes not grouped at top"));
    }

    #[test]
    fn using_a_value_that_does_not_dominate_its_use() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let (func, entry) = start(&context, &module);
        let then_blk = context.append_basic_block(func, "then");
        let merge = context.append_basic_block(func, "merge");
        let x = func.get_nth_param(0).unwrap().into_float_value();
        let cond = {
            builder.position_at_end(entry);
            builder
                .build_float_compare(FloatPredicate::UNE, x, x, "c")
                .unwrap()
        };
        builder
            .build_conditional_branch(cond, then_blk, merge)
            .unwrap();
        builder.position_at_end(then_blk);
        let sum = builder.build_float_add(x, x, "sum").unwrap(); // defined only on the `then` path
        builder.build_unconditional_branch(merge).unwrap();
        builder.position_at_end(merge);
        builder.build_return(Some(&sum)).unwrap(); // ...but used where `entry` also reaches
        assert!(verify_error(&module).contains("Instruction does not dominate all uses"));
    }
}
