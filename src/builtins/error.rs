use crate::host::HostFunction;
use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn set_ctor_prototype(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
    obj.define_property(
        key,
        crate::object::object::PropertyDescriptor {
            value: Some(val),
            writable: false,
            enumerable: false,
            configurable: false,
            get: None,
            set: None,
        },
    );
}

fn set_own_ne(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
    obj.define_property(
        key,
        crate::object::object::PropertyDescriptor {
            value: Some(val),
            writable: true,
            enumerable: false,
            configurable: true,
            get: None,
            set: None,
        },
    );
}

fn create_builtin_function(ctx: &mut JSContext, name: &str) -> JSValue {
    let arity = ctx.get_builtin_arity(name).unwrap_or(1);
    let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern(name), arity);
    func.set_builtin_marker(ctx, name);
    ctx.allocate_function(func)
}

pub fn init_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let error_atom = ctx.intern("Error");

    let mut error_func = crate::object::function::JSFunction::new_builtin(error_atom, 1);
    error_func.set_builtin_marker(ctx, "error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        error_func.base.prototype = Some(func_proto);
    }
    let error_value = ctx.allocate_function(error_func);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, error_atom, error_value);
    }

    let mut proto_obj = JSObject::new();
    set_ne(
        &mut proto_obj,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("Error")),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("toString"),
        create_builtin_function(ctx, "error_toString"),
    );

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        proto_obj.prototype = Some(obj_proto_ptr);
    }

    set_ne(&mut proto_obj, ctx.intern("constructor"), error_value);

    let proto_value = ctx.allocate_object(proto_obj);
    ctx.set_error_prototype(proto_value.get_ptr());
    if ctx.get_builtin_func("error_constructor").is_some() {
        let mut f_binding = error_value;
        let f = f_binding.try_as_function_mut().expect("function value");
        set_ctor_prototype(&mut f.base, ctx.intern("prototype"), proto_value);
    }

    let proto_atom = ctx.intern("ErrorPrototype");
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, proto_atom, proto_value);
    }

    init_type_error(ctx);
    init_reference_error(ctx);
    init_syntax_error(ctx);
    init_range_error(ctx);
    init_uri_error(ctx);
    init_eval_error(ctx);
    init_suppressed_error(ctx);
    init_aggregate_error(ctx);

    let global = ctx.global();
    if global.is_object() {
        let error_ctor_val = global.try_as_object().expect("object value").get(error_atom);
        if let Some(error_ctor) = error_ctor_val {
            if error_ctor.is_function() {
                let error_ctor_ptr = error_ctor.get_ptr();
                for name in ["TypeError", "ReferenceError", "SyntaxError", "RangeError", "URIError", "EvalError", "SuppressedError", "AggregateError"] {
                    let atom = ctx.intern(name);
                    if let Some(mut ctor_val) = global.try_as_object().expect("object value").get(atom) {
                        if ctor_val.is_function() {
                            let ctor = ctor_val.try_as_function_mut().expect("function value");
                            ctor.base.set_prototype_raw(error_ctor_ptr);
                        }
                    }
                }
            }
        }
    }
}

fn init_type_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("TypeError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "type_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("TypeError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );

    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_type_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);
}

fn init_reference_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("ReferenceError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "reference_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("ReferenceError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_reference_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }
}

fn init_syntax_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("SyntaxError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "syntax_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("SyntaxError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_syntax_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }
}

fn init_range_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("RangeError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "range_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("RangeError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_range_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }
}

fn init_uri_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("URIError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "uri_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("URIError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );

    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_uri_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);
}

fn init_eval_error(ctx: &mut JSContext) {
    fn set_ne(
        obj: &mut crate::object::object::JSObject,
        key: crate::runtime::atom::Atom,
        val: crate::value::JSValue,
    ) {
        obj.define_property(
            key,
            crate::object::object::PropertyDescriptor {
                value: Some(val),
                writable: true,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }

    let atom = ctx.intern("EvalError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 1);
    ctor.set_builtin_marker(ctx, "eval_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }

    let mut proto = JSObject::new();
    set_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("EvalError")),
    );
    set_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );

    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_eval_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);
}

fn get_property(
    ctx: &mut JSContext,
    obj: &crate::object::object::JSObject,
    prop: crate::runtime::atom::Atom,
    this: &JSValue,
) -> Option<JSValue> {
    if let Some(val) = obj.get_own_accessor_value(prop) {
        if val.is_function() {
            if let Some(r) = ctx.with_vm(|vm, ctx| {
                let saved_exception = ctx.pending_exception.take();
                let result = vm.call_function_with_this(ctx, val, this.clone(), &[]);
                if result.is_err() {
                    let exc = ctx
                        .pending_exception
                        .take()
                        .or_else(|| vm.last_caught_exception.take())
                        .or(saved_exception);
                    if let Some(e) = exc {
                        ctx.pending_exception = Some(e);
                    }
                    return None;
                }
                ctx.pending_exception = saved_exception;
                result.ok()
            }) {
                return r;
            }
        }
        return Some(val);
    }
    obj.get(prop)
}

fn js_to_string_value(ctx: &mut JSContext, val: &JSValue) -> Result<String, ()> {
    if val.is_string() {
        return Ok(ctx.get_atom_str(val.get_atom()).to_string());
    }
    if val.is_undefined() {
        return Ok("undefined".to_string());
    }
    if val.is_null() {
        return Ok("null".to_string());
    }
    if val.is_bool() {
        if val.is_truthy() {
            return Ok("true".to_string());
        }
        return Ok("false".to_string());
    }
    if val.is_int() {
        let n = val.get_int();
        return Ok(format!("{}", n));
    }
    if val.is_float() {
        let n = val.to_number();
        if n == n.floor() && n.is_finite() && n.abs() < 1e15 {
            return Ok(format!("{}", n as i64));
        }
        return Ok(format!("{}", n));
    }
    if val.is_symbol() {
        let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("Cannot convert a Symbol value to a string")),
        );
        let error_value = ctx.allocate_object(err);
        ctx.pending_exception = Some(error_value);
        return Err(());
    }
    if val.is_object_like() {
        return ctx
            .with_vm(|vm, ctx| {
                let prim = vm.ordinary_to_primitive(val, "string", ctx);
                if let Some(exc) = ctx
                    .pending_exception
                    .take()
                    .or_else(|| vm.pending_throw.take())
                    .or_else(|| vm.last_caught_exception.take())
                {
                    ctx.pending_exception = Some(exc);
                    return Err(());
                }
                if !prim.is_object_like() {
                    return js_to_string_value(ctx, &prim);
                }
                let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
                if let Some(proto) = ctx.get_type_error_prototype() {
                    err.prototype = Some(proto);
                }
                err.set(
                    ctx.common_atoms.message,
                    JSValue::new_string(ctx.intern("Cannot convert object to primitive value")),
                );
                let error_value = ctx.allocate_object(err);
                ctx.pending_exception = Some(error_value);
                Err(())
            })
            .unwrap_or(Ok(String::new()));
    }
    Ok(String::new())
}

fn build_error(
    ctx: &mut JSContext,
    name: &str,
    args: &[JSValue],
    proto: Option<*mut JSObject>,
) -> JSValue {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    set_own_ne(
        &mut err,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern(name)),
    );
    if !args.is_empty() && !args[0].is_undefined() {
        match js_to_string_value(ctx, &args[0]) {
            Ok(s) => {
                set_own_ne(
                    &mut err,
                    ctx.intern("message"),
                    JSValue::new_string(ctx.intern(&s)),
                );
            }
            Err(()) => return JSValue::undefined(),
        }
    }
    if args.len() > 1 && args[1].is_object() {
        let cause_atom = ctx.intern("cause");
        let opts_this = args[1];
        let opts = opts_this.try_as_object().expect("object value");
        let cause = get_property(ctx, opts, cause_atom, &opts_this);
        let _ = opts;
        if let Some(cause) = cause {
            set_own_ne(&mut err, cause_atom, cause);
        }
    }
    if let Some(p) = proto {
        err.prototype = Some(p);
    }
    ctx.allocate_object(err)
}

pub fn error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx.get_error_prototype().map(|p| p as *mut _);
    build_error(ctx, "Error", args, proto)
}

pub fn type_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx.get_type_error_prototype().map(|p| p as *mut _);
    build_error(ctx, "TypeError", args, proto)
}

pub fn reference_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx
        .get_reference_error_prototype()
        .or_else(|| ctx.get_error_prototype())
        .map(|p| p as *mut _);
    build_error(ctx, "ReferenceError", args, proto)
}

pub fn syntax_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx.get_syntax_error_prototype().map(|p| p as *mut _);
    build_error(ctx, "SyntaxError", args, proto)
}

pub fn range_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx
        .get_range_error_prototype()
        .or_else(|| ctx.get_error_prototype())
        .map(|p| p as *mut _);
    build_error(ctx, "RangeError", args, proto)
}

pub fn uri_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx
        .get_uri_error_prototype()
        .or_else(|| ctx.get_error_prototype())
        .map(|p| p as *mut _);
    build_error(ctx, "URIError", args, proto)
}

pub fn eval_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let proto = ctx
        .get_eval_error_prototype()
        .or_else(|| ctx.get_error_prototype())
        .map(|p| p as *mut _);
    build_error(ctx, "EvalError", args, proto)
}

fn init_suppressed_error(ctx: &mut JSContext) {
    let atom = ctx.intern("SuppressedError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 3);
    ctor.set_builtin_marker(ctx, "suppressed_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }

    let mut proto = JSObject::new();
    set_own_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("SuppressedError")),
    );
    set_own_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_own_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_suppressed_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);
}

pub fn create_suppressed_error(
    ctx: &mut JSContext,
    error: JSValue,
    suppressed: JSValue,
) -> JSValue {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    set_own_ne(&mut err, ctx.intern("error"), error);
    set_own_ne(&mut err, ctx.intern("suppressed"), suppressed);
    if let Some(proto) = ctx.get_suppressed_error_prototype() {
        err.prototype = Some(proto);
    }
    ctx.allocate_object(err)
}

pub fn suppressed_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_suppressed_error_prototype() {
        err.prototype = Some(proto);
    }
    if let Some(msg) = args.get(2) {
        if !msg.is_undefined() {
            let s = match js_to_string_value(ctx, msg) {
                Ok(s) => s,
                Err(()) => return JSValue::undefined(),
            };
            set_own_ne(
                &mut err,
                ctx.intern("message"),
                JSValue::new_string(ctx.intern(&s)),
            );
        }
    }
    let error = args.first().cloned().unwrap_or_else(JSValue::undefined);
    let suppressed = args.get(1).cloned().unwrap_or_else(JSValue::undefined);
    set_own_ne(&mut err, ctx.intern("error"), error);
    set_own_ne(&mut err, ctx.intern("suppressed"), suppressed);
    ctx.allocate_object(err)
}

fn init_aggregate_error(ctx: &mut JSContext) {
    let atom = ctx.intern("AggregateError");
    let mut ctor = crate::object::function::JSFunction::new_builtin(atom, 2);
    ctor.set_builtin_marker(ctx, "aggregate_error_constructor");
    if let Some(func_proto) = ctx.get_function_prototype() {
        ctor.base.prototype = Some(func_proto);
    }
    let value = ctx.allocate_function(ctor);

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, atom, value);
    }

    let mut proto = JSObject::new();
    set_own_ne(
        &mut proto,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("AggregateError")),
    );
    set_own_ne(
        &mut proto,
        ctx.intern("message"),
        JSValue::new_string(ctx.intern("")),
    );
    if let Some(error_proto_ptr) = ctx.get_error_prototype() {
        proto.prototype = Some(error_proto_ptr);
    }
    set_own_ne(&mut proto, ctx.intern("constructor"), value);

    let proto_value = ctx.allocate_object(proto);
    ctx.set_aggregate_error_prototype(proto_value.get_ptr());
    let mut ctor_binding = value;
    let ctor = ctor_binding.try_as_function_mut().expect("function value");
    set_ctor_prototype(&mut ctor.base, ctx.intern("prototype"), proto_value);
}

pub fn aggregate_error_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    set_own_ne(
        &mut err,
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("AggregateError")),
    );
    if let Some(proto) = ctx.get_aggregate_error_prototype() {
        err.prototype = Some(proto);
    }
    if let Some(msg) = args.get(1) {
        if !msg.is_undefined() {
            let s = match js_to_string_value(ctx, msg) {
                Ok(s) => s,
                Err(()) => return JSValue::undefined(),
            };
            set_own_ne(
                &mut err,
                ctx.intern("message"),
                JSValue::new_string(ctx.intern(&s)),
            );
        }
    }
    if let Some(opts) = args.get(2) {
        if opts.is_object() {
            let cause_atom = ctx.intern("cause");
            let opts_this = *opts;
            let opts_obj = opts.try_as_object().expect("object value");
            let cause = get_property(ctx, &opts_obj, cause_atom, &opts_this);
            let _ = opts_obj;
            if let Some(cause) = cause {
                set_own_ne(&mut err, cause_atom, cause);
            }
        }
    }
    let errors = args.first().cloned().unwrap_or_else(JSValue::undefined);
    let error_values = crate::builtins::map_set::iterate_values(ctx, errors);
    let mut errors_array = crate::object::array_obj::JSArrayObject::with_capacity(error_values.len());
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        errors_array.header.set_prototype_raw(proto_ptr);
    }
    for error_value in &error_values {
        errors_array.push(*error_value);
    }
    errors_array
        .header
        .set(ctx.common_atoms.length, JSValue::new_int(error_values.len() as i64));
    let errors_value = ctx.allocate_array(errors_array);
    set_own_ne(
        &mut err,
        ctx.intern("errors"),
        errors_value,
    );
    ctx.allocate_object(err)
}

fn error_to_string(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        let mut err =
            crate::object::object::JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("Error.prototype.toString called on undefined")),
        );
        let error_value = ctx.allocate_object(err);
        ctx.pending_exception = Some(error_value);
        return JSValue::undefined();
    }
    let this = &args[0];
    if !this.is_object() {
        let mut err =
            crate::object::object::JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("Error.prototype.toString called on non-object")),
        );
        let error_value = ctx.allocate_object(err);
        ctx.pending_exception = Some(error_value);
        return JSValue::undefined();
    }

    let obj = this.try_as_object().expect("object value");
    let name_atom = ctx.intern("name");
    let name_val = get_property(ctx, &obj, name_atom, this);
    let _ = obj;
    if ctx.pending_exception.is_some() {
        return JSValue::undefined();
    }
    let name: Result<String, ()> = match name_val {
        Some(n) => {
            if n.is_undefined() {
                Ok("Error".to_string())
            } else {
                match js_to_string_value(ctx, &n) {
                    Ok(s) => Ok(s),
                    Err(()) => return JSValue::undefined(),
                }
            }
        }
        None => Ok("Error".to_string()),
    };
    if ctx.pending_exception.is_some() {
        return JSValue::undefined();
    }

    let obj2 = this.try_as_object().expect("object value");
    let msg_atom = ctx.intern("message");
    let message_val = get_property(ctx, &obj2, msg_atom, this);
    let _ = obj2;
    if ctx.pending_exception.is_some() {
        return JSValue::undefined();
    }
    let message: Result<String, ()> = match message_val {
        Some(m) => {
            if m.is_undefined() {
                Ok(String::new())
            } else {
                match js_to_string_value(ctx, &m) {
                    Ok(s) => Ok(s),
                    Err(()) => return JSValue::undefined(),
                }
            }
        }
        None => Ok(String::new()),
    };

    match (name, message) {
        (Ok(n), Ok(m)) => {
            if n.is_empty() {
                JSValue::new_string(ctx.intern(&m))
            } else if m.is_empty() {
                JSValue::new_string(ctx.intern(&n))
            } else {
                JSValue::new_string(ctx.intern(&format!("{}: {}", n, m)))
            }
        }
        _ => JSValue::undefined(),
    }
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin(
        "error_constructor",
        HostFunction::ctor("Error", 1, error_constructor),
    );
    ctx.register_builtin(
        "type_error_constructor",
        HostFunction::ctor("TypeError", 1, type_error_constructor),
    );
    ctx.register_builtin(
        "reference_error_constructor",
        HostFunction::ctor("ReferenceError", 1, reference_error_constructor),
    );
    ctx.register_builtin(
        "syntax_error_constructor",
        HostFunction::ctor("SyntaxError", 1, syntax_error_constructor),
    );
    ctx.register_builtin(
        "range_error_constructor",
        HostFunction::ctor("RangeError", 1, range_error_constructor),
    );
    ctx.register_builtin(
        "uri_error_constructor",
        HostFunction::ctor("URIError", 1, uri_error_constructor),
    );
    ctx.register_builtin(
        "eval_error_constructor",
        HostFunction::ctor("EvalError", 1, eval_error_constructor),
    );
    ctx.register_builtin(
        "suppressed_error_constructor",
        HostFunction::ctor("SuppressedError", 3, suppressed_error_constructor),
    );
    ctx.register_builtin(
        "aggregate_error_constructor",
        HostFunction::ctor("AggregateError", 2, aggregate_error_constructor),
    );
    ctx.register_builtin(
        "error_toString",
        HostFunction::method("toString", 0, error_to_string),
    );
}
