use crate::host::HostFunction;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::runtime::context::JSContext;
use crate::value::JSValue;

fn create_builtin_function(ctx: &mut JSContext, name: &str) -> JSValue {
    create_builtin_fn(ctx, name, 1)
}

fn create_builtin_fn(ctx: &mut JSContext, name: &str, arity: u32) -> JSValue {
    let mut func = JSFunction::new_builtin(ctx.intern(name), arity);
    func.set_builtin_marker(ctx, name);
    let ptr = Box::into_raw(Box::new(func)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    JSValue::new_function(ptr)
}

fn set_ne(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
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

fn set_ne_readonly(obj: &mut JSObject, key: crate::runtime::atom::Atom, val: JSValue) {
    obj.define_property(
        key,
        crate::object::object::PropertyDescriptor {
            value: Some(val),
            writable: false,
            enumerable: false,
            configurable: true,
            get: None,
            set: None,
        },
    );
}

fn define_ctor_prototype(
    obj: &mut JSObject,
    key: crate::runtime::atom::Atom,
    val: JSValue,
) {
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

fn make_accessor_getter(ctx: &mut JSContext, display_name: &str, builtin_name: &str) -> JSValue {
    let name_atom = ctx.intern(display_name);
    let mut f = JSFunction::new_builtin(name_atom, 0);
    f.builtin_atom = Some(ctx.intern(builtin_name));
    f.builtin_func = ctx.get_builtin_func(builtin_name);
    let name_desc = crate::object::object::PropertyDescriptor {
        value: Some(JSValue::new_string(name_atom)),
        writable: false,
        enumerable: false,
        configurable: true,
        get: None,
        set: None,
    };
    f.base.define_property(ctx.common_atoms.name, name_desc);
    let length_desc = crate::object::object::PropertyDescriptor {
        value: Some(JSValue::new_int(0)),
        writable: false,
        enumerable: false,
        configurable: true,
        get: None,
        set: None,
    };
    f.base.define_property(ctx.common_atoms.length, length_desc);
    let ptr = Box::into_raw(Box::new(f)) as usize;
    ctx.runtime_mut().gc_heap_mut().track_function(ptr);
    JSValue::new_function(ptr)
}

fn define_accessor(
    obj: &mut JSObject,
    key: crate::runtime::atom::Atom,
    getter: JSValue,
    enumerable: bool,
) {
    obj.define_property(
        key,
        crate::object::object::PropertyDescriptor {
            value: None,
            writable: false,
            enumerable,
            configurable: true,
            get: Some(getter),
            set: None,
        },
    );
}

fn is_callable(ctx: &mut JSContext, v: &JSValue) -> bool {
    if v.is_function() {
        return true;
    }
    if v.is_object() {
        return v.as_object().get(ctx.common_atoms.__boundFn).is_some();
    }
    false
}

fn throw_type_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new();
    if let Some(proto_ptr) = ctx.get_type_error_prototype() {
        err.prototype = Some(proto_ptr);
    }
    err.set(ctx.common_atoms.name, JSValue::new_string(ctx.intern("TypeError")));
    err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(msg)));
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    ctx.pending_exception = Some(JSValue::new_object(ptr));
}

fn has_mapdata_slot(ctx: &mut JSContext, obj: &JSObject) -> bool {
    let size_atom = ctx.intern("__map_size__");
    obj.get(size_atom).is_some()
}

fn has_setdata_slot(ctx: &mut JSContext, obj: &JSObject) -> bool {
    let size_atom = ctx.intern("__set_size__");
    obj.get(size_atom).is_some()
}

fn has_weakmapdata_slot(ctx: &mut JSContext, obj: &JSObject) -> bool {
    let size_atom = ctx.intern("__weakmap_size__");
    obj.get(size_atom).is_some()
}

fn has_weaksetdata_slot(ctx: &mut JSContext, obj: &JSObject) -> bool {
    let size_atom = ctx.intern("__weakset_size__");
    obj.get(size_atom).is_some()
}

fn map_size(ctx: &mut JSContext, obj: &JSObject) -> usize {
    let size_atom = ctx.intern("__map_size__");
    obj.get(size_atom)
        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
        .unwrap_or(0)
}

fn map_key_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__mk_{}__", i))
}

fn map_val_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__mv_{}__", i))
}

fn map_find(ctx: &mut JSContext, obj: &JSObject, search_key: &JSValue) -> Option<usize> {
    let size = map_size(ctx, obj);
    for i in 0..size {
        let k_atom = map_key_atom(ctx, i);
        if let Some(k) = obj.get(k_atom) {
            if key_same_value_zero(&k, search_key) {
                return Some(i);
            }
        }
    }
    None
}

fn set_size(ctx: &mut JSContext, obj: &JSObject) -> usize {
    let size_atom = ctx.intern("__set_size__");
    obj.get(size_atom)
        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
        .unwrap_or(0)
}

fn set_val_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__sv_{}__", i))
}

fn set_find(ctx: &mut JSContext, obj: &JSObject, search: &JSValue) -> Option<usize> {
    let size = set_size(ctx, obj);
    for i in 0..size {
        let v_atom = set_val_atom(ctx, i);
        if let Some(v) = obj.get(v_atom) {
            if key_same_value_zero(&v, search) {
                return Some(i);
            }
        }
    }
    None
}

fn call_callback(
    ctx: &mut JSContext,
    callback: JSValue,
    args: &[JSValue],
) -> Result<JSValue, String> {
    if let Some(ptr) = ctx.get_register_vm_ptr() {
        let vm = unsafe { &mut *(ptr as *mut crate::runtime::vm::VM) };
        vm.call_function(ctx, callback, args)
    } else {
        Err("call_callback: VM not available".to_string())
    }
}

fn call_callback_with_this(
    ctx: &mut JSContext,
    callback: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Result<JSValue, String> {
    if let Some(ptr) = ctx.get_register_vm_ptr() {
        let vm = unsafe { &mut *(ptr as *mut crate::runtime::vm::VM) };
        vm.call_function_with_this(ctx, callback, this_value, args)
    } else {
        Err("call_callback: VM not available".to_string())
    }
}

fn get_property_with_getter(
    ctx: &mut JSContext,
    obj_ptr: usize,
    prop: crate::runtime::atom::Atom,
) -> Result<JSValue, String> {
    let obj = unsafe { &*(obj_ptr as *const JSObject) };
    if let Some(entry) = obj.get_own_accessor_entry(prop) {
        if let Some(getter) = entry.get {
            if getter.is_function() {
                let this = JSValue::new_object(obj_ptr);
                return call_callback_with_this(ctx, getter, this, &[]);
            }
        }
    }
    if let Some(v) = obj.get(prop) {
        return Ok(v);
    }
    if obj.is_dense_array() {
        if let Some(idx) = ctx.atom_table().get_array_index(prop) {
            let arr = unsafe { &*(obj_ptr as *const crate::object::array_obj::JSArrayObject) };
            if let Some(v) = arr.get(idx) {
                return Ok(v);
            }
        }
    }
    Ok(JSValue::undefined())
}

fn make_array(ctx: &mut JSContext, items: Vec<JSValue>) -> JSValue {
    let mut arr = JSObject::new_array();
    if let Some(proto_ptr) = ctx.get_array_prototype() {
        arr.prototype = Some(proto_ptr);
    }
    let len = items.len();
    for (i, v) in items.into_iter().enumerate() {
        let key = ctx.intern(&i.to_string());
        arr.set(key, v);
    }
    arr.set(ctx.common_atoms.length, JSValue::new_int(len as i64));
    JSValue::new_object(Box::into_raw(Box::new(arr)) as usize)
}

pub fn iterate_values(ctx: &mut JSContext, iterable: JSValue) -> Vec<JSValue> {
    let mut values = Vec::new();

    let sym_iter = crate::builtins::symbol::get_symbol_iterator(ctx);
    if !sym_iter.is_symbol() {
        return values;
    }
    let sym_atom = crate::runtime::atom::Atom(0x40000000 | sym_iter.get_symbol_id());

    let iter_obj = if iterable.is_object() {
        iterable.as_object()
    } else {
        return values;
    };

    let iter_fn = iter_obj.get(sym_atom).or_else(|| {
        let mut current = iter_obj.prototype;
        while let Some(p) = current {
            let pobj = unsafe { &*p };
            if let Some(v) = pobj.get(sym_atom) {
                return Some(v);
            }
            current = pobj.prototype;
        }
        None
    });

    let fn_val = match iter_fn {
        Some(f) if f.is_function() => f,
        _ => return values,
    };

    let iterator = match call_callback_with_this(ctx, fn_val, iterable, &[]) {
        Ok(v) => v,
        Err(_) => return values,
    };

    if !iterator.is_object() {
        return values;
    }

    let iter_obj = iterator.as_object();
    let arr_atom = ctx.common_atoms.__iter_arr__;
    let idx_atom = ctx.common_atoms.__iter_idx__;

    if let Some(arr_val) = iter_obj.get(arr_atom) {
        let mut idx = iter_obj.get(idx_atom).map(|v| v.get_int() as usize).unwrap_or(0);
        if arr_val.is_string() {
            let atom = arr_val.get_atom();
            let s = ctx.atom_table().get(atom);
            let chars: Vec<char> = s.chars().collect();
            while idx < chars.len() {
                let ch = chars[idx].to_string();
                let ch_atom = ctx.atom_table_mut().intern(&ch);
                values.push(JSValue::new_string(ch_atom));
                idx += 1;
            }
        } else if arr_val.is_object() {
            let arr_ptr = arr_val.get_ptr();
            let is_jsarray = unsafe {
                &*(arr_ptr as *const JSObject)
            }.is_dense_array();
            let length = if is_jsarray {
                let arr = unsafe {
                    &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                };
                arr.len()
            } else {
                let obj = unsafe {
                    &*(arr_ptr as *const JSObject)
                };
                let len_atom = ctx.common_atoms.length;
                obj.get(len_atom).map(|v| v.get_int() as usize).unwrap_or(0)
            };
            while idx < length {
                let value = if is_jsarray {
                    let arr = unsafe {
                        &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                    };
                    arr.get(idx).unwrap_or(JSValue::undefined())
                } else {
                    let obj = unsafe {
                        &*(arr_ptr as *const JSObject)
                    };
                    let key = ctx.intern(&idx.to_string());
                    obj.get(key).unwrap_or(JSValue::undefined())
                };
                values.push(value);
                idx += 1;
            }
        }
    } else {
        let next_atom = ctx.intern("next");
        let next_fn = iter_obj.get(next_atom).or_else(|| {
            let mut proto = iter_obj.prototype;
            while let Some(p) = proto {
                let pobj = unsafe { &*p };
                if let Some(v) = pobj.get(next_atom) {
                    return Some(v);
                }
                proto = pobj.prototype;
            }
            None
        });

        let done_atom = ctx.intern("done");
        let value_atom = ctx.intern("value");

        loop {
            let next_val = match &next_fn {
                Some(f) => {
                    match call_callback_with_this(ctx, *f, iterator, &[]) {
                        Ok(v) => v,
                        Err(_) => break,
                    }
                }
                None => break,
            };

            let (done, value) = if next_val.is_object() {
                let done_val =
                    crate::builtins::object::object_get_property(ctx, &next_val, done_atom);
                if ctx.pending_exception.is_some() {
                    break;
                }
                let done = done_val.is_truthy();
                let value =
                    crate::builtins::object::object_get_property(ctx, &next_val, value_atom);
                if ctx.pending_exception.is_some() {
                    break;
                }
                (done, value)
            } else {
                (true, JSValue::undefined())
            };

            if done {
                break;
            }

            values.push(value);
        }
    }

    values
}

pub fn create_iterator_from_iterable(ctx: &mut JSContext, iterable: JSValue) -> Option<JSValue> {
    let sym_iter = crate::builtins::symbol::get_symbol_iterator(ctx);
    if !sym_iter.is_symbol() {
        return None;
    }
    let sym_atom = crate::runtime::atom::Atom(0x40000000 | sym_iter.get_symbol_id());

    let iter_obj = if iterable.is_object() {
        iterable.as_object()
    } else {
        return None;
    };

    let iter_fn = iter_obj.get(sym_atom).or_else(|| {
        let mut current = iter_obj.prototype;
        while let Some(p) = current {
            let pobj = unsafe { &*p };
            if let Some(v) = pobj.get(sym_atom) {
                return Some(v);
            }
            current = pobj.prototype;
        }
        None
    });

    let fn_val = match iter_fn {
        Some(f) if f.is_function() => f,
        _ => return None,
    };

    call_callback_with_this(ctx, fn_val, iterable, &[]).ok()
}

pub fn iterator_next(ctx: &mut JSContext, iterator: JSValue) -> Option<(JSValue, bool)> {
    if !iterator.is_object() {
        return None;
    }
    let iter_ptr = iterator.get_ptr();
    let iter_obj = unsafe { &*(iter_ptr as *const JSObject) };
    let iter_obj_mut = unsafe { &mut *(iter_ptr as *mut JSObject) };
    let arr_atom = ctx.common_atoms.__iter_arr__;
    let idx_atom = ctx.common_atoms.__iter_idx__;

    if let Some(arr_val) = iter_obj.get(arr_atom) {
        let idx = iter_obj.get(idx_atom).map(|v| v.get_int() as usize).unwrap_or(0);
        if arr_val.is_string() {
            let atom = arr_val.get_atom();
            let s = ctx.atom_table().get(atom);
            let chars: Vec<char> = s.chars().collect();
            if idx < chars.len() {
                let ch = chars[idx].to_string();
                let ch_atom = ctx.atom_table_mut().intern(&ch);
                iter_obj_mut.set(idx_atom, JSValue::new_int((idx + 1) as i64));
                return Some((JSValue::new_string(ch_atom), false));
            } else {
                return Some((JSValue::undefined(), true));
            }
        } else if arr_val.is_object() {
            let arr_ptr = arr_val.get_ptr();
            let is_jsarray = unsafe {
                &*(arr_ptr as *const JSObject)
            }.is_dense_array();
            let length = if is_jsarray {
                let arr = unsafe {
                    &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                };
                arr.len()
            } else {
                let obj = unsafe {
                    &*(arr_ptr as *const JSObject)
                };
                let len_atom = ctx.common_atoms.length;
                obj.get(len_atom).map(|v| v.get_int() as usize).unwrap_or(0)
            };
            if idx < length {
                let value = if is_jsarray {
                    let arr = unsafe {
                        &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                    };
                    arr.get(idx).unwrap_or(JSValue::undefined())
                } else {
                    let obj = unsafe {
                        &*(arr_ptr as *const JSObject)
                    };
                    let key = ctx.intern(&idx.to_string());
                    obj.get(key).unwrap_or(JSValue::undefined())
                };
                iter_obj_mut.set(idx_atom, JSValue::new_int((idx + 1) as i64));
                return Some((value, false));
            } else {
                return Some((JSValue::undefined(), true));
            }
        }
        return Some((JSValue::undefined(), true));
    }

    let next_atom = ctx.intern("next");
    let next_fn = iter_obj.get(next_atom).or_else(|| {
        let mut proto = iter_obj.prototype;
        while let Some(p) = proto {
            let pobj = unsafe { &*p };
            if let Some(v) = pobj.get(next_atom) {
                return Some(v);
            }
            proto = pobj.prototype;
        }
        None
    });

    let f = match next_fn {
        Some(f) if f.is_function() => f,
        _ => return None,
    };

    let next_val = match call_callback_with_this(ctx, f, iterator, &[]) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let done_atom = ctx.intern("done");
    let value_atom = ctx.intern("value");

    if next_val.is_object() {
        let done_val = crate::builtins::object::object_get_property(ctx, &next_val, done_atom);
        if ctx.pending_exception.is_some() {
            return None;
        }
        let done = done_val.is_truthy();
        let value = crate::builtins::object::object_get_property(ctx, &next_val, value_atom);
        if ctx.pending_exception.is_some() {
            return None;
        }
        Some((value, done))
    } else {
        Some((JSValue::undefined(), true))
    }
}

pub fn iterator_return(ctx: &mut JSContext, iterator: JSValue) {
    if !iterator.is_object() {
        return;
    }
    let iter_obj = iterator.as_object();
    let return_atom = ctx.intern("return");
    let return_fn = iter_obj.get(return_atom).or_else(|| {
        let mut proto = iter_obj.prototype;
        while let Some(p) = proto {
            let pobj = unsafe { &*p };
            if let Some(v) = pobj.get(return_atom) {
                return Some(v);
            }
            proto = pobj.prototype;
        }
        None
    });

    if let Some(f) = return_fn {
        if f.is_function() {
            let _ = call_callback_with_this(ctx, f, iterator, &[]);
        }
    }
}

pub fn map_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();

    if let Some(proto_ptr) = ctx.get_map_prototype() {
        obj.prototype = Some(proto_ptr);
    }

    let size_atom = ctx.intern("__map_size__");
    obj.set(size_atom, JSValue::new_int(0));

    let this_val = JSValue::new_object(Box::into_raw(Box::new(obj)) as usize);
    let obj_ptr = this_val.get_ptr();
    let obj_ref = unsafe { &mut *(obj_ptr as *mut JSObject) };

    if let Some(iterable) = args.first() {
        if iterable.is_object() || iterable.is_string() {
            if let Some(iterator) = create_iterator_from_iterable(ctx, *iterable) {
                let set_atom = ctx.intern("set");
                let adder = this_val.as_object().get(set_atom).or_else(|| {
                    let mut proto = this_val.as_object().prototype;
                    while let Some(p) = proto {
                        let pobj = unsafe { &*p };
                        if let Some(v) = pobj.get(set_atom) {
                            return Some(v);
                        }
                        proto = pobj.prototype;
                    }
                    None
                });

                if let Some(adder_fn) = adder {
                    let k_atom_0 = ctx.intern("0");
                    let k_atom_1 = ctx.intern("1");
                    loop {
                        match iterator_next(ctx, iterator) {
                            Some((_value, true)) => {
                                break;
                            },
                            Some((value, false)) => {
                                if !value.is_object() {
                                    throw_type_error(ctx, "Iterator value is not an object");
                                    let saved_exc = ctx.pending_exception.take();
                                    iterator_return(ctx, iterator);
                                    ctx.pending_exception = saved_exc;
                                    return JSValue::undefined();
                                }
                                if value.is_object() {
                                    let pair_obj = value.as_object();
                                    let pair_ptr = pair_obj as *const JSObject as usize;
                                    let k = match get_property_with_getter(ctx, pair_ptr, k_atom_0) {
                                        Ok(v) => v,
                                        Err(_) => {
                                            let saved_exc = ctx.pending_exception.take();
                                            iterator_return(ctx, iterator);
                                            ctx.pending_exception = saved_exc;
                                            return JSValue::undefined();
                                        }
                                    };
                                    let v = match get_property_with_getter(ctx, pair_ptr, k_atom_1) {
                                        Ok(v) => v,
                                        Err(_) => {
                                            let saved_exc = ctx.pending_exception.take();
                                            iterator_return(ctx, iterator);
                                            ctx.pending_exception = saved_exc;
                                            return JSValue::undefined();
                                        }
                                    };

                                    if call_callback_with_this(ctx, adder_fn, this_val, &[k, v]).is_err() {
                                        let saved_exc = ctx.pending_exception.take();
                                        iterator_return(ctx, iterator);
                                        ctx.pending_exception = saved_exc;
                                        return JSValue::undefined();
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                } else {
                    let k_atom_0 = ctx.intern("0");
                    let k_atom_1 = ctx.intern("1");
                    loop {
                        match iterator_next(ctx, iterator) {
                            Some((_value, true)) => {
                                break;
                            },
                            Some((value, false)) => {
                                if !value.is_object() {
                                    throw_type_error(ctx, "Iterator value is not an object");
                                    let saved_exc = ctx.pending_exception.take();
                                    iterator_return(ctx, iterator);
                                    ctx.pending_exception = saved_exc;
                                    return JSValue::undefined();
                                }
                                if value.is_object() {
                                    let pair_obj = value.as_object();
                                    let pair_ptr = pair_obj as *const JSObject as usize;
                                    let k = match get_property_with_getter(ctx, pair_ptr, k_atom_0) {
                                        Ok(v) => v,
                                        Err(_) => {
                                            let saved_exc = ctx.pending_exception.take();
                                            iterator_return(ctx, iterator);
                                            ctx.pending_exception = saved_exc;
                                            return JSValue::undefined();
                                        }
                                    };
                                    let v = match get_property_with_getter(ctx, pair_ptr, k_atom_1) {
                                        Ok(v) => v,
                                        Err(_) => {
                                            let saved_exc = ctx.pending_exception.take();
                                            iterator_return(ctx, iterator);
                                            ctx.pending_exception = saved_exc;
                                            return JSValue::undefined();
                                        }
                                    };

                                    let cur_size = map_size(ctx, obj_ref);
                                    if let Some(idx) = map_find(ctx, obj_ref, &k) {
                                        let mv_atom = map_val_atom(ctx, idx);
                                        obj_ref.set(mv_atom, v);
                                    } else {
                                        let mk_atom = map_key_atom(ctx, cur_size);
                                        let mv_atom = map_val_atom(ctx, cur_size);
                                        obj_ref.set(mk_atom, k);
                                        obj_ref.set(mv_atom, v);
                                        obj_ref.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        }
    }

    let size_val = obj_ref.get(size_atom).unwrap_or(JSValue::new_int(0));
    let size_prop_atom = ctx.intern("size");
    obj_ref.set(size_prop_atom, size_val);

    this_val
}

fn sync_map_size(ctx: &mut JSContext, obj: &mut JSObject) {
    let size_atom = ctx.intern("__map_size__");
    let size_prop_atom = ctx.intern("size");
    let size_val = obj.get(size_atom).unwrap_or(JSValue::new_int(0));
    obj.set(size_prop_atom, size_val);
}

fn map_set(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 3 {
        return if args.is_empty() {
            JSValue::undefined()
        } else {
            args[0]
        };
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.set called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.set called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = args[1];
    let value = args[2];

    let obj = this.as_object_mut();

    let size_atom = ctx.intern("__map_size__");

    if let Some(idx) = map_find(ctx, obj, &key) {
        let mv_atom = map_val_atom(ctx, idx);
        obj.set(mv_atom, value);
    } else {
        let cur_size = map_size(ctx, obj);
        let mk_atom = map_key_atom(ctx, cur_size);
        let mv_atom = map_val_atom(ctx, cur_size);
        obj.set(mk_atom, key);
        obj.set(mv_atom, value);
        obj.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
    }

    sync_map_size(ctx, obj);
    this
}

fn map_get(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.get called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.get called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = args[1];

    if let Some(idx) = map_find(ctx, obj, &key) {
        let mv_atom = map_val_atom(ctx, idx);
        obj.get(mv_atom).unwrap_or_else(JSValue::undefined)
    } else {
        JSValue::undefined()
    }
}

fn map_size_getter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or(JSValue::undefined());
    if !this.is_object() {
        throw_type_error(ctx, "Method get Map.prototype.size called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method get Map.prototype.size called on incompatible receiver");
        return JSValue::undefined();
    }
    let size_atom = ctx.intern("__map_size__");
    obj.get(size_atom).unwrap_or(JSValue::new_int(0))
}

fn set_size_getter(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or(JSValue::undefined());
    if !this.is_object() {
        throw_type_error(ctx, "Method get Set.prototype.size called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method get Set.prototype.size called on incompatible receiver");
        return JSValue::undefined();
    }
    let size_atom = ctx.intern("__set_size__");
    obj.get(size_atom).unwrap_or(JSValue::new_int(0))
}

fn map_get_or_insert(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 3 {
        return JSValue::undefined();
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.getOrInsert called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.getOrInsert called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = normalize_key(&args[1]);
    let value = args[2];

    if let Some(idx) = map_find(ctx, obj, &key) {
        let mv_atom = map_val_atom(ctx, idx);
        return obj.get(mv_atom).unwrap_or_else(JSValue::undefined);
    }
    let obj_mut = this.as_object_mut();
    let cur_size = map_size(ctx, obj_mut);
    let mk_atom = map_key_atom(ctx, cur_size);
    let mv_atom = map_val_atom(ctx, cur_size);
    obj_mut.set(mk_atom, key);
    obj_mut.set(mv_atom, value);
    let size_atom = ctx.intern("__map_size__");
    obj_mut.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
    sync_map_size(ctx, obj_mut);
    value
}

fn map_get_or_insert_computed(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 3 {
        return JSValue::undefined();
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.getOrInsertComputed called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.getOrInsertComputed called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = normalize_key(&args[1]);
    let callback = args[2];
    if !is_callable(ctx, &callback) {
        throw_type_error(ctx, "callback is not a function");
        return JSValue::undefined();
    }

    if let Some(idx) = map_find(ctx, obj, &key) {
        let mv_atom = map_val_atom(ctx, idx);
        return obj.get(mv_atom).unwrap_or_else(JSValue::undefined);
    }
    let value = match call_callback(ctx, callback, &[key]) {
        Ok(v) => v,
        Err(e) => {
            let mut err = JSObject::new();
            err.set(ctx.common_atoms.message, JSValue::new_string(ctx.intern(&e)));
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            ctx.pending_exception = Some(JSValue::new_object(ptr));
            return JSValue::undefined();
        }
    };
    let obj_mut = this.as_object_mut();
    if let Some(idx) = map_find(ctx, obj_mut, &key) {
        let mv_atom = map_val_atom(ctx, idx);
        obj_mut.set(mv_atom, value);
        return value;
    }
    let cur_size = map_size(ctx, obj_mut);
    let mk_atom = map_key_atom(ctx, cur_size);
    let mv_atom = map_val_atom(ctx, cur_size);
    obj_mut.set(mk_atom, key);
    obj_mut.set(mv_atom, value);
    let size_atom = ctx.intern("__map_size__");
    obj_mut.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
    sync_map_size(ctx, obj_mut);
    value
}

fn normalize_key(v: &JSValue) -> JSValue {
    if v.is_float() && v.get_float() == 0.0 {
        return JSValue::new_int(0);
    }
    v.clone()
}

fn key_same_value_zero(a: &JSValue, b: &JSValue) -> bool {
    if a.is_float() && b.is_float() {
        let af = a.get_float();
        let bf = b.get_float();
        if af.is_nan() && bf.is_nan() {
            return true;
        }
        return af == bf;
    }
    if a.is_float() && b.is_int() {
        let af = a.get_float();
        return !af.is_nan() && af == (b.get_int() as f64);
    }
    if a.is_int() && b.is_float() {
        let bf = b.get_float();
        return !bf.is_nan() && (a.get_int() as f64) == bf;
    }
    a.strict_eq(b)
}

fn map_group_by(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let items = args[0];
    let callback = args[1];
    if !callback.is_function() {
        throw_type_error(ctx, "callback is not a function");
        return JSValue::undefined();
    }

    let mut result_map = JSObject::new();
    if let Some(proto_ptr) = ctx.get_map_prototype() {
        result_map.prototype = Some(proto_ptr);
    }
    let size_atom = ctx.intern("__map_size__");
    result_map.set(size_atom, JSValue::new_int(0));
    let result_ptr = Box::into_raw(Box::new(result_map)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(result_ptr);
    let result_val = JSValue::new_object(result_ptr);

    let iter_values = iterate_values(ctx, items);
    for (i, item) in iter_values.into_iter().enumerate() {
        let key_val = match call_callback(ctx, callback, &[item, JSValue::new_int(i as i64)]) {
            Ok(v) => v,
            Err(_) => return JSValue::undefined(),
        };
        let key = normalize_key(&key_val);

        let result_obj = unsafe { &mut *(result_ptr as *mut JSObject) };
        if let Some(idx) = map_find(ctx, result_obj, &key) {
            let mv_atom = map_val_atom(ctx, idx);
            let group_arr_val = result_obj.get(mv_atom).unwrap_or_else(JSValue::undefined);
            if group_arr_val.is_object() {
                let group_ptr = group_arr_val.get_ptr();
                let group_obj = unsafe { &*(group_ptr as *const JSObject) };
                let len = group_obj.get(ctx.common_atoms.length).map(|v| v.get_int() as usize).unwrap_or(0);
                let group_obj_mut = unsafe { &mut *(group_ptr as *mut JSObject) };
                let key_atom = ctx.intern(&len.to_string());
                group_obj_mut.set(key_atom, item);
                group_obj_mut.set(ctx.common_atoms.length, JSValue::new_int((len + 1) as i64));
            }
        } else {
            let cur_size = map_size(ctx, result_obj);
            let mk_atom = map_key_atom(ctx, cur_size);
            let mv_atom = map_val_atom(ctx, cur_size);
            result_obj.set(mk_atom, key);
            let group_arr = make_array(ctx, vec![item]);
            result_obj.set(mv_atom, group_arr);
            result_obj.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
        }
    }

    let result_obj = unsafe { &mut *(result_ptr as *mut JSObject) };
    sync_map_size(ctx, result_obj);
    result_val
}

fn map_has(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = args[1];
    JSValue::bool(map_find(ctx, obj, &key).is_some())
}

fn map_delete(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = args[1];
    let obj = this.as_object_mut();

    let size = map_size(ctx, obj);
    let idx = match map_find(ctx, obj, &key) {
        Some(i) => i,
        None => return JSValue::bool(false),
    };

    for i in idx..(size - 1) {
        let mk_next = map_key_atom(ctx, i + 1);
        let mv_next = map_val_atom(ctx, i + 1);
        let mk_cur = map_key_atom(ctx, i);
        let mv_cur = map_val_atom(ctx, i);
        let k = obj.get(mk_next).unwrap_or_else(JSValue::undefined);
        let v = obj.get(mv_next).unwrap_or_else(JSValue::undefined);
        obj.set(mk_cur, k);
        obj.set(mv_cur, v);
    }

    let mk_last = map_key_atom(ctx, size - 1);
    let mv_last = map_val_atom(ctx, size - 1);
    obj.delete(mk_last);
    obj.delete(mv_last);

    let size_atom = ctx.intern("__map_size__");
    obj.set(size_atom, JSValue::new_int((size - 1) as i64));
    sync_map_size(ctx, obj);
    JSValue::bool(true)
}

fn map_clear(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.clear called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.clear called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object_mut();

    let size = map_size(ctx, obj);
    for i in 0..size {
        let mk = map_key_atom(ctx, i);
        let mv = map_val_atom(ctx, i);
        obj.delete(mk);
        obj.delete(mv);
    }
    let size_atom = ctx.intern("__map_size__");
    obj.set(size_atom, JSValue::new_int(0));
    sync_map_size(ctx, obj);
    JSValue::undefined()
}

fn map_for_each(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let this = args[0];
    let callback = args[1];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.forEach called on incompatible receiver");
        return JSValue::undefined();
    }
    if !callback.is_function() {
        throw_type_error(ctx, "Callback is not a function");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.forEach called on incompatible receiver");
        return JSValue::undefined();
    }
    let size = map_size(ctx, obj);

    for i in 0..size {
        let mk_atom = map_key_atom(ctx, i);
        let mv_atom = map_val_atom(ctx, i);
        let k = obj.get(mk_atom).unwrap_or_else(JSValue::undefined);
        let v = obj.get(mv_atom).unwrap_or_else(JSValue::undefined);
        let _ = call_callback(ctx, callback, &[v, k, this]);
    }

    JSValue::undefined()
}

fn map_keys(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_map_iter(ctx, JSValue::undefined(), "key");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.keys called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.keys called on incompatible receiver");
        return JSValue::undefined();
    }
    create_map_iter(ctx, this, "key")
}

fn map_values(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_map_iter(ctx, JSValue::undefined(), "key");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.values called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.values called on incompatible receiver");
        return JSValue::undefined();
    }
    create_map_iter(ctx, this, "value")
}

fn map_entries(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_map_iter(ctx, JSValue::undefined(), "key");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype.entries called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype.entries called on incompatible receiver");
        return JSValue::undefined();
    }
    create_map_iter(ctx, this, "entry")
}

fn map_symbol_iterator(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_map_iter(ctx, JSValue::undefined(), "key");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Map.prototype[Symbol.iterator] called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_mapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Map.prototype[Symbol.iterator] called on incompatible receiver");
        return JSValue::undefined();
    }
    create_map_iter(ctx, this, "entry")
}

fn create_map_iter(ctx: &mut JSContext, map_obj: JSValue, kind: &str) -> JSValue {
    create_live_iter(ctx, map_obj, kind, ctx.get_map_iterator_prototype())
}

fn create_set_iter(ctx: &mut JSContext, set_obj: JSValue, kind: &str) -> JSValue {
    create_live_iter(ctx, set_obj, kind, ctx.get_set_iterator_prototype())
}

fn create_live_iter(
    ctx: &mut JSContext,
    collection: JSValue,
    kind: &str,
    proto: Option<usize>,
) -> JSValue {
    let mut iter_obj = JSObject::new();
    let map_atom = ctx.intern("__iter_map__");
    let kind_atom = ctx.intern("__iter_kind__");
    let idx_atom = ctx.common_atoms.__iter_idx__;
    iter_obj.set(map_atom, collection);
    iter_obj.set(kind_atom, JSValue::new_string(ctx.intern(kind)));
    iter_obj.set(idx_atom, JSValue::new_int(0));
    if let Some(proto_ptr) = proto {
        iter_obj.prototype = Some(proto_ptr as *mut JSObject);
    }
    let ptr = Box::into_raw(Box::new(iter_obj)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

fn map_set_iterator_next(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let this = args.first().copied().unwrap_or(JSValue::undefined());
    if !this.is_object_like() {
        throw_type_error(ctx, "Iterator.prototype.next called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj_ptr = this.get_ptr();
    let obj = unsafe { &*(obj_ptr as *const JSObject) };
    let obj_mut = unsafe { &mut *(obj_ptr as *mut JSObject) };
    let arr_atom = ctx.common_atoms.__iter_arr__;
    let idx_atom = ctx.common_atoms.__iter_idx__;
    let map_atom = ctx.intern("__iter_map__");
    let kind_atom = ctx.intern("__iter_kind__");
    let done_atom = ctx.intern("done");
    let value_atom = ctx.intern("value");
    let iter_idx = obj.get(idx_atom).map(|v| if v.is_int() { v.get_int() as usize } else { 0 }).unwrap_or(0);
    if let Some(collection) = obj.get(map_atom).filter(|value| value.is_object_like()) {
        let kind = obj
            .get(kind_atom)
            .filter(|value| value.is_string())
            .map(|value| ctx.get_atom_str(value.get_atom()).to_string())
            .unwrap_or_else(|| "key".to_string());
        let coll = collection.as_object();
        let is_set = kind == "set-value" || kind == "set-entry";
        let size = if is_set { set_size(ctx, coll) } else { map_size(ctx, coll) };
        if iter_idx < size {
            let value = if is_set {
                let v = coll.get(set_val_atom(ctx, iter_idx)).unwrap_or_else(JSValue::undefined);
                if kind == "set-entry" {
                    make_array(ctx, vec![v, v])
                } else {
                    v
                }
            } else {
                let k = coll.get(map_key_atom(ctx, iter_idx)).unwrap_or_else(JSValue::undefined);
                let v = coll.get(map_val_atom(ctx, iter_idx)).unwrap_or_else(JSValue::undefined);
                match kind.as_str() {
                    "key" => k,
                    "value" => v,
                    _ => make_array(ctx, vec![k, v]),
                }
            };
            obj_mut.set(idx_atom, JSValue::new_int((iter_idx + 1) as i64));
            let mut result = JSObject::new();
            result.set(value_atom, value);
            result.set(done_atom, JSValue::bool(false));
            let ptr = Box::into_raw(Box::new(result)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            return JSValue::new_object(ptr);
        }
        let mut result = JSObject::new();
        result.set(value_atom, JSValue::undefined());
        result.set(done_atom, JSValue::bool(true));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        return JSValue::new_object(ptr);
    }
    let arr_val = match obj.get(arr_atom) {
        Some(v) => v,
        None => {
            throw_type_error(ctx, "Iterator has no [[IteratedObject]] slot");
            return JSValue::undefined();
        }
    };
    if !arr_val.is_object_like() {
        throw_type_error(ctx, "Iterator [[IteratedObject]] is not an object");
        return JSValue::undefined();
    }
    let arr_obj = arr_val.as_object();
    let len = arr_obj.get(ctx.common_atoms.length).map(|v| super::string::js_to_length(&v) as usize).unwrap_or(0);
    if iter_idx < len {
        let value = super::array::array_get(arr_obj, iter_idx, ctx).unwrap_or_else(JSValue::undefined);
        obj_mut.set(idx_atom, JSValue::new_int((iter_idx + 1) as i64));
        let mut result = JSObject::new();
        result.set(value_atom, value);
        result.set(done_atom, JSValue::bool(false));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        JSValue::new_object(ptr)
    } else {
        let mut result = JSObject::new();
        result.set(value_atom, JSValue::undefined());
        result.set(done_atom, JSValue::bool(true));
        let ptr = Box::into_raw(Box::new(result)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        JSValue::new_object(ptr)
    }
}

fn iterator_symbol_iterator(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    args.first().copied().unwrap_or_else(JSValue::undefined)
}

fn set_symbol_iterator(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_set_iter(ctx, JSValue::undefined(), "set-value");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype[Symbol.iterator] called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype[Symbol.iterator] called on incompatible receiver");
        return JSValue::undefined();
    }
    create_set_iter(ctx, this, "set-value")
}

fn weakmap_constructor(ctx: &mut JSContext, _args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();
    if let Some(proto_ptr) = ctx.get_weakmap_prototype() {
        obj.prototype = Some(proto_ptr);
    }
    let size_atom = ctx.intern("__weakmap_size__");
    obj.set(size_atom, JSValue::new_int(0));
    let ptr = Box::into_raw(Box::new(obj)) as usize;
    JSValue::new_object(ptr)
}

fn weakmap_size(ctx: &mut JSContext, obj: &JSObject) -> usize {
    let size_atom = ctx.intern("__weakmap_size__");
    obj.get(size_atom)
        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
        .unwrap_or(0)
}

fn weakmap_key_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__wmk_{}__", i))
}

fn weakmap_val_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__wmv_{}__", i))
}

fn weakmap_find(ctx: &mut JSContext, obj: &JSObject, search_key: &JSValue) -> Option<usize> {
    let size = weakmap_size(ctx, obj);
    for i in 0..size {
        let k_atom = weakmap_key_atom(ctx, i);
        if let Some(k) = obj.get(k_atom) {
            if k.strict_eq(search_key) {
                return Some(i);
            }
        }
    }
    None
}

fn weakmap_set(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 3 {
        return JSValue::undefined();
    }
    let this = &args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method WeakMap.prototype.set called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weakmapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakMap.prototype.set called on incompatible receiver");
        return JSValue::undefined();
    }
    let key = &args[1];
    let value = &args[2];

    if !key.is_object() && !key.is_symbol() {
        throw_type_error(ctx, "Invalid value used as weak map key");
        return JSValue::undefined();
    }

    let obj = this.as_object_mut();

    if let Some(idx) = weakmap_find(ctx, obj, key) {
        let v_atom = weakmap_val_atom(ctx, idx);
        obj.set(v_atom, value.clone());
    } else {
        let size_atom = ctx.intern("__weakmap_size__");
        let size = weakmap_size(ctx, obj);
        let k_atom = weakmap_key_atom(ctx, size);
        let v_atom = weakmap_val_atom(ctx, size);
        obj.set(k_atom, key.clone());
        obj.set(v_atom, value.clone());
        obj.set(size_atom, JSValue::new_int((size + 1) as i64));
    }
    this.clone()
}

fn weakmap_get(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let this = &args[0];
    let key = &args[1];

    if !this.is_object() {
        throw_type_error(ctx, "Method WeakMap.prototype.get called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weakmapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakMap.prototype.get called on incompatible receiver");
        return JSValue::undefined();
    }

    if let Some(idx) = weakmap_find(ctx, obj, key) {
        let v_atom = weakmap_val_atom(ctx, idx);
        obj.get(v_atom).unwrap_or_else(JSValue::undefined)
    } else {
        JSValue::undefined()
    }
}

fn weakmap_has(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = &args[0];
    let key = &args[1];

    if !this.is_object() {
        throw_type_error(ctx, "Method WeakMap.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weakmapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakMap.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }

    JSValue::bool(weakmap_find(ctx, obj, key).is_some())
}

fn weakmap_delete(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = &args[0];
    let key = &args[1];

    if !this.is_object() {
        throw_type_error(ctx, "Method WeakMap.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weakmapdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakMap.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object_mut();

    if let Some(idx) = weakmap_find(ctx, obj, key) {
        let k_atom = weakmap_key_atom(ctx, idx);
        let v_atom = weakmap_val_atom(ctx, idx);
        obj.delete(k_atom);
        obj.delete(v_atom);
        JSValue::bool(true)
    } else {
        JSValue::bool(false)
    }
}

fn weakset_constructor(ctx: &mut JSContext, _args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();
    if let Some(proto_ptr) = ctx.get_weakset_prototype() {
        obj.prototype = Some(proto_ptr);
    }
    let size_atom = ctx.intern("__weakset_size__");
    obj.set(size_atom, JSValue::new_int(0));
    let ptr = Box::into_raw(Box::new(obj)) as usize;
    JSValue::new_object(ptr)
}

fn weakset_size(ctx: &mut JSContext, obj: &JSObject) -> usize {
    let size_atom = ctx.intern("__weakset_size__");
    obj.get(size_atom)
        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
        .unwrap_or(0)
}

fn weakset_val_atom(ctx: &mut JSContext, i: usize) -> crate::runtime::atom::Atom {
    ctx.intern(&format!("__wsv_{}__", i))
}

fn weakset_find(ctx: &mut JSContext, obj: &JSObject, search: &JSValue) -> Option<usize> {
    let size = weakset_size(ctx, obj);
    for i in 0..size {
        let v_atom = weakset_val_atom(ctx, i);
        if let Some(v) = obj.get(v_atom) {
            if v.strict_eq(search) {
                return Some(i);
            }
        }
    }
    None
}

fn weakset_add(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let this = &args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method WeakSet.prototype.add called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weaksetdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakSet.prototype.add called on incompatible receiver");
        return JSValue::undefined();
    }
    let value = &args[1];

    if !value.is_object() && !value.is_symbol() {
        throw_type_error(ctx, "Invalid value used as weak set key");
        return JSValue::undefined();
    }

    let obj = this.as_object_mut();

    if weakset_find(ctx, obj, value).is_none() {
        let size_atom = ctx.intern("__weakset_size__");
        let size = weakset_size(ctx, obj);
        let v_atom = weakset_val_atom(ctx, size);
        obj.set(v_atom, value.clone());
        obj.set(size_atom, JSValue::new_int((size + 1) as i64));
    }
    this.clone()
}

fn weakset_has(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = &args[0];
    let value = &args[1];

    if !this.is_object() {
        throw_type_error(ctx, "Method WeakSet.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weaksetdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakSet.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }

    JSValue::bool(weakset_find(ctx, obj, value).is_some())
}

fn weakset_delete(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = &args[0];
    let value = &args[1];

    if !this.is_object() {
        throw_type_error(ctx, "Method WeakSet.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_weaksetdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method WeakSet.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object_mut();

    if let Some(idx) = weakset_find(ctx, obj, value) {
        let v_atom = weakset_val_atom(ctx, idx);
        obj.delete(v_atom);
        JSValue::bool(true)
    } else {
        JSValue::bool(false)
    }
}

pub fn set_constructor(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let mut obj = JSObject::new();

    if let Some(proto_ptr) = ctx.get_set_prototype() {
        obj.prototype = Some(proto_ptr);
    }

    let size_atom = ctx.intern("__set_size__");
    obj.set(size_atom, JSValue::new_int(0));

    let this_val = JSValue::new_object(Box::into_raw(Box::new(obj)) as usize);
    let obj_ptr = this_val.get_ptr();
    let obj_ref = unsafe { &mut *(obj_ptr as *mut JSObject) };

    if let Some(iterable) = args.first() {
        if iterable.is_object() || iterable.is_string() {
            if let Some(iterator) = create_iterator_from_iterable(ctx, *iterable) {
                let add_atom = ctx.intern("add");
                let adder = this_val.as_object().get(add_atom).or_else(|| {
                    let mut proto = this_val.as_object().prototype;
                    while let Some(p) = proto {
                        let pobj = unsafe { &*p };
                        if let Some(v) = pobj.get(add_atom) {
                            return Some(v);
                        }
                        proto = pobj.prototype;
                    }
                    None
                });

                if let Some(adder_fn) = adder {
                    loop {
                        match iterator_next(ctx, iterator) {
                            Some((_value, true)) => break,
                            Some((value, false)) => {
                                if call_callback_with_this(ctx, adder_fn, this_val, &[value]).is_err() {
                                    let saved_exc = ctx.pending_exception.take();
                                    iterator_return(ctx, iterator);
                                    ctx.pending_exception = saved_exc;
                                    return JSValue::undefined();
                                }
                            }
                            None => break,
                        }
                    }
                } else {
                    loop {
                        match iterator_next(ctx, iterator) {
                            Some((_value, true)) => break,
                            Some((value, false)) => {
                                let cur_size = set_size(ctx, obj_ref);
                                if set_find(ctx, obj_ref, &value).is_none() {
                                    let sv_atom = set_val_atom(ctx, cur_size);
                                    obj_ref.set(sv_atom, value);
                                    obj_ref.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        }
    }

    let size_val = obj_ref.get(size_atom).unwrap_or(JSValue::new_int(0));
    let size_prop_atom = ctx.intern("size");
    obj_ref.set(size_prop_atom, size_val);

    this_val
}

fn sync_set_size(ctx: &mut JSContext, obj: &mut JSObject) {
    let size_atom = ctx.intern("__set_size__");
    let size_prop_atom = ctx.intern("size");
    let size_val = obj.get(size_atom).unwrap_or(JSValue::new_int(0));
    obj.set(size_prop_atom, size_val);
}

fn set_add(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return if args.is_empty() {
            JSValue::undefined()
        } else {
            args[0]
        };
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.add called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.add called on incompatible receiver");
        return JSValue::undefined();
    }
    let value = args[1];

    let obj = this.as_object_mut();

    if set_find(ctx, obj, &value).is_none() {
        let size_atom = ctx.intern("__set_size__");
        let cur_size = set_size(ctx, obj);
        let sv_atom = set_val_atom(ctx, cur_size);
        obj.set(sv_atom, value);
        obj.set(size_atom, JSValue::new_int((cur_size + 1) as i64));
        sync_set_size(ctx, obj);
    }

    this
}

fn set_has(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.has called on incompatible receiver");
        return JSValue::undefined();
    }
    let value = args[1];
    JSValue::bool(set_find(ctx, obj, &value).is_some())
}

fn set_delete(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::bool(false);
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.delete called on incompatible receiver");
        return JSValue::undefined();
    }
    let value = args[1];
    let obj = this.as_object_mut();

    let size = set_size(ctx, obj);
    let idx = match set_find(ctx, obj, &value) {
        Some(i) => i,
        None => return JSValue::bool(false),
    };

    for i in idx..(size - 1) {
        let sv_next = set_val_atom(ctx, i + 1);
        let sv_cur = set_val_atom(ctx, i);
        let v = obj.get(sv_next).unwrap_or_else(JSValue::undefined);
        obj.set(sv_cur, v);
    }

    let sv_last = set_val_atom(ctx, size - 1);
    obj.delete(sv_last);

    let size_atom = ctx.intern("__set_size__");
    obj.set(size_atom, JSValue::new_int((size - 1) as i64));
    sync_set_size(ctx, obj);
    JSValue::bool(true)
}

fn set_clear(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return JSValue::undefined();
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.clear called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.clear called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object_mut();

    let size = set_size(ctx, obj);
    for i in 0..size {
        let sv = set_val_atom(ctx, i);
        obj.delete(sv);
    }
    let size_atom = ctx.intern("__set_size__");
    obj.set(size_atom, JSValue::new_int(0));
    sync_set_size(ctx, obj);
    JSValue::undefined()
}

fn set_values(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_set_iter(ctx, JSValue::undefined(), "set-value");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.values called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.values called on incompatible receiver");
        return JSValue::undefined();
    }
    create_set_iter(ctx, this, "set-value")
}

fn set_keys(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    set_values(ctx, args)
}

fn set_entries(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.is_empty() {
        return create_set_iter(ctx, JSValue::undefined(), "set-value");
    }
    let this = args[0];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.entries called on incompatible receiver");
        return JSValue::undefined();
    }
    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.entries called on incompatible receiver");
        return JSValue::undefined();
    }
    create_set_iter(ctx, this, "set-entry")
}

fn set_for_each(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    if args.len() < 2 {
        return JSValue::undefined();
    }
    let this = args[0];
    let callback = args[1];
    if !this.is_object() {
        throw_type_error(ctx, "Method Set.prototype.forEach called on incompatible receiver");
        return JSValue::undefined();
    }
    if !callback.is_function() {
        throw_type_error(ctx, "Callback is not a function");
        return JSValue::undefined();
    }

    let obj = this.as_object();
    if !has_setdata_slot(ctx, obj) {
        throw_type_error(ctx, "Method Set.prototype.forEach called on incompatible receiver");
        return JSValue::undefined();
    }
    let size = set_size(ctx, obj);

    for i in 0..size {
        let sv_atom = set_val_atom(ctx, i);
        let v = obj.get(sv_atom).unwrap_or_else(JSValue::undefined);

        let _ = call_callback(ctx, callback, &[v, v, this]);
    }

    JSValue::undefined()
}

fn throw_range_error(ctx: &mut JSContext, msg: &str) {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    if let Some(proto) = ctx.get_range_error_prototype() {
        err.prototype = Some(proto);
    }
    err.set(
        ctx.common_atoms.message,
        JSValue::new_string(ctx.intern(msg)),
    );
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    ctx.pending_exception = Some(JSValue::new_object(ptr));
}

fn create_empty_set(ctx: &mut JSContext) -> (JSValue, usize) {
    let mut obj = JSObject::new();
    if let Some(proto_ptr) = ctx.get_set_prototype() {
        obj.prototype = Some(proto_ptr);
    }
    obj.set(ctx.intern("__set_size__"), JSValue::new_int(0));
    let ptr = Box::into_raw(Box::new(obj)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    (JSValue::new_object(ptr), ptr)
}

fn set_elements(ctx: &mut JSContext, obj: &JSObject) -> Vec<JSValue> {
    let size = set_size(ctx, obj);
    let mut out = Vec::with_capacity(size);
    for i in 0..size {
        let atom = set_val_atom(ctx, i);
        if let Some(v) = obj.get(atom) {
            out.push(v);
        }
    }
    out
}

fn set_add_value(ctx: &mut JSContext, obj_ptr: usize, value: JSValue) {
    let obj = unsafe { &mut *(obj_ptr as *mut JSObject) };
    if set_find(ctx, obj, &value).is_none() {
        let cur = set_size(ctx, obj);
        obj.set(set_val_atom(ctx, cur), value);
        obj.set(ctx.intern("__set_size__"), JSValue::new_int((cur + 1) as i64));
    }
}

fn set_remove_value(ctx: &mut JSContext, obj_ptr: usize, value: JSValue) {
    let obj = unsafe { &mut *(obj_ptr as *mut JSObject) };
    let size = set_size(ctx, obj);
    let elems = set_elements(ctx, obj);
    let filtered: Vec<JSValue> = elems
        .into_iter()
        .filter(|v| !key_same_value_zero(v, &value))
        .collect();
    for i in 0..size {
        let atom = set_val_atom(ctx, i);
        obj.delete(atom);
    }
    for (i, v) in filtered.iter().enumerate() {
        obj.set(set_val_atom(ctx, i), *v);
    }
    obj.set(
        ctx.intern("__set_size__"),
        JSValue::new_int(filtered.len() as i64),
    );
}

struct SetRecord {
    is_set: bool,
    set_ptr: usize,
    has_fn: JSValue,
    keys_fn: JSValue,
}

fn get_set_record(ctx: &mut JSContext, other: JSValue) -> Option<SetRecord> {
    if !other.is_object() {
        throw_type_error(ctx, "Argument is not an Object");
        return None;
    }
    {
        let obj = other.as_object();
        if has_setdata_slot(ctx, obj) {
            return Some(SetRecord {
                is_set: true,
                set_ptr: other.get_ptr(),
                has_fn: JSValue::undefined(),
                keys_fn: JSValue::undefined(),
            });
        }
    }
    let size_atom = ctx.intern("size");
    let size_val = crate::builtins::object::object_get_property(ctx, &other, size_atom);
    if ctx.pending_exception.is_some() {
        return None;
    }
    let size_num = match crate::builtins::global::js_to_number_value(ctx, &size_val) {
        Ok(n) => n,
        Err(()) => return None,
    };
    if size_num.is_nan() {
        throw_type_error(ctx, "size must be a number");
        return None;
    }
    if size_num < 0.0 {
        throw_range_error(ctx, "size must not be negative");
        return None;
    }
    let has_atom = ctx.intern("has");
    let has_fn = crate::builtins::object::object_get_property(ctx, &other, has_atom);
    if ctx.pending_exception.is_some() {
        return None;
    }
    if !is_callable(ctx, &has_fn) {
        throw_type_error(ctx, "has must be callable");
        return None;
    }
    let keys_atom = ctx.intern("keys");
    let keys_fn = crate::builtins::object::object_get_property(ctx, &other, keys_atom);
    if ctx.pending_exception.is_some() {
        return None;
    }
    if !is_callable(ctx, &keys_fn) {
        throw_type_error(ctx, "keys must be callable");
        return None;
    }
    Some(SetRecord {
        is_set: false,
        set_ptr: 0,
        has_fn,
        keys_fn,
    })
}

fn record_has(
    ctx: &mut JSContext,
    record: &SetRecord,
    other: JSValue,
    value: JSValue,
) -> Option<bool> {
    if record.is_set {
        let obj = unsafe { &*(record.set_ptr as *const JSObject) };
        Some(set_find(ctx, obj, &value).is_some())
    } else {
        match call_callback_with_this(ctx, record.has_fn, other, &[value]) {
            Ok(v) => Some(v.is_truthy()),
            Err(_) => None,
        }
    }
}

fn iterate_set_like_keys(
    ctx: &mut JSContext,
    record: &SetRecord,
    other: JSValue,
) -> Option<Vec<JSValue>> {
    let iterator = match call_callback_with_this(ctx, record.keys_fn, other, &[]) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if !iterator.is_object() {
        throw_type_error(ctx, "keys() must return an Object");
        return None;
    }
    let mut out = Vec::new();
    loop {
        match iterator_next(ctx, iterator) {
            Some((_v, true)) => break,
            Some((v, false)) => out.push(v),
            None => return None,
        }
    }
    Some(out)
}

fn require_set_receiver(ctx: &mut JSContext, args: &[JSValue], method: &str) -> Option<usize> {
    let this = args.first().copied().unwrap_or(JSValue::undefined());
    if !this.is_object() {
        throw_type_error(
            ctx,
            &format!("Method Set.prototype.{} called on incompatible receiver", method),
        );
        return None;
    }
    if !has_setdata_slot(ctx, this.as_object()) {
        throw_type_error(
            ctx,
            &format!("Method Set.prototype.{} called on incompatible receiver", method),
        );
        return None;
    }
    Some(this.get_ptr())
}

fn set_union(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "union") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let (result, result_ptr) = create_empty_set(ctx);
    {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        for e in set_elements(ctx, this_obj) {
            set_add_value(ctx, result_ptr, e);
        }
    }
    if record.is_set {
        let other_obj = unsafe { &*(record.set_ptr as *const JSObject) };
        for e in set_elements(ctx, other_obj) {
            set_add_value(ctx, result_ptr, e);
        }
    } else {
        let Some(keys) = iterate_set_like_keys(ctx, &record, other) else {
            return JSValue::undefined();
        };
        for k in keys {
            set_add_value(ctx, result_ptr, k);
        }
    }
    result
}

fn set_intersection(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "intersection") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let (result, result_ptr) = create_empty_set(ctx);
    let this_elems = {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        set_elements(ctx, this_obj)
    };
    for e in this_elems {
        match record_has(ctx, &record, other, e) {
            Some(true) => set_add_value(ctx, result_ptr, e),
            Some(false) => {}
            None => return JSValue::undefined(),
        }
    }
    result
}

fn set_difference(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "difference") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let (result, result_ptr) = create_empty_set(ctx);
    let this_elems = {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        set_elements(ctx, this_obj)
    };
    for e in this_elems {
        match record_has(ctx, &record, other, e) {
            Some(true) => {}
            Some(false) => set_add_value(ctx, result_ptr, e),
            None => return JSValue::undefined(),
        }
    }
    result
}

fn set_symmetric_difference(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "symmetricDifference") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let (result, result_ptr) = create_empty_set(ctx);
    let this_elems = {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        set_elements(ctx, this_obj)
    };
    for e in &this_elems {
        set_add_value(ctx, result_ptr, *e);
    }
    let other_elems: Vec<JSValue> = if record.is_set {
        let other_obj = unsafe { &*(record.set_ptr as *const JSObject) };
        set_elements(ctx, other_obj)
    } else {
        let Some(keys) = iterate_set_like_keys(ctx, &record, other) else {
            return JSValue::undefined();
        };
        keys
    };
    for e in other_elems {
        let in_this = this_elems.iter().any(|x| key_same_value_zero(x, &e));
        if in_this {
            set_remove_value(ctx, result_ptr, e);
        } else {
            set_add_value(ctx, result_ptr, e);
        }
    }
    result
}

fn set_is_subset_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "isSubsetOf") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let this_elems = {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        set_elements(ctx, this_obj)
    };
    for e in this_elems {
        match record_has(ctx, &record, other, e) {
            Some(true) => {}
            Some(false) => return JSValue::bool(false),
            None => return JSValue::undefined(),
        }
    }
    JSValue::bool(true)
}

fn set_is_superset_of(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "isSupersetOf") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let other_elems: Vec<JSValue> = if record.is_set {
        let other_obj = unsafe { &*(record.set_ptr as *const JSObject) };
        set_elements(ctx, other_obj)
    } else {
        let Some(keys) = iterate_set_like_keys(ctx, &record, other) else {
            return JSValue::undefined();
        };
        keys
    };
    let this_obj = unsafe { &*(this_ptr as *const JSObject) };
    for e in other_elems {
        if set_find(ctx, this_obj, &e).is_none() {
            return JSValue::bool(false);
        }
    }
    JSValue::bool(true)
}

fn set_is_disjoint_from(ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
    let Some(this_ptr) = require_set_receiver(ctx, args, "isDisjointFrom") else {
        return JSValue::undefined();
    };
    let other = args.get(1).copied().unwrap_or(JSValue::undefined());
    let Some(record) = get_set_record(ctx, other) else {
        return JSValue::undefined();
    };
    let this_elems = {
        let this_obj = unsafe { &*(this_ptr as *const JSObject) };
        set_elements(ctx, this_obj)
    };
    for e in this_elems {
        match record_has(ctx, &record, other, e) {
            Some(true) => return JSValue::bool(false),
            Some(false) => {}
            None => return JSValue::undefined(),
        }
    }
    JSValue::bool(true)
}

pub fn init_map_set(ctx: &mut JSContext) {
    let mut map_proto = JSObject::new();
    set_ne(&mut map_proto, ctx.intern("set"), create_builtin_fn(ctx, "map_set", 2));
    set_ne(&mut map_proto, ctx.intern("get"), create_builtin_fn(ctx, "map_get", 1));
    set_ne(&mut map_proto, ctx.intern("getOrInsert"), create_builtin_fn(ctx, "map_get_or_insert", 2));
    set_ne(&mut map_proto, ctx.intern("getOrInsertComputed"), create_builtin_fn(ctx, "map_get_or_insert_computed", 2));
    set_ne(&mut map_proto, ctx.intern("has"), create_builtin_fn(ctx, "map_has", 1));
    set_ne(
        &mut map_proto,
        ctx.intern("delete"),
        create_builtin_fn(ctx, "map_delete", 1),
    );
    set_ne(
        &mut map_proto,
        ctx.intern("clear"),
        create_builtin_fn(ctx, "map_clear", 0),
    );
    set_ne(
        &mut map_proto,
        ctx.intern("forEach"),
        create_builtin_fn(ctx, "map_forEach", 1),
    );
    set_ne(&mut map_proto, ctx.intern("keys"), create_builtin_fn(ctx, "map_keys", 0));
    set_ne(
        &mut map_proto,
        ctx.intern("values"),
        create_builtin_fn(ctx, "map_values", 0),
    );
    let map_entries_fn = create_builtin_fn(ctx, "map_entries", 0);
    set_ne(
        &mut map_proto,
        ctx.intern("entries"),
        map_entries_fn.clone(),
    );

    let map_size_getter_fn = make_accessor_getter(ctx, "get size", "map_size_getter");
    define_accessor(&mut map_proto, ctx.intern("size"), map_size_getter_fn, false);
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut map_proto, tag_key, JSValue::new_string(ctx.intern("Map")));
    }

    let sym_iter_atom = crate::builtins::symbol::get_symbol_iterator_atom(ctx);
    set_ne(
        &mut map_proto,
        sym_iter_atom,
        map_entries_fn,
    );

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        map_proto.prototype = Some(obj_proto_ptr);
    }

    let map_proto_ptr = Box::into_raw(Box::new(map_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(map_proto_ptr);
    ctx.set_map_prototype(map_proto_ptr);

    let map_proto_value = JSValue::new_object(map_proto_ptr);

    let map_ctor = create_builtin_fn(ctx, "map_constructor", 0);
    if map_ctor.is_function() {
        let map_func_ref = map_ctor.as_function_mut();
        define_ctor_prototype(&mut map_func_ref.base, ctx.common_atoms.prototype, map_proto_value);
        let group_by_fn = create_builtin_fn(ctx, "map_group_by", 2);
        map_func_ref.base.set(ctx.intern("groupBy"), group_by_fn);
    }
    crate::builtins::symbol::install_species_accessor(ctx, &map_ctor);
    unsafe {
        let proto_ref = &mut *(map_proto_ptr as *mut crate::object::object::JSObject);
        set_ne(proto_ref, ctx.common_atoms.constructor, map_ctor);
    }

    let map_atom = ctx.intern("Map");
    let global = ctx.global();
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, map_atom, map_ctor);
    }

    let mut map_iter_proto = JSObject::new();
    set_ne(&mut map_iter_proto, ctx.intern("next"), create_builtin_fn(ctx, "map_set_iterator_next", 0));
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut map_iter_proto, tag_key, JSValue::new_string(ctx.intern("Map Iterator")));
    }
    let map_sym_iter = crate::builtins::symbol::get_symbol_iterator_atom(ctx);
    set_ne(&mut map_iter_proto, map_sym_iter, create_builtin_fn(ctx, "iterator_symbol_iterator", 0));
    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        map_iter_proto.prototype = Some(obj_proto_ptr);
    }
    let map_iter_proto_ptr = Box::into_raw(Box::new(map_iter_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(map_iter_proto_ptr);
    ctx.set_map_iterator_prototype(map_iter_proto_ptr);

    let mut set_proto = JSObject::new();
    set_ne(&mut set_proto, ctx.intern("add"), create_builtin_fn(ctx, "set_add", 1));
    set_ne(&mut set_proto, ctx.intern("has"), create_builtin_fn(ctx, "set_has", 1));
    set_ne(
        &mut set_proto,
        ctx.intern("delete"),
        create_builtin_fn(ctx, "set_delete", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("clear"),
        create_builtin_fn(ctx, "set_clear", 0),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("forEach"),
        create_builtin_fn(ctx, "set_forEach", 1),
    );
    let set_values_fn = create_builtin_fn(ctx, "set_values", 0);
    set_ne(
        &mut set_proto,
        ctx.intern("values"),
        set_values_fn.clone(),
    );
    set_ne(&mut set_proto, ctx.intern("keys"), set_values_fn.clone());
    set_ne(
        &mut set_proto,
        ctx.intern("entries"),
        create_builtin_fn(ctx, "set_entries", 0),
    );
    set_ne(&mut set_proto, ctx.intern("union"), create_builtin_fn(ctx, "set_union", 1));
    set_ne(
        &mut set_proto,
        ctx.intern("intersection"),
        create_builtin_fn(ctx, "set_intersection", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("difference"),
        create_builtin_fn(ctx, "set_difference", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("symmetricDifference"),
        create_builtin_fn(ctx, "set_symmetricDifference", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("isSubsetOf"),
        create_builtin_fn(ctx, "set_isSubsetOf", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("isSupersetOf"),
        create_builtin_fn(ctx, "set_isSupersetOf", 1),
    );
    set_ne(
        &mut set_proto,
        ctx.intern("isDisjointFrom"),
        create_builtin_fn(ctx, "set_isDisjointFrom", 1),
    );

    let set_size_getter_fn = make_accessor_getter(ctx, "get size", "set_size_getter");
    define_accessor(&mut set_proto, ctx.intern("size"), set_size_getter_fn, false);
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut set_proto, tag_key, JSValue::new_string(ctx.intern("Set")));
    }

    let sym_iter_atom = crate::builtins::symbol::get_symbol_iterator_atom(ctx);
    set_ne(
        &mut set_proto,
        sym_iter_atom,
        set_values_fn,
    );

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        set_proto.prototype = Some(obj_proto_ptr);
    }

    let set_proto_ptr = Box::into_raw(Box::new(set_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(set_proto_ptr);
    ctx.set_set_prototype(set_proto_ptr);

    let set_proto_value = JSValue::new_object(set_proto_ptr);

    let set_ctor = create_builtin_fn(ctx, "set_constructor", 0);
    if set_ctor.is_function() {
        let set_func_ref = set_ctor.as_function_mut();
        define_ctor_prototype(&mut set_func_ref.base, ctx.common_atoms.prototype, set_proto_value);
    }
    crate::builtins::symbol::install_species_accessor(ctx, &set_ctor);
    unsafe {
        let proto_ref = &mut *(set_proto_ptr as *mut crate::object::object::JSObject);
        set_ne(proto_ref, ctx.common_atoms.constructor, set_ctor);
    }

    let set_atom = ctx.intern("Set");
    let global = ctx.global();
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, set_atom, set_ctor);
    }

    let mut set_iter_proto = JSObject::new();
    set_ne(&mut set_iter_proto, ctx.intern("next"), create_builtin_fn(ctx, "map_set_iterator_next", 0));
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut set_iter_proto, tag_key, JSValue::new_string(ctx.intern("Set Iterator")));
    }
    let set_sym_iter = crate::builtins::symbol::get_symbol_iterator_atom(ctx);
    set_ne(&mut set_iter_proto, set_sym_iter, create_builtin_fn(ctx, "iterator_symbol_iterator", 0));
    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        set_iter_proto.prototype = Some(obj_proto_ptr);
    }
    let set_iter_proto_ptr = Box::into_raw(Box::new(set_iter_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(set_iter_proto_ptr);
    ctx.set_set_iterator_prototype(set_iter_proto_ptr);

    let mut weakmap_proto = JSObject::new();
    set_ne(
        &mut weakmap_proto,
        ctx.intern("set"),
        create_builtin_fn(ctx, "weakmap_set", 2),
    );
    set_ne(
        &mut weakmap_proto,
        ctx.intern("get"),
        create_builtin_fn(ctx, "weakmap_get", 1),
    );
    set_ne(
        &mut weakmap_proto,
        ctx.intern("has"),
        create_builtin_fn(ctx, "weakmap_has", 1),
    );
    set_ne(
        &mut weakmap_proto,
        ctx.intern("delete"),
        create_builtin_fn(ctx, "weakmap_delete", 1),
    );
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut weakmap_proto, tag_key, JSValue::new_string(ctx.intern("WeakMap")));
    }

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        weakmap_proto.prototype = Some(obj_proto_ptr);
    }

    let weakmap_proto_ptr = Box::into_raw(Box::new(weakmap_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(weakmap_proto_ptr);
    ctx.set_weakmap_prototype(weakmap_proto_ptr);

    let weakmap_proto_value = JSValue::new_object(weakmap_proto_ptr);

    let weakmap_ctor = create_builtin_fn(ctx, "weakmap_constructor", 0);
    if weakmap_ctor.is_function() {
        let wm_func_ref = weakmap_ctor.as_function_mut();
        define_ctor_prototype(&mut wm_func_ref.base, ctx.common_atoms.prototype, weakmap_proto_value);
    }
    unsafe {
        let proto_ref = &mut *(weakmap_proto_ptr as *mut crate::object::object::JSObject);
        set_ne(proto_ref, ctx.common_atoms.constructor, weakmap_ctor);
    }

    let weakmap_atom = ctx.intern("WeakMap");
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, weakmap_atom, weakmap_ctor);
    }

    let mut weakset_proto = JSObject::new();
    set_ne(
        &mut weakset_proto,
        ctx.intern("add"),
        create_builtin_fn(ctx, "weakset_add", 1),
    );
    set_ne(
        &mut weakset_proto,
        ctx.intern("has"),
        create_builtin_fn(ctx, "weakset_has", 1),
    );
    set_ne(
        &mut weakset_proto,
        ctx.intern("delete"),
        create_builtin_fn(ctx, "weakset_delete", 1),
    );
    {
        let tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
        set_ne_readonly(&mut weakset_proto, tag_key, JSValue::new_string(ctx.intern("WeakSet")));
    }

    if let Some(obj_proto_ptr) = ctx.get_object_prototype() {
        weakset_proto.prototype = Some(obj_proto_ptr);
    }

    let weakset_proto_ptr = Box::into_raw(Box::new(weakset_proto)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(weakset_proto_ptr);
    ctx.set_weakset_prototype(weakset_proto_ptr);

    let weakset_proto_value = JSValue::new_object(weakset_proto_ptr);

    let weakset_ctor = create_builtin_fn(ctx, "weakset_constructor", 0);
    if weakset_ctor.is_function() {
        let ws_func_ref = weakset_ctor.as_function_mut();
        define_ctor_prototype(&mut ws_func_ref.base, ctx.common_atoms.prototype, weakset_proto_value);
    }
    unsafe {
        let proto_ref = &mut *(weakset_proto_ptr as *mut crate::object::object::JSObject);
        set_ne(proto_ref, ctx.common_atoms.constructor, weakset_ctor);
    }

    let weakset_atom = ctx.intern("WeakSet");
    if global.is_object() {
        let global_obj = global.as_object_mut();
        crate::builtins::global::set_non_enumerable(global_obj, weakset_atom, weakset_ctor);
    }
}

pub fn register_builtins(ctx: &mut JSContext) {
    ctx.register_builtin(
        "map_constructor",
        HostFunction::ctor("Map", 0, map_constructor),
    );
    ctx.register_builtin("map_set", HostFunction::method("set", 2, map_set));
    ctx.register_builtin("map_get", HostFunction::method("get", 1, map_get));
    ctx.register_builtin("map_get_or_insert", HostFunction::method("getOrInsert", 2, map_get_or_insert));
    ctx.register_builtin("map_get_or_insert_computed", HostFunction::method("getOrInsertComputed", 2, map_get_or_insert_computed));
    ctx.register_builtin("map_group_by", HostFunction::new("groupBy", 2, map_group_by));
    ctx.register_builtin("map_has", HostFunction::method("has", 1, map_has));
    ctx.register_builtin("map_size_getter", HostFunction::method("get size", 0, map_size_getter));
    ctx.register_builtin("map_delete", HostFunction::method("delete", 1, map_delete));
    ctx.register_builtin("map_clear", HostFunction::method("clear", 0, map_clear));
    ctx.register_builtin(
        "map_forEach",
        HostFunction::method("forEach", 1, map_for_each),
    );
    ctx.register_builtin("map_keys", HostFunction::method("keys", 0, map_keys));
    ctx.register_builtin("map_values", HostFunction::method("values", 0, map_values));
    ctx.register_builtin(
        "map_entries",
        HostFunction::method("entries", 0, map_entries),
    );
    ctx.register_builtin(
        "map_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, map_symbol_iterator),
    );

    ctx.register_builtin(
        "set_constructor",
        HostFunction::ctor("Set", 0, set_constructor),
    );
    ctx.register_builtin("set_add", HostFunction::method("add", 1, set_add));
    ctx.register_builtin("set_has", HostFunction::method("has", 1, set_has));
    ctx.register_builtin("set_size_getter", HostFunction::method("get size", 0, set_size_getter));
    ctx.register_builtin("set_delete", HostFunction::method("delete", 1, set_delete));
    ctx.register_builtin("set_clear", HostFunction::method("clear", 0, set_clear));
    ctx.register_builtin(
        "set_forEach",
        HostFunction::method("forEach", 1, set_for_each),
    );
    ctx.register_builtin("set_values", HostFunction::method("values", 0, set_values));
    ctx.register_builtin("set_keys", HostFunction::method("keys", 0, set_keys));
    ctx.register_builtin(
        "set_entries",
        HostFunction::method("entries", 0, set_entries),
    );
    ctx.register_builtin("set_union", HostFunction::method("union", 1, set_union));
    ctx.register_builtin(
        "set_intersection",
        HostFunction::method("intersection", 1, set_intersection),
    );
    ctx.register_builtin(
        "set_difference",
        HostFunction::method("difference", 1, set_difference),
    );
    ctx.register_builtin(
        "set_symmetricDifference",
        HostFunction::method("symmetricDifference", 1, set_symmetric_difference),
    );
    ctx.register_builtin(
        "set_isSubsetOf",
        HostFunction::method("isSubsetOf", 1, set_is_subset_of),
    );
    ctx.register_builtin(
        "set_isSupersetOf",
        HostFunction::method("isSupersetOf", 1, set_is_superset_of),
    );
    ctx.register_builtin(
        "set_isDisjointFrom",
        HostFunction::method("isDisjointFrom", 1, set_is_disjoint_from),
    );
    ctx.register_builtin(
        "set_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, set_symbol_iterator),
    );

    ctx.register_builtin(
        "map_set_iterator_next",
        HostFunction::method("next", 0, map_set_iterator_next),
    );
    ctx.register_builtin(
        "iterator_symbol_iterator",
        HostFunction::method("[Symbol.iterator]", 0, iterator_symbol_iterator),
    );

    ctx.register_builtin(
        "weakmap_constructor",
        HostFunction::ctor("WeakMap", 0, weakmap_constructor),
    );
    ctx.register_builtin("weakmap_set", HostFunction::method("set", 2, weakmap_set));
    ctx.register_builtin("weakmap_get", HostFunction::method("get", 1, weakmap_get));
    ctx.register_builtin("weakmap_has", HostFunction::method("has", 1, weakmap_has));
    ctx.register_builtin(
        "weakmap_delete",
        HostFunction::method("delete", 1, weakmap_delete),
    );

    ctx.register_builtin(
        "weakset_constructor",
        HostFunction::ctor("WeakSet", 0, weakset_constructor),
    );
    ctx.register_builtin("weakset_add", HostFunction::method("add", 1, weakset_add));
    ctx.register_builtin("weakset_has", HostFunction::method("has", 1, weakset_has));
    ctx.register_builtin(
        "weakset_delete",
        HostFunction::method("delete", 1, weakset_delete),
    );
}
