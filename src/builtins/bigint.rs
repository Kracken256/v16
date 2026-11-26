use crate::builtins::bignum::BigInt;
use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::object::object::PropertyDescriptor;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn create_builtin_function(ctx: &mut JSContext, name: &str) -> JSValue {
    let mut func = JSFunction::new_builtin(ctx.intern(name), 2);
    func.set_builtin_marker(ctx, name);
    ctx.allocate_function(func)
}

fn set_ne(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
    obj.define_property(key, PropertyDescriptor {
        value: Some(val), writable: true, enumerable: false, configurable: true,
        get: None, set: None,
    });
}

fn throw_type_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_type_error_prototype() { err.prototype = Some(proto); }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
    err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    let error_value = ctx.allocate_object(err);
    ctx.pending_exception = Some(error_value);
}

fn throw_range_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_range_error_prototype() { err.prototype = Some(proto); }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("RangeError")));
    err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    let error_value = ctx.allocate_object(err);
    ctx.pending_exception = Some(error_value);
}

fn throw_syntax_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_syntax_error_prototype() { err.prototype = Some(proto); }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("SyntaxError")));
    err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    let error_value = ctx.allocate_object(err);
    ctx.pending_exception = Some(error_value);
}

fn create_bigint_value(value: BigInt) -> JSValue {
    let mut bigint_obj = JSObject::new_bigint();
    bigint_obj.set_bigint_value(value);
    JSValue::new_bigint_boxed(Box::new(bigint_obj))
}

fn to_primitive_number(ctx: &mut JSContext, value: JSValue) -> Option<JSValue> {
    if !value.is_object_like() {
        return Some(value);
    }
    ctx.with_vm(|vm, ctx| {
        let to_prim_sym =
            crate::builtins::symbol::get_or_create_well_known_symbol(ctx, "Symbol.toPrimitive");
        let to_prim_key = crate::runtime::atom::Atom(0x40000000 | to_prim_sym.get_symbol_id());
        let to_primitive = value.try_as_object().expect("object value").get(to_prim_key);
        if let Some(method) = to_primitive {
            if !method.is_function() {
                throw_type_error(ctx, "Cannot convert object to primitive value");
                return None;
            }
            let hint = JSValue::new_string(ctx.intern("number"));
            match vm.call_function_with_this(ctx, method, value, &[hint]) {
                Ok(result) => {
                    if result.is_object_like() {
                        throw_type_error(ctx, "Cannot convert object to primitive value");
                        return None;
                    }
                    return Some(result);
                }
                Err(_) => return None,
            }
        }
        for atom in [ctx.common_atoms.value_of, ctx.common_atoms.to_string] {
            let method = value.try_as_object().expect("object value").get(atom);
            if let Some(f) = method {
                if f.is_function() {
                    match vm.call_function_with_this(ctx, f, value, &[]) {
                        Ok(result) => {
                            if !result.is_object_like() {
                                return Some(result);
                            }
                        }
                        Err(_) => return None,
                    }
                }
            }
        }
        throw_type_error(ctx, "Cannot convert object to primitive value");
        None
    })
    .flatten()
}

fn string_to_bigint(ctx: &mut JSContext, text: &str) -> Option<BigInt> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(BigInt::zero());
    }
    let parsed = if let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        BigInt::from_str_radix(rest, 16)
    } else if let Some(rest) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        BigInt::from_str_radix(rest, 8)
    } else if let Some(rest) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        BigInt::from_str_radix(rest, 2)
    } else {
        BigInt::from_str_radix(trimmed, 10)
    };
    match parsed {
        Some(value) => Some(value),
        None => {
            throw_syntax_error(ctx, "Cannot convert string to BigInt");
            None
        }
    }
}

fn number_to_bigint(ctx: &mut JSContext, number: f64) -> Option<BigInt> {
    if number.is_nan() || number.is_infinite() || number.fract() != 0.0 {
        throw_range_error(
            ctx,
            "The number cannot be converted to a BigInt because it is not an integer",
        );
        return None;
    }
    BigInt::from_f64_integral(number)
}

fn to_bigint(ctx: &mut JSContext, value: JSValue) -> Option<BigInt> {
    let primitive = to_primitive_number(ctx, value)?;
    if primitive.is_bigint() {
        return Some((*primitive.try_as_object().expect("object value").get_bigint_value()).clone());
    }
    if primitive.is_bool() {
        return Some(BigInt::from_i64(if primitive.get_bool() { 1 } else { 0 }));
    }
    if primitive.is_string() {
        let text = ctx.get_atom_str(primitive.get_atom()).to_string();
        return string_to_bigint(ctx, &text);
    }
    throw_type_error(ctx, "Cannot convert value to a BigInt");
    None
}

fn to_index(ctx: &mut JSContext, value: JSValue) -> Option<u64> {
    let primitive = to_primitive_number(ctx, value)?;
    let number = match crate::builtins::global::js_to_number_value(ctx, &primitive) {
        Ok(n) => n,
        Err(()) => return None,
    };
    if number.is_nan() {
        return Some(0);
    }
    let integer = number.trunc();
    if integer < 0.0 || integer > 9007199254740991.0 {
        throw_range_error(ctx, "The number is out of range");
        return None;
    }
    Some(integer as u64)
}

pub fn init_bigint(ctx: &mut JSContext) {
    let bigint_atom = ctx.intern("BigInt");

    let mut func = JSFunction::new_builtin(bigint_atom, 1);
    func.set_builtin_marker(ctx, "bigint_constructor");
    let mut bigint_value = ctx.allocate_function(func);

    let bigint_func_obj = bigint_value.try_as_object_mut().expect("object value");
    set_ne(bigint_func_obj, ctx.intern("asIntN"), create_builtin_function(ctx, "bigint_as_int_n"));
    set_ne(bigint_func_obj, ctx.intern("asUintN"), create_builtin_function(ctx, "bigint_as_uint_n"));

    let proto_atom = ctx.intern("BigIntPrototype");
    let mut proto_obj = JSObject::new();
    proto_obj.prototype = ctx.get_object_prototype();
    let to_string_fn = create_builtin_function(ctx, "bigint_to_string");
    let to_locale_fn = create_builtin_function(ctx, "bigint_to_locale_string");
    let value_of_fn = create_builtin_function(ctx, "bigint_value_of");
    for mut f in [to_string_fn, to_locale_fn, value_of_fn] {
        if f.is_function() {
            f.try_as_function_mut().expect("function value").base.define_property(
                ctx.common_atoms.length,
                PropertyDescriptor {
                    value: Some(JSValue::new_int(0)),
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    get: None,
                    set: None,
                },
            );
        }
    }
    set_ne(&mut proto_obj, ctx.intern("toString"), to_string_fn);
    set_ne(
        &mut proto_obj,
        ctx.intern("toLocaleString"),
        to_locale_fn,
    );
    set_ne(&mut proto_obj, ctx.intern("valueOf"), value_of_fn);
    set_ne(&mut proto_obj, ctx.common_atoms.constructor, bigint_value);
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        proto_obj.define_property(
            tag_key,
            PropertyDescriptor {
                value: Some(JSValue::new_string(ctx.intern("BigInt"))),
                writable: false,
                enumerable: false,
                configurable: true,
                get: None,
                set: None,
            },
        );
    }
    let proto_value = ctx.allocate_object(proto_obj);
    ctx.set_bigint_prototype(proto_value.get_ptr());

    let bigint_func = bigint_value.try_as_object_mut().expect("object value");
    bigint_func.define_property(
        ctx.intern("prototype"),
        PropertyDescriptor {
            value: Some(proto_value),
            writable: false,
            enumerable: false,
            configurable: false,
            get: None,
            set: None,
        },
    );

    let mut global = ctx.global();
    if global.is_object() {
        let global_obj = global.try_as_object_mut().expect("object value");
        crate::builtins::global::set_non_enumerable(global_obj, bigint_atom, bigint_value);
        crate::builtins::global::set_non_enumerable(global_obj, proto_atom, proto_value);
    }
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin("bigint_constructor", HostFunction::ctor("BigInt", 1, bigint_constructor));
    ctx.register_builtin("bigint_as_int_n", HostFunction::new("asIntN", 2, bigint_as_int_n));
    ctx.register_builtin("bigint_as_uint_n", HostFunction::new("asUintN", 2, bigint_as_uint_n));
    ctx.register_builtin("bigint_to_string", HostFunction::method("toString", 0, bigint_to_string));
    ctx.register_builtin("bigint_to_locale_string", HostFunction::method("toLocaleString", 0, bigint_to_locale_string));
    ctx.register_builtin("bigint_value_of", HostFunction::method("valueOf", 0, bigint_value_of));
}

fn bigint_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if ctx.is_call_with_new() {
        throw_type_error(ctx, "BigInt is not a constructor");
        return JSValue::undefined();
    }
    if args.is_empty() {
        return create_bigint_value(BigInt::zero());
    }
    let value = args[0];
    let primitive = match to_primitive_number(ctx, value) {
        Some(p) => p,
        None => return JSValue::undefined(),
    };
    let result = if primitive.is_int() || primitive.is_float() {
        let n = if primitive.is_int() {
            primitive.get_int() as f64
        } else {
            primitive.get_float()
        };
        number_to_bigint(ctx, n)
    } else {
        to_bigint(ctx, primitive)
    };
    match result {
        Some(value) => create_bigint_value(value),
        None => JSValue::undefined(),
    }
}

fn bigint_as_int_n(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let bits_val = args.first().copied().unwrap_or(JSValue::new_int(0));
    let bits = match to_index(ctx, bits_val) {
        Some(b) => b,
        None => return JSValue::undefined(),
    };
    let bigint_val = args.get(1).copied().unwrap_or(JSValue::undefined());
    let value = match to_bigint(ctx, bigint_val) {
        Some(v) => v,
        None => return JSValue::undefined(),
    };
    match value.as_int_n(bits) {
        Some(result) => create_bigint_value(result),
        None => {
            throw_range_error(ctx, "Maximum BigInt size exceeded");
            JSValue::undefined()
        }
    }
}

fn bigint_as_uint_n(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let bits_val = args.first().copied().unwrap_or(JSValue::new_int(0));
    let bits = match to_index(ctx, bits_val) {
        Some(b) => b,
        None => return JSValue::undefined(),
    };
    let bigint_val = args.get(1).copied().unwrap_or(JSValue::undefined());
    let value = match to_bigint(ctx, bigint_val) {
        Some(v) => v,
        None => return JSValue::undefined(),
    };
    match value.as_uint_n(bits) {
        Some(result) => create_bigint_value(result),
        None => {
            throw_range_error(ctx, "Maximum BigInt size exceeded");
            JSValue::undefined()
        }
    }
}

fn this_bigint_value(ctx: &mut JSContext, this_val: &JSValue) -> Option<BigInt> {
    if this_val.is_bigint() {
        return Some((*this_val.try_as_object().expect("object value").get_bigint_value()).clone());
    }
    if this_val.is_object() {
        if let Some(inner) = this_val.try_as_object().expect("object value").get(ctx.common_atoms.__value__) {
            if inner.is_bigint() {
                return Some((*inner.try_as_object().expect("object value").get_bigint_value()).clone());
            }
        }
    }
    None
}

fn throw_bigint_type_error(ctx: &mut JSContext) {
    throw_type_error(ctx, "BigInt.prototype method called on incompatible receiver");
}

fn bigint_to_string(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this_val = args.first().copied().unwrap_or(JSValue::undefined());
    let Some(value) = this_bigint_value(ctx, &this_val) else {
        throw_bigint_type_error(ctx);
        return JSValue::undefined();
    };

    let radix = if args.len() >= 2 && !args[1].is_undefined() {
        let primitive = match to_primitive_number(ctx, args[1]) {
            Some(p) => p,
            None => return JSValue::undefined(),
        };
        match crate::builtins::global::js_to_number_value(ctx, &primitive) {
            Ok(n) => n,
            Err(()) => return JSValue::undefined(),
        }
    } else {
        10.0
    };

    if radix.is_nan() {
        throw_range_error(ctx, "toString() radix must be between 2 and 36");
        return JSValue::undefined();
    }
    let radix = radix.trunc();
    if radix < 2.0 || radix > 36.0 {
        throw_range_error(ctx, "toString() radix must be between 2 and 36");
        return JSValue::undefined();
    }

    let result = value.to_string_radix(radix as u32);
    JSValue::new_string(ctx.intern(&result))
}

fn bigint_to_locale_string(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    bigint_to_string(ctx, args)
}

fn bigint_value_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this_val = args.first().copied().unwrap_or(JSValue::undefined());
    match this_bigint_value(ctx, &this_val) {
        Some(value) => create_bigint_value(value),
        None => {
            throw_bigint_type_error(ctx);
            JSValue::undefined()
        }
    }
}
