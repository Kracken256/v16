use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn throw_uri_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_uri_error_prototype() {
        err.prototype = Some(proto);
    }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("URIError")));
    if !msg.is_empty() {
        err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    }
    let ptr = Box::into_raw(Box::new(err));
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    ctx.pending_exception = Some(JSValue::new_object(ptr));
}

fn to_string_vm(ctx: &mut JSContext, value: JSValue) -> Option<String> {
    if value.is_string() {
        return Some(ctx.get_atom_str(value.get_atom()).to_string());
    }
    if value.is_int() { return Some(value.get_int().to_string()); }
    if value.is_float() { return Some(value.get_float().to_string()); }
    if value.is_bool() { return Some(if value.get_bool() { "true" } else { "false" }.to_string()); }
    if value.is_undefined() { return Some("undefined".to_string()); }
    if value.is_null() { return Some("null".to_string()); }
    if value.is_symbol() { return None; }
    if value.is_bigint() { return None; }
    if value.is_object() {
        return ctx
            .with_vm(|vm, ctx| {
                let obj = value.as_object();
                let to_string_fn = obj.get(ctx.common_atoms.to_string);
                if let Some(f) = to_string_fn {
                    if f.is_function() {
                        if let Ok(r) = vm.call_function_with_this(ctx, f, value.clone(), &[]) {
                            if r.is_string() { return Some(ctx.get_atom_str(r.get_atom()).to_string()); }
                            if !r.is_object() {
                                let s = crate::builtins::global::jsvalue_to_string(&r, ctx);
                                return Some(s);
                            }
                        }
                    }
                }
                let value_of_fn = obj.get(ctx.common_atoms.value_of);
                if let Some(f) = value_of_fn {
                    if f.is_function() {
                        if let Ok(r) = vm.call_function_with_this(ctx, f, value.clone(), &[]) {
                            if !r.is_object() {
                                let s = crate::builtins::global::jsvalue_to_string(&r, ctx);
                                return Some(s);
                            }
                        }
                    }
                }
                None
            })
            .flatten();
    }
    None
}

fn coerce_uri_input(ctx: &mut JSContext, args: &[JSValue]) -> Result<String, ()> {
    if args.is_empty() {
        throw_uri_error(ctx, "malformed URI");
        return Err(());
    }
    if args[0].is_string() {
        return Ok(ctx.get_atom_str(args[0].get_atom()).to_string());
    }
    if args[0].is_null() {
        return Ok("null".to_string());
    }
    if args[0].is_undefined() {
        return Ok("undefined".to_string());
    }
    if args[0].is_symbol() {
        throw_uri_error(ctx, "malformed URI");
        return Err(());
    }
    if let Some(s) = to_string_vm(ctx, args[0]) {
        Ok(s)
    } else if args[0].is_object() {
        // ToPrimitive failed - both toString and valueOf returned non-primitives
        // This should throw TypeError, not URIError
        let mut err = crate::object::object::JSObject::new_typed(
            crate::object::object::ObjectType::Error,
        );
        if let Some(proto) = ctx.get_type_error_prototype() { err.prototype = Some(proto); }
        err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
        err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern("Cannot convert object to primitive value")));
        let ptr = Box::into_raw(Box::new(err));
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        ctx.pending_exception = Some(JSValue::new_object(ptr));
        Err(())
    } else {
        throw_uri_error(ctx, "malformed URI");
        Err(())
    }
}

fn is_hex_digit(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='f' | 'A'..='F')
}

fn hex_val(c: char) -> u8 {
    match c {
        '0'..='9' => (c as u8) - b'0',
        'a'..='f' => (c as u8) - b'a' + 10,
        'A'..='F' => (c as u8) - b'A' + 10,
        _ => 0,
    }
}

const URI_UNENCODED: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789;,/?:@&=+$-_.!~*'()#";

const COMPONENT_UNENCODED: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";

fn encode_uri_common(ctx: &mut JSContext, input: &str, unencoded: &str) -> Result<String, ()> {
    let mut result = String::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        let ch = chars[i];
        if unencoded.contains(ch) {
            result.push(ch);
            i += 1;
            continue;
        }
        let code = ch as u32;
        if code >= 0xDC00 && code <= 0xDFFF {
            throw_uri_error(ctx, "malformed URI");
            return Err(());
        }
        if code >= 0xD800 && code <= 0xDBFF {
            if i + 1 >= len {
                throw_uri_error(ctx, "malformed URI");
                return Err(());
            }
            let next = chars[i + 1] as u32;
            if next < 0xDC00 || next > 0xDFFF {
                throw_uri_error(ctx, "malformed URI");
                return Err(());
            }
            let cp = 0x10000 + ((code - 0xD800) << 10) + (next - 0xDC00);
            let bytes = char_to_utf8_bytes(cp);
            for b in &bytes {
                result.push_str(&format!("%{:02X}", b));
            }
            i += 2;
            continue;
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        for b in s.bytes() {
            result.push_str(&format!("%{:02X}", b));
        }
        i += 1;
    }
    Ok(result)
}

fn char_to_utf8_bytes(cp: u32) -> Vec<u8> {
    if cp < 0x80 {
        vec![cp as u8]
    } else if cp < 0x800 {
        vec![0xC0 | (cp >> 6) as u8, 0x80 | (cp & 0x3F) as u8]
    } else if cp < 0x10000 {
        vec![0xE0 | (cp >> 12) as u8, 0x80 | ((cp >> 6) & 0x3F) as u8, 0x80 | (cp & 0x3F) as u8]
    } else {
        vec![0xF0 | (cp >> 18) as u8, 0x80 | ((cp >> 12) & 0x3F) as u8, 0x80 | ((cp >> 6) & 0x3F) as u8, 0x80 | (cp & 0x3F) as u8]
    }
}

fn decode_uri_common(ctx: &mut JSContext, input: &str, reserved_set: &str) -> Result<String, ()> {
    let bytes: Vec<u8> = input.bytes().collect();
    let len = bytes.len();
    let mut result_bytes: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        if b != b'%' {
            result_bytes.push(b);
            i += 1;
            continue;
        }
        if i + 2 >= len || !is_hex_digit(bytes[i+1] as char) || !is_hex_digit(bytes[i+2] as char) {
            throw_uri_error(ctx, "malformed URI");
            return Err(());
        }
        let hi = hex_val(bytes[i+1] as char);
        let lo = hex_val(bytes[i+2] as char);
        let decoded_byte = (hi << 4) | lo;
        if !reserved_set.is_empty() {
            let reserved_chars: Vec<char> = reserved_set.chars().collect();
            if decoded_byte <= 127 && reserved_chars.contains(&(decoded_byte as char)) {
                result_bytes.push(b'%');
                result_bytes.push(bytes[i+1]);
                result_bytes.push(bytes[i+2]);
                i += 3;
                continue;
            }
        }
        result_bytes.push(decoded_byte);
        i += 3;
        if decoded_byte >= 0x80 {
            let seq_len = if decoded_byte & 0xE0 == 0xC0 { 2 }
                else if decoded_byte & 0xF0 == 0xE0 { 3 }
                else if decoded_byte & 0xF8 == 0xF0 { 4 }
                else { 1 };
            if seq_len == 1 || i + (seq_len - 1) * 3 > len {
                throw_uri_error(ctx, "malformed URI");
                return Err(());
            }
            for _ in 1..seq_len {
                if i + 2 >= len || bytes[i] != b'%' || !is_hex_digit(bytes[i+1] as char) || !is_hex_digit(bytes[i+2] as char) {
                    throw_uri_error(ctx, "malformed URI");
                    return Err(());
                }
                let hi2 = hex_val(bytes[i+1] as char);
                let lo2 = hex_val(bytes[i+2] as char);
                let cb = (hi2 << 4) | lo2;
                if cb & 0xC0 != 0x80 {
                    throw_uri_error(ctx, "malformed URI");
                    return Err(());
                }
                result_bytes.push(cb);
                i += 3;
            }
        }
    }
    match String::from_utf8(result_bytes) {
        Ok(s) => Ok(s),
        Err(_) => {
            throw_uri_error(ctx, "malformed URI");
            Err(())
        }
    }
}

pub fn global_encodeuri(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = match coerce_uri_input(ctx, args) {
        Ok(v) => v,
        Err(()) => return JSValue::undefined(),
    };
    match encode_uri_common(ctx, &input, URI_UNENCODED) {
        Ok(r) => JSValue::new_string(ctx.intern(&r)),
        Err(()) => JSValue::undefined(),
    }
}

pub fn global_decodeuri(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = match coerce_uri_input(ctx, args) {
        Ok(v) => v,
        Err(()) => return JSValue::undefined(),
    };
    let reserved = ";/?:@&=+$,#";
    match decode_uri_common(ctx, &input, reserved) {
        Ok(r) => JSValue::new_string(ctx.intern(&r)),
        Err(()) => JSValue::undefined(),
    }
}

pub fn global_encodeuricomponent(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = match coerce_uri_input(ctx, args) {
        Ok(v) => v,
        Err(()) => return JSValue::undefined(),
    };
    match encode_uri_common(ctx, &input, COMPONENT_UNENCODED) {
        Ok(r) => JSValue::new_string(ctx.intern(&r)),
        Err(()) => JSValue::undefined(),
    }
}

pub fn global_decodeuricomponent(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = match coerce_uri_input(ctx, args) {
        Ok(v) => v,
        Err(()) => return JSValue::undefined(),
    };
    match decode_uri_common(ctx, &input, "") {
        Ok(r) => JSValue::new_string(ctx.intern(&r)),
        Err(()) => JSValue::undefined(),
    }
}

fn coerce_to_string(ctx: &mut JSContext, value: &JSValue) -> Result<String, ()> {
    if value.is_string() {
        return Ok(ctx.get_atom_str(value.get_atom()).to_string());
    }
    if value.is_int() {
        return Ok(value.get_int().to_string());
    }
    if value.is_float() {
        return Ok(crate::builtins::string::js_float_to_string(value.get_float()));
    }
    if value.is_bool() {
        return Ok(if value.get_bool() { "true" } else { "false" }.to_string());
    }
    if value.is_null() {
        return Ok("null".to_string());
    }
    if value.is_undefined() {
        return Ok("undefined".to_string());
    }
    if value.is_bigint() {
        return Ok(value.as_object().get_bigint_value().to_string());
    }
    if value.is_symbol() {
        crate::builtins::global::throw_type_error(
            ctx,
            "Cannot convert a Symbol value to a string",
        );
        return Err(());
    }
    if value.is_object() || value.is_function() {
        return ctx
            .with_vm(|vm, ctx| {
                let prim = vm.ordinary_to_primitive(value, "string", ctx);
                if ctx.pending_exception.is_some() {
                    return Err(());
                }
                if vm.pending_throw.is_some() {
                    return Err(());
                }
                if prim.is_object() || prim.is_function() {
                    crate::builtins::global::throw_type_error(
                        ctx,
                        "Cannot convert object to primitive value",
                    );
                    return Err(());
                }
                coerce_to_string(ctx, &prim)
            })
            .unwrap_or(Err(()));
    }
    Err(())
}

fn escape_unescaped(unit: u16) -> bool {
    matches!(unit,
        0x41..=0x5A | 0x61..=0x7A | 0x30..=0x39
        | 0x40 | 0x2A | 0x5F | 0x2B | 0x2D | 0x2E | 0x2F)
}

pub fn global_escape(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = if args.is_empty() {
        "undefined".to_string()
    } else {
        match coerce_to_string(ctx, &args[0]) {
            Ok(s) => s,
            Err(()) => return JSValue::undefined(),
        }
    };
    let mut out = String::new();
    for unit in input.encode_utf16() {
        if escape_unescaped(unit) {
            out.push(char::from_u32(unit as u32).unwrap_or('\u{FFFD}'));
        } else if unit < 256 {
            out.push_str(&format!("%{:02X}", unit));
        } else {
            out.push_str(&format!("%u{:04X}", unit));
        }
    }
    JSValue::new_string(ctx.intern(&out))
}

fn is_hex_unit(unit: u16) -> bool {
    matches!(unit, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66)
}

fn hex_unit_val(unit: u16) -> u16 {
    match unit {
        0x30..=0x39 => unit - 0x30,
        0x41..=0x46 => unit - 0x41 + 10,
        0x61..=0x66 => unit - 0x61 + 10,
        _ => 0,
    }
}

pub fn global_unescape(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let input = if args.is_empty() {
        "undefined".to_string()
    } else {
        match coerce_to_string(ctx, &args[0]) {
            Ok(s) => s,
            Err(()) => return JSValue::undefined(),
        }
    };
    let units: Vec<u16> = input.encode_utf16().collect();
    let len = units.len();
    let mut out: Vec<u16> = Vec::with_capacity(len);
    let mut k = 0usize;
    while k < len {
        let c = units[k];
        if c == 0x25 {
            if k + 5 < len
                && units[k + 1] == 0x75
                && is_hex_unit(units[k + 2])
                && is_hex_unit(units[k + 3])
                && is_hex_unit(units[k + 4])
                && is_hex_unit(units[k + 5])
            {
                let v = (hex_unit_val(units[k + 2]) << 12)
                    | (hex_unit_val(units[k + 3]) << 8)
                    | (hex_unit_val(units[k + 4]) << 4)
                    | hex_unit_val(units[k + 5]);
                out.push(v);
                k += 6;
                continue;
            }
            if k + 2 < len && is_hex_unit(units[k + 1]) && is_hex_unit(units[k + 2]) {
                let v = (hex_unit_val(units[k + 1]) << 4) | hex_unit_val(units[k + 2]);
                out.push(v);
                k += 3;
                continue;
            }
        }
        out.push(c);
        k += 1;
    }
    JSValue::new_string(ctx.intern(&String::from_utf16_lossy(&out)))
}
