use crate::object::array_obj::JSArrayObject;
use crate::object::function::JSFunction;
use crate::object::object::JSObject;
use crate::object::object::SmallPropVec;
use crate::util::FxHashSet;
use crate::value::JSValue;
use std::env;

pub const TAG_OBJECT: u8 = 0;
pub const TAG_ARRAY: u8 = 1;
pub const TAG_FUNCTION: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeakObjectHandle {
    slot: u32,
    generation: u64,
}

const COLOR_WHITE: u8 = 0;
const COLOR_GRAY: u8 = 1;
const COLOR_BLACK: u8 = 2;
const MIN_HEAP_THRESHOLD: usize = 32 * 1024 * 1024;
const GC_GROWTH_FACTOR: usize = 12;
const NURSERY_PINNED_STREAK_LIMIT: usize = 10;

fn nursery_pinned_streak_limit() -> usize {
    env::var("V16_NURSERY_PINNED_STREAK_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.max(1))
        .unwrap_or(NURSERY_PINNED_STREAK_LIMIT)
}

#[derive(Clone, Copy, Debug, Default)]
struct GcDebugConfig {
    enabled: bool,
    dump_roots: bool,
    dump_live: bool,
    dump_dead: bool,
    dump_alloc: bool,
    limit: usize,
}

impl GcDebugConfig {
    fn from_env() -> Self {
        let raw = match env::var("V16_GCDUMP") {
            Ok(value) => value,
            Err(_) => return Self::default(),
        };

        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || normalized == "0"
            || normalized == "false"
            || normalized == "off"
        {
            return Self::default();
        }

        let limit = env::var("V16_GCDUMP_LIMIT")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .map(|value| value.max(1))
            .unwrap_or(16);

        let mut config = Self {
            enabled: true,
            limit,
            ..Self::default()
        };

        if normalized == "1" || normalized == "true" || normalized == "summary" {
            return config;
        }

        for token in normalized
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            match token {
                "1" | "true" | "summary" => {}
                "all" => {
                    config.dump_roots = true;
                    config.dump_live = true;
                    config.dump_dead = true;
                    config.dump_alloc = true;
                }
                "roots" => config.dump_roots = true,
                "live" => config.dump_live = true,
                "dead" => config.dump_dead = true,
                "alloc" => config.dump_alloc = true,
                _ => {}
            }
        }

        config
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct GcDebugStats {
    collections: usize,
    full_collections: usize,
    alloc_events: usize,
    minor_freed: usize,
    full_freed: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct GcSnapshot {
    objects: usize,
    nursery: usize,
    bytes: usize,
    threshold: usize,
}

#[derive(Clone, Copy)]
struct GcDebugEntry {
    handle: usize,
    ptr: usize,
    tag: u8,
    in_nursery: bool,
}

pub struct Nursery {
    backing: Vec<u64>,
    capacity: usize,
    offset: usize,
}

impl Nursery {
    pub fn new(capacity: usize) -> Self {
        let words = capacity.div_ceil(std::mem::size_of::<u64>());
        Self {
            backing: Vec::with_capacity(words),
            capacity,
            offset: 0,
        }
    }

    #[inline]
    pub fn alloc<T>(&mut self, value: T) -> Option<*mut T> {
        let layout = std::alloc::Layout::new::<T>();
        let align = layout.align();
        let size = layout.size();
        debug_assert!(align <= std::mem::align_of::<u64>());
        let aligned = (self.offset + align - 1) & !(align - 1);
        if aligned + size > self.capacity {
            return None;
        }
        let base = self.backing.as_mut_ptr() as *mut u8;
        // SAFETY: the capacity check above guarantees the aligned slot fits in the backing buffer, whose alignment is at least 8.
        let ptr = unsafe { base.add(aligned) as *mut T };
        // SAFETY: `ptr` addresses a fresh, properly aligned slot inside the nursery buffer that is not yet initialized.
        unsafe {
            std::ptr::write(ptr, value);
        }
        self.offset = aligned + size;
        Some(ptr)
    }

    #[inline]
    pub fn reset(&mut self) {
        self.offset = 0;
    }

    #[inline]
    pub fn can_fit<T>(&self) -> bool {
        let layout = std::alloc::Layout::new::<T>();
        let aligned = (self.offset + layout.align() - 1) & !(layout.align() - 1);
        aligned + layout.size() <= self.capacity
    }

    #[inline]
    pub fn usage(&self) -> usize {
        self.offset
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    pub fn contains<T>(&self, ptr: *const T) -> bool {
        let start = self.backing.as_ptr() as usize;
        let end = start + self.capacity;
        let p = ptr as usize;
        p >= start && p < end
    }
}

pub struct GcHeap {
    objects: Vec<*mut JSObject>,
    marks: Vec<u8>,
    tags: Vec<u8>,
    generations: Vec<u64>,
    free_list: Vec<usize>,
    gray_stack: Vec<usize>,
    total_allocated: usize,
    threshold: usize,

    live_count: usize,

    count_threshold: usize,
    nursery_capacity: usize,
    pub nursery: Nursery,
    nursery_indices: Vec<usize>,
    nursery_enabled: bool,
    nursery_pinned_streak: usize,
    last_minor_survivor_count: usize,
    pub minor_collections: usize,

    pub deleted_props_count: usize,

    nursery_pinned_streak_limit: usize,
    debug: GcDebugConfig,
    debug_stats: GcDebugStats,

    pub prop_pool: Vec<SmallPropVec>,

    pub extra_roots: Vec<JSValue>,

    pub gc_suppressed: bool,
}

impl GcHeap {
    pub fn new() -> Self {
        GcHeap {
            objects: Vec::new(),
            marks: Vec::new(),
            tags: Vec::new(),
            generations: Vec::new(),
            free_list: Vec::new(),
            gray_stack: Vec::new(),
            total_allocated: 0,
            threshold: MIN_HEAP_THRESHOLD,
            live_count: 0,
            count_threshold: 2_000_000,
            nursery_capacity: 32 * 1024 * 1024,
            nursery: Nursery::new(32 * 1024 * 1024),
            nursery_indices: Vec::new(),
            nursery_enabled: true,
            nursery_pinned_streak: 0,
            nursery_pinned_streak_limit: nursery_pinned_streak_limit(),
            last_minor_survivor_count: 0,
            minor_collections: 0,
            deleted_props_count: 0,

            debug: GcDebugConfig::from_env(),
            debug_stats: GcDebugStats::default(),
            prop_pool: Vec::new(),
            extra_roots: Vec::new(),
            gc_suppressed: false,
        }
    }

    fn track_tenured(&mut self, ptr: *mut JSObject, tag: u8) -> usize {
        let idx = if let Some(idx) = self.free_list.pop() {
            self.objects[idx] = ptr;
            self.marks[idx] = 0;
            self.tags[idx] = tag;
            self.generations[idx] = self.generations[idx].wrapping_add(1);
            idx
        } else {
            let idx = self.objects.len();
            self.objects.push(ptr);
            self.marks.push(0);
            self.tags.push(tag);
            self.generations.push(0);
            idx
        };

        JSValue::new_object(ptr).as_object_mut().gc_slot = idx as u32;
        let size = if tag == TAG_ARRAY {
            std::mem::size_of::<JSArrayObject>()
        } else if tag == TAG_FUNCTION {
            std::mem::size_of::<JSFunction>()
        } else {
            std::mem::size_of::<JSObject>()
        };
        self.total_allocated += size;
        self.live_count += 1;
        self.debug_alloc(idx, ptr as usize, tag, false);
        idx
    }

    fn track_nursery(&mut self, ptr: *mut JSObject, tag: u8) -> usize {
        let idx = if let Some(idx) = self.free_list.pop() {
            self.objects[idx] = ptr;
            self.marks[idx] = 0;
            self.tags[idx] = tag;
            self.generations[idx] = self.generations[idx].wrapping_add(1);
            idx
        } else {
            let idx = self.objects.len();
            self.objects.push(ptr);
            self.marks.push(0);
            self.tags.push(tag);
            self.generations.push(0);
            idx
        };

        JSValue::new_object(ptr).as_object_mut().gc_slot = idx as u32;
        self.nursery_indices.push(idx);
        self.debug_alloc(idx, ptr as usize, tag, true);
        idx
    }

    pub fn track(&mut self, ptr: *mut JSObject) -> usize {
        self.track_tenured(ptr, TAG_OBJECT)
    }

    pub fn track_array(&mut self, ptr: *mut JSArrayObject) -> usize {
        self.track_tenured(ptr.cast::<JSObject>(), TAG_ARRAY)
    }

    pub fn track_function(&mut self, ptr: *mut JSFunction) -> usize {
        self.track_tenured(ptr.cast::<JSObject>(), TAG_FUNCTION)
    }

    #[inline]
    pub fn take_prop_vec(&mut self) -> SmallPropVec {
        self.prop_pool.pop().unwrap_or_else(|| SmallPropVec::new())
    }

    pub fn alloc_object(&mut self) -> Option<*mut JSObject> {
        if !self.nursery_enabled {
            return None;
        }
        let props = self.take_prop_vec();
        self.nursery
            .alloc(JSObject::new_typed_from_pool(
                crate::object::object::ObjectType::Ordinary,
                props,
            ))
            .map(|ptr| {
                self.track_nursery(ptr, TAG_OBJECT);
                ptr
            })
    }

    pub fn alloc_object_with_value(&mut self, obj: JSObject) -> Option<*mut JSObject> {
        if !self.nursery_enabled || !self.nursery.can_fit::<JSObject>() {
            return None;
        }
        self.nursery.alloc(obj).map(|ptr| {
            self.track_nursery(ptr, TAG_OBJECT);
            ptr
        })
    }

    #[inline]
    pub fn nursery_enabled_and_can_fit_object(&self) -> bool {
        self.nursery_enabled && self.nursery.can_fit::<JSObject>()
    }

    #[inline]
    pub fn nursery_enabled_and_can_fit_array(&self) -> bool {
        self.nursery_enabled && self.nursery.can_fit::<JSArrayObject>()
    }

    pub fn alloc_array(&mut self) -> Option<*mut JSArrayObject> {
        if !self.nursery_enabled {
            return None;
        }
        self.nursery.alloc(JSArrayObject::new()).map(|ptr| {
            self.track_nursery(ptr.cast::<JSObject>(), TAG_ARRAY);
            ptr
        })
    }

    pub fn alloc_array_with_value(&mut self, arr: JSArrayObject) -> Option<*mut JSArrayObject> {
        if !self.nursery_enabled || !self.nursery.can_fit::<JSArrayObject>() {
            return None;
        }
        self.nursery.alloc(arr).map(|ptr| {
            self.track_nursery(ptr.cast::<JSObject>(), TAG_ARRAY);
            ptr
        })
    }

    pub fn nursery_is_full(&self) -> bool {
        !self.gc_suppressed && self.nursery_enabled && !self.nursery.can_fit::<JSObject>()
    }

    pub fn nursery_collectible_count(&self) -> usize {
        self.nursery_indices.len().saturating_sub(self.nursery_pinned_count())
    }

    pub fn nursery_pinned_count(&self) -> usize {
        // Count objects in nursery_indices that were NOT recently allocated
        // (they were promoted/survived previous GCs and persist in the nursery).
        // We estimate this as: if nursery is full and offset hasn't changed,
        // the remaining objects are pinned.
        if self.nursery.usage() >= self.nursery_capacity {
            self.nursery_indices.len()
        } else {
            0
        }
    }

    pub fn untrack(&mut self, ptr: *mut JSObject) {
        let idx = JSValue::new_object(ptr).as_object().gc_slot;
        if idx != u32::MAX {
            let idx = idx as usize;
            if self.objects.get(idx) == Some(&ptr) {
                self.objects[idx] = std::ptr::null_mut();
                self.marks[idx] = 0;
                self.tags[idx] = TAG_OBJECT;
                self.free_list.push(idx);
            }
        }
        self.total_allocated = self
            .total_allocated
            .saturating_sub(std::mem::size_of::<JSObject>());
        self.live_count = self.live_count.saturating_sub(1);
    }

    pub fn mark_value(&mut self, value: &JSValue) {
        if value.is_object() || value.is_function() {
            self.mark(value.get_ptr());
        }
    }

    fn trace_function(&mut self, func: &JSFunction) {
        func.base.for_each_property(|_atom, value, _attrs| {
            self.mark_value(&value);
        });

        func.base.for_each_accessor(|_atom, getter, setter| {
            if let Some(ref g) = getter {
                self.mark_value(g);
            }
            if let Some(ref s) = setter {
                self.mark_value(s);
            }
        });

        if let Some(items_to_mark) = func.base.get_private_accessors_for_gc() {
            for item in items_to_mark {
                self.mark_value(&item);
            }
        }

        func.base.for_each_private_field(|_atom, value| {
            self.mark_value(&value);
        });

        func.base.for_each_private_method(|_key, value| {
            self.mark_value(&value);
        });

        if let Some(proto_ptr) = func.base.prototype {
            self.mark(proto_ptr);
        }

        if !func.cached_prototype_ptr.is_null() {
            self.mark(func.cached_prototype_ptr);
        }

        if let Some(ref env) = func.with_env {
            self.mark_value(env);
        }

        self.mark_value(&func.param_env);

        self.mark_value(&func.realm_global);

        self.mark_value(&func.arrow_this);

        if let Some(ref cell) = func.arrow_this_cell {
            let (this_value, _) = cell.get();
            self.mark_value(&this_value);
        }

        self.mark_value(&func.arrow_super);
        self.mark_value(&func.arrow_new_target);
        if !func.arrow_enclosing_ctor_ptr.is_null() {
            let enclosing_ctor = JSValue::new_function(func.arrow_enclosing_ctor_ptr);
            self.mark_value(&enclosing_ctor);
        }
        self.mark_value(&func.home_object);
        self.mark_value(&func.arrow_super_base);

        for field in &func.instance_fields {
            self.mark_value(&field.key);
            self.mark_value(&field.private_owner);
            self.mark_value(&field.initializer);
        }

        if let Some(uv) = func.upvalues_ref() {
            for (_, upvalue) in &uv.upvalues {
                self.mark_value(upvalue);
            }
            for upvalue_cell in &uv.upvalue_slots {
                self.mark_value(&upvalue_cell.get());
            }
        }
    }

    #[inline(always)]
    pub fn is_array_ptr(&self, ptr: *mut JSObject) -> bool {
        JSValue::new_object(ptr).as_object().is_dense_array()
    }

    #[inline]
    pub fn mark(&mut self, ptr: *mut JSObject) -> bool {
        let idx = JSValue::new_object(ptr).as_object().gc_slot;
        if idx == u32::MAX {
            return false;
        }
        let idx = idx as usize;
        if self.objects.get(idx) != Some(&ptr) {
            return false;
        }
        if self.marks[idx] == COLOR_WHITE {
            self.marks[idx] = COLOR_GRAY;
            self.gray_stack.push(idx);
        }
        true
    }

    fn trace_object(&mut self, idx: usize, ptr: *mut JSObject) {
        if ptr.is_null() {
            return;
        }
        self.marks[idx] = COLOR_BLACK;
        let tag = self.tags[idx];

        if tag == TAG_ARRAY {
            // SAFETY: objects tracked with TAG_ARRAY are allocated as JSArrayObject with the JSObject header first.
            let arr = unsafe { &*ptr.cast::<JSArrayObject>() };
            arr.for_each_element(|value| {
                self.mark_value(value);
            });
            arr.header.for_each_property(|_atom, value, _attrs| {
                self.mark_value(&value);
            });
            arr.header.for_each_accessor(|_atom, getter, setter| {
                if let Some(ref g) = getter {
                    self.mark_value(g);
                }
                if let Some(ref s) = setter {
                    self.mark_value(s);
                }
            });

            if let Some(items_to_mark) = arr.header.get_private_accessors_for_gc() {
                for item in items_to_mark {
                    self.mark_value(&item);
                }
            }
            arr.header.for_each_private_field(|_atom, value| {
                self.mark_value(&value);
            });
            arr.header.for_each_private_method(|_key, value| {
                self.mark_value(&value);
            });
            if let Some(proto_ptr) = arr.header.prototype {
                self.mark(proto_ptr);
            }
        } else if tag == TAG_FUNCTION {
            let func_value = JSValue::new_function(ptr.cast::<JSFunction>());
            self.trace_function(func_value.as_function());
        } else {
            let obj = JSValue::new_object(ptr);
            let obj = obj.as_object();
            obj.for_each_property(|_atom, value, _attrs| {
                self.mark_value(&value);
            });
            obj.for_each_accessor(|_atom, getter, setter| {
                if let Some(ref g) = getter {
                    self.mark_value(g);
                }
                if let Some(ref s) = setter {
                    self.mark_value(s);
                }
            });

            if let Some(items_to_mark) = obj.get_private_accessors_for_gc() {
                for item in items_to_mark {
                    self.mark_value(&item);
                }
            }
            obj.for_each_private_field(|_atom, value| {
                self.mark_value(&value);
            });
            obj.for_each_private_method(|_key, value| {
                self.mark_value(&value);
            });
            obj.for_each_array_element(|value| {
                self.mark_value(value);
            });
            if let Some(entries) = obj.argument_entries() {
                for entry in entries {
                    if !entry.present {
                        continue;
                    }
                    if let Some(cell) = entry.cell.as_ref() {
                        self.mark_value(&cell.get());
                    } else {
                        self.mark_value(&entry.value);
                    }
                    if entry.is_accessor() {
                        if let Some(getter) = entry.accessor.get {
                            self.mark_value(&getter);
                        }
                        if let Some(setter) = entry.accessor.set {
                            self.mark_value(&setter);
                        }
                    }
                }
            }
            if let Some(proto_ptr) = obj.prototype {
                self.mark(proto_ptr);
            }

            if let Some(target) = obj.proxy_target() {
                self.mark_value(&target);
            }
            if let Some(handler) = obj.proxy_handler() {
                self.mark_value(&handler);
            }

            if let Some(state) = obj.get_generator_state() {
                for v in &state.snapshot {
                    self.mark_value(v);
                }
                if let Some(func_ptr) = state.func_ptr {
                    let func = JSValue::new_function(func_ptr);
                    self.mark_value(&func);
                }
                self.mark_value(&state.param_env);
            }

            if let Some(state) = obj.get_async_state() {
                for v in &state.snapshot {
                    self.mark_value(v);
                }
                if let Some(func_ptr) = state.func_ptr {
                    let func = JSValue::new_function(func_ptr);
                    self.mark_value(&func);
                }
                if let Some(exc) = &state.pending_throw {
                    self.mark_value(exc);
                }
                self.mark_value(&state.resume_value);
                self.mark_value(&state.param_env);
                if let Some((_, value)) = &state.pending_finally_return {
                    self.mark_value(value);
                }
                if let Some((_, value)) = &state.pending_finally_rethrow {
                    self.mark_value(value);
                }
            }

            if let Some(state) = obj.get_async_generator_state() {
                self.mark_value(&state.current_promise);
                for request in &state.queue {
                    self.mark_value(&request.value);
                    self.mark_value(&request.promise);
                }
            }
        }
    }

    pub fn mark_roots(&mut self, roots: &[JSValue]) {
        for value in roots {
            self.mark_value(value);
        }
    }

    #[inline(always)]
    unsafe fn drop_tracked(ptr: *mut JSObject, tag: u8) {
        if tag == TAG_ARRAY {
            // SAFETY: `ptr` was allocated as a boxed JSArrayObject and the collector owns the allocation.
            unsafe {
                let _ = Box::from_raw(ptr.cast::<JSArrayObject>());
            }
        } else if tag == TAG_FUNCTION {
            // SAFETY: `ptr` was allocated as a boxed JSFunction and the collector owns the allocation.
            unsafe {
                let _ = Box::from_raw(ptr.cast::<JSFunction>());
            }
        } else {
            // SAFETY: `ptr` was allocated as a boxed JSObject and the collector owns the allocation.
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    }

    #[inline(always)]
    unsafe fn drop_in_place_tracked(ptr: *mut JSObject, tag: u8) {
        if tag == TAG_ARRAY {
            // SAFETY: `ptr` points at a live JSArrayObject that the collector has removed from its tables.
            unsafe {
                std::ptr::drop_in_place(ptr.cast::<JSArrayObject>());
            }
        } else if tag == TAG_FUNCTION {
            // SAFETY: `ptr` points at a live JSFunction that the collector has removed from its tables.
            unsafe {
                std::ptr::drop_in_place(ptr.cast::<JSFunction>());
            }
        } else {
            // SAFETY: `ptr` points at a live JSObject that the collector has removed from its tables.
            unsafe {
                std::ptr::drop_in_place(ptr);
            }
        }
    }

    pub(crate) fn snapshot(&mut self) -> GcSnapshot {
        GcSnapshot {
            objects: self.object_count(),
            nursery: self.nursery_indices.len(),
            bytes: self.total_allocated,
            threshold: self.threshold,
        }
    }

    fn tag_name(tag: u8) -> &'static str {
        match tag {
            TAG_ARRAY => "array",
            TAG_OBJECT => "object",
            _ => "unknown",
        }
    }

    fn debug_alloc(&mut self, handle: usize, ptr: usize, tag: u8, in_nursery: bool) {
        self.debug_stats.alloc_events += 1;
        if self.debug.enabled && self.debug.dump_alloc {
            eprintln!(
                "[gcdump] alloc#{} handle={} ptr={:#x} tag={} nursery={}",
                self.debug_stats.alloc_events,
                handle,
                ptr,
                Self::tag_name(tag),
                in_nursery
            );
        }
    }

    fn describe_value(&self, value: &JSValue) -> String {
        if value.is_undefined() {
            "undefined".to_string()
        } else if value.is_null() {
            "null".to_string()
        } else if value.is_bool() {
            format!("bool({})", value.get_bool())
        } else if value.is_int() {
            format!("int({})", value.get_int())
        } else if value.is_float() {
            format!("float({})", value.get_float())
        } else if value.is_string() {
            format!("string(atom={})", value.get_atom().0)
        } else if value.is_symbol() {
            format!("symbol(atom={})", value.get_atom().0)
        } else if value.is_bigint() {
            format!("bigint(ptr={:#x})", value.get_ptr() as usize)
        } else if value.is_function() {
            let ptr = value.get_ptr();
            let handle = value.as_object().gc_slot;
            if handle != u32::MAX {
                format!("function(handle={} ptr={:#x})", handle, ptr as usize)
            } else {
                format!("function(ptr={:#x})", ptr as usize)
            }
        } else if value.is_object() {
            let ptr = value.get_ptr();
            let handle = value.as_object().gc_slot;
            if handle != u32::MAX {
                format!("object(handle={} ptr={:#x})", handle, ptr as usize)
            } else {
                format!("object(ptr={:#x})", ptr as usize)
            }
        } else if value.is_tdz() {
            "tdz".to_string()
        } else {
            format!("raw({:#x})", value.raw_bits())
        }
    }

    fn collect_entries_from_indices(&self, indices: &[usize]) -> Vec<GcDebugEntry> {
        indices
            .iter()
            .take(self.debug.limit)
            .filter_map(|&idx| {
                let ptr = self.objects.get(idx).copied().unwrap_or(std::ptr::null_mut());
                if ptr.is_null() {
                    return None;
                }
                Some(GcDebugEntry {
                    handle: idx,
                    ptr: ptr as usize,
                    tag: self.tags[idx],
                    in_nursery: self.nursery.contains(ptr as *const u8),
                })
            })
            .collect()
    }

    fn collect_live_entries(&self) -> Vec<GcDebugEntry> {
        let mut entries = Vec::new();
        for (idx, &ptr) in self.objects.iter().enumerate() {
            if ptr.is_null() {
                continue;
            }
            entries.push(GcDebugEntry {
                handle: idx,
                ptr: ptr as usize,
                tag: self.tags[idx],
                in_nursery: self.nursery.contains(ptr as *const u8),
            });
            if entries.len() >= self.debug.limit {
                break;
            }
        }
        entries
    }

    fn debug_collection_start(&mut self, kind: &str, roots: &[JSValue]) -> usize {
        self.debug_stats.collections += 1;
        if kind == "full" {
            self.debug_stats.full_collections += 1;
        }
        let seq = self.debug_stats.collections;
        if !self.debug.enabled {
            return seq;
        }

        let snapshot = self.snapshot();
        eprintln!(
            "[gcdump] gc#{} {} start roots={} objects={} nursery={} bytes={} threshold={} nursery_enabled={} minor_count={} full_count={}",
            seq,
            kind,
            roots.len(),
            snapshot.objects,
            snapshot.nursery,
            snapshot.bytes,
            snapshot.threshold,
            self.nursery_enabled,
            self.minor_collections,
            self.debug_stats.full_collections
        );

        if self.debug.dump_roots {
            for (index, value) in roots.iter().take(self.debug.limit).enumerate() {
                eprintln!(
                    "[gcdump] gc#{} {} root[{}]={}",
                    seq,
                    kind,
                    index,
                    self.describe_value(value)
                );
            }
            if roots.len() > self.debug.limit {
                eprintln!(
                    "[gcdump] gc#{} {} roots omitted={}",
                    seq,
                    kind,
                    roots.len() - self.debug.limit
                );
            }
        }

        seq
    }

    fn debug_dump_entries(&self, seq: usize, kind: &str, label: &str, entries: &[GcDebugEntry]) {
        if !self.debug.enabled || entries.is_empty() {
            return;
        }
        for entry in entries {
            eprintln!(
                "[gcdump] gc#{} {} {} handle={} ptr={:#x} tag={} nursery={}",
                seq,
                kind,
                label,
                entry.handle,
                entry.ptr,
                Self::tag_name(entry.tag),
                entry.in_nursery
            );
        }
    }

    fn debug_collection_end(
        &mut self,
        seq: usize,
        kind: &str,
        before: GcSnapshot,
        freed: usize,
        dead_entries: &[GcDebugEntry],
    ) {
        if kind == "minor" {
            self.debug_stats.minor_freed += freed;
        } else {
            self.debug_stats.full_freed += freed;
        }

        if !self.debug.enabled {
            return;
        }

        if self.debug.dump_dead {
            self.debug_dump_entries(seq, kind, "dead", dead_entries);
            if freed > dead_entries.len() {
                eprintln!(
                    "[gcdump] gc#{} {} dead omitted={}",
                    seq,
                    kind,
                    freed - dead_entries.len()
                );
            }
        }

        if self.debug.dump_live {
            let live_entries = self.collect_live_entries();
            self.debug_dump_entries(seq, kind, "live", &live_entries);
            let live_total = self.object_count();
            if live_total > live_entries.len() {
                eprintln!(
                    "[gcdump] gc#{} {} live omitted={}",
                    seq,
                    kind,
                    live_total - live_entries.len()
                );
            }
        }

        let after = self.snapshot();
        eprintln!(
            "[gcdump] gc#{} {} end freed={} objects={}=>{} nursery={}=>{} bytes={}=>{} threshold={} nursery_enabled={} minor_freed_total={} full_freed_total={}",
            seq,
            kind,
            freed,
            before.objects,
            after.objects,
            before.nursery,
            after.nursery,
            before.bytes,
            after.bytes,
            after.threshold,
            self.nursery_enabled,
            self.debug_stats.minor_freed,
            self.debug_stats.full_freed
        );
    }

    fn refresh_threshold_after_full_gc(&mut self) {
        self.threshold = (self.total_allocated.saturating_mul(GC_GROWTH_FACTOR))
            .max(MIN_HEAP_THRESHOLD);
        self.count_threshold = (self.live_count.saturating_mul(GC_GROWTH_FACTOR))
            .max(2_000_000);
    }

    fn note_minor_gc_result(&mut self, freed: usize) {
        let survivor_count = self.nursery_indices.len();
        if survivor_count == 0 {
            self.nursery_enabled = true;
            self.nursery_pinned_streak = 0;
            self.last_minor_survivor_count = 0;
            return;
        }

        if freed == 0 && survivor_count == self.last_minor_survivor_count {
            self.nursery_pinned_streak += 1;
            if self.nursery_pinned_streak >= self.nursery_pinned_streak_limit {
                self.nursery_enabled = false;
            }
        } else {
            self.nursery_pinned_streak = 0;
        }

        self.last_minor_survivor_count = survivor_count;
    }

    pub fn minor_gc(&mut self, roots: &[JSValue]) -> usize {
        let before = self.snapshot();
        let seq = self.debug_collection_start("minor", roots);

        for m in self.marks.iter_mut() {
            *m = 0;
        }
        self.gray_stack.clear();

        self.mark_roots(roots);

        while let Some(idx) = self.gray_stack.pop() {
            let ptr = self.objects[idx];
            self.trace_object(idx, ptr);
        }

        let mut dead = Vec::new();
        let mut new_nursery_indices = Vec::new();
        for &idx in &self.nursery_indices {
            if self.marks[idx] == COLOR_WHITE {
                dead.push(idx);
            } else {
                new_nursery_indices.push(idx);
            }
        }

        let dead_entries = if self.debug.enabled && self.debug.dump_dead {
            self.collect_entries_from_indices(&dead)
        } else {
            Vec::new()
        };
        let dead_ptrs: FxHashSet<*mut JSObject> =
            dead.iter().map(|&idx| self.objects[idx]).collect();

        for &idx in &dead {
            let ptr = self.objects[idx];
            let tag = self.tags[idx];
            let mut pooled_props = None;
            if tag == TAG_OBJECT {
                let object_value = JSValue::new_object(ptr);
                let mut props = object_value.as_object_mut().take_props();
                props.clear();
                pooled_props = Some(props);
            }
            // SAFETY: this object was classified as dead and is owned solely by the collector, so it can be dropped in place.
            unsafe {
                if tag == TAG_OBJECT {
                    std::ptr::drop_in_place(ptr);
                } else {
                    Self::drop_in_place_tracked(ptr, tag);
                }
            }
            if let Some(props) = pooled_props {
                if props.capacity() <= 8 {
                    self.prop_pool.push(props);
                }
            }
            self.objects[idx] = std::ptr::null_mut();
            self.marks[idx] = 0;
            self.tags[idx] = TAG_OBJECT;
            self.free_list.push(idx);
        }
        self.gray_stack.clear();
        self.clean_stale_properties_on_nursery_survivors(&dead_ptrs);

        self.nursery_indices = new_nursery_indices;
        if self.nursery_indices.is_empty() {
            self.nursery.reset();
            self.nursery_enabled = true;
        }
        self.gray_stack.clear();
        self.minor_collections += 1;
        let freed = dead.len();
        self.note_minor_gc_result(freed);
        self.debug_collection_end(seq, "minor", before, freed, &dead_entries);
        freed
    }

    pub fn run_gc(&mut self, roots: &[JSValue]) -> usize {
        let before = self.snapshot();
        let seq = self.debug_collection_start("full", roots);

        for m in self.marks.iter_mut() {
            *m = 0;
        }
        self.gray_stack.clear();

        self.mark_roots(roots);

        while let Some(idx) = self.gray_stack.pop() {
            let ptr = self.objects[idx];
            self.trace_object(idx, ptr);
        }

        let mut to_free: Vec<usize> = Vec::new();
        for (idx, &ptr) in self.objects.iter().enumerate() {
            if !ptr.is_null() && self.marks[idx] == COLOR_WHITE {
                to_free.push(idx);
            }
        }

        let dead_entries = if self.debug.enabled && self.debug.dump_dead {
            self.collect_entries_from_indices(&to_free)
        } else {
            Vec::new()
        };

        let freed = to_free.len();
        let mut live_nursery_indices = Vec::new();
        for idx in to_free {
            let ptr = self.objects[idx];
            let tag = self.tags[idx];
            let in_nursery = self.nursery.contains(ptr as *const u8);
            let size = if tag == TAG_ARRAY {
                std::mem::size_of::<JSArrayObject>()
            } else if tag == TAG_FUNCTION {
                std::mem::size_of::<JSFunction>()
            } else {
                std::mem::size_of::<JSObject>()
            };
            let mut pooled_props = None;
            if tag == TAG_OBJECT {
                let object_value = JSValue::new_object(ptr);
                let mut props = object_value.as_object_mut().take_props();
                props.clear();
                pooled_props = Some(props);
            }
            // SAFETY: unreachable objects are owned solely by the collector, and the drop path matches the allocation kind recorded by `tag`.
            unsafe {
                if tag == TAG_OBJECT {
                    if in_nursery {
                        std::ptr::drop_in_place(ptr);
                    } else {
                        let _ = Box::from_raw(ptr);
                    }
                } else if in_nursery {
                    Self::drop_in_place_tracked(ptr, tag);
                } else {
                    Self::drop_tracked(ptr, tag);
                }
            }
            if let Some(props) = pooled_props {
                if props.capacity() <= 8 {
                    self.prop_pool.push(props);
                }
            }
            self.objects[idx] = std::ptr::null_mut();
            self.marks[idx] = 0;
            self.tags[idx] = TAG_OBJECT;
            self.free_list.push(idx);
            self.total_allocated = self.total_allocated.saturating_sub(size);
            self.live_count = self.live_count.saturating_sub(1);
        }

        for &idx in &self.nursery_indices {
            if ! self.objects[idx].is_null() {
                live_nursery_indices.push(idx);
            }
        }

        self.nursery_indices = live_nursery_indices;
        if self.nursery_indices.is_empty() {
            self.nursery.reset();
            self.nursery_enabled = true;
        } else {
            self.nursery_enabled = false;
        }
        self.gray_stack.clear();
        self.refresh_threshold_after_full_gc();
        self.debug_collection_end(seq, "full", before, freed, &dead_entries);
        freed
    }

    pub fn start_incremental_mark(&mut self, roots: &[JSValue]) {
        for m in self.marks.iter_mut() {
            *m = COLOR_WHITE;
        }
        self.gray_stack.clear();
        self.mark_roots(roots);
    }

    pub fn incremental_mark_step(&mut self, budget: usize) -> bool {
        for _ in 0..budget {
            if let Some(idx) = self.gray_stack.pop() {
                if self.marks[idx] == COLOR_GRAY {
                    let ptr = self.objects[idx];
                    self.trace_object(idx, ptr);
                }
            } else {
                return false;
            }
        }
        !self.gray_stack.is_empty()
    }

    pub fn write_barrier(&mut self, obj_ptr: *mut JSObject, new_value: &JSValue) {
        let idx = JSValue::new_object(obj_ptr).as_object().gc_slot;
        if idx != u32::MAX {
            let idx = idx as usize;
            if self.marks[idx] == COLOR_BLACK {
                self.mark_value(new_value);
            }
        }
    }

    pub fn finish_incremental_sweep(&mut self) -> usize {
        while let Some(idx) = self.gray_stack.pop() {
            if self.marks[idx] == COLOR_GRAY {
                let ptr = self.objects[idx];
                self.trace_object(idx, ptr);
            }
        }

        let mut to_free: Vec<usize> = Vec::new();
        for (idx, &ptr) in self.objects.iter().enumerate() {
            if !ptr.is_null() && self.marks[idx] == COLOR_WHITE {
                to_free.push(idx);
            }
        }

        let freed = to_free.len();
        for idx in to_free {
            let ptr = self.objects[idx];
            let tag = self.tags[idx];
            let in_nursery = self.nursery.contains(ptr as *const u8);
            // SAFETY: unreachable objects are owned solely by the collector, and the drop path matches nursery versus boxed allocation.
            unsafe {
                if in_nursery {
                    Self::drop_in_place_tracked(ptr, tag);
                } else {
                    Self::drop_tracked(ptr, tag);
                }
            }
            self.objects[idx] = std::ptr::null_mut();
            self.marks[idx] = 0;
            self.tags[idx] = TAG_OBJECT;
            self.free_list.push(idx);
            self.live_count = self.live_count.saturating_sub(1);
        }

        self.nursery_indices.clear();
        self.nursery.reset();
        self.gray_stack.clear();
        self.total_allocated = self
            .total_allocated
            .saturating_sub(freed * std::mem::size_of::<JSObject>());
        freed
    }

    pub fn for_each_live_object(&self, mut f: impl FnMut(*mut JSObject, u8)) {
        for (idx, &ptr) in self.objects.iter().enumerate() {
            if !ptr.is_null() && self.marks[idx] != COLOR_WHITE {
                f(ptr, self.tags[idx]);
            }
        }
    }

    pub fn object_count(&self) -> usize {
        self.objects.iter().filter(|&&p| !p.is_null()).count()
    }

    #[inline]
    pub fn tracked_ptr_at(&self, idx: usize) -> Option<*mut JSObject> {
        self.objects.get(idx).copied().filter(|&p| !p.is_null())
    }

    #[inline]
    pub fn weak_handle(&self, object: &JSObject) -> Option<WeakObjectHandle> {
        let slot = object.gc_slot;
        if slot == u32::MAX {
            return None;
        }
        let idx = slot as usize;
        if self.objects.get(idx) != Some(&(object as *const JSObject as *mut JSObject)) {
            return None;
        }
        Some(WeakObjectHandle {
            slot,
            generation: *self.generations.get(idx)?,
        })
    }

    #[inline]
    pub fn resolve_weak(&self, handle: WeakObjectHandle) -> Option<JSValue> {
        let idx = handle.slot as usize;
        let ptr = *self.objects.get(idx)?;
        if ptr.is_null() || *self.generations.get(idx)? != handle.generation {
            return None;
        }
        if *self.tags.get(idx)? == TAG_FUNCTION {
            return Some(JSValue::new_function(ptr.cast::<JSFunction>()));
        }
        Some(JSValue::new_object(ptr))
    }

    fn clean_stale_properties_on_nursery_survivors(&mut self, dead: &FxHashSet<*mut JSObject>) {
        for &idx in &self.nursery_indices {
            if self.marks[idx] == COLOR_WHITE {
                continue;
            }
            let tag = self.tags[idx];
            match tag {
                TAG_OBJECT | TAG_FUNCTION | TAG_ARRAY => {
                    JSValue::new_object(self.objects[idx])
                        .as_object_mut()
                        .clean_stale_properties(dead);
                }
                _ => {}
            }
        }
    }

    pub fn total_size(&self) -> usize {
        self.total_allocated
    }

    pub fn should_collect(&self) -> bool {
        if self.gc_suppressed {
            return false;
        }
        self.total_allocated > self.threshold || self.live_count > self.count_threshold
    }

    pub fn set_threshold(&mut self, threshold: usize) {
        self.threshold = threshold;
    }
}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::object::JSObject;
    use crate::runtime::atom::Atom;
    use crate::value::JSValue;

    #[test]
    fn test_gc_track_untrack() {
        let mut heap = GcHeap::new();
        let obj = JSObject::new();
        let ptr = Box::into_raw(Box::new(obj));

        heap.track(ptr);
        assert_eq!(heap.object_count(), 1);

        heap.untrack(ptr);
        assert_eq!(heap.object_count(), 0);

        // SAFETY: the pointer came from `Box::into_raw` above and is reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }

    #[test]
    fn test_gc_mark_value() {
        let mut heap = GcHeap::new();
        let obj = JSObject::new();
        let ptr = Box::into_raw(Box::new(obj));

        heap.track(ptr);

        let value = JSValue::new_object(ptr);
        heap.mark_value(&value);

        // SAFETY: `ptr` refers to the live boxed object created above.
        let idx = unsafe { (*(ptr as *const JSObject)).gc_slot } as usize;
        assert_eq!(heap.marks[idx], 1);

        // SAFETY: the pointer came from `Box::into_raw` above and is reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }

    #[test]
    fn test_gc_run_gc_frees_unreachable() {
        let mut heap = GcHeap::new();

        let reachable_obj = JSObject::new();
        let reachable_ptr = Box::into_raw(Box::new(reachable_obj));
        heap.track(reachable_ptr);

        let unreachable_obj = JSObject::new();
        let unreachable_ptr = Box::into_raw(Box::new(unreachable_obj));
        heap.track(unreachable_ptr);

        assert_eq!(heap.object_count(), 2);

        let roots = vec![JSValue::new_object(reachable_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 1);
        assert_eq!(heap.object_count(), 1);
        assert!(heap.objects.contains(&reachable_ptr));
        assert!(!heap.objects.contains(&unreachable_ptr));

        // SAFETY: the collector kept the reachable object alive and this test reclaims it exactly once.
        unsafe {
            let _ = Box::from_raw(reachable_ptr);
        }
    }

    #[test]
    fn test_gc_trace_object_properties() {
        let mut heap = GcHeap::new();

        let child_obj = JSObject::new();
        let child_ptr = Box::into_raw(Box::new(child_obj));
        heap.track(child_ptr);

        let mut parent_obj = JSObject::new();
        parent_obj.set(Atom(0), JSValue::new_object(child_ptr));
        let parent_ptr = Box::into_raw(Box::new(parent_obj));
        heap.track(parent_ptr);

        let roots = vec![JSValue::new_object(parent_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(parent_ptr);
            let _ = Box::from_raw(child_ptr);
        }
    }

    #[test]
    fn test_gc_trace_function_realm_global() {
        let mut heap = GcHeap::new();

        let realm_global = JSObject::new();
        let realm_global_ptr = Box::into_raw(Box::new(realm_global));
        heap.track(realm_global_ptr);

        let mut func = JSFunction::new_builtin(Atom(0), 0);
        func.realm_global = JSValue::new_object(realm_global_ptr);
        let func_ptr = Box::into_raw(Box::new(func));
        heap.track_function(func_ptr);

        let roots = vec![JSValue::new_function(func_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(func_ptr);
            let _ = Box::from_raw(realm_global_ptr);
        }
    }

    #[test]
    fn test_gc_trace_function_arrow_this_cell() {
        let mut heap = GcHeap::new();

        let this_obj = JSObject::new();
        let this_ptr = Box::into_raw(Box::new(this_obj));
        heap.track(this_ptr);

        let mut func = JSFunction::new_builtin(Atom(0), 0);
        func.arrow_this_cell = Some(std::rc::Rc::new(std::cell::Cell::new((
            JSValue::new_object(this_ptr),
            true,
        ))));
        let func_ptr = Box::into_raw(Box::new(func));
        heap.track_function(func_ptr);

        let roots = vec![JSValue::new_function(func_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(func_ptr);
            let _ = Box::from_raw(this_ptr);
        }
    }

    #[test]
    fn test_gc_trace_function_arrow_enclosing_ctor() {
        let mut heap = GcHeap::new();

        let ctor = JSFunction::new_builtin(Atom(0), 0);
        let ctor_ptr = Box::into_raw(Box::new(ctor));
        heap.track_function(ctor_ptr);

        let mut arrow = JSFunction::new_builtin(Atom(0), 0);
        arrow.arrow_enclosing_ctor_ptr = ctor_ptr;
        let arrow_ptr = Box::into_raw(Box::new(arrow));
        heap.track_function(arrow_ptr);

        let roots = vec![JSValue::new_function(arrow_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(arrow_ptr);
            let _ = Box::from_raw(ctor_ptr);
        }
    }

    #[test]
    fn test_gc_trace_function_cached_prototype() {
        let mut heap = GcHeap::new();

        let proto_obj = JSObject::new();
        let proto_ptr = Box::into_raw(Box::new(proto_obj));
        heap.track(proto_ptr);

        let mut func = JSFunction::new_builtin(Atom(0), 0);
        func.cached_prototype_ptr = proto_ptr;
        let func_ptr = Box::into_raw(Box::new(func));
        heap.track_function(func_ptr);

        let roots = vec![JSValue::new_function(func_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(func_ptr);
            let _ = Box::from_raw(proto_ptr);
        }
    }

    #[test]
    fn test_gc_trace_generator_state_function() {
        let mut heap = GcHeap::new();

        let func = JSFunction::new_builtin(Atom(0), 0);
        let func_ptr = Box::into_raw(Box::new(func));
        heap.track_function(func_ptr);

        let state = crate::object::object::GeneratorState {
            bytecode: Box::new(crate::compiler::opcode::Bytecode::new()),
            snapshot: Vec::new(),
            pc: 0,
            done: false,
            send_reg: None,
            argc: 0,
            saved_args: Vec::new(),
            func_ptr: Some(func_ptr),
            handlers: Vec::new(),
            yield_is_result: false,
            upvalue_cells: Vec::new(),
            param_env: JSValue::undefined(),
            param_env_own: false,
        };
        let mut gen_obj = JSObject::new();
        gen_obj.set_is_generator(true);
        gen_obj.set_generator_state(state);
        let gen_ptr = Box::into_raw(Box::new(gen_obj));
        heap.track(gen_ptr);

        let roots = vec![JSValue::new_object(gen_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(gen_ptr);
            let _ = Box::from_raw(func_ptr);
        }
    }

    #[test]
    fn test_gc_trace_async_state_function() {
        let mut heap = GcHeap::new();

        let func = JSFunction::new_builtin(Atom(0), 0);
        let func_ptr = Box::into_raw(Box::new(func));
        heap.track_function(func_ptr);

        let state = crate::object::object::AsyncState {
            bytecode: Box::new(crate::compiler::opcode::Bytecode::new()),
            snapshot: Vec::new(),
            pc: 0,
            done: false,
            send_reg: None,
            saved_args: Vec::new(),
            func_ptr: Some(func_ptr),
            handlers: Vec::new(),
            pending_throw: None,
            resume_kind: 0,
            resume_value: JSValue::undefined(),
            yield_is_result: false,
            pending_yield: false,
            pending_finally_return: None,
            pending_finally_rethrow: None,
            pending_finally_jump: None,
            pending_finally_depth: 0,
            pending_close_state: None,
            upvalue_cells: Vec::new(),
            param_env: JSValue::undefined(),
            param_env_own: false,
        };
        let mut async_obj = JSObject::new();
        async_obj.set_async_state(state);
        let async_ptr = Box::into_raw(Box::new(async_obj));
        heap.track(async_ptr);

        let roots = vec![JSValue::new_object(async_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(async_ptr);
            let _ = Box::from_raw(func_ptr);
        }
    }

    #[test]
    fn test_gc_reachable_through_nested_object() {
        let mut heap = GcHeap::new();

        let grandchild = JSObject::new();
        let grandchild_ptr = Box::into_raw(Box::new(grandchild));
        heap.track(grandchild_ptr);

        let mut child = JSObject::new();
        child.set(Atom(0), JSValue::new_object(grandchild_ptr));
        let child_ptr = Box::into_raw(Box::new(child));
        heap.track(child_ptr);

        let mut parent = JSObject::new();
        parent.set(Atom(0), JSValue::new_object(child_ptr));
        let parent_ptr = Box::into_raw(Box::new(parent));
        heap.track(parent_ptr);

        let roots = vec![JSValue::new_object(parent_ptr)];

        let freed = heap.run_gc(&roots);

        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 3);

        // SAFETY: all pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(parent_ptr);
            let _ = Box::from_raw(child_ptr);
            let _ = Box::from_raw(grandchild_ptr);
        }
    }

    #[test]
    fn test_gc_run_gc_compacts_live_object_props() {
        use crate::object::shape::ShapeCache;
        let mut heap = GcHeap::new();
        let mut cache = ShapeCache::new();

        let mut obj = JSObject::new();
        let a = Atom(1);
        let b = Atom(2);
        let c = Atom(3);
        obj.set(a, JSValue::new_int(10));
        obj.set(b, JSValue::new_int(20));
        obj.set(c, JSValue::new_int(30));
        obj.delete(b);

        assert_eq!(obj.get(a).unwrap().get_int(), 10);
        assert!(obj.get(b).is_none());
        assert_eq!(obj.get(c).unwrap().get_int(), 30);

        let ptr = Box::into_raw(Box::new(obj));
        heap.track(ptr);

        let roots = vec![JSValue::new_object(ptr)];
        let freed = heap.run_gc(&roots);
        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 1);

        // SAFETY: `heap` visits only live tracked objects; each callback pointer stays valid for the call.
        heap.for_each_live_object(|live_ptr, tag| unsafe {
            if tag == TAG_OBJECT {
                let obj_ref = &mut *live_ptr;
                obj_ref.compact_props(&mut cache);
            }
        });

        // SAFETY: `ptr` is a live tracked object that the collector kept reachable above.
        let obj_ref = unsafe { &*(ptr as *const JSObject) };
        assert_eq!(obj_ref.get(a).unwrap().get_int(), 10);
        assert!(obj_ref.get(b).is_none());
        assert_eq!(obj_ref.get(c).unwrap().get_int(), 30);

        let mut count = 0;
        obj_ref.for_each_property(|_, _, _| count += 1);
        assert_eq!(count, 2);

        // SAFETY: the pointer came from `Box::into_raw` above and is reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }

    #[test]
    fn test_nursery_bump_alloc_basic() {
        let mut nursery = Nursery::new(1024);
        let ptr1 = nursery
            .alloc(JSObject::new())
            .expect("first alloc should succeed");
        let ptr2 = nursery
            .alloc(JSObject::new())
            .expect("second alloc should succeed");
        assert!(!ptr1.is_null());
        assert!(!ptr2.is_null());
        assert_ne!(ptr1, ptr2);

        assert!(nursery.usage() >= std::mem::size_of::<JSObject>() * 2);
    }

    #[test]
    fn test_nursery_alloc_respects_capacity() {
        let capacity = std::mem::size_of::<JSObject>() * 4;
        let mut nursery = Nursery::new(capacity);

        let _ = nursery.alloc(JSObject::new()).expect("should fit");

        let mut count = 1;
        while let Some(_ptr) = nursery.alloc(JSObject::new()) {
            count += 1;
            if count > 100 {
                panic!("nursery should have filled up by now");
            }
        }
        assert!(count >= 1);
    }

    #[test]
    fn test_gc_heap_alloc_object_in_nursery() {
        let mut heap = GcHeap::new();
        let ptr = heap.alloc_object().expect("nursery alloc should succeed");
        assert_eq!(heap.object_count(), 1);
        assert_eq!(heap.nursery_indices.len(), 1);

        // SAFETY: `ptr` is a live nursery object owned by `heap`.
        unsafe {
            (*ptr).set(Atom(1), JSValue::new_int(42));
            assert_eq!((*ptr).get(Atom(1)).unwrap().get_int(), 42);
        }

        let freed = heap.minor_gc(&[]);
        assert_eq!(freed, 1);
        assert_eq!(heap.object_count(), 0);
        assert!(heap.nursery_indices.is_empty());
    }

    #[test]
    fn test_weak_handle_resolves_only_live_objects() {
        let mut heap = GcHeap::new();
        let ptr = heap.alloc_object().expect("nursery alloc should succeed");
        let handle = heap
            .weak_handle(JSValue::new_object(ptr).as_object())
            .expect("nursery object should have a weak handle");

        assert_eq!(
            heap.resolve_weak(handle).map(|value| value.get_ptr()),
            Some(ptr)
        );

        let roots = [JSValue::new_object(ptr)];
        heap.minor_gc(&roots);
        assert_eq!(
            heap.resolve_weak(handle).map(|value| value.get_ptr()),
            Some(ptr)
        );

        heap.minor_gc(&[]);
        assert!(heap.resolve_weak(handle).is_none());

        let reused = heap.alloc_object().expect("nursery alloc should succeed");
        assert!(heap.resolve_weak(handle).is_none());

        let reused_handle = heap
            .weak_handle(JSValue::new_object(reused).as_object())
            .expect("reused nursery object should have a weak handle");
        assert_eq!(
            heap.resolve_weak(reused_handle)
                .map(|value| value.get_ptr()),
            Some(reused)
        );

        let func_ptr = Box::into_raw(Box::new(JSFunction::new()));
        heap.track_function(func_ptr);
        let func_handle = heap
            .weak_handle(JSValue::new_function(func_ptr).as_object())
            .expect("function should have a weak handle");
        let resolved = heap
            .resolve_weak(func_handle)
            .expect("live function should resolve");
        assert!(resolved.is_function());
        heap.untrack(func_ptr.cast::<JSObject>());
        // SAFETY: the function pointer came from `Box::into_raw` above and is reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(func_ptr);
        }
    }

    #[test]
    fn test_gc_heap_minor_gc_promotes_reachable() {
        let mut heap = GcHeap::new();
        let ptr = heap.alloc_object().expect("nursery alloc should succeed");
        // SAFETY: `ptr` is a live nursery object owned by `heap`.
        unsafe {
            (*ptr).set(Atom(1), JSValue::new_int(99));
        }

        let roots = vec![JSValue::new_object(ptr)];
        let freed = heap.minor_gc(&roots);
        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 1);
        assert_eq!(heap.nursery_indices.len(), 1);

        // SAFETY: `ptr` is a live nursery object owned by `heap`.
        let idx = unsafe { (*(ptr as usize as *const JSObject)).gc_slot } as usize;
        assert_eq!(heap.nursery_indices[0], idx);
        let new_ptr = heap.objects[idx];
        // SAFETY: `new_ptr` is the promoted copy of the live object recorded in the heap.
        let obj_ref = unsafe { &*(new_ptr as *const JSObject) };
        assert_eq!(obj_ref.get(Atom(1)).unwrap().get_int(), 99);

        // SAFETY: the promoted object is owned by the heap and this test reclaims it exactly once.
        unsafe {
            std::ptr::drop_in_place(new_ptr);
        }
    }

    #[test]
    fn test_gc_heap_minor_gc_frees_unreachable_nursery() {
        let mut heap = GcHeap::new();
        let reachable = heap.alloc_object().expect("alloc should succeed");
        let _unreachable = heap.alloc_object().expect("alloc should succeed");

        let roots = vec![JSValue::new_object(reachable)];
        let freed = heap.minor_gc(&roots);
        assert_eq!(freed, 1);
        assert_eq!(heap.object_count(), 1);

        let freed_full = heap.run_gc(&roots);
        assert_eq!(freed_full, 0);
        assert_eq!(heap.object_count(), 1);

        // SAFETY: `reachable` is the live object promoted by the collector above.
        let idx = unsafe { (*(reachable as *const JSObject)).gc_slot } as usize;
        let new_ptr = heap.objects[idx];
        // SAFETY: the promoted object is owned by the heap and this test reclaims it exactly once.
        unsafe {
            std::ptr::drop_in_place(new_ptr);
        }
    }

    #[test]
    fn test_gc_heap_nursery_array_alloc_and_minor_gc() {
        let mut heap = GcHeap::new();
        let arr_ptr = heap
            .alloc_array()
            .expect("nursery array alloc should succeed");
        // SAFETY: `arr_ptr` is a live nursery array owned by `heap`.
        unsafe {
            (*arr_ptr).set(0, JSValue::new_int(1));
            (*arr_ptr).set(1, JSValue::new_int(2));
        }

        let roots = vec![JSValue::new_object(arr_ptr.cast())];
        let freed = heap.minor_gc(&roots);
        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 1);
        assert_eq!(heap.nursery_indices.len(), 1);

        // SAFETY: `arr_ptr` is a live nursery array owned by `heap`.
        let idx = unsafe { (*(arr_ptr as *const JSObject)).gc_slot } as usize;
        assert_eq!(heap.nursery_indices[0], idx);
        let new_ptr = heap.objects[idx];
        // SAFETY: `new_ptr` is the promoted copy of the live dense array allocated above.
        let arr_ref = unsafe { &*(new_ptr as *const JSArrayObject) };
        assert_eq!(arr_ref.get(0).unwrap().get_int(), 1);
        assert_eq!(arr_ref.get(1).unwrap().get_int(), 2);

        // SAFETY: the promoted array is owned by the heap and this test reclaims it exactly once.
        unsafe {
            std::ptr::drop_in_place(new_ptr as *mut JSArrayObject);
        }
    }

    #[test]
    fn test_incremental_mark_frees_unreachable() {
        let mut heap = GcHeap::new();

        let reachable_obj = JSObject::new();
        let reachable_ptr = Box::into_raw(Box::new(reachable_obj));
        heap.track(reachable_ptr);

        let unreachable_obj = JSObject::new();
        let unreachable_ptr = Box::into_raw(Box::new(unreachable_obj));
        heap.track(unreachable_ptr);

        let roots = vec![JSValue::new_object(reachable_ptr)];
        heap.start_incremental_mark(&roots);

        let still_marking = heap.incremental_mark_step(1);
        assert!(still_marking || heap.gray_stack.is_empty());

        while heap.incremental_mark_step(10) {}

        let freed = heap.finish_incremental_sweep();
        assert_eq!(freed, 1);
        assert_eq!(heap.object_count(), 1);
        assert!(heap.objects.contains(&reachable_ptr));
        assert!(!heap.objects.contains(&unreachable_ptr));

        // SAFETY: the collector kept the reachable object alive and this test reclaims it exactly once.
        unsafe {
            let _ = Box::from_raw(reachable_ptr);
        }
    }

    #[test]
    fn test_write_barrier_protects_new_reference() {
        let mut heap = GcHeap::new();

        let parent = JSObject::new();
        let parent_ptr = Box::into_raw(Box::new(parent));
        heap.track(parent_ptr);

        let child = JSObject::new();
        let child_ptr = Box::into_raw(Box::new(child));
        heap.track(child_ptr);

        let roots = vec![JSValue::new_object(parent_ptr)];
        heap.start_incremental_mark(&roots);
        while heap.incremental_mark_step(1) {
            // SAFETY: `parent_ptr` is a live boxed object rooted above.
            if heap.marks[unsafe { (*(parent_ptr as *const JSObject)).gc_slot } as usize]
                == COLOR_BLACK
            {
                break;
            }
        }

        let child_value = JSValue::new_object(child_ptr);
        heap.write_barrier(parent_ptr, &child_value);

        while heap.incremental_mark_step(10) {}
        let freed = heap.finish_incremental_sweep();
        assert_eq!(freed, 0);
        assert_eq!(heap.object_count(), 2);

        // SAFETY: both pointers came from `Box::into_raw` above and are reclaimed exactly once.
        unsafe {
            let _ = Box::from_raw(parent_ptr);
            let _ = Box::from_raw(child_ptr);
        }
    }

    #[test]
    fn test_incremental_mark_step_budget() {
        let mut heap = GcHeap::new();
        for _ in 0..10 {
            let obj = JSObject::new();
            let ptr = Box::into_raw(Box::new(obj));
            heap.track(ptr);
        }

        heap.start_incremental_mark(&[]);

        let still_marking = heap.incremental_mark_step(5);
        assert!(!still_marking);
        let freed = heap.finish_incremental_sweep();
        assert_eq!(freed, 10);
    }
}
