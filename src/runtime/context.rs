use super::atom::{Atom, AtomTable};
use super::event_loop::EventLoop;
use super::extension::MacroTaskExtension;
use super::runtime::JSRuntime;
use crate::builtins;
use crate::builtins::promise::{Microtask, MicrotaskQueue};
use crate::compiler::codegen::OptLevel;
use crate::host::HostFunction;
use crate::object::JSObject;
use crate::object::ShapeCache;
use crate::util::FxHashMap;
use crate::value::JSValue;

#[allow(non_snake_case)]
pub struct CommonAtoms {
    pub length: Atom,
    pub prototype: Atom,
    pub constructor: Atom,
    pub call: Atom,
    pub apply: Atom,
    pub bind: Atom,
    pub to_string: Atom,
    pub value_of: Atom,
    pub push: Atom,
    pub pop: Atom,
    pub shift: Atom,
    pub unshift: Atom,
    pub slice: Atom,
    pub splice: Atom,
    pub join: Atom,
    pub map: Atom,
    pub for_each: Atom,
    pub filter: Atom,
    pub reduce: Atom,
    pub index_of: Atom,
    pub last_index_of: Atom,
    pub includes: Atom,
    pub concat: Atom,
    pub reverse: Atom,
    pub sort: Atom,
    pub flat: Atom,
    pub flat_map: Atom,
    pub find: Atom,
    pub find_index: Atom,
    pub every: Atom,
    pub some: Atom,
    pub fill: Atom,
    pub keys: Atom,
    pub values: Atom,
    pub entries: Atom,
    pub has_own_property: Atom,
    pub is_prototype_of: Atom,
    pub property_is_enumerable: Atom,
    pub to_locale_string: Atom,
    pub split: Atom,
    pub replace: Atom,
    pub match_atom: Atom,
    pub search: Atom,
    pub test: Atom,
    pub exec: Atom,
    pub then: Atom,
    pub catch: Atom,
    pub finally: Atom,
    pub message: Atom,
    pub name: Atom,
    pub stack: Atom,
    pub __proto__: Atom,
    pub __super__: Atom,
    pub __boundFn: Atom,
    pub __boundArgs: Atom,
    pub __boundThis: Atom,
    pub __value__: Atom,
    pub __dateValue__: Atom,
    pub __pattern__: Atom,
    pub __flags__: Atom,
    pub source: Atom,
    pub global: Atom,
    pub ignore_case: Atom,
    pub multiline: Atom,
    pub sticky: Atom,
    pub unicode: Atom,
    pub undefined: Atom,
    pub null: Atom,
    pub object: Atom,
    pub function: Atom,
    pub number: Atom,
    pub string: Atom,
    pub boolean: Atom,
    pub symbol: Atom,
    pub bigint: Atom,
    pub math: Atom,
    pub json: Atom,
    pub array: Atom,
    pub regexp: Atom,
    pub error: Atom,
    pub date: Atom,
    pub promise: Atom,
    pub map_ctor: Atom,
    pub set_ctor: Atom,
    pub weak_map_ctor: Atom,
    pub weak_set_ctor: Atom,
    pub proxy: Atom,
    pub reflect: Atom,
    pub console: Atom,
    pub index: Atom,
    pub input: Atom,
    pub empty: Atom,
    pub n0: Atom,
    pub n1: Atom,

    pub typeof_undefined: Atom,
    pub typeof_object: Atom,
    pub typeof_boolean: Atom,
    pub typeof_number: Atom,
    pub typeof_bigint: Atom,
    pub typeof_symbol: Atom,
    pub typeof_string: Atom,
    pub typeof_function: Atom,

    pub dot_all: Atom,
    pub last_index: Atom,
    pub callee: Atom,
    pub default_: Atom,
    pub url: Atom,
    pub __promise_state__: Atom,
    pub __promise_result__: Atom,
    pub __done__: Atom,
    pub __iter_arr__: Atom,
    pub __iter_idx__: Atom,
    pub __iter_kind__: Atom,
    pub __iter_obj__: Atom,
    pub __iter_next__: Atom,
    pub __iter_async__: Atom,
    pub to_string_tag: Atom,
}

impl CommonAtoms {
    pub fn new(tbl: &mut AtomTable) -> Self {
        CommonAtoms {
            length: tbl.intern("length"),
            prototype: tbl.intern("prototype"),
            constructor: tbl.intern("constructor"),
            call: tbl.intern("call"),
            apply: tbl.intern("apply"),
            bind: tbl.intern("bind"),
            to_string: tbl.intern("toString"),
            value_of: tbl.intern("valueOf"),
            push: tbl.intern("push"),
            pop: tbl.intern("pop"),
            shift: tbl.intern("shift"),
            unshift: tbl.intern("unshift"),
            slice: tbl.intern("slice"),
            splice: tbl.intern("splice"),
            join: tbl.intern("join"),
            map: tbl.intern("map"),
            for_each: tbl.intern("forEach"),
            filter: tbl.intern("filter"),
            reduce: tbl.intern("reduce"),
            index_of: tbl.intern("indexOf"),
            last_index_of: tbl.intern("lastIndexOf"),
            includes: tbl.intern("includes"),
            concat: tbl.intern("concat"),
            reverse: tbl.intern("reverse"),
            sort: tbl.intern("sort"),
            flat: tbl.intern("flat"),
            flat_map: tbl.intern("flatMap"),
            find: tbl.intern("find"),
            find_index: tbl.intern("findIndex"),
            every: tbl.intern("every"),
            some: tbl.intern("some"),
            fill: tbl.intern("fill"),
            keys: tbl.intern("keys"),
            values: tbl.intern("values"),
            entries: tbl.intern("entries"),
            has_own_property: tbl.intern("hasOwnProperty"),
            is_prototype_of: tbl.intern("isPrototypeOf"),
            property_is_enumerable: tbl.intern("propertyIsEnumerable"),
            to_locale_string: tbl.intern("toLocaleString"),
            split: tbl.intern("split"),
            replace: tbl.intern("replace"),
            match_atom: tbl.intern("match"),
            search: tbl.intern("search"),
            test: tbl.intern("test"),
            exec: tbl.intern("exec"),
            then: tbl.intern("then"),
            catch: tbl.intern("catch"),
            finally: tbl.intern("finally"),
            message: tbl.intern("message"),
            name: tbl.intern("name"),
            stack: tbl.intern("stack"),
            __proto__: tbl.intern("__proto__"),
            __super__: tbl.intern("__super__"),
            __boundFn: tbl.intern("__boundFn"),
            __boundArgs: tbl.intern("__boundArgs"),
            __boundThis: tbl.intern("__boundThis"),
            __value__: tbl.intern("__value__"),
            __dateValue__: tbl.intern("__dateValue__"),
            __pattern__: tbl.intern("__pattern__"),
            __flags__: tbl.intern("__flags__"),
            source: tbl.intern("source"),
            global: tbl.intern("global"),
            ignore_case: tbl.intern("ignoreCase"),
            multiline: tbl.intern("multiline"),
            sticky: tbl.intern("sticky"),
            unicode: tbl.intern("unicode"),
            undefined: tbl.intern("undefined"),
            null: tbl.intern("null"),
            object: tbl.intern("Object"),
            function: tbl.intern("Function"),
            number: tbl.intern("Number"),
            string: tbl.intern("String"),
            boolean: tbl.intern("Boolean"),
            symbol: tbl.intern("Symbol"),
            bigint: tbl.intern("BigInt"),
            math: tbl.intern("Math"),
            json: tbl.intern("JSON"),
            array: tbl.intern("Array"),
            regexp: tbl.intern("RegExp"),
            error: tbl.intern("Error"),
            date: tbl.intern("Date"),
            promise: tbl.intern("Promise"),
            map_ctor: tbl.intern("Map"),
            set_ctor: tbl.intern("Set"),
            weak_map_ctor: tbl.intern("WeakMap"),
            weak_set_ctor: tbl.intern("WeakSet"),
            proxy: tbl.intern("Proxy"),
            reflect: tbl.intern("Reflect"),
            console: tbl.intern("console"),
            index: tbl.intern("index"),
            input: tbl.intern("input"),
            empty: tbl.intern(""),
            n0: tbl.intern("0"),
            n1: tbl.intern("1"),
            typeof_undefined: tbl.intern("undefined"),
            typeof_object: tbl.intern("object"),
            typeof_boolean: tbl.intern("boolean"),
            typeof_number: tbl.intern("number"),
            typeof_bigint: tbl.intern("bigint"),
            typeof_symbol: tbl.intern("symbol"),
            typeof_string: tbl.intern("string"),
            typeof_function: tbl.intern("function"),
            dot_all: tbl.intern("dotAll"),
            last_index: tbl.intern("lastIndex"),
            callee: tbl.intern("callee"),
            default_: tbl.intern("default"),
            url: tbl.intern("url"),
            __promise_state__: tbl.intern("__promise_state__"),
            __promise_result__: tbl.intern("__promise_result__"),
            __done__: tbl.intern("__done__"),
            __iter_arr__: tbl.intern("__iter_arr__"),
            __iter_idx__: tbl.intern("__iter_idx__"),
            __iter_kind__: tbl.intern("__iter_kind__"),
            __iter_obj__: tbl.intern("__iter_obj__"),
            __iter_next__: tbl.intern("__iter_next__"),
            __iter_async__: tbl.intern("__iter_async__"),
            to_string_tag: tbl.intern("Symbol.toStringTag"),
        }
    }
}

pub struct JSContext {
    runtime: *mut JSRuntime,
    global_object: JSValue,
    interrupt_counter: i32,
    builtin_functions: FxHashMap<String, HostFunction>,

    string_prototype: Option<*mut JSObject>,
    number_prototype: Option<*mut JSObject>,
    array_prototype: Option<*mut JSObject>,
    array_iterator_prototype: Option<*mut JSObject>,

    array_iterator_fn: Option<*mut JSObject>,
    map_iterator_prototype: Option<*mut JSObject>,
    set_iterator_prototype: Option<*mut JSObject>,
    regexp_prototype: Option<*mut JSObject>,
    date_prototype: Option<*mut JSObject>,
    object_prototype: Option<*mut JSObject>,
    function_prototype: Option<*mut JSObject>,
    map_prototype: Option<*mut JSObject>,
    set_prototype: Option<*mut JSObject>,
    weakmap_prototype: Option<*mut JSObject>,
    weakset_prototype: Option<*mut JSObject>,
    dataview_prototype: Option<*mut JSObject>,
    typed_array_prototype: Option<*mut JSObject>,
    typed_array_prototypes: [Option<*mut JSObject>; 11],
    typed_array_iterator_prototype: Option<*mut JSObject>,
    array_buffer_prototype: Option<*mut JSObject>,
    error_prototype: Option<*mut JSObject>,
    type_error_prototype: Option<*mut JSObject>,
    reference_error_prototype: Option<*mut JSObject>,
    syntax_error_prototype: Option<*mut JSObject>,
    range_error_prototype: Option<*mut JSObject>,
    uri_error_prototype: Option<*mut JSObject>,
    eval_error_prototype: Option<*mut JSObject>,
    suppressed_error_prototype: Option<*mut JSObject>,
    aggregate_error_prototype: Option<*mut JSObject>,
    pub eval_private_names: Option<Vec<crate::runtime::atom::Atom>>,
    symbol_prototype: Option<*mut JSObject>,
    bool_prototype: Option<*mut JSObject>,
    bigint_prototype: Option<*mut JSObject>,
    weakref_prototype: Option<*mut JSObject>,
    finalization_registry_prototype: Option<*mut JSObject>,
    pub finalization_registries: std::cell::RefCell<Vec<*mut JSObject>>,
    pub tagged_template_cache: FxHashMap<u32, JSValue>,
    pub global_lexical_bindings: FxHashMap<u32, JSValue>,
    pub global_lexical_kinds: FxHashMap<u32, u8>,
    generator_prototype: Option<*mut JSObject>,
    async_generator_prototype: Option<*mut JSObject>,
    generator_function_prototype: Option<*mut JSObject>,
    async_generator_function_prototype: Option<*mut JSObject>,
    promise_prototype: Option<*mut JSObject>,
    promise_constructor: Option<*mut JSObject>,

    active_realm: *mut JSContext,

    register_vm_ptr: Option<*mut crate::runtime::vm::VM>,
    compiler_opt_level: OptLevel,

    pub pending_exception: Option<JSValue>,

    call_with_new: bool,

    pending_callback: Option<PendingCallback>,

    microtask_queue: MicrotaskQueue,

    current_module_specifier: Option<String>,
    script_base_path: Option<String>,
    import_meta_object: Option<*mut JSObject>,

    event_loop: EventLoop,

    running_event_loop: Option<*mut EventLoop>,

    shape_cache: ShapeCache,

    pub common_atoms: CommonAtoms,

    cached_int_atoms: [Atom; 100],

    args_length_shape: Option<std::rc::Rc<crate::object::shape::Shape>>,
}

#[derive(Clone)]
pub struct PendingCallback {
    pub func: JSValue,

    pub args: Vec<JSValue>,

    pub is_method_call: bool,
}

impl JSContext {
    const INTERRUPT_THRESHOLD: i32 = 1000;

    pub fn new(runtime: &mut JSRuntime) -> Self {
        let global = JSObject::new_global();
        let global_ptr = Box::into_raw(Box::new(global));
        runtime.gc_heap_mut().track(global_ptr);
        let global_value = JSValue::new_object(global_ptr);

        let mut ctx = JSContext {
            runtime: runtime as *mut JSRuntime,
            global_object: global_value,
            interrupt_counter: Self::INTERRUPT_THRESHOLD,
            builtin_functions: FxHashMap::default(),
            string_prototype: None,
            number_prototype: None,
            array_prototype: None,
            array_iterator_prototype: None,
            array_iterator_fn: None,
            map_iterator_prototype: None,
            set_iterator_prototype: None,
            regexp_prototype: None,
            date_prototype: None,
            object_prototype: None,
            function_prototype: None,
            map_prototype: None,
            set_prototype: None,
            weakmap_prototype: None,
            weakset_prototype: None,
            dataview_prototype: None,
            typed_array_prototype: None,
            typed_array_prototypes: [None; 11],
            typed_array_iterator_prototype: None,
            array_buffer_prototype: None,
            error_prototype: None,
            type_error_prototype: None,
            reference_error_prototype: None,
            syntax_error_prototype: None,
            range_error_prototype: None,
            uri_error_prototype: None,
            eval_error_prototype: None,
            suppressed_error_prototype: None,
            aggregate_error_prototype: None,
            eval_private_names: None,
            symbol_prototype: None,
            bool_prototype: None,
            bigint_prototype: None,
            weakref_prototype: None,
            finalization_registry_prototype: None,
            finalization_registries: std::cell::RefCell::new(Vec::new()),
            tagged_template_cache: FxHashMap::default(),
            global_lexical_bindings: FxHashMap::default(),
            global_lexical_kinds: FxHashMap::default(),
            generator_prototype: None,
            async_generator_prototype: None,
            generator_function_prototype: None,
            async_generator_function_prototype: None,
            promise_prototype: None,
            promise_constructor: None,

            active_realm: std::ptr::null_mut(),
            register_vm_ptr: None,
            compiler_opt_level: OptLevel::default(),
            pending_exception: None,
            call_with_new: false,
            pending_callback: None,
            microtask_queue: MicrotaskQueue::new(),
            current_module_specifier: None,
            script_base_path: None,
            import_meta_object: None,
            event_loop: EventLoop::new(),
            running_event_loop: None,
            shape_cache: ShapeCache::new(),
            common_atoms: CommonAtoms::new(runtime.atom_table_mut()),
            cached_int_atoms: {
                let mut arr = [Atom::empty(); 100];
                for i in 0..100 {
                    arr[i] = runtime.atom_table_mut().intern(&i.to_string());
                }
                arr
            },
            args_length_shape: None,
        };

        builtins::init_globals(&mut ctx);

        ctx
    }

    #[inline]
    pub fn is_call_with_new(&self) -> bool {
        self.call_with_new
    }

    #[inline]
    pub fn set_call_with_new(&mut self, value: bool) {
        self.call_with_new = value;
    }

    #[inline(always)]
    pub fn check_interrupt(&mut self) -> Result<(), String> {
        self.interrupt_counter -= 1;
        if self.interrupt_counter <= 0 {
            if self.runtime_mut().is_interrupted()
                || crate::runtime::runtime::EXTERNAL_INTERRUPT
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err("interrupted".to_string());
            }
            self.interrupt_counter = Self::INTERRUPT_THRESHOLD;
        }
        Ok(())
    }

    #[inline(always)]
    pub fn reset_interrupt_counter(&mut self) {
        self.interrupt_counter = 0;
    }

    pub fn runtime(&self) -> &JSRuntime {
        // SAFETY: the runtime pointer is set at context construction and the context is never used after its runtime is dropped.
        unsafe { &*self.runtime }
    }

    pub fn runtime_mut(&mut self) -> &mut JSRuntime {
        // SAFETY: the runtime pointer is set at context construction, is valid for the context's lifetime, and `&mut self` serializes access.
        unsafe { &mut *self.runtime }
    }

    pub fn array_index_has_own(&self, value: &JSValue, atom: Atom) -> bool {
        if !value.is_object() {
            return false;
        }
        let obj = value.as_object();
        let Some(array) = obj.as_dense_array() else {
            return false;
        };
        let Some(index) = self.atom_table().get_array_index(atom) else {
            return false;
        };
        index < array.elements.len()
    }

    pub fn array_index_own_descriptor(
        &self,
        value: &JSValue,
        atom: Atom,
    ) -> Option<crate::object::object::PropertyDescriptor> {
        if !value.is_object() {
            return None;
        }
        let obj = value.as_object();
        let Some(array) = obj.as_dense_array() else {
            return None;
        };
        let index = self.atom_table().get_array_index(atom)?;
        let element = *array.elements.get(index)?;
        let frozen = obj.frozen();
        Some(crate::object::object::PropertyDescriptor {
            value: Some(element),
            writable: !frozen,
            enumerable: true,
            configurable: !(frozen || obj.sealed()),
            get: None,
            set: None,
        })
    }

    pub fn atom_table(&self) -> &AtomTable {
        self.runtime().atom_table()
    }

    pub fn atom_table_mut(&mut self) -> &mut AtomTable {
        self.runtime_mut().atom_table_mut()
    }

    pub fn shape_cache(&self) -> &ShapeCache {
        &self.shape_cache
    }

    pub fn shape_cache_mut(&mut self) -> &mut ShapeCache {
        &mut self.shape_cache
    }

    #[inline]
    pub fn get_or_create_args_length_shape(
        &mut self,
    ) -> std::rc::Rc<crate::object::shape::Shape> {
        if let Some(s) = self.args_length_shape.as_ref() {
            return std::rc::Rc::clone(s);
        }
        let root = self.shape_cache.root_shape();
        let s = self.shape_cache.transition(&root, self.common_atoms.length);
        self.args_length_shape = Some(std::rc::Rc::clone(&s));
        s
    }

    pub fn refresh_builtin_realm(&mut self) {
        let context = self as *mut JSContext;
        let global = self.global();
        if !global.is_object_like() {
            return;
        }
        let mut function_ptrs: Vec<*mut crate::object::function::JSFunction> = Vec::new();
        global.as_object().for_each_property(|_, value, _| {
            if value.is_function() {
                function_ptrs.push(value.get_ptr().cast());
            }
        });
        for ptr in function_ptrs {
            let func_val = JSValue::new_function(ptr);
            let func = func_val.as_function_mut();
            func.realm_context = context;
            func.realm_global = global;
        }
    }

    pub fn swap_global_environment(&mut self, other: &mut JSContext) {
        std::mem::swap(&mut self.global_object, &mut other.global_object);
        std::mem::swap(
            &mut self.global_lexical_bindings,
            &mut other.global_lexical_bindings,
        );
        std::mem::swap(
            &mut self.global_lexical_kinds,
            &mut other.global_lexical_kinds,
        );
        std::mem::swap(&mut self.string_prototype, &mut other.string_prototype);
        std::mem::swap(&mut self.number_prototype, &mut other.number_prototype);
        std::mem::swap(&mut self.array_prototype, &mut other.array_prototype);
        std::mem::swap(
            &mut self.array_iterator_prototype,
            &mut other.array_iterator_prototype,
        );
        std::mem::swap(
            &mut self.array_iterator_fn,
            &mut other.array_iterator_fn,
        );
        std::mem::swap(
            &mut self.map_iterator_prototype,
            &mut other.map_iterator_prototype,
        );
        std::mem::swap(
            &mut self.set_iterator_prototype,
            &mut other.set_iterator_prototype,
        );
        std::mem::swap(&mut self.regexp_prototype, &mut other.regexp_prototype);
        std::mem::swap(&mut self.date_prototype, &mut other.date_prototype);
        std::mem::swap(&mut self.object_prototype, &mut other.object_prototype);
        std::mem::swap(&mut self.function_prototype, &mut other.function_prototype);
        std::mem::swap(&mut self.map_prototype, &mut other.map_prototype);
        std::mem::swap(&mut self.set_prototype, &mut other.set_prototype);
        std::mem::swap(&mut self.weakmap_prototype, &mut other.weakmap_prototype);
        std::mem::swap(&mut self.weakset_prototype, &mut other.weakset_prototype);
        std::mem::swap(&mut self.dataview_prototype, &mut other.dataview_prototype);
        std::mem::swap(
            &mut self.typed_array_prototype,
            &mut other.typed_array_prototype,
        );
        std::mem::swap(
            &mut self.typed_array_prototypes,
            &mut other.typed_array_prototypes,
        );
        std::mem::swap(
            &mut self.typed_array_iterator_prototype,
            &mut other.typed_array_iterator_prototype,
        );
        std::mem::swap(
            &mut self.array_buffer_prototype,
            &mut other.array_buffer_prototype,
        );
        std::mem::swap(&mut self.error_prototype, &mut other.error_prototype);
        std::mem::swap(
            &mut self.type_error_prototype,
            &mut other.type_error_prototype,
        );
        std::mem::swap(
            &mut self.reference_error_prototype,
            &mut other.reference_error_prototype,
        );
        std::mem::swap(
            &mut self.syntax_error_prototype,
            &mut other.syntax_error_prototype,
        );
        std::mem::swap(
            &mut self.range_error_prototype,
            &mut other.range_error_prototype,
        );
        std::mem::swap(&mut self.uri_error_prototype, &mut other.uri_error_prototype);
        std::mem::swap(
            &mut self.eval_error_prototype,
            &mut other.eval_error_prototype,
        );
        std::mem::swap(
            &mut self.suppressed_error_prototype,
            &mut other.suppressed_error_prototype,
        );
        std::mem::swap(
            &mut self.aggregate_error_prototype,
            &mut other.aggregate_error_prototype,
        );
        std::mem::swap(&mut self.symbol_prototype, &mut other.symbol_prototype);
        std::mem::swap(&mut self.bool_prototype, &mut other.bool_prototype);
        std::mem::swap(&mut self.bigint_prototype, &mut other.bigint_prototype);
        std::mem::swap(&mut self.weakref_prototype, &mut other.weakref_prototype);
        std::mem::swap(
            &mut self.finalization_registry_prototype,
            &mut other.finalization_registry_prototype,
        );
        std::mem::swap(
            &mut self.generator_prototype,
            &mut other.generator_prototype,
        );
        std::mem::swap(
            &mut self.async_generator_prototype,
            &mut other.async_generator_prototype,
        );
        std::mem::swap(
            &mut self.generator_function_prototype,
            &mut other.generator_function_prototype,
        );
        std::mem::swap(
            &mut self.async_generator_function_prototype,
            &mut other.async_generator_function_prototype,
        );
        std::mem::swap(
            &mut self.promise_prototype,
            &mut other.promise_prototype,
        );
        std::mem::swap(
            &mut self.promise_constructor,
            &mut other.promise_constructor,
        );
    }

    pub fn active_realm(&self) -> *mut JSContext {
        self.active_realm
    }

    pub fn set_active_realm(&mut self, realm: *mut JSContext) {
        self.active_realm = realm;
    }

    pub fn global(&self) -> JSValue {
        self.global_object
    }

    pub fn set_global(&mut self, value: JSValue) {
        self.global_object = value;
    }

    pub fn intern(&mut self, s: &str) -> Atom {
        self.atom_table_mut().intern(s)
    }

    #[inline]
    pub fn intern_concat(&mut self, a: &str, b: &str) -> Atom {
        self.atom_table_mut().intern_concat(a, b)
    }

    #[inline]
    pub fn intern_concat_atoms(&mut self, a: Atom, b: Atom) -> Atom {
        self.atom_table_mut().intern_concat_atoms(a, b)
    }

    #[inline]
    pub fn int_atom(&self, n: usize) -> Atom {
        if n < 100 {
            self.cached_int_atoms
                .get(n)
                .copied()
                .unwrap_or(self.common_atoms.empty)
        } else {
            self.common_atoms.empty
        }
    }

    #[inline]
    pub fn int_atom_mut(&mut self, n: usize) -> Atom {
        if n < 100 {
            self.cached_int_atoms
                .get(n)
                .copied()
                .unwrap_or(self.common_atoms.empty)
        } else {
            self.atom_table_mut().intern(&n.to_string())
        }
    }

    pub fn intern_fast(&mut self, s: &str) -> Atom {
        self.atom_table_mut().intern_fast(s)
    }

    pub fn lookup_atom(&self, s: &str) -> Option<Atom> {
        self.atom_table().lookup(s)
    }

    pub fn get_atom_str(&self, atom: Atom) -> &str {
        self.atom_table().get(atom)
    }

    #[inline]
    pub fn string_code_unit_count(&self, atom: Atom) -> usize {
        self.atom_table().code_unit_count(atom)
    }

    #[inline]
    pub fn string_code_unit_at(&self, atom: Atom, index: usize) -> Option<u32> {
        self.atom_table().code_unit_at(atom, index)
    }

    pub fn mark_symbol_atom(&mut self, atom: Atom) {
        self.atom_table_mut().mark_symbol_atom(atom.index());
    }

    pub fn is_symbol_atom(&self, atom: Atom) -> bool {
        self.atom_table().is_symbol_atom(atom.index())
    }

    pub fn register_builtin(&mut self, name: &str, func: HostFunction) {
        self.builtin_functions.insert(name.to_string(), func);
    }

    pub fn register_global_builtin(
        &mut self,
        name: &'static str,
        arity: u32,
        func: crate::host::HostFunc,
    ) {
        self.register_builtin(name, HostFunction::new(name, arity, func));
        let mut f = crate::object::function::JSFunction::new_builtin(self.intern(name), arity);
        f.set_builtin_marker(self, name);
        let ptr = Box::into_raw(Box::new(f));
        self.runtime_mut().gc_heap_mut().track_function(ptr);
        self.global()
            .as_object_mut()
            .set(self.intern(name), crate::value::JSValue::new_function(ptr));
    }

    pub fn call_builtin(&mut self, name: &str, args: &[JSValue]) -> JSValue {
        let func = self.builtin_functions.get(name).cloned();
        if let Some(f) = func {
            f.call(self, args)
        } else {
            JSValue::undefined()
        }
    }

    pub fn get_builtin_func(&self, name: &str) -> Option<crate::host::func::HostFunc> {
        self.builtin_functions.get(name).map(|f| f.func)
    }

    pub fn get_builtin_name(&self, name: &str) -> Option<&'static str> {
        self.builtin_functions.get(name).map(|f| f.name)
    }

    pub fn builtin_needs_this(&self, name: &str) -> bool {
        self.builtin_functions
            .get(name)
            .map(|f| f.needs_this)
            .unwrap_or(false)
    }

    pub fn builtin_is_constructor(&self, name: &str) -> bool {
        self.builtin_functions
            .get(name)
            .map(|f| f.is_constructor)
            .unwrap_or(false)
    }

    pub fn get_builtin_arity(&self, name: &str) -> Option<u32> {
        self.builtin_functions.get(name).map(|f| f.arity)
    }

    pub fn call_builtin_direct(
        &mut self,
        func: crate::host::func::HostFunc,
        args: &[JSValue],
    ) -> JSValue {
        func(self, args)
    }

    pub fn set_string_prototype(&mut self, ptr: *mut JSObject) {
        self.string_prototype = Some(ptr);
    }

    pub fn set_number_prototype(&mut self, ptr: *mut JSObject) {
        self.number_prototype = Some(ptr);
    }

    pub fn set_array_prototype(&mut self, ptr: *mut JSObject) {
        self.array_prototype = Some(ptr);
    }

    pub fn set_array_iterator_prototype(&mut self, ptr: *mut JSObject) {
        self.array_iterator_prototype = Some(ptr);
    }

    pub fn get_array_iterator_prototype(&self) -> Option<*mut JSObject> {
        self.array_iterator_prototype
    }

    pub fn set_array_iterator_fn(&mut self, ptr: *mut JSObject) {
        self.array_iterator_fn = Some(ptr);
    }

    pub fn get_array_iterator_fn(&self) -> Option<*mut JSObject> {
        self.array_iterator_fn
    }

    pub fn set_map_iterator_prototype(&mut self, ptr: *mut JSObject) {
        self.map_iterator_prototype = Some(ptr);
    }

    pub fn get_map_iterator_prototype(&self) -> Option<*mut JSObject> {
        self.map_iterator_prototype
    }

    pub fn set_set_iterator_prototype(&mut self, ptr: *mut JSObject) {
        self.set_iterator_prototype = Some(ptr);
    }

    pub fn get_set_iterator_prototype(&self) -> Option<*mut JSObject> {
        self.set_iterator_prototype
    }

    pub fn set_regexp_prototype(&mut self, ptr: *mut JSObject) {
        self.regexp_prototype = Some(ptr);
    }

    pub fn set_object_prototype(&mut self, ptr: *mut JSObject) {
        self.object_prototype = Some(ptr);
    }

    pub fn set_function_prototype(&mut self, ptr: *mut JSObject) {
        self.function_prototype = Some(ptr);
    }

    pub fn set_map_prototype(&mut self, ptr: *mut JSObject) {
        self.map_prototype = Some(ptr);
    }

    pub fn set_set_prototype(&mut self, ptr: *mut JSObject) {
        self.set_prototype = Some(ptr);
    }

    pub fn get_string_prototype(&self) -> Option<*mut JSObject> {
        self.string_prototype
    }

    pub fn get_number_prototype(&self) -> Option<*mut JSObject> {
        self.number_prototype
    }

    pub fn get_bool_prototype(&self) -> Option<*mut JSObject> {
        self.bool_prototype
    }

    pub fn set_bool_prototype(&mut self, ptr: *mut JSObject) {
        self.bool_prototype = Some(ptr);
    }

    pub fn get_bigint_prototype(&self) -> Option<*mut JSObject> {
        self.bigint_prototype
    }

    pub fn set_bigint_prototype(&mut self, ptr: *mut JSObject) {
        self.bigint_prototype = Some(ptr);
    }

    pub fn get_array_prototype(&self) -> Option<*mut JSObject> {
        self.array_prototype
    }

    pub fn get_regexp_prototype(&self) -> Option<*mut JSObject> {
        self.regexp_prototype
    }

    pub fn get_object_prototype(&self) -> Option<*mut JSObject> {
        self.object_prototype
    }

    pub fn set_date_prototype(&mut self, ptr: *mut JSObject) {
        self.date_prototype = Some(ptr);
    }

    pub fn get_date_prototype(&self) -> Option<*mut JSObject> {
        self.date_prototype
    }

    pub fn get_function_prototype(&self) -> Option<*mut JSObject> {
        self.function_prototype
    }

    pub fn get_map_prototype(&self) -> Option<*mut JSObject> {
        self.map_prototype
    }

    pub fn get_set_prototype(&self) -> Option<*mut JSObject> {
        self.set_prototype
    }

    pub fn set_weakmap_prototype(&mut self, ptr: *mut JSObject) {
        self.weakmap_prototype = Some(ptr);
    }

    pub fn get_weakmap_prototype(&self) -> Option<*mut JSObject> {
        self.weakmap_prototype
    }

    pub fn set_weakset_prototype(&mut self, ptr: *mut JSObject) {
        self.weakset_prototype = Some(ptr);
    }

    pub fn set_dataview_prototype(&mut self, ptr: *mut JSObject) {
        self.dataview_prototype = Some(ptr);
    }

    pub fn set_typed_array_prototype(&mut self, ptr: *mut JSObject) {
        self.typed_array_prototype = Some(ptr);
    }

    pub fn set_typed_array_prototype_for_kind(&mut self, kind_index: usize, ptr: *mut JSObject) {
        if let Some(slot) = self.typed_array_prototypes.get_mut(kind_index) {
            *slot = Some(ptr);
        }
    }

    pub fn get_typed_array_prototype_for_kind(&self, kind_index: usize) -> Option<*mut JSObject> {
        self.typed_array_prototypes
            .get(kind_index)
            .copied()
            .flatten()
    }

    pub fn set_typed_array_iterator_prototype(&mut self, ptr: *mut JSObject) {
        self.typed_array_iterator_prototype = Some(ptr);
    }

    pub fn set_array_buffer_prototype(&mut self, ptr: *mut JSObject) {
        self.array_buffer_prototype = Some(ptr);
    }

    pub fn get_array_buffer_prototype(&self) -> Option<*mut JSObject> {
        self.array_buffer_prototype
    }

    pub fn get_typed_array_iterator_prototype(&self) -> Option<*mut JSObject> {
        self.typed_array_iterator_prototype
    }

    pub fn get_weakset_prototype(&self) -> Option<*mut JSObject> {
        self.weakset_prototype
    }

    pub fn get_dataview_prototype(&self) -> Option<*mut JSObject> {
        self.dataview_prototype
    }

    pub fn get_typed_array_prototype(&self) -> Option<*mut JSObject> {
        self.typed_array_prototype
    }

    pub fn set_error_prototype(&mut self, ptr: *mut JSObject) {
        self.error_prototype = Some(ptr);
    }

    pub fn get_error_prototype(&self) -> Option<*mut JSObject> {
        self.error_prototype
    }

    pub fn set_type_error_prototype(&mut self, ptr: *mut JSObject) {
        self.type_error_prototype = Some(ptr);
    }

    pub fn get_type_error_prototype(&self) -> Option<*mut JSObject> {
        self.type_error_prototype
    }

    pub fn set_reference_error_prototype(&mut self, ptr: *mut JSObject) {
        self.reference_error_prototype = Some(ptr);
    }

    pub fn get_reference_error_prototype(&self) -> Option<*mut JSObject> {
        self.reference_error_prototype
    }

    pub fn set_range_error_prototype(&mut self, ptr: *mut JSObject) {
        self.range_error_prototype = Some(ptr);
    }

    pub fn get_range_error_prototype(&self) -> Option<*mut JSObject> {
        self.range_error_prototype
    }

    pub fn set_uri_error_prototype(&mut self, ptr: *mut JSObject) {
        self.uri_error_prototype = Some(ptr);
    }

    pub fn get_uri_error_prototype(&self) -> Option<*mut JSObject> {
        self.uri_error_prototype
    }

    pub fn set_syntax_error_prototype(&mut self, ptr: *mut JSObject) {
        self.syntax_error_prototype = Some(ptr);
    }

    pub fn get_syntax_error_prototype(&self) -> Option<*mut JSObject> {
        self.syntax_error_prototype
    }

    pub fn set_eval_error_prototype(&mut self, ptr: *mut JSObject) {
        self.eval_error_prototype = Some(ptr);
    }

    pub fn get_eval_error_prototype(&self) -> Option<*mut JSObject> {
        self.eval_error_prototype
    }

    pub fn set_suppressed_error_prototype(&mut self, ptr: *mut JSObject) {
        self.suppressed_error_prototype = Some(ptr);
    }

    pub fn get_suppressed_error_prototype(&self) -> Option<*mut JSObject> {
        self.suppressed_error_prototype
    }

    pub fn set_aggregate_error_prototype(&mut self, ptr: *mut JSObject) {
        self.aggregate_error_prototype = Some(ptr);
    }

    pub fn get_aggregate_error_prototype(&self) -> Option<*mut JSObject> {
        self.aggregate_error_prototype
    }

    pub fn set_symbol_prototype(&mut self, ptr: *mut JSObject) {
        self.symbol_prototype = Some(ptr);
    }

    pub fn get_symbol_prototype(&self) -> Option<*mut JSObject> {
        self.symbol_prototype
    }

    pub fn set_weakref_prototype(&mut self, ptr: *mut JSObject) {
        self.weakref_prototype = Some(ptr);
    }

    pub fn get_weakref_prototype(&self) -> Option<*mut JSObject> {
        self.weakref_prototype
    }

    pub fn set_finalization_registry_prototype(&mut self, ptr: *mut JSObject) {
        self.finalization_registry_prototype = Some(ptr);
    }

    pub fn get_finalization_registry_prototype(&self) -> Option<*mut JSObject> {
        self.finalization_registry_prototype
    }

    pub fn set_generator_prototype(&mut self, ptr: *mut JSObject) {
        self.generator_prototype = Some(ptr);
    }

    pub fn get_generator_prototype(&self) -> Option<*mut JSObject> {
        self.generator_prototype
    }

    pub fn set_async_generator_prototype(&mut self, ptr: *mut JSObject) {
        self.async_generator_prototype = Some(ptr);
    }

    pub fn get_async_generator_prototype(&self) -> Option<*mut JSObject> {
        self.async_generator_prototype
    }

    pub fn set_generator_function_prototype(&mut self, ptr: *mut JSObject) {
        self.generator_function_prototype = Some(ptr);
    }

    pub fn get_generator_function_prototype(&self) -> Option<*mut JSObject> {
        self.generator_function_prototype
    }

    pub fn set_async_generator_function_prototype(&mut self, ptr: *mut JSObject) {
        self.async_generator_function_prototype = Some(ptr);
    }

    pub fn get_async_generator_function_prototype(&self) -> Option<*mut JSObject> {
        self.async_generator_function_prototype
    }

    pub fn set_promise_prototype(&mut self, ptr: *mut JSObject) {
        self.promise_prototype = Some(ptr);
    }

    pub fn get_promise_prototype(&self) -> Option<*mut JSObject> {
        self.promise_prototype
    }

    pub fn set_promise_constructor(&mut self, ptr: *mut JSObject) {
        self.promise_constructor = Some(ptr);
    }

    pub fn get_promise_constructor(&self) -> Option<*mut JSObject> {
        self.promise_constructor
    }

    pub fn set_register_vm_ptr(&mut self, ptr: Option<*mut crate::runtime::vm::VM>) {
        self.register_vm_ptr = ptr;
    }

    pub fn get_register_vm_ptr(&self) -> Option<*mut crate::runtime::vm::VM> {
        self.register_vm_ptr
    }

    pub fn with_vm<R>(
        &mut self,
        f: impl FnOnce(&mut crate::runtime::vm::VM, &mut JSContext) -> R,
    ) -> Option<R> {
        let ptr = self.register_vm_ptr?;
        // SAFETY: the driver installs a live VM pointer for the duration of execution and clears it before dropping the VM.
        let vm = unsafe { &mut *ptr };
        Some(f(vm, self))
    }

    pub fn add_extension(&mut self, ext: Box<dyn MacroTaskExtension>) {
        self.event_loop_mut().extensions.push(ext);
    }

    pub fn set_compiler_opt_level(&mut self, opt_level: OptLevel) {
        self.compiler_opt_level = opt_level;
    }

    pub fn get_compiler_opt_level(&self) -> OptLevel {
        self.compiler_opt_level
    }

    pub fn set_pending_callback(&mut self, callback: PendingCallback) {
        self.pending_callback = Some(callback);
    }

    pub fn take_pending_callback(&mut self) -> Option<PendingCallback> {
        self.pending_callback.take()
    }

    pub fn has_pending_callback(&self) -> bool {
        self.pending_callback.is_some()
    }

    pub fn microtask_enqueue(&mut self, task: Microtask) {
        self.microtask_queue.enqueue(task);
    }

    pub fn microtask_is_empty(&self) -> bool {
        self.microtask_queue.is_empty()
    }

    pub fn microtask_dequeue(&mut self) -> Option<Microtask> {
        self.microtask_queue.dequeue()
    }

    pub fn set_current_module(&mut self, specifier: Option<String>) {
        self.current_module_specifier = specifier;
    }

    pub fn get_current_module(&self) -> Option<&str> {
        self.current_module_specifier.as_deref()
    }

    pub fn set_script_base_path(&mut self, path: Option<String>) {
        self.script_base_path = path;
    }

    pub fn get_script_base_path(&self) -> Option<&str> {
        self.script_base_path.as_deref()
    }

    pub fn get_symbol_registry_ptr(&self) -> Option<*mut JSObject> {
        self.runtime().symbol_registry()
    }

    pub fn set_symbol_registry_ptr(&mut self, ptr: Option<*mut JSObject>) {
        self.runtime_mut().set_symbol_registry(ptr);
    }

    pub fn set_import_meta(&mut self, ptr: Option<*mut JSObject>) {
        self.import_meta_object = ptr;
    }

    pub fn get_import_meta(&self) -> Option<*mut JSObject> {
        self.import_meta_object
    }

    pub fn event_loop(&self) -> &EventLoop {
        &self.event_loop
    }

    pub fn event_loop_mut(&mut self) -> &mut EventLoop {
        if let Some(ptr) = self.running_event_loop {
            // SAFETY: the pointer is installed only while an outer event loop drives a nested loop and refers to a live caller-owned loop.
            unsafe { &mut *ptr }
        } else {
            &mut self.event_loop
        }
    }

    pub fn run_event_loop(&mut self) -> Result<super::event_loop::EventLoopResult, String> {
        self.run_event_loop_impl(None)
    }

    pub fn run_event_loop_with_timeout(
        &mut self,
        timeout_ms: u64,
    ) -> Result<super::event_loop::EventLoopResult, String> {
        self.run_event_loop_impl(Some(timeout_ms))
    }

    fn run_event_loop_impl(
        &mut self,
        timeout_ms: Option<u64>,
    ) -> Result<super::event_loop::EventLoopResult, String> {
        let saved_register_vm_ptr = self.register_vm_ptr;

        let mut owned_vm = if saved_register_vm_ptr.is_none() {
            let mut vm = crate::runtime::vm::VM::new();
            vm.prime();
            Some(vm)
        } else {
            None
        };
        if let Some(vm) = owned_vm.as_mut() {
            self.register_vm_ptr = Some(vm as *mut _);
        }

        let result = {
            let mut event_loop = std::mem::take(&mut self.event_loop);

            self.running_event_loop = Some(&mut event_loop as *mut EventLoop);
            let result = event_loop.run_until_complete(self, timeout_ms);
            self.running_event_loop = None;
            self.event_loop = event_loop;

            result
        };

        drop(owned_vm.take());
        self.register_vm_ptr = saved_register_vm_ptr;
        result
    }
}
