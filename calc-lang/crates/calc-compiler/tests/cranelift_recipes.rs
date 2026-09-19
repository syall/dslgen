//! The worked examples from `docs/a6-cranelift-codegen-backend.md`'s "recipes"
//! section, built as tests so those examples can't silently drift from working code.
//! `define_function` runs Cranelift's verifier, so a malformed recipe fails here.
//!
//! An integration test (Cargo builds everything in `tests/` as its own test-only
//! binary), so it needs no `mod` declaration. It uses only Cranelift, and
//! `new_object_module` below is deliberately a copy of the private one in
//! `src/cranelift_backend.rs`, so a reader can follow a recipe start to finish
//! without leaving this file.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

fn new_object_module() -> ObjectModule {
    let isa_builder =
        cranelift_native::builder().expect("host architecture is supported by cranelift-native");
    let mut flag_builder = settings::builder();
    flag_builder.set("is_pic", "false").expect("valid setting");
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .expect("host ISA settings are valid");

    let object_builder = ObjectBuilder::new(isa, "recipes", default_libcall_names())
        .expect("object builder configuration is valid");
    ObjectModule::new(object_builder)
}

fn f64_sig(module: &ObjectModule, params: usize) -> Signature {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.params.extend(vec![AbiParam::new(types::F64); params]);
    sig.returns.push(AbiParam::new(types::F64));
    sig
}

#[test]
fn recipe_1_function_with_a_parameter() {
    let mut module = new_object_module();
    let sig = f64_sig(&module, 1);
    let id = module
        .declare_function("twice", Linkage::Export, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func.signature = sig;
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);

    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);

    let x = b.block_params(entry)[0];
    let doubled = b.ins().fadd(x, x);
    b.ins().return_(&[doubled]);
    b.finalize(module.isa().frontend_config());

    let clif = ctx.func.display().to_string();
    println!("{clif}");
    module.define_function(id, &mut ctx).unwrap();
    assert!(clif.contains("fadd v0, v0"));
}

#[test]
fn recipe_2_if_else_producing_a_value() {
    let mut module = new_object_module();
    let sig = f64_sig(&module, 3);
    let id = module
        .declare_function("select", Linkage::Export, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func.signature = sig;
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);

    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let (c, x, y) = {
        let p = b.block_params(entry);
        (p[0], p[1], p[2])
    };

    let result = b.declare_var(types::F64);
    let zero = b.ins().f64const(0.0);
    let truthy = b.ins().fcmp(FloatCC::NotEqual, c, zero);

    let then_blk = b.create_block();
    let else_blk = b.create_block();
    let merge_blk = b.create_block();
    b.ins().brif(truthy, then_blk, &[], else_blk, &[]);

    b.switch_to_block(then_blk);
    b.seal_block(then_blk);
    b.def_var(result, x);
    b.ins().jump(merge_blk, &[]);

    b.switch_to_block(else_blk);
    b.seal_block(else_blk);
    b.def_var(result, y);
    b.ins().jump(merge_blk, &[]);

    b.switch_to_block(merge_blk);
    b.seal_block(merge_blk);
    let r = b.use_var(result);
    b.ins().return_(&[r]);
    b.finalize(module.isa().frontend_config());

    let clif = ctx.func.display().to_string();
    println!("{clif}");
    module.define_function(id, &mut ctx).unwrap();
    // `use_var` at the merge point made Cranelift give `block3` a parameter: the phi.
    assert!(clif.contains("block3(v"));
}

#[test]
fn recipe_3_one_function_calling_another() {
    let mut module = new_object_module();
    let callee_sig = f64_sig(&module, 0);
    let callee = module
        .declare_function("five", Linkage::Export, &callee_sig)
        .unwrap();
    let caller = module
        .declare_function("caller", Linkage::Export, &callee_sig)
        .unwrap();

    // Define `five` first so the call below has something to link against.
    let mut ctx = Context::new();
    ctx.func.signature = callee_sig.clone();
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
    let entry = b.create_block();
    b.switch_to_block(entry);
    b.seal_block(entry);
    let five = b.ins().f64const(5.0);
    b.ins().return_(&[five]);
    b.finalize(module.isa().frontend_config());
    module.define_function(callee, &mut ctx).unwrap();
    module.clear_context(&mut ctx);

    ctx.func.signature = callee_sig;
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
    let callee_ref = module.declare_func_in_func(callee, b.func);
    let entry = b.create_block();
    b.switch_to_block(entry);
    b.seal_block(entry);
    let call = b.ins().call(callee_ref, &[]);
    let result = b.inst_results(call)[0];
    b.ins().return_(&[result]);
    b.finalize(module.isa().frontend_config());

    let clif = ctx.func.display().to_string();
    println!("{clif}");
    module.define_function(caller, &mut ctx).unwrap();
    assert!(clif.contains("call fn0()"));
}
