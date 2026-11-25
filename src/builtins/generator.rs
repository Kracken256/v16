use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

pub fn register_builtins(_ctx: &mut JSContext) {}

pub fn init_generator(ctx: &mut JSContext) {
    let mut gen_proto = JSObject::new();

    ctx.register_builtin(
        "generator_next",
        HostFunction::method("next", 1, generator_next),
    );

    let next_func = create_builtin_function(ctx, "generator_next");

    let next_atom = ctx.intern("next");
    gen_proto.set(next_atom, next_func);

    ctx.register_builtin(
        "generator_return",
        HostFunction::method("return", 1, generator_return),
    );
    let return_func = create_builtin_function(ctx, "generator_return");
    let return_atom = ctx.intern("return");
    gen_proto.set(return_atom, return_func);

    ctx.register_builtin(
        "generator_throw",
        HostFunction::method("throw", 1, generator_throw),
    );
    let throw_func = create_builtin_function(ctx, "generator_throw");
    gen_proto.set(ctx.intern("throw"), throw_func);

    // Generator.prototype[Symbol.iterator] = function() { return this; }
    ctx.register_builtin(
        "generator_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, generator_symbol_iterator),
    );
    let sym_iter_func = create_builtin_function(ctx, "generator_symbol_iterator");
    let sym_iter_val = crate::builtins::symbol::get_symbol_iterator(ctx);
    if sym_iter_val.is_symbol() {
        let sym_atom = crate::runtime::atom::Atom(0x40000000 | sym_iter_val.get_symbol_id());
        gen_proto.set(sym_atom, sym_iter_func);
    }

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        gen_proto.prototype = Some(obj_proto_ptr);
    }

    let proto_ptr = Box::into_raw(Box::new(gen_proto)) as usize;
    ctx.set_generator_prototype(proto_ptr);

    let mut async_gen_proto = JSObject::new();

    ctx.register_builtin(
        "async_generator_next",
        HostFunction::method("next", 1, async_generator_next),
    );
    let async_next_func = create_builtin_function(ctx, "async_generator_next");
    async_gen_proto.set(next_atom, async_next_func);

    ctx.register_builtin(
        "async_generator_return",
        HostFunction::method("return", 1, async_generator_return),
    );
    let async_return_func = create_builtin_function(ctx, "async_generator_return");
    async_gen_proto.set(return_atom, async_return_func);

    ctx.register_builtin(
        "async_generator_throw",
        HostFunction::method("throw", 1, async_generator_throw),
    );
    let async_throw_func = create_builtin_function(ctx, "async_generator_throw");
    async_gen_proto.set(ctx.intern("throw"), async_throw_func);

    ctx.register_builtin(
        "async_generator_symbol_async_iterator",
        HostFunction::method("[Symbol.asyncIterator]", 0, generator_symbol_iterator),
    );
    let async_sym_iter_func =
        create_builtin_function(ctx, "async_generator_symbol_async_iterator");
    let async_sym_iter_val = crate::builtins::symbol::get_symbol_async_iterator(ctx);
    if async_sym_iter_val.is_symbol() {
        let sym_atom =
            crate::runtime::atom::Atom(0x40000000 | async_sym_iter_val.get_symbol_id());
        async_gen_proto.set(sym_atom, async_sym_iter_func);
    }

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        async_gen_proto.prototype = Some(obj_proto_ptr);
    }

    let async_proto_ptr = Box::into_raw(Box::new(async_gen_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(async_proto_ptr);
    ctx.set_async_generator_prototype(async_proto_ptr);
}

fn create_builtin_function(ctx: &mut JSContext, builtin_name: &str) -> JSValue {
    let name_atom = ctx.intern(builtin_name);
    let mut func = JSFunction::new_builtin(name_atom, 1);
    func.set_builtin_marker(ctx, builtin_name);
    let proto_atom = ctx.intern("prototype");
    let proto_obj = JSObject::new();
    let proto_ptr = Box::into_raw(Box::new(proto_obj)) as usize;
    func.base.set(proto_atom, JSValue::new_object(proto_ptr));

    let ptr = Box::into_raw(Box::new(func)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    JSValue::new_function(ptr)
}

const GENERATOR_RESUME_NEXT: u8 = 0;
const GENERATOR_RESUME_THROW: u8 = 1;
const GENERATOR_RESUME_RETURN: u8 = 2;

fn run_generator_step(
    ctx: &mut JSContext,
    gen_obj: &mut JSObject,
    resume_kind: u8,
    resume_value: JSValue,
) -> Result<(JSValue, usize, bool, bool), (String, Option<JSValue>)> {
    let state = match gen_obj.get_generator_state() {
        Some(s) => s,
        None => return Ok((JSValue::undefined(), 0, true, false)),
    };

    if state.done {
        return Ok((JSValue::undefined(), state.pc, true, false));
    }

    let current_pc = state.pc;
    let snapshot_clone = state.snapshot.clone();
    let current_send_reg = state.send_reg;
    let saved_args = state.saved_args.clone();
    let func_ptr = state.func_ptr;
    let handlers = state.handlers.clone();
    let yield_is_result = state.yield_is_result;

    let (value, new_snapshot, new_pc, done, new_send_reg, new_handlers, new_yield_is_result) = {
        let bytecode = &*gen_obj.get_generator_state().unwrap().bytecode;
        let mut vm = crate::runtime::vm::VM::new();

        ctx.runtime_mut().gc_heap_mut().gc_suppressed = true;
        let result = vm.execute_generator_step(
            ctx,
            bytecode,
            &snapshot_clone,
            current_pc,
            resume_value,
            current_send_reg,
            &saved_args,
            func_ptr,
            &handlers,
            resume_kind,
            resume_value,
            yield_is_result,
        );
        let uncaught = vm.last_uncaught;
        ctx.runtime_mut().gc_heap_mut().gc_suppressed = false;
        match result {
            Ok(v) => v,
            Err(e) => return Err((e, uncaught)),
        }
    };

    if let Some(state) = gen_obj.get_generator_state_mut() {
        state.snapshot = new_snapshot;
        state.pc = new_pc;
        state.done = done;
        state.send_reg = new_send_reg;
        state.handlers = new_handlers;
        state.yield_is_result = new_yield_is_result;
    }

    Ok((value, new_pc, done, new_yield_is_result))
}

fn generator_next(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if !this.is_object() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let gen_obj = this.as_object_mut();

    if !gen_obj.is_generator() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let is_done = gen_obj
        .get_generator_state()
        .map(|s| s.done)
        .unwrap_or(true);

    if is_done {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let send_value = args.get(1).copied().unwrap_or(JSValue::undefined());

    let step = run_generator_step(ctx, gen_obj, GENERATOR_RESUME_NEXT, send_value);
    finish_generator_step(ctx, gen_obj, step)
}

fn finish_generator_step(
    ctx: &mut JSContext,
    gen_obj: &mut JSObject,
    step: Result<(JSValue, usize, bool, bool), (String, Option<JSValue>)>,
) -> JSValue {
    match step {
        Ok((value, _new_pc, done, yield_is_result)) => {
            if yield_is_result && !done {
                value
            } else {
                create_iterator_result(ctx, value, done)
            }
        }
        Err((msg, exc)) => {
            if let Some(state) = gen_obj.get_generator_state_mut() {
                state.done = true;
            }
            let thrown = exc.unwrap_or_else(|| {
                let mut err = JSObject::new();
                err.set(ctx.intern("name"), JSValue::new_string(ctx.intern("Error")));
                err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&msg)));
                let ptr = Box::into_raw(Box::new(err)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(ptr);
                JSValue::new_object(ptr)
            });
            ctx.pending_exception = Some(thrown);
            create_iterator_result(ctx, JSValue::undefined(), true)
        }
    }
}

fn generator_return(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if !this.is_object() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let gen_obj = this.as_object_mut();

    if !gen_obj.is_generator() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let value = args.get(1).copied().unwrap_or(JSValue::undefined());
    let is_done = gen_obj
        .get_generator_state()
        .map(|s| s.done)
        .unwrap_or(true);

    if is_done {
        return create_iterator_result(ctx, value, true);
    }

    let step = run_generator_step(ctx, gen_obj, GENERATOR_RESUME_RETURN, value);
    finish_generator_step(ctx, gen_obj, step)
}

fn generator_throw(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if !this.is_object() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let gen_obj = this.as_object_mut();

    if !gen_obj.is_generator() {
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let value = args.get(1).copied().unwrap_or(JSValue::undefined());
    let is_done = gen_obj
        .get_generator_state()
        .map(|s| s.done)
        .unwrap_or(true);

    if is_done {
        ctx.pending_exception = Some(value);
        return create_iterator_result(ctx, JSValue::undefined(), true);
    }

    let step = run_generator_step(ctx, gen_obj, GENERATOR_RESUME_THROW, value);
    finish_generator_step(ctx, gen_obj, step)
}

fn generator_symbol_iterator(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    args.get(0).copied().unwrap_or(JSValue::undefined())
}

fn async_generator_next(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if !this.is_object() {
        return ctx.call_builtin("promise_reject", &[JSValue::undefined()]);
    }

    let gen_obj = this.as_object_mut();

    if !gen_obj.is_generator() {
        let result = create_iterator_result(ctx, JSValue::undefined(), true);
        return ctx.call_builtin("promise_resolve", &[result]);
    }

    let is_done = gen_obj
        .get_generator_state()
        .map(|s| s.done)
        .unwrap_or(true);

    if is_done {
        let result = create_iterator_result(ctx, JSValue::undefined(), true);
        return ctx.call_builtin("promise_resolve", &[result]);
    }

    let send_value = args.get(1).copied().unwrap_or(JSValue::undefined());

    match run_generator_step(ctx, gen_obj, GENERATOR_RESUME_NEXT, send_value) {
        Ok((value, _new_pc, done, yield_is_result)) => {
            if done {
                let iter_result = create_iterator_result(ctx, value, true);
                return ctx.call_builtin("promise_resolve", &[iter_result]);
            }
            if yield_is_result {
                let iter_result = value;
                return ctx.call_builtin("promise_resolve", &[iter_result]);
            }
            if crate::builtins::promise::is_promise(&value) {
                let ptr = value.get_ptr();
                let promise_obj = unsafe { &*(ptr as *mut JSObject) };
                let state_atom = ctx.common_atoms.__promise_state__;
                let result_atom = ctx.common_atoms.__promise_result__;
                let state = promise_obj
                    .get(state_atom)
                    .unwrap_or(JSValue::new_int(0))
                    .get_int();
                let result = promise_obj.get(result_atom).unwrap_or(JSValue::undefined());
                match state {
                    1 => {
                        let iter_result = create_iterator_result(ctx, result, false);
                        ctx.call_builtin("promise_resolve", &[iter_result])
                    }
                    2 => {
                        if let Some(state) = gen_obj.get_generator_state_mut() {
                            state.done = true;
                        }
                        ctx.call_builtin("promise_reject", &[result])
                    }
                    _ => {
                        let iter_result = create_iterator_result(ctx, value, false);
                        ctx.call_builtin("promise_resolve", &[iter_result])
                    }
                }
            } else {
                let iter_result = create_iterator_result(ctx, value, false);
                ctx.call_builtin("promise_resolve", &[iter_result])
            }
        }
        Err((e, exc)) => {
            if let Some(state) = gen_obj.get_generator_state_mut() {
                state.done = true;
            }
            let err_val = match exc {
                Some(v) => v,
                None => JSValue::new_string(ctx.intern(&e)),
            };
            ctx.call_builtin("promise_reject", &[err_val])
        }
    }
}

fn async_generator_return(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if !this.is_object() {
        return ctx.call_builtin("promise_resolve", &[JSValue::undefined()]);
    }

    let gen_obj = this.as_object_mut();

    if let Some(state) = gen_obj.get_generator_state_mut() {
        state.done = true;
    }

    let value = args.get(1).copied().unwrap_or(JSValue::undefined());
    let result = create_iterator_result(ctx, value, true);
    ctx.call_builtin("promise_resolve", &[result])
}

fn async_generator_throw(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.get(0).copied().unwrap_or(JSValue::undefined());

    if this.is_object() {
        let gen_obj = this.as_object_mut();
        if let Some(state) = gen_obj.get_generator_state_mut() {
            state.done = true;
        }
    }

    let value = args.get(1).copied().unwrap_or(JSValue::undefined());
    ctx.call_builtin("promise_reject", &[value])
}

fn create_iterator_result(ctx: &mut JSContext, value: JSValue, done: bool) -> JSValue {
    let mut result = JSObject::new();
    result.set(ctx.intern("value"), value);
    result.set(ctx.intern("done"), JSValue::bool(done));

    let ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(ptr)
}
