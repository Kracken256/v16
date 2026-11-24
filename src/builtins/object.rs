use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::{JSObject, ObjectType};
use crate::runtime::atom::Atom;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn throw_type_error_obj(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new();
    if let Some(proto_ptr) = ctx.get_type_error_prototype() {
        err.prototype = Some(proto_ptr);
    }
    err.set(
        ctx.common_atoms.name,
        JSValue::new_string(ctx.intern("TypeError")),
    );
    err.set(
        ctx.common_atoms.message,
        JSValue::new_string(ctx.intern(msg)),
    );
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    ctx.pending_exception = Some(JSValue::new_object(ptr));
}

pub fn object_to_object(ctx: &mut JSContext, value: &JSValue) -> JSValue {
    if value.is_string() {
        let mut obj = JSObject::new();
        if let Some(proto_ptr) = ctx.get_string_prototype() {
            obj.prototype = Some(proto_ptr);
        }
        obj.set(ctx.common_atoms.__value__, *value);
        let s = ctx.get_atom_str(value.get_atom()).to_string();
        let len = s.len();
        obj.set(ctx.common_atoms.length, JSValue::new_int(len as i64));
        if len > 0 {
            let elements = obj.ensure_elements();
            for c in s.chars() {
                let atom = ctx.intern(&c.to_string());
                elements.push(JSValue::new_string(atom));
            }
        }
        let ptr = Box::into_raw(Box::new(obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    if value.is_int() || value.is_float() {
        let mut obj = JSObject::new();
        if let Some(proto_ptr) = ctx.get_number_prototype() {
            obj.prototype = Some(proto_ptr);
        }
        obj.set(ctx.common_atoms.__value__, *value);
        let ptr = Box::into_raw(Box::new(obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    if value.is_bool() {
        let mut obj = JSObject::new();
        if let Some(proto_ptr) = ctx.get_bool_prototype() {
            obj.prototype = Some(proto_ptr);
        }
        obj.set(ctx.common_atoms.__value__, *value);
        let ptr = Box::into_raw(Box::new(obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    if value.is_symbol() {
        let mut obj = JSObject::new();
        if let Some(proto_ptr) = ctx.get_symbol_prototype() {
            obj.prototype = Some(proto_ptr);
        }
        obj.set(ctx.common_atoms.__value__, *value);
        let ptr = Box::into_raw(Box::new(obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    if value.is_bigint() {
        let mut obj = JSObject::new();
        if let Some(proto_ptr) = ctx.get_bigint_prototype() {
            obj.prototype = Some(proto_ptr);
        }
        obj.set(ctx.common_atoms.__value__, *value);
        let ptr = Box::into_raw(Box::new(obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    JSValue::undefined()
}

#[inline(always)]
fn is_object_like(value: &JSValue) -> bool {
    value.is_object() || value.is_function()
}

fn to_property_atom(ctx: &mut JSContext, value: &JSValue) -> Option<Atom> {
    if value.is_string() {
        return Some(value.get_atom());
    }
    if value.is_symbol() {
        let sym_id = value.get_symbol_id();
        return Some(crate::runtime::atom::Atom(0x40000000 | sym_id));
    }
    let s = crate::builtins::global::to_property_key_string(ctx, value);
    if ctx.pending_exception.is_some() {
        return None;
    }
    Some(ctx.intern(&s))
}

fn get_symbol_to_string_tag_atom(ctx: &mut JSContext) -> Option<Atom> {
    let symbol_atom = ctx.intern("Symbol");
    let to_string_tag_atom = ctx.intern("toStringTag");
    let global = ctx.global();
    if !global.is_object() {
        return None;
    }
    let global_obj = global.as_object();
    let symbol_ctor = global_obj.get(symbol_atom)?;
    if !is_object_like(&symbol_ctor) {
        return None;
    }
    let key = symbol_ctor.as_object().get(to_string_tag_atom)?;
    if key.is_symbol() {
        Some(key.get_atom())
    } else {
        None
    }
}

fn create_builtin_function(ctx: &mut JSContext, name: &str) -> JSValue {
    let arity = ctx.get_builtin_arity(name).unwrap_or(1);
    let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern(name), arity);
    func.set_builtin_marker(ctx, name);
    let ptr = Box::into_raw(Box::new(func)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    JSValue::new_function(ptr)
}

pub fn init_object(ctx: &mut JSContext) {
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

    let object_atom = ctx.common_atoms.object;

    let proto_atom = ctx.intern("ObjectPrototype");
    let mut proto_obj = JSObject::new();
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.has_own_property,
        create_builtin_function(ctx, "object_hasOwnProperty"),
    );
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.value_of,
        create_builtin_function(ctx, "object_valueOf"),
    );
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.to_string,
        create_builtin_function(ctx, "object_toString"),
    );
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.is_prototype_of,
        create_builtin_function(ctx, "object_isPrototypeOf"),
    );
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.property_is_enumerable,
        create_builtin_function(ctx, "object_property_is_enumerable"),
    );
    set_ne(
        &mut proto_obj,
        ctx.common_atoms.to_locale_string,
        create_builtin_function(ctx, "object_to_locale_string"),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("__defineGetter__"),
        create_builtin_function(ctx, "object_define_getter"),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("__defineSetter__"),
        create_builtin_function(ctx, "object_define_setter"),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("__lookupGetter__"),
        create_builtin_function(ctx, "object_lookup_getter"),
    );
    set_ne(
        &mut proto_obj,
        ctx.intern("__lookupSetter__"),
        create_builtin_function(ctx, "object_lookup_setter"),
    );
    let proto_ptr = Box::into_raw(Box::new(proto_obj)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(proto_ptr);
    ctx.set_object_prototype(proto_ptr);
    let proto_value = JSValue::new_object(proto_ptr);

    let mut object_obj = JSFunction::new_builtin(object_atom, 1);
    object_obj.set_builtin_marker(ctx, "object_constructor");
    object_obj.base.prototype = Some(proto_ptr as *mut JSObject);
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.common_atoms.keys,
        create_builtin_function(ctx, "object_keys"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.common_atoms.values,
        create_builtin_function(ctx, "object_values"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.common_atoms.entries,
        create_builtin_function(ctx, "object_entries"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("assign"),
        create_builtin_function(ctx, "object_assign"),
    );
    {
        let mut create_func =
            crate::object::function::JSFunction::new_builtin(ctx.intern("object_create"), 2);
        create_func.set_builtin_marker(ctx, "object_create");
        let ptr = Box::into_raw(Box::new(create_func)) as usize;
        ctx.runtime_mut().gc_heap_mut().track_function(ptr);
        object_obj
            .base
            .set(ctx.intern("create"), JSValue::new_function(ptr));
    }
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("getPrototypeOf"),
        create_builtin_function(ctx, "object_getPrototypeOf"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("setPrototypeOf"),
        create_builtin_function(ctx, "object_setPrototypeOf"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("fromEntries"),
        create_builtin_function(ctx, "object_fromEntries"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("hasOwn"),
        create_builtin_function(ctx, "object_hasOwn"),
    );
    crate::builtins::global::set_non_enumerable(
        &mut object_obj.base,
        ctx.intern("is"),
        create_builtin_function(ctx, "object_is"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("getOwnPropertySymbols"),
        create_builtin_function(ctx, "object_get_own_property_symbols"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("preventExtensions"),
        create_builtin_function(ctx, "object_prevent_extensions"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("isExtensible"),
        create_builtin_function(ctx, "object_is_extensible"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("seal"),
        create_builtin_function(ctx, "object_seal"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("isSealed"),
        create_builtin_function(ctx, "object_is_sealed"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("freeze"),
        create_builtin_function(ctx, "object_freeze"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("isFrozen"),
        create_builtin_function(ctx, "object_is_frozen"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("defineProperty"),
        create_builtin_function(ctx, "object_define_property"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("defineProperties"),
        create_builtin_function(ctx, "object_define_properties"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("getOwnPropertyDescriptor"),
        create_builtin_function(ctx, "object_get_own_property_descriptor"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("getOwnPropertyDescriptors"),
        create_builtin_function(ctx, "object_get_own_property_descriptors"),
    );
    crate::builtins::global::set_non_enumerable(&mut object_obj.base, 
        ctx.intern("getOwnPropertyNames"),
        create_builtin_function(ctx, "object_get_own_property_names"),
    );
    object_obj.base.define_property(
        ctx.common_atoms.prototype,
        crate::object::object::PropertyDescriptor {
            value: Some(proto_value),
            writable: false,
            enumerable: false,
            configurable: false,
            get: None,
            set: None,
        },
    );

    let object_ptr = Box::into_raw(Box::new(object_obj)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(object_ptr);
    let object_value = JSValue::new_function(object_ptr);

    if let Some(op) = ctx.get_object_prototype() {
        unsafe {
            (*op).set(ctx.common_atoms.constructor, object_value);
        }
    }
    let global = ctx.global();
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, object_atom, object_value);
        crate::builtins::global::set_non_enumerable(global_obj, proto_atom, proto_value);
    }
}

fn object_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() == 1 {
        let arg = args[0];
        if arg.is_object() || arg.is_function() {
            return arg;
        }

        if arg.is_bigint() {
            let mut obj = JSObject::new();
            if let Some(proto_ptr) = ctx.get_object_prototype() {
                obj.prototype = Some(proto_ptr);
            }
            obj.set(ctx.common_atoms.__value__, arg);
            let ptr = Box::into_raw(Box::new(obj)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }

        if arg.is_string() {
            let mut obj = JSObject::new();
            if let Some(proto_ptr) = ctx.get_string_prototype() {
                obj.prototype = Some(proto_ptr);
            }
            obj.set(ctx.common_atoms.__value__, arg);
            let s = ctx.get_atom_str(arg.get_atom());
            obj.set(ctx.common_atoms.length, JSValue::new_int(s.len() as i64));
            let ptr = Box::into_raw(Box::new(obj)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }

        if arg.is_int() || arg.is_float() {
            let mut obj = JSObject::new();
            if let Some(proto_ptr) = ctx.get_number_prototype() {
                obj.prototype = Some(proto_ptr);
            }
            obj.set(ctx.common_atoms.__value__, arg);
            let ptr = Box::into_raw(Box::new(obj)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }

        if arg.is_bool() {
            let mut obj = JSObject::new();
            if let Some(proto_ptr) = ctx.get_bool_prototype() {
                obj.prototype = Some(proto_ptr);
            }
            obj.set(ctx.common_atoms.__value__, arg);
            let ptr = Box::into_raw(Box::new(obj)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }

        if arg.is_symbol() {
            let mut obj = JSObject::new();
            if let Some(proto_ptr) = ctx.get_symbol_prototype() {
                obj.prototype = Some(proto_ptr);
            }
            obj.set(ctx.common_atoms.__value__, arg);
            let ptr = Box::into_raw(Box::new(obj)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }
    }
    let mut obj = JSObject::new();
    if let Some(proto_ptr) = ctx.get_object_prototype() {
        obj.prototype = Some(proto_ptr);
    }
    let ptr = Box::into_raw(Box::new(obj)) as usize;
    JSValue::new_object(ptr)
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin(
        "object_constructor",
        HostFunction::ctor("Object", 1, object_constructor),
    );
    ctx.register_builtin("object_keys", HostFunction::new("keys", 1, object_keys));
    ctx.register_builtin(
        "object_values",
        HostFunction::new("values", 1, object_values),
    );
    ctx.register_builtin(
        "object_entries",
        HostFunction::new("entries", 1, object_entries),
    );
    ctx.register_builtin(
        "object_assign",
        HostFunction::new("assign", 2, object_assign),
    );
    ctx.register_builtin(
        "object_create",
        HostFunction::new("create", 2, object_create),
    );
    ctx.register_builtin(
        "object_getPrototypeOf",
        HostFunction::new("getPrototypeOf", 1, object_get_prototype_of),
    );
    ctx.register_builtin(
        "object_setPrototypeOf",
        HostFunction::new("setPrototypeOf", 2, object_set_prototype_of),
    );
    ctx.register_builtin(
        "object_fromEntries",
        HostFunction::new("fromEntries", 1, object_from_entries),
    );
    ctx.register_builtin(
        "object_hasOwn",
        HostFunction::new("hasOwn", 2, object_has_own),
    );
    ctx.register_builtin("object_is", HostFunction::new("is", 2, object_is));
    ctx.register_builtin(
        "object_get_own_property_symbols",
        HostFunction::new("getOwnPropertySymbols", 1, object_get_own_property_symbols),
    );
    ctx.register_builtin(
        "object_prevent_extensions",
        HostFunction::new("preventExtensions", 1, object_prevent_extensions),
    );
    ctx.register_builtin(
        "object_is_extensible",
        HostFunction::new("isExtensible", 1, object_is_extensible),
    );
    ctx.register_builtin("object_seal", HostFunction::new("seal", 1, object_seal));
    ctx.register_builtin(
        "object_is_sealed",
        HostFunction::new("isSealed", 1, object_is_sealed),
    );
    ctx.register_builtin(
        "object_freeze",
        HostFunction::new("freeze", 1, object_freeze),
    );
    ctx.register_builtin(
        "object_is_frozen",
        HostFunction::new("isFrozen", 1, object_is_frozen),
    );
    ctx.register_builtin(
        "object_define_property",
        HostFunction::new("defineProperty", 3, object_define_property),
    );
    ctx.register_builtin(
        "object_define_properties",
        HostFunction::new("defineProperties", 2, object_define_properties),
    );
    ctx.register_builtin(
        "object_get_own_property_descriptor",
        HostFunction::new(
            "getOwnPropertyDescriptor",
            2,
            object_get_own_property_descriptor,
        ),
    );
    ctx.register_builtin(
        "object_get_own_property_descriptors",
        HostFunction::new(
            "getOwnPropertyDescriptors",
            1,
            object_get_own_property_descriptors,
        ),
    );
    ctx.register_builtin(
        "object_get_own_property_names",
        HostFunction::new("getOwnPropertyNames", 1, object_get_own_property_names),
    );

    ctx.register_builtin(
        "object_hasOwnProperty",
        HostFunction::method("hasOwnProperty", 1, object_has_own_property),
    );
    ctx.register_builtin(
        "object_valueOf",
        HostFunction::method("valueOf", 0, object_value_of),
    );
    ctx.register_builtin(
        "object_toString",
        HostFunction::method("toString", 0, object_to_string),
    );
    ctx.register_builtin(
        "object_isPrototypeOf",
        HostFunction::method("isPrototypeOf", 1, object_is_prototype_of),
    );
    ctx.register_builtin(
        "object_property_is_enumerable",
        HostFunction::method("propertyIsEnumerable", 1, object_property_is_enumerable),
    );
    ctx.register_builtin(
        "object_to_locale_string",
        HostFunction::method("toLocaleString", 0, object_to_locale_string),
    );
    ctx.register_builtin(
        "object_define_getter",
        HostFunction::method("__defineGetter__", 2, object_define_getter),
    );
    ctx.register_builtin(
        "object_define_setter",
        HostFunction::method("__defineSetter__", 2, object_define_setter),
    );
    ctx.register_builtin(
        "object_lookup_getter",
        HostFunction::method("__lookupGetter__", 1, object_lookup_getter),
    );
    ctx.register_builtin(
        "object_lookup_setter",
        HostFunction::method("__lookupSetter__", 1, object_lookup_setter),
    );
}

fn object_keys(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this = &args[0];
    if this.is_undefined() || this.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Cannot convert undefined or null to object");
        return JSValue::undefined();
    }
    if !this.is_object_like() {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }
    let obj = this.as_object();
    let keys = obj.enumerable_keys();
    let mut result = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        result.prototype = Some(proto_ptr);
    }
    let length_atom = ctx.common_atoms.length;
    result.set(length_atom, JSValue::new_int(keys.len() as i64));
    for (i, key) in keys.iter().enumerate() {
        let key_str = ctx.get_atom_str(*key).to_string();
        result.set(
            ctx.int_atom_mut(i),
            JSValue::new_string(ctx.intern(&key_str)),
        );
    }
    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(result_ptr)
}

fn object_values(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this = &args[0];
    if this.is_undefined() || this.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Cannot convert undefined or null to object");
        return JSValue::undefined();
    }
    if !this.is_object_like() {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }
    let obj = this.as_object();
    let values: Vec<_> = obj.own_properties().into_iter().map(|(_, v)| v).collect();
    let mut result = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        result.prototype = Some(proto_ptr);
    }
    let length_atom = ctx.common_atoms.length;
    result.set(length_atom, JSValue::new_int(values.len() as i64));
    for (i, val) in values.iter().enumerate() {
        result.set(ctx.int_atom_mut(i), *val);
    }
    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(result_ptr)
}

fn object_entries(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this = &args[0];
    if this.is_undefined() || this.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Cannot convert undefined or null to object");
        return JSValue::undefined();
    }
    if !this.is_object_like() {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }
    let obj = this.as_object();
    let entries: Vec<_> = obj.own_properties().into_iter().collect();
    let mut result = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        result.prototype = Some(proto_ptr);
    }
    let length_atom = ctx.common_atoms.length;
    result.set(length_atom, JSValue::new_int(entries.len() as i64));

    for (i, (key, val)) in entries.iter().enumerate() {
        let key_str = ctx.get_atom_str(*key).to_string();

        let mut pair = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            pair.prototype = Some(proto_ptr);
        }
        pair.set(ctx.common_atoms.length, JSValue::new_int(2));
        pair.set(
            ctx.common_atoms.n0,
            JSValue::new_string(ctx.intern(&key_str)),
        );
        pair.set(ctx.intern("1"), *val);

        let pair_ptr = Box::into_raw(Box::new(pair)) as usize;
        result.set(ctx.int_atom_mut(i), JSValue::new_object(pair_ptr));
    }

    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(result_ptr)
}

fn object_assign(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        throw_type_error_obj(ctx, "Object.assign requires at least 1 argument");
        return JSValue::undefined();
    }

    let target = &args[0];
    let to = if target.is_object() || target.is_function() {
        target.clone()
    } else if target.is_undefined() || target.is_null() {
        throw_type_error_obj(
            ctx,
            "Object.assign cannot convert undefined or null to object",
        );
        return JSValue::undefined();
    } else {
        object_to_object(ctx, target)
    };

    if !to.is_object() {
        return to;
    }

    let to_obj = to.as_object_mut();

    for arg in args.iter().skip(1) {
        let from = if arg.is_object() || arg.is_function() {
            arg.as_object()
        } else if arg.is_string() {
            continue;
        } else {
            continue;
        };

        from.for_each_property(|key, val, attrs| {
            if attrs & crate::object::object::ATTR_ENUMERABLE != 0 {
                to_obj.set(key, val);
            }
        });
    }

    to
}

fn object_has_property(obj_val: &JSValue, prop: Atom) -> bool {
    if !obj_val.is_object_like() {
        return false;
    }
    let mut current = Some(obj_val.get_ptr() as *mut JSObject);
    let mut depth = 0;
    while let Some(ptr) = current {
        let obj = unsafe { &*ptr };
        if obj.get_own(prop).is_some() {
            return true;
        }
        current = obj.prototype;
        depth += 1;
        if depth > 1000 {
            break;
        }
    }
    false
}

pub fn object_get_property(ctx: &mut JSContext, obj_val: &JSValue, prop: Atom) -> JSValue {
    if !obj_val.is_object_like() {
        return JSValue::undefined();
    }
    let mut current = Some(obj_val.get_ptr() as *mut JSObject);
    let mut depth = 0;
    while let Some(ptr) = current {
        let obj = unsafe { &*ptr };
        if let Some(entry) = obj.get_own_accessor_entry(prop) {
            if let Some(getter) = entry.get {
                if getter.is_function() {
                    if let Some(vm_ptr) = ctx.get_register_vm_ptr() {
                        let vm = unsafe { &mut *(vm_ptr as *mut crate::runtime::vm::VM) };
                        let this = *obj_val;
                        return vm
                            .call_function_with_this(ctx, getter, this, &[])
                            .unwrap_or(JSValue::undefined());
                    }
                }
            }
            return JSValue::undefined();
        }
        if let Some(v) = obj.get_own(prop) {
            return v;
        }
        current = obj.prototype;
        depth += 1;
        if depth > 1000 {
            break;
        }
    }
    JSValue::undefined()
}

fn throw_desc_type_error(ctx: &mut JSContext, msg: &str) {
    let mut err = crate::object::object::JSObject::new_typed(
        crate::object::object::ObjectType::Error,
    );
    if let Some(proto) = ctx.get_type_error_prototype() {
        err.prototype = Some(proto);
    }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
    err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    ctx.pending_exception = Some(JSValue::new_object(ptr));
}

pub fn to_property_descriptor(
    ctx: &mut JSContext,
    desc_val: &JSValue,
) -> Option<crate::object::object::PropertyDescriptor> {
    if !desc_val.is_object_like() {
        throw_desc_type_error(ctx, "Property description must be an object");
        return None;
    }
    let value_atom = ctx.intern("value");
    let writable_atom = ctx.intern("writable");
    let get_atom = ctx.intern("get");
    let set_atom = ctx.intern("set");
    let enumerable_atom = ctx.intern("enumerable");
    let configurable_atom = ctx.intern("configurable");

    let has_value = object_has_property(desc_val, value_atom);
    let has_writable = object_has_property(desc_val, writable_atom);
    let has_get = object_has_property(desc_val, get_atom);
    let has_set = object_has_property(desc_val, set_atom);

    let value = if has_value {
        Some(object_get_property(ctx, desc_val, value_atom))
    } else {
        None
    };
    let get = if has_get {
        Some(object_get_property(ctx, desc_val, get_atom))
    } else {
        None
    };
    let set = if has_set {
        Some(object_get_property(ctx, desc_val, set_atom))
    } else {
        None
    };

    if let Some(g) = get {
        if !g.is_undefined() && !g.is_function() {
            throw_desc_type_error(ctx, "Getter must be a function");
            return None;
        }
    }
    if let Some(s) = set {
        if !s.is_undefined() && !s.is_function() {
            throw_desc_type_error(ctx, "Setter must be a function");
            return None;
        }
    }

    let is_accessor = has_get || has_set;
    if is_accessor && (has_value || has_writable) {
        throw_desc_type_error(ctx, "Invalid property descriptor");
        return None;
    }

    let enumerable = if object_has_property(desc_val, enumerable_atom) {
        object_get_property(ctx, desc_val, enumerable_atom).is_truthy()
    } else {
        false
    };
    let configurable = if object_has_property(desc_val, configurable_atom) {
        object_get_property(ctx, desc_val, configurable_atom).is_truthy()
    } else {
        false
    };
    let writable = if has_writable {
        object_get_property(ctx, desc_val, writable_atom).is_truthy()
    } else {
        false
    };

    if is_accessor {
        Some(crate::object::object::PropertyDescriptor {
            value: None,
            writable: false,
            enumerable,
            configurable,
            get: get.filter(|g| !g.is_undefined()),
            set: set.filter(|s| !s.is_undefined()),
        })
    } else {
        Some(crate::object::object::PropertyDescriptor {
            value,
            writable,
            enumerable,
            configurable,
            get: None,
            set: None,
        })
    }
}

fn object_create(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut new_obj = JSObject::new();

    if !args.is_empty() && args[0].is_object() {
        let proto_ptr = args[0].get_ptr();
        new_obj.prototype = Some(proto_ptr as *mut JSObject);
    } else if !args.is_empty() && args[0].is_null() {
    }

    // Handle property descriptors (second argument)
    if args.len() > 1 {
        if args[1].is_null() {
            let mut err = crate::object::object::JSObject::new_typed(
                crate::object::object::ObjectType::Error,
            );
            if let Some(proto) = ctx.get_type_error_prototype() { err.prototype = Some(proto); }
            err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
            err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern("Cannot convert undefined or null to object")));
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            ctx.pending_exception = Some(JSValue::new_object(ptr));
            return JSValue::undefined();
        }
        if args[1].is_string() {
            let s = ctx.get_atom_str(args[1].get_atom());
            if !s.is_empty() {
                // ToObject(string) creates a String wrapper; its enumerable indices
                // hold string values which cause TypeError in ToPropertyDescriptor
                let mut err = crate::object::object::JSObject::new_typed(
                    crate::object::object::ObjectType::Error,
                );
                if let Some(proto) = ctx.get_type_error_prototype() { err.prototype = Some(proto); }
                err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
                err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern("Property description must be an object")));
                let ptr = Box::into_raw(Box::new(err)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(ptr);
                ctx.pending_exception = Some(JSValue::new_object(ptr));
                return JSValue::undefined();
            }
        }
    }
    if args.len() > 1 && args[1].is_object_like() {
        let props_list: Vec<(Atom, JSValue)> = args[1].as_object().own_properties();
        for (atom, _) in props_list {
            let desc_val = object_get_property(ctx, &args[1], atom);
            if ctx.pending_exception.is_some() {
                return JSValue::undefined();
            }
            let Some(desc) = to_property_descriptor(ctx, &desc_val) else {
                if ctx.pending_exception.is_some() {
                    return JSValue::undefined();
                }
                continue;
            };
            new_obj.define_property_ext(atom, desc, true, true, true);
        }
    }

    let ptr = Box::into_raw(Box::new(new_obj)) as usize;
    JSValue::new_object(ptr)
}

fn object_get_prototype_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::null();
    }
    let obj_val = &args[0];
    if obj_val.is_undefined() || obj_val.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Cannot convert undefined or null to object");
        return JSValue::undefined();
    }
    let obj = if obj_val.is_object_like() {
        *obj_val
    } else {
        object_to_object(ctx, obj_val)
    };
    let obj_ref = obj.as_object();
    if let Some(proto_ptr) = obj_ref.prototype {
        return JSValue::new_object(proto_ptr as usize);
    }
    JSValue::null()
}

fn object_set_prototype_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 || !args[0].is_object_like() {
        crate::builtins::global::throw_type_error(ctx, "Object.setPrototypeOf requires an object");
        return JSValue::undefined();
    }

    let obj = args[0].as_object_mut();
    if args[1].is_object_like() {
        let new_proto = args[1].get_ptr() as *mut JSObject;
        // Object.prototype is immutable
        if let Some(proto_ptr) = ctx.get_object_prototype() {
            if obj as *mut JSObject == proto_ptr {
                // Check if the new prototype is the same as current (null for Object.prototype)
                if obj.prototype != Some(new_proto) {
                    let mut err = crate::object::object::JSObject::new_typed(
                        crate::object::object::ObjectType::Error,
                    );
                    if let Some(proto) = ctx.get_type_error_prototype() {
                        err.prototype = Some(proto);
                    }
                    err.set(
                        ctx.common_atoms.message,
                        JSValue::new_string(ctx.intern("Object.prototype is immutable")),
                    );
                    let eptr = Box::into_raw(Box::new(err)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track(eptr);
                    ctx.pending_exception = Some(JSValue::new_object(eptr));
                    return JSValue::undefined();
                }
            }
        }
        // Check for prototype chain cycles
        if !obj.extensible() && obj.prototype != Some(new_proto) {
            let mut err = crate::object::object::JSObject::new_typed(
                crate::object::object::ObjectType::Error,
            );
            if let Some(proto) = ctx.get_type_error_prototype() {
                err.prototype = Some(proto);
            }
            err.set(
                ctx.common_atoms.message,
                JSValue::new_string(ctx.intern("Cannot set prototype of a non-extensible object")),
            );
            let eptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(eptr);
            ctx.pending_exception = Some(JSValue::new_object(eptr));
            return JSValue::undefined();
        }
        let mut current = new_proto;
        let mut depth = 0u32;
        while let Some(ptr) = unsafe { current.as_ref() }.and_then(|o| o.prototype) {
            if ptr == obj as *mut JSObject || depth > 1000 {
                let mut err = crate::object::object::JSObject::new_typed(
                    crate::object::object::ObjectType::Error,
                );
                if let Some(proto) = ctx.get_type_error_prototype() {
                    err.prototype = Some(proto);
                }
                err.set(
                    ctx.common_atoms.message,
                    JSValue::new_string(ctx.intern("Cyclic __proto__ value")),
                );
                let eptr = Box::into_raw(Box::new(err)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track(eptr);
                ctx.pending_exception = Some(JSValue::new_object(eptr));
                return JSValue::undefined();
            }
            current = ptr;
            depth += 1;
        }
        obj.prototype = Some(new_proto);
    } else if args[1].is_null() {
        obj.prototype = None;
    }

    args[0].clone()
}

fn object_has_own_property(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::bool(false);
    }
    let key_atom = if args.len() > 1 {
        to_property_atom(ctx, &args[1])
    } else {
        None
    };
    let this_val = &args[0];
    if this_val.is_undefined() || this_val.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Object.prototype.hasOwnProperty called on incompatible receiver");
        return JSValue::undefined();
    }
    let this_obj = if this_val.is_object_like() {
        this_val.clone()
    } else {
        object_to_object(ctx, this_val)
    };
    match key_atom {
        Some(atom) => JSValue::bool(
            this_obj.as_object().has_own(atom) || ctx.array_index_has_own(&this_obj, atom),
        ),
        None => JSValue::bool(false),
    }
}

fn object_value_of(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    args[0].clone()
}

fn object_to_string(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::new_string(_ctx.intern("[object Object]"));
    }
    let this = &args[0];
    if this.is_undefined() {
        return JSValue::new_string(_ctx.intern("[object Undefined]"));
    }
    if this.is_null() {
        return JSValue::new_string(_ctx.intern("[object Null]"));
    }
    if is_object_like(this) {
        let obj = this.as_object();
        let mut tag = "Object";
        if this.is_function() {
            tag = "Function";
        } else {
            match obj.obj_type() {
                ObjectType::Array => tag = "Array",
                ObjectType::Error => tag = "Error",
                ObjectType::Date => tag = "Date",
                ObjectType::RegExp => tag = "RegExp",
                ObjectType::BigInt => tag = "BigInt",
                ObjectType::Boolean => tag = "Boolean",
                ObjectType::Promise => tag = "Promise",
                _ => {
                    if obj.get(_ctx.common_atoms.__value__).is_some() {
                        let v = obj.get(_ctx.common_atoms.__value__).unwrap();
                        if v.is_int() || v.is_float() {
                            tag = "Number";
                        } else if v.is_bool() {
                            tag = "Boolean";
                        } else if v.is_string() {
                            tag = "String";
                        }
                    }
                    if tag == "Number" || tag == "Boolean" {
                        if let Some(date_proto) = _ctx.get_date_prototype() {
                            let mut p = obj.prototype;
                            while let Some(pp) = p {
                                if pp == date_proto {
                                    tag = "Date";
                                    break;
                                }
                                unsafe {
                                    p = (*pp).prototype;
                                }
                            }
                        }
                    }
                }
            }
        }
        let sym_tag_atom = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(_ctx);
        if let Some(v) = obj.get(sym_tag_atom) {
            if v.is_string() {
                let s = _ctx.get_atom_str(v.get_atom());
                if !s.is_empty() {
                    return JSValue::new_string(_ctx.intern(&format!("[object {}]", s)));
                }
            }
        }
        return JSValue::new_string(_ctx.intern(&format!("[object {}]", tag)));
    }

    JSValue::new_string(_ctx.intern("[object Object]"))
}

fn object_to_locale_string(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    object_to_string(_ctx, args)
}

fn object_is_prototype_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let v = &args[1];
    if !v.is_object_like() {
        return JSValue::bool(false);
    }
    let this = &args[0];
    if !this.is_object_like() {
        crate::builtins::global::throw_type_error(ctx, "isPrototypeOf called on incompatible receiver");
        return JSValue::undefined();
    }
    let this_ptr = this.get_ptr() as *mut JSObject;
    let mut current = v.get_ptr() as *mut JSObject;
    loop {
        if current.is_null() {
            return JSValue::bool(false);
        }
        if current == this_ptr {
            return JSValue::bool(true);
        }
        unsafe {
            if let Some(proto) = (*current).prototype {
                current = proto;
            } else {
                return JSValue::bool(false);
            }
        }
    }
}

fn object_property_is_enumerable(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = &args[0];
    if !is_object_like(this) {
        return JSValue::bool(false);
    }
    let Some(atom) = to_property_atom(ctx, &args[1]) else {
        return JSValue::bool(false);
    };
    let obj = this.as_object();
    if let Some(desc) = ctx
        .array_index_own_descriptor(this, atom)
        .or_else(|| obj.get_own_descriptor(atom))
    {
        return JSValue::bool(desc.enumerable);
    }
    JSValue::bool(false)
}

fn lookup_accessor(ctx: &mut JSContext, args: &[JSValue], want_getter: bool) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this_val = &args[0];
    if this_val.is_undefined() || this_val.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Object.prototype.__lookupGetter__ called on incompatible receiver");
        return JSValue::undefined();
    }
    let this = if this_val.is_object_like() {
        this_val.clone()
    } else {
        object_to_object(ctx, this_val)
    };
    let Some(atom) = to_property_atom(ctx, &args[1]) else {
        return JSValue::undefined();
    };
    let mut current = Some(this.get_ptr() as *mut JSObject);
    while let Some(ptr) = current {
        let obj = unsafe { &*ptr };
        if let Some(entry) = obj.get_own_accessor_entry(atom) {
            let v = if want_getter { entry.get } else { entry.set };
            return v.unwrap_or(JSValue::undefined());
        }
        if obj.has_own(atom) {
            return JSValue::undefined();
        }
        current = obj.prototype;
    }
    JSValue::undefined()
}

fn object_lookup_getter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    lookup_accessor(ctx, args, true)
}

fn object_lookup_setter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    lookup_accessor(ctx, args, false)
}

fn define_legacy_accessor(
    ctx: &mut JSContext,
    args: &[JSValue],
    getter: bool,
) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(p) = ctx.get_type_error_prototype() {
            err.prototype = Some(p);
        }
        err.set(
            ctx.common_atoms.name,
            JSValue::new_string(ctx.intern("TypeError")),
        );
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("this is not an object")),
        );
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    let Some(atom) = to_property_atom(ctx, &args[1]) else {
        return args[0].clone();
    };
    let func = args.get(2).copied().unwrap_or(JSValue::undefined());
    if !func.is_undefined() && !func.is_function() {
        let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(p) = ctx.get_type_error_prototype() {
            err.prototype = Some(p);
        }
        err.set(
            ctx.common_atoms.name,
            JSValue::new_string(ctx.intern("TypeError")),
        );
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("getter/setter must be a function")),
        );
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    let pd = crate::object::object::PropertyDescriptor {
        value: None,
        writable: false,
        enumerable: true,
        configurable: true,
        get: if getter { Some(func) } else { None },
        set: if getter { None } else { Some(func) },
    };
    let obj = args[0].as_object_mut();
    let ok = obj.define_property_ext(atom, pd, false, true, true);
    if !ok {
        let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(p) = ctx.get_type_error_prototype() {
            err.prototype = Some(p);
        }
        err.set(
            ctx.common_atoms.name,
            JSValue::new_string(ctx.intern("TypeError")),
        );
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("Cannot redefine property")),
        );
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    args[0].clone()
}

fn object_define_getter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    define_legacy_accessor(ctx, args, true)
}

fn object_define_setter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    define_legacy_accessor(ctx, args, false)
}

fn object_from_entries(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut result = JSObject::new();
    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        result.prototype = Some(obj_proto_ptr);
    }
    if args.is_empty() || !args[0].is_object() {
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }
    let iterable = args[0];
    let len_atom = ctx.common_atoms.length;
    let len = {
        let arr = iterable.as_object();
        arr.get(len_atom).map(|v| v.get_int() as usize).unwrap_or(0)
    };
    for i in 0..len {
        let key = ctx.int_atom_mut(i);
        let entry = {
            let arr = iterable.as_object();
            arr.get(key)
        };
        if let Some(entry_val) = entry {
            if entry_val.is_object() {
                let entry_obj = entry_val.as_object();
                let key0_atom = ctx.intern("0");
                let key1_atom = ctx.intern("1");
                if let (Some(k), Some(v)) = (entry_obj.get(key0_atom), entry_obj.get(key1_atom)) {
                    if k.is_string() {
                        let prop_atom = k.get_atom();
                        result.set(prop_atom, v);
                    }
                }
            }
        }
    }
    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(result_ptr)
}

fn object_has_own(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let obj_val = &args[0];
    if obj_val.is_undefined() || obj_val.is_null() {
        crate::builtins::global::throw_type_error(ctx, "Cannot convert undefined or null to object");
        return JSValue::undefined();
    }
    let obj = if obj_val.is_object_like() {
        *obj_val
    } else {
        object_to_object(ctx, obj_val)
    };
    let prop = &args[1];
    let Some(atom) = to_property_atom(ctx, prop) else {
        return JSValue::bool(false);
    };
    JSValue::bool(obj.as_object().has_own(atom) || ctx.array_index_has_own(&obj, atom))
}

fn object_is(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let a = args.get(0).copied().unwrap_or(JSValue::undefined());
    let b = args.get(1).copied().unwrap_or(JSValue::undefined());

    let af = if a.is_float() {
        Some(a.get_float())
    } else if a.is_int() {
        Some(a.get_int() as f64)
    } else {
        None
    };
    let bf = if b.is_float() {
        Some(b.get_float())
    } else if b.is_int() {
        Some(b.get_int() as f64)
    } else {
        None
    };

    if let (Some(af), Some(bf)) = (af, bf) {
        if af.is_nan() && bf.is_nan() {
            return JSValue::bool(true);
        }
        if af == 0.0 && bf == 0.0 {
            return JSValue::bool(af.signum() == bf.signum());
        }
        return JSValue::bool(af == bf);
    }
    JSValue::bool(a.strict_eq(&b))
}

fn object_get_own_property_symbols(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }

    let obj_val = &args[0];
    if !is_object_like(obj_val) {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }

    let obj = obj_val.as_object();

    let symbols = obj.symbol_keys();

    let mut result = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        result.prototype = Some(proto_ptr);
    }
    let length_atom = ctx.common_atoms.length;
    result.set(length_atom, JSValue::new_int(symbols.len() as i64));

    for (i, sym) in symbols.iter().enumerate() {
        let symbol_id = sym.0 & 0x3FFFFFFF;
        let description = ctx
            .runtime()
            .symbol_description(symbol_id)
            .unwrap_or(crate::builtins::symbol::NO_DESCRIPTION);
        let value = JSValue::new_symbol_with_id(description, symbol_id);
        result.set(ctx.int_atom_mut(i), value);
    }

    let result_ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(result_ptr)
}

fn object_prevent_extensions(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return args.get(0).cloned().unwrap_or(JSValue::undefined());
    }
    let obj = args[0].as_object_mut();
    obj.set_extensible(false);
    args[0].clone()
}

fn object_is_extensible(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return JSValue::bool(false);
    }
    let obj = args[0].as_object();
    JSValue::bool(obj.extensible())
}

fn object_seal(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return args.get(0).cloned().unwrap_or(JSValue::undefined());
    }
    let obj = args[0].as_object_mut();
    obj.set_extensible(false);
    obj.set_sealed(true);
    obj.seal_accessors();

    obj.for_each_property_attrs_mut(|_atom, _val, attrs| {
        *attrs &= !crate::object::object::ATTR_CONFIGURABLE;
    });
    args[0].clone()
}

fn object_is_sealed(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return JSValue::bool(true);
    }
    let obj = args[0].as_object();
    if obj.extensible() {
        return JSValue::bool(false);
    }

    let mut all_non_configurable = true;
    obj.for_each_property(|_atom, _val, attrs| {
        if attrs & crate::object::object::ATTR_CONFIGURABLE != 0 {
            all_non_configurable = false;
        }
    });
    JSValue::bool(all_non_configurable)
}

fn object_freeze(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return args.get(0).cloned().unwrap_or(JSValue::undefined());
    }
    let obj = args[0].as_object_mut();
    obj.set_extensible(false);
    obj.set_sealed(true);
    obj.set_frozen(true);
    obj.seal_accessors();

    obj.for_each_property_attrs_mut(|_atom, _val, attrs| {
        *attrs &=
            !(crate::object::object::ATTR_WRITABLE | crate::object::object::ATTR_CONFIGURABLE);
    });
    args[0].clone()
}

fn object_is_frozen(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || !is_object_like(&args[0]) {
        return JSValue::bool(true);
    }
    let obj = args[0].as_object();
    if obj.extensible() || !obj.sealed() || !obj.frozen() {
        return JSValue::bool(false);
    }

    let mut all_frozen = true;
    obj.for_each_property(|_atom, _val, attrs| {
        if attrs & (crate::object::object::ATTR_WRITABLE | crate::object::object::ATTR_CONFIGURABLE)
            != 0
        {
            all_frozen = false;
        }
    });
    JSValue::bool(all_frozen)
}

fn object_define_property(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let obj_val = args.first().cloned().unwrap_or(JSValue::undefined());
    if !is_object_like(&obj_val) {
        let mut err = crate::object::object::JSObject::new_typed(
            crate::object::object::ObjectType::Error,
        );
        if let Some(proto) = _ctx.get_type_error_prototype() { err.prototype = Some(proto); }
        err.set(_ctx.common_atoms.name, JSValue::new_string(_ctx.intern("TypeError")));
        err.set(_ctx.common_atoms.message, JSValue::new_string(_ctx.intern("Object.defineProperty called on non-object")));
        let ptr = Box::into_raw(Box::new(err)) as usize;
        _ctx.runtime_mut().gc_heap_mut().track(ptr);
        _ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    if args.len() < 3 {
        return obj_val;
    }
    let obj = &obj_val;
    let prop = &args[1];
    let descriptor = &args[2];

    if !descriptor.is_object_like() {
        let mut err = crate::object::object::JSObject::new_typed(
            crate::object::object::ObjectType::Error,
        );
        if let Some(proto) = _ctx.get_type_error_prototype() { err.prototype = Some(proto); }
        err.set(_ctx.common_atoms.name, JSValue::new_string(_ctx.intern("TypeError")));
        err.set(_ctx.common_atoms.message, JSValue::new_string(_ctx.intern("Property description must be an object")));
        let ptr = Box::into_raw(Box::new(err)) as usize;
        _ctx.runtime_mut().gc_heap_mut().track(ptr);
        _ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    let Some(prop_atom) = to_property_atom(_ctx, prop) else {
        return obj.clone();
    };

    let has_enumerable = object_has_property(descriptor, _ctx.intern("enumerable"));
    let has_writable = object_has_property(descriptor, _ctx.intern("writable"));
    let has_configurable = object_has_property(descriptor, _ctx.intern("configurable"));

    let Some(pd) = to_property_descriptor(_ctx, descriptor) else {
        if _ctx.pending_exception.is_some() {
            return JSValue::undefined();
        }
        return obj.clone();
    };

    if let Some(allowed) = namespace_define_own_property(
        _ctx,
        obj,
        prop_atom,
        &pd,
        has_writable,
        has_enumerable,
        has_configurable,
        pd.value.is_some(),
    ) {
        if !allowed {
            let mut err = crate::object::object::JSObject::new_typed(
                crate::object::object::ObjectType::Error,
            );
            if let Some(proto) = _ctx.get_type_error_prototype() {
                err.prototype = Some(proto);
            }
            err.set(
                _ctx.common_atoms.name,
                JSValue::new_string(_ctx.intern("TypeError")),
            );
            err.set(
                _ctx.common_atoms.message,
                JSValue::new_string(_ctx.intern("Cannot redefine property")),
            );
            let ptr = Box::into_raw(Box::new(err)) as usize;
            _ctx.runtime_mut().gc_heap_mut().track(ptr);
            _ctx.pending_exception = Some(JSValue::new_object(ptr));
            return JSValue::undefined();
        }
        return obj.clone();
    }

    let obj_ref = obj.as_object_mut();

    // Array exotic object: length is always non-configurable
    if obj_ref.is_array() {
        let len_atom = _ctx.intern("length");
        if prop_atom == len_atom {
            if has_configurable && pd.configurable {
                let mut err = crate::object::object::JSObject::new_typed(
                    crate::object::object::ObjectType::Error,
                );
                if let Some(proto) = _ctx.get_type_error_prototype() {
                    err.prototype = Some(proto);
                }
                err.set(
                    _ctx.common_atoms.name,
                    JSValue::new_string(_ctx.intern("TypeError")),
                );
                err.set(
                    _ctx.common_atoms.message,
                    JSValue::new_string(_ctx.intern("Cannot redefine property: length")),
                );
                let ptr = Box::into_raw(Box::new(err)) as usize;
                _ctx.runtime_mut().gc_heap_mut().track(ptr);
                _ctx.pending_exception = Some(JSValue::new_object(ptr));
                return JSValue::undefined();
            }
            if let Some(v) = pd.value {
                let valid_len = if v.is_int() {
                    let i = v.get_int();
                    i >= 0 && i <= 0xFFFF_FFFF
                } else if v.is_float() {
                    let f = v.get_float();
                    f.is_finite() && f.fract() == 0.0 && f >= 0.0 && f <= 4294967295.0
                } else if v.is_undefined() {
                    true
                } else {
                    false
                };
                if !valid_len {
                    let mut err = crate::object::object::JSObject::new_typed(
                        crate::object::object::ObjectType::Error,
                    );
                    if let Some(proto) = _ctx.get_range_error_prototype() {
                        err.prototype = Some(proto);
                    }
                    err.set(
                        _ctx.common_atoms.name,
                        JSValue::new_string(_ctx.intern("RangeError")),
                    );
                    err.set(
                        _ctx.common_atoms.message,
                        JSValue::new_string(_ctx.intern("Invalid array length")),
                    );
                    let ptr = Box::into_raw(Box::new(err)) as usize;
                    _ctx.runtime_mut().gc_heap_mut().track(ptr);
                    _ctx.pending_exception = Some(JSValue::new_object(ptr));
                    return JSValue::undefined();
                }
            }
        }
    }

    let ok = obj_ref.define_property_ext(
        prop_atom,
        pd,
        has_writable,
        has_enumerable,
        has_configurable,
    );
    if !ok {
        let name_str = if prop.is_string() {
            _ctx.get_atom_str(prop.get_atom()).to_string()
        } else if prop.is_int() {
            prop.get_int().to_string()
        } else {
            String::new()
        };
        let mut err = crate::object::object::JSObject::new_typed(
            crate::object::object::ObjectType::Error,
        );
        if let Some(proto) = _ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        err.set(
            _ctx.common_atoms.name,
            JSValue::new_string(_ctx.intern("TypeError")),
        );
        err.set(
            _ctx.common_atoms.message,
            JSValue::new_string(_ctx.intern(&format!("Cannot redefine property: {}", name_str))),
        );
        let ptr = Box::into_raw(Box::new(err)) as usize;
        _ctx.runtime_mut().gc_heap_mut().track(ptr);
        _ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }
    obj.clone()
}

fn object_define_properties(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return args.get(0).cloned().unwrap_or(JSValue::undefined());
    }
    let obj = &args[0];
    let props = &args[1];

    if !obj.is_object_like() || !props.is_object_like() {
        return obj.clone();
    }

    let props_list: Vec<(Atom, JSValue)> = props.as_object().own_properties();

    for (key_atom, _) in props_list {
        let desc_val = object_get_property(_ctx, props, key_atom);
        if _ctx.pending_exception.is_some() {
            return JSValue::undefined();
        }
        let Some(pd) = to_property_descriptor(_ctx, &desc_val) else {
            if _ctx.pending_exception.is_some() {
                return JSValue::undefined();
            }
            continue;
        };

        let obj_ref = obj.as_object_mut();
        let ok = obj_ref.define_property(key_atom, pd);
        if !ok {
            let mut err = crate::object::object::JSObject::new_typed(
                crate::object::object::ObjectType::Error,
            );
            if let Some(proto) = _ctx.get_type_error_prototype() {
                err.prototype = Some(proto);
            }
            err.set(
                _ctx.common_atoms.name,
                JSValue::new_string(_ctx.intern("TypeError")),
            );
            err.set(
                _ctx.common_atoms.message,
                JSValue::new_string(_ctx.intern("Cannot redefine property")),
            );
            let ptr = Box::into_raw(Box::new(err)) as usize;
            _ctx.runtime_mut().gc_heap_mut().track(ptr);
            _ctx.pending_exception = Some(JSValue::new_object(ptr));
            return JSValue::undefined();
        }
    }

    obj.clone()
}

pub fn namespace_same_value(a: &JSValue, b: &JSValue) -> bool {
    if a.is_string() && b.is_string() {
        return a.get_atom().0 == b.get_atom().0;
    }
    if a.is_int() && b.is_int() {
        return a.get_int() == b.get_int();
    }
    if a.is_float() && b.is_float() {
        let (x, y) = (a.get_float(), b.get_float());
        return x == y || (x.is_nan() && y.is_nan());
    }
    a.raw_bits() == b.raw_bits()
}

pub fn namespace_define_own_property(
    ctx: &mut JSContext,
    obj: &JSValue,
    prop_atom: crate::runtime::atom::Atom,
    pd: &crate::object::object::PropertyDescriptor,
    has_writable: bool,
    has_enumerable: bool,
    has_configurable: bool,
    has_value: bool,
) -> Option<bool> {
    let obj_ref = obj.as_object();
    if !obj_ref.is_module_namespace() {
        return None;
    }
    let Some(own) = obj_ref.get_own_descriptor(prop_atom) else {
        return Some(false);
    };
    if pd.get.is_some() || pd.set.is_some() {
        return Some(false);
    }
    let is_symbol = prop_atom.0 & 0x40000000 != 0;
    if has_value {
        let projected = namespace_live_descriptor_value(ctx, obj, &own);
        if let Some(value) = pd.value {
            if !namespace_same_value(&value, &projected) {
                return Some(false);
            }
        }
    }
    if has_writable && pd.writable != (!is_symbol || own.writable) {
        return Some(false);
    }
    if has_enumerable && pd.enumerable != own.enumerable {
        return Some(false);
    }
    if has_configurable && pd.configurable {
        return Some(false);
    }
    Some(true)
}

pub fn namespace_live_descriptor_value(
    ctx: &mut JSContext,
    obj: &JSValue,
    desc: &crate::object::object::PropertyDescriptor,
) -> JSValue {
    if let Some(getter) = desc.get {
        if getter.is_function() {
            if let Some(vm_ptr) = ctx.get_register_vm_ptr() {
                let vm = unsafe { &mut *(vm_ptr as *mut crate::runtime::vm::VM) };
                if let Ok(value) = vm.call_function_with_this(ctx, getter, *obj, &[]) {
                    return value;
                }
            }
        }
        return JSValue::undefined();
    }
    desc.value.unwrap_or(JSValue::undefined())
}

fn object_get_own_property_descriptor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 || !(args[0].is_object() || args[0].is_function()) {
        return JSValue::undefined();
    }
    let obj = &args[0];
    let Some(prop_atom) = to_property_atom(ctx, &args[1]) else {
        return JSValue::undefined();
    };

    let obj_ref = obj.as_object();

    if let Some(pd) = ctx
        .array_index_own_descriptor(obj, prop_atom)
        .or_else(|| obj_ref.get_own_descriptor(prop_atom))
    {
        let mut desc_obj = JSObject::new();
        desc_obj.prototype = ctx.get_object_prototype();
        if obj_ref.is_module_namespace() && prop_atom.0 & 0x40000000 == 0 {
            let value = namespace_live_descriptor_value(ctx, obj, &pd);
            desc_obj.set(ctx.intern("value"), value);
            desc_obj.set(ctx.intern("writable"), JSValue::bool(true));
        } else if pd.get.is_some() || pd.set.is_some() {
            desc_obj.set(ctx.intern("get"), pd.get.unwrap_or(JSValue::undefined()));
            desc_obj.set(ctx.intern("set"), pd.set.unwrap_or(JSValue::undefined()));
        } else {
            if let Some(v) = pd.value {
                desc_obj.set(ctx.intern("value"), v);
            }
            desc_obj.set(ctx.intern("writable"), JSValue::bool(pd.writable));
        }
        desc_obj.set(ctx.intern("enumerable"), JSValue::bool(pd.enumerable));
        desc_obj.set(ctx.intern("configurable"), JSValue::bool(pd.configurable));

        let ptr = Box::into_raw(Box::new(desc_obj)) as usize;
        return JSValue::new_object(ptr);
    }

    JSValue::undefined()
}

fn object_get_own_property_descriptors(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut result = JSObject::new();

    if args.is_empty() || !(args[0].is_object() || args[0].is_function()) {
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    }

    let obj = &args[0];
    let obj_ref = obj.as_object();

    let is_namespace = obj_ref.is_module_namespace();
    for key_atom in obj_ref.keys() {
        if let Some(pd) = obj_ref.get_own_descriptor(key_atom) {
            let mut desc_obj = JSObject::new();
            desc_obj.prototype = ctx.get_object_prototype();
            if is_namespace && key_atom.0 & 0x40000000 == 0 {
                let value = namespace_live_descriptor_value(ctx, obj, &pd);
                desc_obj.set(ctx.intern("value"), value);
                desc_obj.set(ctx.intern("writable"), JSValue::bool(true));
            } else if pd.get.is_some() || pd.set.is_some() {
                desc_obj.set(ctx.intern("get"), pd.get.unwrap_or(JSValue::undefined()));
                desc_obj.set(ctx.intern("set"), pd.set.unwrap_or(JSValue::undefined()));
            } else {
                if let Some(v) = pd.value {
                    desc_obj.set(ctx.intern("value"), v);
                }
                desc_obj.set(ctx.intern("writable"), JSValue::bool(pd.writable));
            }
            desc_obj.set(ctx.intern("enumerable"), JSValue::bool(pd.enumerable));
            desc_obj.set(ctx.intern("configurable"), JSValue::bool(pd.configurable));

            let desc_ptr = Box::into_raw(Box::new(desc_obj)) as usize;
            result.set(key_atom, JSValue::new_object(desc_ptr));
        }
    }

    let ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(ptr)
}

fn object_get_own_property_names(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() || args[0].is_null() || args[0].is_undefined() {
        let mut err =
            crate::object::object::JSObject::new_typed(crate::object::object::ObjectType::Error);
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        err.set(
            ctx.common_atoms.message,
            JSValue::new_string(ctx.intern("Object.getOwnPropertyNames called on non-object")),
        );
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        ctx.pending_exception = Some(JSValue::new_object(ptr));
        return JSValue::undefined();
    }

    let arg = &args[0];
    let obj_val = if arg.is_object() || arg.is_function() {
        arg.clone()
    } else if arg.is_string() {
        let mut wrapper = crate::object::object::JSObject::new();
        if let Some(proto) = ctx.get_object_prototype() {
            wrapper.prototype = Some(proto);
        }
        wrapper.set(
            ctx.common_atoms.length,
            JSValue::new_int(arg.get_atom().0 as i64),
        );
        let ptr = Box::into_raw(Box::new(wrapper)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        JSValue::new_object(ptr)
    } else {
        let mut result = JSObject::new_array();
        if let Some(proto_ptr) = ctx.get_array_prototype() {
            result.prototype = Some(proto_ptr);
        }
        result.set(ctx.common_atoms.length, JSValue::new_int(0));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        return JSValue::new_object(ptr);
    };

    let obj_ref = obj_val.as_object();
    let is_namespace = obj_ref.is_module_namespace();

    let mut names: Vec<Atom> = obj_ref
        .keys()
        .into_iter()
        .filter(|atom| !ctx.is_symbol_atom(*atom))
        .collect();
    names.sort_by(|a, b| {
        let a_str = ctx.get_atom_str(*a);
        let b_str = ctx.get_atom_str(*b);
        let a_is_idx = a_str.parse::<i64>().is_ok();
        let b_is_idx = b_str.parse::<i64>().is_ok();
        match (a_is_idx, b_is_idx) {
            (true, true) => a_str
                .parse::<i64>()
                .unwrap()
                .cmp(&b_str.parse::<i64>().unwrap()),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => {
                if is_namespace {
                    a_str.encode_utf16().cmp(b_str.encode_utf16())
                } else {
                    std::cmp::Ordering::Equal
                }
            }
        }
    });

    let mut result = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        result.prototype = Some(proto_ptr);
    }

    result.set(
        ctx.common_atoms.length,
        JSValue::new_int(names.len() as i64),
    );
    for (i, key) in names.iter().enumerate() {
        let key_str = ctx.get_atom_str(*key).to_string();
        result.set(
            ctx.int_atom_mut(i),
            JSValue::new_string(ctx.intern(&key_str)),
        );
    }

    let ptr = Box::into_raw(Box::new(result)) as usize;
    JSValue::new_object(ptr)
}
