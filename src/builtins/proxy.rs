use crate::host::func::HostFunction;
use crate::object::object::{JSObject, ObjectType};
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn is_callable_target(ctx: &mut JSContext, target: JSValue) -> bool {
    if target.is_function() {
        return true;
    }
    if target.is_object() {
        let obj = target.as_object();
        if obj.is_proxy() {
            return obj.proxy_callable();
        }
        if obj.get(ctx.common_atoms.__boundFn).is_some() {
            return true;
        }
        if let Some(function_prototype) = ctx.get_function_prototype() {
            return target.get_ptr() as *mut JSObject == function_prototype;
        }
    }
    false
}

fn is_constructor_target(ctx: &mut JSContext, target: JSValue) -> bool {
    if target.is_function() {
        let f = target.as_function();
        if f.is_builtin() {
            let name = f
                .builtin_atom
                .map(|ba| ctx.get_atom_str(ba).to_string())
                .unwrap_or_default();
            return ctx.builtin_is_constructor(&name);
        }
        return !f.is_arrow() && !f.is_async() && !f.is_generator() && !f.is_method;
    }
    if target.is_object() {
        let obj = target.as_object();
        if obj.is_proxy() {
            return obj.proxy_constructable();
        }
        if let Some(bound) = obj.get(ctx.common_atoms.__boundFn) {
            return is_constructor_target(ctx, bound);
        }
    }
    false
}

pub fn create_proxy(
    ctx: &mut JSContext,
    target: JSValue,
    handler: JSValue,
) -> Result<JSValue, String> {
    if !target.is_object_like() {
        let msg = "Cannot create proxy with a non-object as target".to_string();
        crate::builtins::global::throw_type_error(ctx, &msg);
        return Err(msg);
    }
    if !handler.is_object_like() {
        let msg = "Cannot create proxy with a non-object as handler".to_string();
        crate::builtins::global::throw_type_error(ctx, &msg);
        return Err(msg);
    }
    let callable = is_callable_target(ctx, target);
    let constructable = is_constructor_target(ctx, target);
    let mut proxy_obj = JSObject::new_typed(ObjectType::Proxy);
    proxy_obj.prototype = None;
    proxy_obj.set_proxy_slots(target, handler, callable, constructable);
    let ptr = Box::into_raw(Box::new(proxy_obj)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    Ok(JSValue::new_object(ptr))
}

fn proxy_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if !ctx.is_call_with_new() {
        crate::builtins::global::throw_type_error(
            ctx,
            "Proxy constructor requires new",
        );
        return JSValue::undefined();
    }
    let target = args.first().copied().unwrap_or_else(JSValue::undefined);
    let handler = args.get(1).copied().unwrap_or_else(JSValue::undefined);
    match create_proxy(ctx, target, handler) {
        Ok(proxy) => proxy,
        Err(_) => JSValue::undefined(),
    }
}

fn proxy_revocable(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let target = args.first().copied().unwrap_or_else(JSValue::undefined);
    let handler = args.get(1).copied().unwrap_or_else(JSValue::undefined);
    let proxy = match create_proxy(ctx, target, handler) {
        Ok(proxy) => proxy,
        Err(_) => return JSValue::undefined(),
    };

    let mut revoke_func =
        crate::object::function::JSFunction::new_builtin(ctx.intern("proxy_revoke"), 0);
    revoke_func.set_builtin_marker(ctx, "proxy_revoke");
    let proxy_atom = ctx.intern("__v16_revoked_proxy__");
    revoke_func.base.define_property(
        proxy_atom,
        crate::object::object::PropertyDescriptor {
            value: Some(proxy),
            writable: false,
            enumerable: false,
            configurable: false,
            get: None,
            set: None,
        },
    );
    let revoke_ptr = Box::into_raw(Box::new(revoke_func)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(revoke_ptr);
    let revoke = JSValue::new_function(revoke_ptr);

    let mut result = JSObject::new();
    result.prototype = ctx.get_object_prototype();
    result.set(ctx.intern("proxy"), proxy);
    result.set(ctx.intern("revoke"), revoke);
    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(result_ptr);
    JSValue::new_object(result_ptr)
}

fn proxy_revoke(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let callee = match args.first() {
        Some(value) => *value,
        None => return JSValue::undefined(),
    };
    if !callee.is_function() {
        return JSValue::undefined();
    }
    let proxy_atom = ctx.intern("__v16_revoked_proxy__");
    let proxy = match callee.as_object().get_own(proxy_atom) {
        Some(proxy) => proxy,
        None => return JSValue::undefined(),
    };
    if proxy.is_object() && unsafe { proxy.as_object() }.is_proxy() {
        unsafe { proxy.as_object_mut() }.revoke_proxy();
    }
    JSValue::undefined()
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin(
        "proxy_constructor",
        HostFunction::ctor("Proxy", 2, proxy_constructor),
    );
    ctx.register_builtin(
        "proxy_revocable",
        HostFunction::new("revocable", 2, proxy_revocable),
    );
    ctx.register_builtin(
        "proxy_revoke",
        HostFunction::new("", 0, proxy_revoke),
    );
}

pub fn init_proxy(ctx: &mut JSContext) {
    let proxy_atom = ctx.intern("Proxy");
    let mut proxy_func = crate::object::function::JSFunction::new_builtin(proxy_atom, 2);
    proxy_func.set_builtin_marker(ctx, "proxy_constructor");
    crate::builtins::global::set_non_enumerable(
        &mut proxy_func.base,
        ctx.intern("revocable"),
        crate::builtins::global::create_builtin_function(ctx, "proxy_revocable"),
    );
    let proxy_ptr = Box::into_raw(Box::new(proxy_func)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(proxy_ptr);
    let proxy_value = JSValue::new_function(proxy_ptr);

    let global = ctx.global();
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, proxy_atom, proxy_value);
    }
}
