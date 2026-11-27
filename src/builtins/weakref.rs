use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::runtime::gc::WeakObjectHandle;
use crate::value::JSValue;

struct WeakRefState {
    target: Option<WeakObjectHandle>,
}

fn create_builtin_function(ctx: &mut JSContext, name: &str) -> JSValue {
    let mut func = JSFunction::new_builtin(ctx.intern(name), 1);
    func.set_builtin_marker(ctx, name);
    let ptr = Box::into_raw(Box::new(func));
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    JSValue::new_function(ptr)
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin(
        "weakref_constructor",
        HostFunction::ctor("WeakRef", 1, weakref_constructor),
    );
    ctx.register_builtin(
        "weakref_deref",
        HostFunction::method("deref", 0, weakref_deref),
    );
    ctx.register_builtin(
        "finalization_registry_constructor",
        HostFunction::ctor("FinalizationRegistry", 1, finalization_registry_constructor),
    );
    ctx.register_builtin(
        "finalization_registry_register",
        HostFunction::method("register", 2, finalization_registry_register),
    );
    ctx.register_builtin(
        "finalization_registry_unregister",
        HostFunction::method("unregister", 1, finalization_registry_unregister),
    );
}

fn set_non_enumerable(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
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

fn define_ctor_prototype(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
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

pub fn init_weakref(ctx: &mut JSContext) {
    let mut weakref_proto = JSObject::new();
    set_non_enumerable(
        &mut weakref_proto,
        ctx.intern("deref"),
        create_builtin_function(ctx, "weakref_deref"),
    );

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        weakref_proto.prototype = Some(obj_proto_ptr);
    }

    let weakref_proto_ptr = Box::into_raw(Box::new(weakref_proto));
    ctx.runtime_mut().gc_heap_mut().track(weakref_proto_ptr);
    ctx.set_weakref_prototype(weakref_proto_ptr);

    let mut weakref_ctor = create_builtin_function(ctx, "weakref_constructor");
    if weakref_ctor.is_function() {
        let weakref_func_ref = weakref_ctor.try_as_function_mut().expect("function value");
        define_ctor_prototype(
            &mut weakref_func_ref.base,
            ctx.common_atoms.prototype,
            JSValue::new_object(weakref_proto_ptr),
        );
        weakref_func_ref.cached_prototype_ptr = weakref_proto_ptr;
    }
    let weakref_atom = ctx.intern("WeakRef");
    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, weakref_atom, weakref_ctor);
    }
    let mut weakref_proto_val = JSValue::new_object(weakref_proto_ptr);
    let proto_ref = weakref_proto_val.try_as_object_mut().expect("object value");
    set_non_enumerable(proto_ref, ctx.common_atoms.constructor, weakref_ctor);

    let mut fr_proto = JSObject::new();
    set_non_enumerable(
        &mut fr_proto,
        ctx.intern("register"),
        create_builtin_function(ctx, "finalization_registry_register"),
    );
    set_non_enumerable(
        &mut fr_proto,
        ctx.intern("unregister"),
        create_builtin_function(ctx, "finalization_registry_unregister"),
    );

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        fr_proto.prototype = Some(obj_proto_ptr);
    }

    let fr_proto_ptr = Box::into_raw(Box::new(fr_proto));
    ctx.runtime_mut().gc_heap_mut().track(fr_proto_ptr);
    ctx.set_finalization_registry_prototype(fr_proto_ptr);

    let mut fr_ctor = create_builtin_function(ctx, "finalization_registry_constructor");
    if fr_ctor.is_function() {
        let fr_func_ref = fr_ctor.try_as_function_mut().expect("function value");
        define_ctor_prototype(
            &mut fr_func_ref.base,
            ctx.common_atoms.prototype,
            JSValue::new_object(fr_proto_ptr),
        );
        fr_func_ref.cached_prototype_ptr = fr_proto_ptr;
    }
    let fr_atom = ctx.intern("FinalizationRegistry");
    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, fr_atom, fr_ctor);
    }
    let mut fr_proto_val = JSValue::new_object(fr_proto_ptr);
    let proto_ref = fr_proto_val.try_as_object_mut().expect("object value");
    set_non_enumerable(proto_ref, ctx.common_atoms.constructor, fr_ctor);
}

pub fn weakref_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();

    if let Some(proto_ptr) = ctx.get_weakref_prototype() {
        obj.prototype = Some(proto_ptr);
    }

    let target = match args.first() {
        Some(value) if value.is_object_like() => {
            ctx.runtime_mut().gc_heap().weak_handle(value.try_as_object().expect("object value"))
        }
        _ => None,
    };
    obj.set_native_data(Box::new(WeakRefState { target }));

    let ptr = Box::into_raw(Box::new(obj));
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

pub fn weakref_deref(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }

    let this = &args[0];
    if !this.is_object() {
        return JSValue::undefined();
    }

    let obj = this.try_as_object().expect("object value");

    let target = match obj.get_native_data::<WeakRefState>() {
        Some(state) => state.target,
        None => return JSValue::undefined(),
    };

    match target.and_then(|handle| ctx.runtime().gc_heap().resolve_weak(handle)) {
        Some(value) => value,
        None => JSValue::undefined(),
    }
}

pub fn finalization_registry_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();

    if let Some(proto_ptr) = ctx.get_finalization_registry_prototype() {
        obj.prototype = Some(proto_ptr);
    }

    obj.set(
        ctx.intern("__is_finalization_registry__"),
        JSValue::bool(true),
    );

    let held_values_obj = JSObject::new();
    let held_values_ptr = Box::into_raw(Box::new(held_values_obj));
    obj.set(
        ctx.intern("__held_values__"),
        JSValue::new_object(held_values_ptr),
    );

    if !args.is_empty() {
        let callback = &args[0];
        if callback.is_function() {
            obj.set(ctx.intern("__callback__"), callback.clone());
        }
    }

    let ptr = Box::into_raw(Box::new(obj));
    let obj_value = JSValue::new_object(ptr);

    ctx.finalization_registries.borrow_mut().push(ptr);

    obj_value
}

pub fn finalization_registry_register(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }

    let mut this = args[0];
    if !this.is_object() {
        return JSValue::undefined();
    }

    let obj = this.try_as_object_mut().expect("object value");

    let is_fr = obj.get(ctx.intern("__is_finalization_registry__"));
    match is_fr {
        Some(v) if !v.is_undefined() && v.get_bool() => {}
        _ => return JSValue::undefined(),
    };

    if args.len() < 3 {
        return JSValue::undefined();
    }

    let _target = &args[1];
    let held_value = &args[2];
    let unregister_token = if args.len() > 3 {
        &args[3]
    } else {
        &JSValue::undefined()
    };

    let held_values_atom = ctx.intern("__held_values__");
    let held_values_val = obj.get(held_values_atom);

    if let Some(mut hvv) = held_values_val {
        if hvv.is_object() {
            let held_obj = hvv.try_as_object_mut().expect("object value");

            let key = if unregister_token.is_object() {
                let token_ptr = unregister_token.get_ptr().addr();
                ctx.intern(&format!("__token_{}__", token_ptr))
            } else {
                ctx.intern(&format!("__entry_{}__", held_obj.inline_values_len()))
            };

            held_obj.set(key, held_value.clone());
        }
    }

    JSValue::undefined()
}

pub fn finalization_registry_unregister(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::bool(false);
    }

    let mut this = args[0];
    if !this.is_object() {
        return JSValue::bool(false);
    }

    let obj = this.try_as_object_mut().expect("object value");

    let is_fr = obj.get(ctx.intern("__is_finalization_registry__"));
    match is_fr {
        Some(v) if !v.is_undefined() && v.get_bool() => {}
        _ => return JSValue::bool(false),
    };

    if args.len() < 2 {
        return JSValue::bool(false);
    }

    let unregister_token = &args[1];

    if !unregister_token.is_object() {
        return JSValue::bool(false);
    }

    let token_ptr = unregister_token.get_ptr().addr();
    let key = ctx.intern(&format!("__token_{}__", token_ptr));

    let held_values_atom = ctx.intern("__held_values__");
    let held_values_val = obj.get(held_values_atom);

    if let Some(mut hvv) = held_values_val {
        if hvv.is_object() {
            let held_obj = hvv.try_as_object_mut().expect("object value");

            if held_obj.has_own(key) {
                held_obj.delete(key);
                return JSValue::bool(true);
            }
        }
    }

    JSValue::bool(false)
}
