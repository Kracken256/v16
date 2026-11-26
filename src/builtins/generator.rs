use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

pub fn register_builtins(_ctx: &mut JSContext) {}

pub fn init_generator(ctx: &mut JSContext) {
    let sym_iter_val = crate::builtins::symbol::get_symbol_iterator(ctx);
    let sym_iter_atom = if sym_iter_val.is_symbol() {
        Some(crate::runtime::atom::Atom(
            0x40000000 | sym_iter_val.get_symbol_id(),
        ))
    } else {
        None
    };
    let sym_async_iter_val = crate::builtins::symbol::get_symbol_async_iterator(ctx);
    let sym_async_iter_atom = if sym_async_iter_val.is_symbol() {
        Some(crate::runtime::atom::Atom(
            0x40000000 | sym_async_iter_val.get_symbol_id(),
        ))
    } else {
        None
    };
    let to_string_tag_atom = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);

    ctx.register_builtin(
        "iterator_prototype_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, generator_symbol_iterator),
    );
    let mut iterator_proto = JSObject::new();
    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        iterator_proto.prototype = Some(obj_proto_ptr);
    }
    if let Some(sym_atom) = sym_iter_atom {
        let method = create_builtin_function(ctx, "iterator_prototype_symbol_iterator");
        define_data_property(&mut iterator_proto, sym_atom, method, true, true);
    }
    define_data_property(
        &mut iterator_proto,
        to_string_tag_atom,
        JSValue::new_string(ctx.intern("Iterator")),
        false,
        true,
    );
    let iterator_proto_ptr = Box::into_raw(Box::new(iterator_proto));
    ctx.runtime_mut().gc_heap_mut().track(iterator_proto_ptr);

    ctx.register_builtin(
        "async_iterator_prototype_symbol_async_iterator",
        HostFunction::method("[Symbol.asyncIterator]", 0, generator_symbol_iterator),
    );
    let mut async_iterator_proto = JSObject::new();
    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        async_iterator_proto.prototype = Some(obj_proto_ptr);
    }
    if let Some(sym_atom) = sym_async_iter_atom {
        let method =
            create_builtin_function(ctx, "async_iterator_prototype_symbol_async_iterator");
        define_data_property(&mut async_iterator_proto, sym_atom, method, true, true);
    }
    let async_iterator_proto_ptr = Box::into_raw(Box::new(async_iterator_proto));
    ctx.runtime_mut().gc_heap_mut().track(async_iterator_proto_ptr);

    ctx.register_builtin(
        "generator_function_constructor",
        HostFunction::ctor("GeneratorFunction", 1, generator_function_constructor),
    );
    ctx.register_builtin(
        "async_generator_function_constructor",
        HostFunction::ctor("AsyncGeneratorFunction", 1, async_generator_function_constructor),
    );
    let generator_function =
        create_generator_constructor(ctx, "GeneratorFunction", "generator_function_constructor");
    let async_generator_function = create_generator_constructor(
        ctx,
        "AsyncGeneratorFunction",
        "async_generator_function_constructor",
    );

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

    ctx.register_builtin(
        "generator_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, generator_symbol_iterator),
    );
    let sym_iter_func = create_builtin_function(ctx, "generator_symbol_iterator");
    if let Some(sym_atom) = sym_iter_atom {
        gen_proto.set(sym_atom, sym_iter_func);
    }

    define_data_property(
        &mut gen_proto,
        to_string_tag_atom,
        JSValue::new_string(ctx.intern("Generator")),
        false,
        true,
    );
    gen_proto.prototype = Some(iterator_proto_ptr);
    let proto_ptr = Box::into_raw(Box::new(gen_proto));
    ctx.runtime_mut().gc_heap_mut().track(proto_ptr);
    ctx.set_generator_prototype(proto_ptr);

    let mut generator_function_proto = JSObject::new();
    if let Some(fn_proto_ptr) = ctx.get_function_prototype() {
        generator_function_proto.prototype = Some(fn_proto_ptr);
    }
    define_data_property(
        &mut generator_function_proto,
        ctx.common_atoms.constructor,
        generator_function,
        false,
        true,
    );
    define_data_property(
        &mut generator_function_proto,
        ctx.common_atoms.prototype,
        JSValue::new_object(proto_ptr),
        false,
        true,
    );
    define_data_property(
        &mut generator_function_proto,
        to_string_tag_atom,
        JSValue::new_string(ctx.intern("GeneratorFunction")),
        false,
        true,
    );
    let generator_function_proto_ptr =
        Box::into_raw(Box::new(generator_function_proto));
    ctx.runtime_mut()
        .gc_heap_mut()
        .track(generator_function_proto_ptr);
    {
        let func = generator_function.as_function_mut();
        define_data_property(
            &mut func.base,
            ctx.common_atoms.prototype,
            JSValue::new_object(generator_function_proto_ptr),
            false,
            false,
        );
    }
    let generator_proto_val = JSValue::new_object(proto_ptr);
    define_data_property(
        generator_proto_val.as_object_mut(),
        ctx.common_atoms.constructor,
        JSValue::new_object(generator_function_proto_ptr),
        false,
        true,
    );
    ctx.set_generator_function_prototype(generator_function_proto_ptr);

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
    if let Some(sym_atom) = sym_async_iter_atom {
        async_gen_proto.set(sym_atom, async_sym_iter_func);
    }

    define_data_property(
        &mut async_gen_proto,
        to_string_tag_atom,
        JSValue::new_string(ctx.intern("AsyncGenerator")),
        false,
        true,
    );
    async_gen_proto.prototype =
        Some(async_iterator_proto_ptr);

    let async_proto_ptr = Box::into_raw(Box::new(async_gen_proto));
    ctx.runtime_mut().gc_heap_mut().track(async_proto_ptr);
    ctx.set_async_generator_prototype(async_proto_ptr);

    let mut async_generator_function_proto = JSObject::new();
    if let Some(fn_proto_ptr) = ctx.get_function_prototype() {
        async_generator_function_proto.prototype = Some(fn_proto_ptr);
    }
    define_data_property(
        &mut async_generator_function_proto,
        ctx.common_atoms.constructor,
        async_generator_function,
        false,
        true,
    );
    define_data_property(
        &mut async_generator_function_proto,
        ctx.common_atoms.prototype,
        JSValue::new_object(async_proto_ptr),
        false,
        true,
    );
    define_data_property(
        &mut async_generator_function_proto,
        to_string_tag_atom,
        JSValue::new_string(ctx.intern("AsyncGeneratorFunction")),
        false,
        true,
    );
    let async_generator_function_proto_ptr =
        Box::into_raw(Box::new(async_generator_function_proto));
    ctx.runtime_mut()
        .gc_heap_mut()
        .track(async_generator_function_proto_ptr);
    {
        let func = async_generator_function.as_function_mut();
        define_data_property(
            &mut func.base,
            ctx.common_atoms.prototype,
            JSValue::new_object(async_generator_function_proto_ptr),
            false,
            false,
        );
    }
    let async_generator_proto_val = JSValue::new_object(async_proto_ptr);
    define_data_property(
        async_generator_proto_val.as_object_mut(),
        ctx.common_atoms.constructor,
        JSValue::new_object(async_generator_function_proto_ptr),
        false,
        true,
    );
    ctx.set_async_generator_function_prototype(async_generator_function_proto_ptr);
}

fn define_data_property(
    obj: &mut JSObject,
    atom: crate::runtime::atom::Atom,
    value: JSValue,
    writable: bool,
    configurable: bool,
) {
    let desc = crate::object::object::PropertyDescriptor {
        value: Some(value),
        writable,
        enumerable: false,
        configurable,
        get: None,
        set: None,
    };
    obj.define_property(atom, desc);
}

fn create_generator_constructor(
    ctx: &mut JSContext,
    name: &str,
    builtin_name: &str,
) -> JSValue {
    let name_atom = ctx.intern(name);
    let mut func = JSFunction::new_builtin(name_atom, 1);
    func.set_builtin_marker(ctx, builtin_name);
    if let Some(fn_pp) = ctx.get_function_prototype() {
        func.base.prototype = Some(fn_pp);
    }
    let ptr = Box::into_raw(Box::new(func));
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    let value = JSValue::new_function(ptr);
    let func_ref = value.as_function_mut();
    define_data_property(
        &mut func_ref.base,
        ctx.common_atoms.name,
        JSValue::new_string(name_atom),
        false,
        true,
    );
    define_data_property(
        &mut func_ref.base,
        ctx.common_atoms.length,
        JSValue::new_int(1),
        false,
        true,
    );
    value
}

fn dynamic_generator_function_constructor(
    ctx: &mut JSContext,
    args: &[JSValue],
    prefix: &str,
) -> JSValue {
    let body_idx = args.len().saturating_sub(1);
    let body_str = if args.is_empty() {
        String::new()
    } else if args[body_idx].is_string() {
        ctx.get_atom_str(args[body_idx].get_atom()).to_string()
    } else {
        let s = crate::builtins::global::to_property_key_string(ctx, &args[body_idx]);
        if ctx.pending_exception.is_some() {
            return JSValue::undefined();
        }
        s
    };
    let mut params = Vec::new();
    for i in 0..body_idx {
        let p = if args[i].is_string() {
            ctx.get_atom_str(args[i].get_atom()).to_string()
        } else {
            let s = crate::builtins::global::to_property_key_string(ctx, &args[i]);
            if ctx.pending_exception.is_some() {
                return JSValue::undefined();
            }
            s
        };
        for part in p.trim().split(',') {
            let part = part.trim();
            if !part.is_empty() {
                params.push(part.to_string());
            }
        }
    }
    let params_str = params.join(",");
    let source = format!("({} anonymous({}){{{}}})", prefix, params_str, body_str);
    match crate::eval(ctx, &source) {
        Ok(val) => {
            if val.is_function() {
                val.as_function_mut().name = ctx.intern("anonymous");
                let name_atom = ctx.intern("name");
                let obj = val.as_object_mut();
                let mut desc = crate::object::object::PropertyDescriptor::new_data(
                    JSValue::new_string(ctx.intern("anonymous")),
                );
                desc.writable = false;
                desc.enumerable = false;
                desc.configurable = true;
                obj.define_property_ext(name_atom, desc, true, true, true);
            }
            val
        }
        Err(e) => {
            let mut err = JSObject::new();
            err.set(
                ctx.intern("name"),
                JSValue::new_string(ctx.intern("SyntaxError")),
            );
            err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&e)));
            if let Some(proto) = ctx.get_syntax_error_prototype() {
                err.prototype = Some(proto);
            }
            let ptr = Box::into_raw(Box::new(err));
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            ctx.pending_exception = Some(JSValue::new_object(ptr));
            JSValue::undefined()
        }
    }
}

fn generator_function_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    dynamic_generator_function_constructor(ctx, args, "function*")
}

fn async_generator_function_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    dynamic_generator_function_constructor(ctx, args, "async function*")
}

fn create_builtin_function(ctx: &mut JSContext, builtin_name: &str) -> JSValue {
    let name_atom = ctx.intern(builtin_name);
    let mut func = JSFunction::new_builtin(name_atom, 1);
    func.set_builtin_marker(ctx, builtin_name);
    let proto_atom = ctx.intern("prototype");
    let proto_obj = JSObject::new();
    let proto_ptr = Box::into_raw(Box::new(proto_obj));
    func.base.set(proto_atom, JSValue::new_object(proto_ptr));

    let ptr = Box::into_raw(Box::new(func));
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
    let upvalue_cells = state.upvalue_cells.clone();
    let param_env = state.param_env;
    let param_env_own = state.param_env_own;

    let (
        value,
        new_snapshot,
        new_pc,
        done,
        new_send_reg,
        new_handlers,
        new_yield_is_result,
        new_upvalue_cells,
        new_param_env,
        new_param_env_own,
    ) = {
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
            &upvalue_cells,
            Some((param_env, param_env_own)),
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
        state.upvalue_cells = new_upvalue_cells;
        state.param_env = new_param_env;
        state.param_env_own = new_param_env_own;
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
                let ptr = Box::into_raw(Box::new(err));
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

fn make_type_error(ctx: &mut JSContext, message: &str) -> JSValue {
    let mut err = JSObject::new();
    err.set(
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("TypeError")),
    );
    err.set(
        ctx.intern("message"),
        JSValue::new_string(ctx.intern(message)),
    );
    if let Some(proto) = ctx.get_type_error_prototype() {
        err.prototype = Some(proto);
    }
    let ptr = Box::into_raw(Box::new(err));
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

fn async_generator_validate(ctx: &mut JSContext, this: JSValue) -> Option<JSValue> {
    if this.is_object() {
        if this.try_as_object().expect("object value").get_async_generator_state().is_some() {
            return None;
        }
    }
    Some(make_type_error(ctx, "value is not an async generator"))
}

fn async_generator_is_done(this: JSValue) -> bool {
    if !this.is_object() {
        return true;
    }
    this.try_as_object().expect("object value")
        .get_async_state()
        .map(|state| state.done)
        .unwrap_or(true)
}

fn async_generator_is_started(this: JSValue) -> bool {
    if !this.is_object() {
        return false;
    }
    this.try_as_object().expect("object value")
        .get_async_generator_state()
        .map(|state| state.started)
        .unwrap_or(false)
}

fn async_generator_enqueue(
    ctx: &mut JSContext,
    this: JSValue,
    kind: u8,
    value: JSValue,
) -> JSValue {
    let promise = crate::builtins::promise::new_pending_promise(ctx);
    if let Some(state) = this.as_object_mut().get_async_generator_state_mut() {
        state
            .queue
            .push(crate::object::object::AsyncGeneratorRequest {
                kind,
                value,
                promise,
            });
    }
    let roots = &mut ctx.runtime_mut().gc_heap_mut().extra_roots;
    if !roots.iter().any(|v| v.raw_bits() == this.raw_bits()) {
        roots.push(this);
    }
    promise
}

fn async_generator_next(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or_else(JSValue::undefined);
    let value = args.get(1).copied().unwrap_or_else(JSValue::undefined);
    if let Some(err) = async_generator_validate(ctx, this) {
        return ctx.call_builtin("promise_reject", &[err]);
    }
    if async_generator_is_done(this) {
        let result = create_iterator_result(ctx, JSValue::undefined(), true);
        return ctx.call_builtin("promise_resolve", &[result]);
    }
    let promise = async_generator_enqueue(ctx, this, 0, value);
    crate::runtime::vm::drive_async_generator_queue(ctx, this);
    promise
}

fn async_generator_return(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or_else(JSValue::undefined);
    let value = args.get(1).copied().unwrap_or_else(JSValue::undefined);
    if let Some(err) = async_generator_validate(ctx, this) {
        return ctx.call_builtin("promise_reject", &[err]);
    }
    let promise = async_generator_enqueue(ctx, this, 2, value);
    crate::runtime::vm::drive_async_generator_queue(ctx, this);
    promise
}

fn async_generator_throw(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or_else(JSValue::undefined);
    let value = args.get(1).copied().unwrap_or_else(JSValue::undefined);
    if let Some(err) = async_generator_validate(ctx, this) {
        return ctx.call_builtin("promise_reject", &[err]);
    }
    if !async_generator_is_started(this) {
        if let Some(state) = this.as_object_mut().get_async_state_mut() {
            state.done = true;
        }
        return ctx.call_builtin("promise_reject", &[value]);
    }
    if async_generator_is_done(this) {
        return ctx.call_builtin("promise_reject", &[value]);
    }
    let promise = async_generator_enqueue(ctx, this, 1, value);
    crate::runtime::vm::drive_async_generator_queue(ctx, this);
    promise
}

fn create_iterator_result(ctx: &mut JSContext, value: JSValue, done: bool) -> JSValue {
    let mut result = JSObject::new();
    result.set(ctx.intern("value"), value);
    result.set(ctx.intern("done"), JSValue::bool(done));
    if let Some(proto_ptr) = ctx.get_object_prototype() {
        result.prototype = Some(proto_ptr);
    }

    let ptr = Box::into_raw(Box::new(result));
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}
