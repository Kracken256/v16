use crate::compiler::opcode::{Bytecode, Opcode};
use crate::runtime::context::JSContext;
#[cfg(test)]
use crate::runtime::runtime::JSRuntime;
use crate::util::FxHashMap;
use crate::value::JSValue;

const GC_CHECK_INTERVAL: u64 = 16384;
const INTERRUPT_POLL_EVERY_MASK: usize = 0xFFF;

thread_local! {
    static RUST_CALL_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

const PRIM_STRING_SHAPE_ID: usize = usize::MAX - 10;
const PRIM_NUMBER_SHAPE_ID: usize = usize::MAX - 11;
const PRIM_BOOL_SHAPE_ID: usize = usize::MAX - 12;

#[derive(Clone, Copy)]
enum BitwiseOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
    UShr,
}

#[derive(Clone, Copy)]
enum ArithOp {
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone)]
pub struct CallFrame {
    pub return_pc: usize,
    pub registers_base: usize,
    pub registers_count: usize,
    pub locals_count: u32,
    pub bytecode_ptr: *const u8,
    pub bytecode_len: usize,
    pub constants_ptr: *const JSValue,
    pub constants_len: usize,
    pub function_ptr: Option<usize>,
    pub ic_table_ptr: *mut crate::compiler::InlineCacheTable,
    pub this_value: JSValue,
    pub saved_args: Vec<JSValue>,
    pub upvalue_sync_map: Option<Box<FxHashMap<u16, std::rc::Rc<std::cell::Cell<JSValue>>>>>,

    pub upvalue_sync_bitset: u64,
    pub dst_reg: u16,
    pub arg_count: u16,
    pub super_ctor: JSValue,
    pub super_base: JSValue,
    pub new_target: JSValue,
    pub this_initialized: bool,
    pub is_super_call: bool,
    pub is_constructor: bool,
    pub this_cell: Option<std::rc::Rc<std::cell::Cell<(JSValue, bool)>>>,
    pub is_async: bool,
    pub uses_arguments: bool,
    pub current_pc: usize,
    pub is_strict_frame: bool,

    pub has_upvalues: bool,

    pub var_name_map: *const Vec<(u32, u16, u8)>,
    pub eval_bindings: Option<Box<std::collections::HashMap<u32, JSValue>>>,
    pub cached_arguments: Option<usize>,
    pub with_env: Option<JSValue>,
    pub in_parameter_initializer: bool,
    pub param_env_has_arguments: bool,
}

impl CallFrame {
    pub fn new() -> Self {
        Self {
            return_pc: 0,
            registers_base: 0,
            registers_count: 0,
            locals_count: 0,
            bytecode_ptr: std::ptr::null(),
            bytecode_len: 0,
            constants_ptr: std::ptr::null(),
            constants_len: 0,
            function_ptr: None,
            ic_table_ptr: std::ptr::null_mut(),
            this_value: JSValue::undefined(),
            saved_args: Vec::new(),
            upvalue_sync_map: None,
            upvalue_sync_bitset: 0,
            dst_reg: 0,
            arg_count: 0,
            super_ctor: JSValue::undefined(),
            super_base: JSValue::undefined(),
            new_target: JSValue::undefined(),
            this_initialized: true,
            is_super_call: false,
            is_constructor: false,
            this_cell: None,
            is_async: false,
            uses_arguments: false,
            current_pc: 0,
            is_strict_frame: false,
            has_upvalues: false,
            var_name_map: std::ptr::null(),
            eval_bindings: None,
            cached_arguments: None,
            with_env: None,
            in_parameter_initializer: false,
            param_env_has_arguments: false,
        }
    }
}

#[derive(Clone)]
pub struct ExceptionHandler {
    pub frame_index: usize,
    pub catch_pc: usize,
    pub finally_pc: Option<usize>,
}

#[derive(Debug)]
pub enum ExecutionOutcome {
    Complete(JSValue),
    Yield(JSValue),
}

pub enum PromiseSettlement {
    Fulfilled(JSValue),
    Rejected(JSValue),
    Pending,
}

pub enum AsyncStepResult {
    Complete(JSValue, Vec<JSValue>, usize, Vec<ExceptionHandler>),
    Suspended(JSValue, u16, Vec<JSValue>, usize, Vec<ExceptionHandler>),
    Yielded(JSValue, bool, Option<u16>, Vec<JSValue>, usize, Vec<ExceptionHandler>),
    YieldSuspended(JSValue, Option<u16>, bool, Vec<JSValue>, usize, Vec<ExceptionHandler>),
    Threw(JSValue),
    Fatal(String),
}

enum ThrowDispatch {
    Caught,

    Uncaught(String),

    AsyncComplete(ExecutionOutcome),
}

enum CallerVarSet {
    Stored,

    Immutable,

    NotFound,
}

pub fn start_async_call(
    ctx: &mut JSContext,
    js_func: &crate::object::function::JSFunction,
    func_ptr: usize,
    this_val: JSValue,
    args: &[JSValue],
) -> Result<JSValue, String> {
    let rb = match js_func.bytecode {
        Some(ref rb) => rb,
        None => return Ok(JSValue::undefined()),
    };
    let this = if js_func.is_arrow() {
        match &js_func.arrow_this_cell {
            Some(cell) => cell.get().0,
            None => js_func.arrow_this,
        }
    } else if !js_func.is_strict() && (this_val.is_undefined() || this_val.is_null()) {
        ctx.global()
    } else {
        this_val
    };
    let min_slots = 1 + args.len();
    let snapshot_len = (rb.locals_count as usize).max(min_slots);
    let mut snapshot = vec![JSValue::undefined(); snapshot_len];
    snapshot[0] = this;
    for (i, arg) in args.iter().enumerate() {
        snapshot[i + 1] = *arg;
    }

    let promise = crate::builtins::promise::new_pending_promise(ctx);

    let mut holder = crate::object::object::JSObject::new();
    let promise_atom = ctx.intern("__v16_promise__");
    holder.set(promise_atom, promise);
    holder.set_async_state(crate::object::object::AsyncState {
        bytecode: Box::new(rb.clone()),
        snapshot,
        pc: 0,
        done: false,
        send_reg: None,
        saved_args: args.to_vec(),
        func_ptr: Some(func_ptr),
        handlers: Vec::new(),
        pending_throw: None,
        resume_kind: 0,
        resume_value: JSValue::undefined(),
        yield_is_result: false,
        pending_yield: false,
    });
    let holder_ptr = Box::into_raw(Box::new(holder)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(holder_ptr);
    let holder_val = JSValue::new_object(holder_ptr);

    let async_atom = ctx.intern("__v16_async__");
    if promise.is_object() {
        promise.as_object_mut().set(async_atom, holder_val);
    }

    ctx.runtime_mut().gc_heap_mut().extra_roots.push(holder_val);

    drive_async_state(ctx, holder_val);

    Ok(promise)
}

pub fn drive_async_state(ctx: &mut JSContext, holder_val: JSValue) {
    if !holder_val.is_object() {
        return;
    }
    let holder_ptr = holder_val.get_ptr();
    let holder = unsafe { &mut *(holder_ptr as *mut crate::object::object::JSObject) };
    let is_generator = holder.get_async_generator_state().is_some();
    let mut state = match holder.take_async_state() {
        Some(s) => s,
        None => return,
    };
    if state.done {
        holder.set_async_state(*state);
        if is_generator {
            drive_async_generator_queue(ctx, holder_val);
        }
        return;
    }

    let mut step_vm = VM::new();
    ctx.runtime_mut().gc_heap_mut().gc_suppressed = true;
    let outcome = step_vm.execute_async_step(ctx, &state, is_generator);
    ctx.runtime_mut().gc_heap_mut().gc_suppressed = false;

    let promise_atom = ctx.intern("__v16_promise__");
    let awaiting_atom = ctx.intern("__v16_awaiting__");
    let promise = holder.get(promise_atom).unwrap_or(JSValue::undefined());

    let mut finished = false;
    match outcome {
        Ok(AsyncStepResult::Complete(value, snapshot, pc, handlers)) => {
            state.snapshot = snapshot;
            state.pc = pc;
            state.handlers = handlers;
            state.done = true;
            holder.set(awaiting_atom, JSValue::null());
            if is_generator {
                settle_async_generator_current(ctx, holder_val, value, true, false);
                holder.set_async_state(*state);
                drive_async_generator_queue(ctx, holder_val);
                return;
            }
            crate::builtins::promise::settle_async_result(ctx, promise, value, false);
            finished = true;
        }
        Ok(AsyncStepResult::Yielded(value, is_result, src, snapshot, pc, handlers)) => {
            state.snapshot = snapshot;
            state.pc = pc;
            state.handlers = handlers;
            state.send_reg = src;
            state.yield_is_result = is_result;
            state.resume_kind = 0;
            state.resume_value = JSValue::undefined();
            state.pending_throw = None;
            holder.set(awaiting_atom, JSValue::null());
            holder.set_async_state(*state);
            settle_async_generator_current(ctx, holder_val, value, false, false);
            drive_async_generator_queue(ctx, holder_val);
            return;
        }
        Ok(AsyncStepResult::Suspended(p, src, snapshot, pc, handlers)) => {
            state.snapshot = snapshot;
            state.pc = pc;
            state.handlers = handlers;
            state.send_reg = Some(src);
            state.pending_throw = None;
            holder.set(awaiting_atom, p);
            crate::builtins::promise::schedule_async_resume(ctx, p, holder_val, false);
            crate::builtins::promise::schedule_async_resume(ctx, p, holder_val, true);
        }
        Ok(AsyncStepResult::YieldSuspended(p, src, is_result, snapshot, pc, handlers)) => {
            state.snapshot = snapshot;
            state.pc = pc;
            state.handlers = handlers;
            state.send_reg = src;
            state.yield_is_result = is_result;
            state.pending_throw = None;
            state.pending_yield = true;
            holder.set(awaiting_atom, p);
            crate::builtins::promise::schedule_async_resume(ctx, p, holder_val, false);
            crate::builtins::promise::schedule_async_resume(ctx, p, holder_val, true);
        }
        Ok(AsyncStepResult::Threw(value)) => {
            state.done = true;
            holder.set(awaiting_atom, JSValue::null());
            if is_generator {
                settle_async_generator_current(ctx, holder_val, value, true, true);
                holder.set_async_state(*state);
                drive_async_generator_queue(ctx, holder_val);
                return;
            }
            crate::builtins::promise::settle_async_result(ctx, promise, value, true);
            finished = true;
        }
        Ok(AsyncStepResult::Fatal(msg)) | Err(msg) => {
            state.done = true;
            holder.set(awaiting_atom, JSValue::null());
            let err = make_async_error(ctx, &msg);
            if is_generator {
                settle_async_generator_current(ctx, holder_val, err, true, true);
                holder.set_async_state(*state);
                drive_async_generator_queue(ctx, holder_val);
                return;
            }
            crate::builtins::promise::settle_async_result(ctx, promise, err, true);
            finished = true;
        }
    }
    holder.set_async_state(*state);

    if finished {
        let roots = &mut ctx.runtime_mut().gc_heap_mut().extra_roots;
        let holder_bits = holder_val.raw_bits();
        roots.retain(|v| v.raw_bits() != holder_bits);
    }
}

fn generator_iterator_result(ctx: &mut JSContext, value: JSValue, done: bool) -> JSValue {
    let mut result = crate::object::object::JSObject::new();
    result.set(ctx.intern("value"), value);
    result.set(ctx.intern("done"), JSValue::bool(done));
    if let Some(proto_ptr) = ctx.get_object_prototype() {
        result.prototype = Some(proto_ptr);
    }
    let ptr = Box::into_raw(Box::new(result)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

fn take_async_generator_current(ctx: &mut JSContext, gen_val: JSValue) -> Option<JSValue> {
    let generator_obj =
        unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
    let state = generator_obj.get_async_generator_state_mut()?;
    state.running = false;
    let promise = state.current_promise;
    state.current_promise = JSValue::undefined();
    let _ = ctx;
    Some(promise)
}

fn settle_async_generator_current(
    ctx: &mut JSContext,
    gen_val: JSValue,
    value: JSValue,
    done: bool,
    is_reject: bool,
) {
    let promise = match take_async_generator_current(ctx, gen_val) {
        Some(p) => p,
        None => return,
    };
    if is_reject {
        crate::builtins::promise::settle_async_result(ctx, promise, value, true);
    } else {
        let result = generator_iterator_result(ctx, value, done);
        crate::builtins::promise::settle_async_result(ctx, promise, result, false);
    }
}

fn resume_async_generator_body_from_request(
    ctx: &mut JSContext,
    gen_val: JSValue,
    kind: u8,
    value: JSValue,
) {
    {
        let generator_obj =
            unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
        let state = match generator_obj.get_async_state_mut() {
            Some(state) => state,
            None => return,
        };
        state.resume_kind = kind;
        state.resume_value = value;
        state.pending_throw = None;
        state.pending_yield = false;
        if kind == 0 || state.yield_is_result {
            if let Some(reg) = state.send_reg {
                let idx = reg as usize;
                if idx < state.snapshot.len() {
                    state.snapshot[idx] = value;
                }
            }
        }
    }
    drive_async_state(ctx, gen_val);
}

fn async_generator_await_return(
    ctx: &mut JSContext,
    gen_val: JSValue,
    promise: JSValue,
    value: JSValue,
) {
    {
        let generator_obj =
            unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
        if let Some(ag) = generator_obj.get_async_generator_state_mut() {
            ag.awaiting_return = true;
            ag.current_promise = promise;
            ag.pending_return_action = 2;
        }
    }
    let awaited = ctx.call_builtin("promise_resolve", &[value]);
    if let Some(exc) = ctx.pending_exception.take() {
        {
            let generator_obj =
                unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
            if let Some(ag) = generator_obj.get_async_generator_state_mut() {
                ag.pending_return_action = 0;
                ag.awaiting_return = false;
                ag.current_promise = JSValue::undefined();
            }
            if let Some(state) = generator_obj.get_async_state_mut() {
                state.done = true;
            }
        }
        crate::builtins::promise::settle_async_result(ctx, promise, exc, true);
        drive_async_generator_queue(ctx, gen_val);
        return;
    }
    crate::builtins::promise::schedule_async_resume(ctx, awaited, gen_val, false);
    crate::builtins::promise::schedule_async_resume(ctx, awaited, gen_val, true);
}

pub fn drive_async_generator_queue(ctx: &mut JSContext, gen_val: JSValue) {
    if !gen_val.is_object() {
        return;
    }
    loop {
        let (running, awaiting_return, empty, started, first_kind) = {
            let generator_obj =
                unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
            match generator_obj.get_async_generator_state() {
                Some(ag) => (
                    ag.running,
                    ag.awaiting_return,
                    ag.queue.is_empty(),
                    ag.started,
                    ag.queue.first().map(|request| request.kind).unwrap_or(0),
                ),
                None => return,
            }
        };
        if running || awaiting_return || empty {
            return;
        }
        let is_done = {
            let generator_obj =
                unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
            generator_obj
                .get_async_state()
                .map(|state| state.done)
                .unwrap_or(true)
        };
        if is_done || (!started && first_kind == 2) {
            let (kind, value, request_promise) = {
                let generator_obj =
                    unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
                let ag = match generator_obj.get_async_generator_state_mut() {
                    Some(ag) => ag,
                    None => return,
                };
                let request = ag.queue.remove(0);
                (request.kind, request.value, request.promise)
            };
            match kind {
                2 => {
                    async_generator_await_return(ctx, gen_val, request_promise, value);
                    return;
                }
                1 => {
                    crate::builtins::promise::settle_async_result(
                        ctx,
                        request_promise,
                        value,
                        true,
                    );
                }
                _ => {
                    let result = generator_iterator_result(ctx, JSValue::undefined(), true);
                    crate::builtins::promise::settle_async_result(
                        ctx,
                        request_promise,
                        result,
                        false,
                    );
                }
            }
            continue;
        }
        let (kind, value) = {
            let generator_obj =
                unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
            let ag = match generator_obj.get_async_generator_state_mut() {
                Some(ag) => ag,
                None => return,
            };
            let request = ag.queue.remove(0);
            ag.running = true;
            ag.started = true;
            ag.current_promise = request.promise;
            (request.kind, request.value)
        };
        if kind == 2 {
            {
                let generator_obj =
                    unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
                if let Some(ag) = generator_obj.get_async_generator_state_mut() {
                    ag.pending_return_action = 1;
                }
            }
            let awaited = ctx.call_builtin("promise_resolve", &[value]);
            if let Some(exc) = ctx.pending_exception.take() {
                {
                    let generator_obj =
                        unsafe { &mut *(gen_val.get_ptr() as *mut crate::object::object::JSObject) };
                    if let Some(ag) = generator_obj.get_async_generator_state_mut() {
                        ag.pending_return_action = 0;
                    }
                }
                resume_async_generator_body_from_request(ctx, gen_val, 1, exc);
                return;
            }
            crate::builtins::promise::schedule_async_resume(ctx, awaited, gen_val, false);
            crate::builtins::promise::schedule_async_resume(ctx, awaited, gen_val, true);
            return;
        }
        resume_async_generator_body_from_request(ctx, gen_val, kind, value);
        return;
    }
}

pub fn resume_async_state(
    ctx: &mut JSContext,
    holder_val: JSValue,
    argument: JSValue,
    is_reject: bool,
) {
    if !holder_val.is_object() {
        return;
    }
    let pending_action = {
        let holder =
            unsafe { &mut *(holder_val.get_ptr() as *mut crate::object::object::JSObject) };
        holder
            .get_async_generator_state()
            .map(|ag| ag.pending_return_action)
            .unwrap_or(0)
    };
    if pending_action != 0 {
        if pending_action == 2 {
            let request_promise = {
                let holder =
                    unsafe { &mut *(holder_val.get_ptr() as *mut crate::object::object::JSObject) };
                let ag = match holder.get_async_generator_state_mut() {
                    Some(ag) => ag,
                    None => return,
                };
                ag.pending_return_action = 0;
                ag.awaiting_return = false;
                let promise = ag.current_promise;
                ag.current_promise = JSValue::undefined();
                promise
            };
            if is_reject {
                crate::builtins::promise::settle_async_result(
                    ctx,
                    request_promise,
                    argument,
                    true,
                );
            } else {
                let result = generator_iterator_result(ctx, argument, true);
                crate::builtins::promise::settle_async_result(
                    ctx,
                    request_promise,
                    result,
                    false,
                );
            }
            {
                let holder =
                    unsafe { &mut *(holder_val.get_ptr() as *mut crate::object::object::JSObject) };
                if let Some(state) = holder.get_async_state_mut() {
                    state.done = true;
                }
            }
            drive_async_generator_queue(ctx, holder_val);
        } else {
            {
                let holder =
                    unsafe { &mut *(holder_val.get_ptr() as *mut crate::object::object::JSObject) };
                if let Some(ag) = holder.get_async_generator_state_mut() {
                    ag.pending_return_action = 0;
                }
            }
            let kind = if is_reject { 1u8 } else { 2u8 };
            resume_async_generator_body_from_request(ctx, holder_val, kind, argument);
        }
        return;
    }
    let holder =
        unsafe { &mut *(holder_val.get_ptr() as *mut crate::object::object::JSObject) };
    let mut state = match holder.take_async_state() {
        Some(s) => s,
        None => return,
    };
    if state.done {
        holder.set_async_state(*state);
        return;
    }
    if state.pending_yield {
        state.pending_yield = false;
        if is_reject {
            state.pending_throw = Some(argument);
            holder.set_async_state(*state);
            drive_async_state(ctx, holder_val);
        } else {
            state.pending_throw = None;
            holder.set_async_state(*state);
            settle_async_generator_current(ctx, holder_val, argument, false, false);
            drive_async_generator_queue(ctx, holder_val);
        }
        return;
    }
    if let Some(reg) = state.send_reg {
        let idx = reg as usize;
        if idx < state.snapshot.len() {
            state.snapshot[idx] = if is_reject {
                JSValue::undefined()
            } else {
                argument
            };
        }
    }
    state.pending_throw = if is_reject { Some(argument) } else { None };
    holder.set_async_state(*state);
    drive_async_state(ctx, holder_val);
}

fn make_async_error(ctx: &mut JSContext, msg: &str) -> JSValue {
    let mut err = crate::object::object::JSObject::new();
    err.set(
        ctx.intern("name"),
        JSValue::new_string(ctx.intern("Error")),
    );
    err.set(
        ctx.intern("message"),
        JSValue::new_string(ctx.intern(msg)),
    );
    if let Some(proto) = ctx.get_error_prototype() {
        err.prototype = Some(proto);
    }
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

pub struct VM {
    pub registers: Vec<JSValue>,
    pub frames: Vec<CallFrame>,
    pub frame_index: usize,
    pub pc: usize,

    cached_code_ptr: *const u8,
    cached_code_len: usize,
    cached_const_ptr: *const JSValue,
    cached_registers_base: usize,
    cached_registers_ptr: *mut JSValue,
    cached_has_upvalue_sync: bool,

    cached_upvalue_sync_bitset: u64,

    cached_ic_table_ptr: *mut crate::compiler::InlineCacheTable,

    cached_upvalue_slot_ptr: *const std::rc::Rc<std::cell::Cell<JSValue>>,
    cached_upvalues_len: usize,

    exception_handlers: Vec<ExceptionHandler>,

    pub(crate) pending_throw: Option<JSValue>,

    pub(crate) last_caught_exception: Option<JSValue>,

    finally_rethrow: Option<JSValue>,
    pending_finally_rethrow: Option<(usize, JSValue)>,
    pending_finally_return: Option<(usize, JSValue)>,
    pending_finally_jump: Option<(usize, usize, usize)>,
    pending_finally_depth: usize,
    caught_exception_value: Option<JSValue>,

    ctx_ptr: *mut crate::runtime::JSContext,

    allocation_count: u64,

    caller_vm: Option<usize>,

    eval_binding_frames: u32,

    eval_direct_flags: u8,

    pending_instance_elements: Option<(JSValue, usize)>,

    pending_super_rebind_error: bool,

    cached_has_instance_atom: crate::runtime::atom::Atom,

    instanceof_cache_ctor: usize,
    instanceof_cache_proto: usize,
    instanceof_cache_ctor_proto: usize,
    instanceof_cache_result: bool,

    regex_lit_cache: std::collections::HashMap<usize, crate::regexp::Regex>,

    pub last_yield_src: Option<u16>,

    pub last_yield_is_result: bool,

    pub generator_resume_kind: u8,

    pub last_uncaught: Option<JSValue>,

    pub pending_saved_args: Option<Vec<JSValue>>,

    pub pending_function_ptr: Option<usize>,

    pub pending_this: Option<JSValue>,

    pub pending_handlers: Option<Vec<ExceptionHandler>>,

    pub pending_return_value: Option<JSValue>,

    pub pending_new_target: Option<JSValue>,

    pub eval_direct_call: bool,

    pub eval_direct_active: bool,

    pub async_step: bool,

    pub await_suspend: Option<(JSValue, u16)>,

    gc_roots: Vec<JSValue>,
}

impl VM {
    fn builtin_needs_callee(name: &str) -> bool {
        matches!(
            name,
            "intl_collator_compare_call" | "promise_internal_resolve" | "promise_internal_reject"
        )
    }

    pub fn new() -> Self {
        let mut vm = Self {
            registers: Vec::with_capacity(4096),
            frames: vec![CallFrame::new(); 64],
            frame_index: 0,
            pc: 0,
            cached_code_ptr: std::ptr::null(),
            cached_code_len: 0,
            cached_const_ptr: std::ptr::null(),
            cached_registers_base: 0,
            cached_registers_ptr: std::ptr::null_mut(),
            cached_has_upvalue_sync: false,
            cached_upvalue_sync_bitset: 0,
            cached_ic_table_ptr: std::ptr::null_mut(),
            cached_upvalue_slot_ptr: std::ptr::null(),
            cached_upvalues_len: 0,
            exception_handlers: Vec::new(),
            pending_throw: None,
            last_caught_exception: None,
            finally_rethrow: None,
            pending_finally_rethrow: None,
            pending_finally_return: None,
            pending_finally_jump: None,
            pending_finally_depth: 0,
            caught_exception_value: None,
            ctx_ptr: std::ptr::null_mut(),
            allocation_count: 0,
            caller_vm: None,
            eval_binding_frames: 0,
            eval_direct_flags: 0,
            pending_instance_elements: None,
            pending_super_rebind_error: false,
            cached_has_instance_atom: crate::runtime::atom::Atom(0),
            instanceof_cache_ctor: 0,
            instanceof_cache_proto: 0,
            instanceof_cache_ctor_proto: 0,
            instanceof_cache_result: false,
            regex_lit_cache: std::collections::HashMap::new(),
            last_yield_src: None,
            last_yield_is_result: false,
            generator_resume_kind: 0,
            last_uncaught: None,
            pending_saved_args: None,
            pending_function_ptr: None,
            pending_this: None,
            pending_handlers: None,
            pending_return_value: None,
            pending_new_target: None,
            eval_direct_call: false,
            eval_direct_active: false,
            async_step: false,
            await_suspend: None,
            gc_roots: Vec::with_capacity(512),
        };
        vm.frames[0] = CallFrame::new();
        vm
    }

    pub fn prime(&mut self) {
        if self.registers.is_empty() {
            self.registers.push(JSValue::undefined());
        }
        self.cached_registers_base = 0;
        self.cached_registers_ptr = self.registers.as_mut_ptr();
        self.frame_index = 0;
        self.pc = 0;
    }

    fn cleanup_handlers_for_frame(&mut self) {
        let fi = self.frame_index;
        while self
            .exception_handlers
            .last()
            .map_or(false, |h| h.frame_index == fi)
        {
            self.exception_handlers.pop();
        }
    }

    fn same_frame_handler_count(&self) -> usize {
        self.exception_handlers
            .iter()
            .filter(|h| h.frame_index == self.frame_index)
            .count()
    }

    fn clear_pending_finally_if_escaped(
        &mut self,
        handler_frame: usize,
        same_frame_after: usize,
    ) {
        let owned = self
            .pending_finally_rethrow
            .map_or(false, |(frame, _)| frame == self.frame_index)
            || self
                .pending_finally_return
                .map_or(false, |(frame, _)| frame == self.frame_index)
            || self
                .pending_finally_jump
                .map_or(false, |(frame, _, _)| frame == self.frame_index);
        if !owned {
            return;
        }
        let escaped = if handler_frame == self.frame_index {
            same_frame_after < self.pending_finally_depth
        } else {
            true
        };
        if escaped {
            self.clear_pending_finally_for_frame(self.frame_index);
        }
    }

    fn clear_pending_finally_for_frame(&mut self, frame: usize) {
        if self
            .pending_finally_rethrow
            .map_or(false, |(owner, _)| owner == frame)
        {
            self.pending_finally_rethrow = None;
        }
        if self
            .pending_finally_return
            .map_or(false, |(owner, _)| owner == frame)
        {
            self.pending_finally_return = None;
        }
        if self
            .pending_finally_jump
            .map_or(false, |(owner, _, _)| owner == frame)
        {
            self.pending_finally_jump = None;
        }
    }

    fn throw_reference_error(&mut self, ctx: &mut JSContext, msg: &str) -> Option<JSValue> {
        let mut err = crate::object::object::JSObject::new();
        err.set(
            ctx.intern("name"),
            JSValue::new_string(ctx.intern("ReferenceError")),
        );
        err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(msg)));
        if let Some(proto) = ctx.get_reference_error_prototype() {
            err.prototype = Some(proto);
        }
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.pending_throw = Some(JSValue::new_object(ptr));
        None
    }

    /// Returns true when `Symbol.iterator` on the array still resolves to the
    /// built-in array iterator, so the fast array iteration path is valid.
    fn array_uses_default_iterator(&mut self, ctx: &mut JSContext, arr: JSValue) -> bool {
        let default_ptr = match ctx.get_array_iterator_fn() {
            Some(p) => p,
            None => return false,
        };
        let sym_iter = crate::builtins::symbol::get_symbol_iterator(ctx);
        if !sym_iter.is_symbol() {
            return false;
        }
        let sym_atom = crate::runtime::atom::Atom(0x40000000 | sym_iter.get_symbol_id());
        let arr_obj = unsafe { arr.as_object() };
        match arr_obj.get(sym_atom) {
            Some(v) => v.is_function() && v.get_ptr() == default_ptr,
            None => false,
        }
    }

    fn create_iter_object(&mut self, ctx: &mut JSContext, iterable: JSValue) -> JSValue {
        let mut iter_obj = crate::object::object::JSObject::new();
        let arr_atom = ctx.common_atoms.__iter_arr__;
        let idx_atom = ctx.common_atoms.__iter_idx__;
        iter_obj.set_cached(arr_atom, iterable, ctx.shape_cache_mut());
        iter_obj.set_cached(idx_atom, JSValue::new_int(0), ctx.shape_cache_mut());
        let iter_ptr = Box::into_raw(Box::new(iter_obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(iter_ptr);
        self.allocation_count += 1;
        JSValue::new_object(iter_ptr)
    }

    fn make_iterator_record(
        &mut self,
        ctx: &mut JSContext,
        iterator: JSValue,
        next: JSValue,
        async_from_sync: bool,
    ) -> JSValue {
        let mut record = crate::object::object::JSObject::new();
        let obj_atom = ctx.common_atoms.__iter_obj__;
        let next_atom = ctx.common_atoms.__iter_next__;
        let async_atom = ctx.common_atoms.__iter_async__;
        record.set_cached(obj_atom, iterator, ctx.shape_cache_mut());
        record.set_cached(next_atom, next, ctx.shape_cache_mut());
        if async_from_sync {
            record.set_cached(async_atom, JSValue::bool(true), ctx.shape_cache_mut());
        }
        let ptr = Box::into_raw(Box::new(record)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.allocation_count += 1;
        JSValue::new_object(ptr)
    }

    pub fn iterator_record_parts(
        ctx: &JSContext,
        iter_val: JSValue,
    ) -> Option<(JSValue, JSValue, bool)> {
        if !iter_val.is_object_like() {
            return None;
        }
        let record = iter_val.as_object();
        let iter_obj = record.get(ctx.common_atoms.__iter_obj__)?;
        let next = record
            .get(ctx.common_atoms.__iter_next__)
            .unwrap_or_else(JSValue::undefined);
        let async_from_sync = record
            .get(ctx.common_atoms.__iter_async__)
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        Some((iter_obj, next, async_from_sync))
    }

    fn get_method(
        &mut self,
        ctx: &mut JSContext,
        value: JSValue,
        atom: crate::runtime::atom::Atom,
        description: &str,
    ) -> Result<Option<JSValue>, String> {
        let method = self.get_prop_value(ctx, value, atom)?;
        if method.is_undefined() || method.is_null() {
            return Ok(None);
        }
        if method.is_function() {
            return Ok(Some(method));
        }
        let msg = format!("{} is not a function", description);
        self.set_pending_type_error(ctx, &msg);
        Err(msg)
    }

    fn get_iterator(
        &mut self,
        ctx: &mut JSContext,
        iterable: JSValue,
        is_async: bool,
    ) -> Result<JSValue, String> {
        if iterable.is_string() && !is_async {
            return Ok(self.create_iter_object(ctx, iterable));
        }
        if iterable.is_object_like() && !is_async {
            let is_dense = iterable.is_object() && iterable.as_object().is_dense_array();
            if is_dense && self.array_uses_default_iterator(ctx, iterable) {
                return Ok(self.create_iter_object(ctx, iterable));
            }
        }
        if is_async {
            let sym_async = crate::builtins::symbol::get_symbol_async_iterator(ctx);
            if sym_async.is_symbol() {
                let atom = crate::runtime::atom::Atom(0x40000000 | sym_async.get_symbol_id());
                if let Some(method) = self.get_method(ctx, iterable, atom, "Symbol.asyncIterator")? {
                    let iterator = self.call_function_with_this(ctx, method, iterable, &[])?;
                    if !iterator.is_object_like() {
                        let msg = "Result of the Symbol.asyncIterator method is not an object";
                        self.set_pending_type_error(ctx, msg);
                        return Err(msg.to_string());
                    }
                    let next_atom = ctx.intern("next");
                    let next = self.get_prop_value(ctx, iterator, next_atom)?;
                    return Ok(self.make_iterator_record(ctx, iterator, next, false));
                }
            }
        }
        let sym_iter = crate::builtins::symbol::get_symbol_iterator(ctx);
        if sym_iter.is_symbol() {
            let atom = crate::runtime::atom::Atom(0x40000000 | sym_iter.get_symbol_id());
            if let Some(method) = self.get_method(ctx, iterable, atom, "Symbol.iterator")? {
                let iterator = self.call_function_with_this(ctx, method, iterable, &[])?;
                if !iterator.is_object_like() {
                    let msg = "Result of the Symbol.iterator method is not an object";
                    self.set_pending_type_error(ctx, msg);
                    return Err(msg.to_string());
                }
                let next_atom = ctx.intern("next");
                let next = self.get_prop_value(ctx, iterator, next_atom)?;
                return Ok(self.make_iterator_record(ctx, iterator, next, is_async));
            }
        }
        let msg = format!(
            "{} is not iterable",
            self.format_thrown_value(&iterable, ctx)
        );
        self.set_pending_type_error(ctx, &msg);
        Err(msg)
    }

    #[inline(always)]
    fn refresh_cache(&mut self) {
        let frame = &self.frames[self.frame_index];
        self.cached_code_ptr = frame.bytecode_ptr;
        self.cached_code_len = frame.bytecode_len;
        self.cached_const_ptr = frame.constants_ptr;
        self.cached_registers_base = frame.registers_base;
        self.cached_has_upvalue_sync = frame.upvalue_sync_map.is_some();
        self.cached_upvalue_sync_bitset = frame.upvalue_sync_bitset;
        self.cached_registers_ptr =
            unsafe { self.registers.as_mut_ptr().add(self.cached_registers_base) };
        self.cached_ic_table_ptr = frame.ic_table_ptr;

        if frame.has_upvalues {
            if let Some(func_ptr) = frame.function_ptr {
                let func_val = JSValue::new_function(func_ptr);
                let func = func_val.as_function();
                if let Some(uv) = func.upvalues.as_ref() {
                    let slots = &uv.upvalue_slots;
                    self.cached_upvalue_slot_ptr = slots.as_ptr();
                    self.cached_upvalues_len = slots.len();
                } else {
                    self.cached_upvalue_slot_ptr = std::ptr::null();
                    self.cached_upvalues_len = 0;
                }
            } else {
                self.cached_upvalue_slot_ptr = std::ptr::null();
                self.cached_upvalues_len = 0;
            }
        } else {
            self.cached_upvalue_slot_ptr = std::ptr::null();
            self.cached_upvalues_len = 0;
        }
    }

    fn push_frame(
        &mut self,
        bytecode: &Bytecode,
        return_pc: usize,
        function_ptr: Option<usize>,
        this_value: JSValue,
        dst_reg: u16,
        arg_count: u16,
        is_constructor: bool,
        is_async: bool,
        args: &[JSValue],
        save_args: bool,
    ) {
        self.push_frame_raw(
            bytecode.locals_count,
            bytecode.effective_code_ptr(),
            bytecode.effective_code_len(),
            bytecode.effective_const_ptr(),
            bytecode.effective_const_len(),
            return_pc,
            function_ptr,
            this_value,
            dst_reg,
            arg_count,
            is_constructor,
            is_async,
            args,
            save_args,
        );
    }

    fn push_frame_raw(
        &mut self,
        locals_count: u32,
        bytecode_ptr: *const u8,
        bytecode_len: usize,
        constants_ptr: *const JSValue,
        constants_len: usize,
        return_pc: usize,
        function_ptr: Option<usize>,
        this_value: JSValue,
        dst_reg: u16,
        arg_count: u16,
        is_constructor: bool,
        is_async: bool,
        args: &[JSValue],
        save_args: bool,
    ) {
        let needed = (locals_count as usize).max(arg_count as usize + 1);
        let prev = &self.frames[self.frame_index];
        let base = prev.registers_base + prev.registers_count;
        let total = base + needed;
        if total > self.registers.len() {
            if total > self.registers.capacity() {
                let new_cap = (self.registers.capacity() * 2 + 64).max(total);
                self.registers.reserve(new_cap - self.registers.len());
            }

            unsafe {
                self.registers.set_len(total);
            }
            self.cached_registers_ptr =
                unsafe { self.registers.as_mut_ptr().add(self.cached_registers_base) };
        }

        unsafe {
            let ptr = self.registers.as_mut_ptr().add(base);
            ptr.write(this_value);
            for (i, arg) in args.iter().enumerate() {
                ptr.add(i + 1).write(*arg);
            }
            let undef = JSValue::undefined();
            for i in (args.len() + 1)..needed {
                ptr.add(i).write(undef);
            }
        }

        self.frame_index += 1;
        self.clear_pending_finally_for_frame(self.frame_index);
        if self.frame_index >= self.frames.len() {
            self.frames.resize(self.frames.len() * 2, CallFrame::new());
        }

        let frame = &mut self.frames[self.frame_index];
        frame.return_pc = return_pc;
        frame.registers_base = base;
        frame.registers_count = needed;
        frame.locals_count = locals_count;
        frame.bytecode_ptr = bytecode_ptr;
        frame.bytecode_len = bytecode_len;
        frame.constants_ptr = constants_ptr;
        frame.constants_len = constants_len;
        frame.function_ptr = function_ptr;
        frame.with_env = function_ptr
            .map(|ptr| unsafe {
                (&*(ptr as *const crate::object::function::JSFunction))
                    .with_env
                    .clone()
            })
            .flatten();
        frame.ic_table_ptr = function_ptr
            .and_then(|fptr| {
                let js_func = unsafe { JSValue::function_from_ptr(fptr) };
                js_func
                    .bytecode
                    .as_ref()
                    .map(|rb| rb.effective_ic_table_ptr())
            })
            .unwrap_or(std::ptr::null_mut());
        frame.this_value = this_value;
        frame.dst_reg = dst_reg;
        frame.arg_count = arg_count;
        frame.super_ctor = JSValue::undefined();
        frame.new_target = JSValue::undefined();
        frame.this_initialized = true;
        frame.this_cell = None;
        frame.is_super_call = false;
        frame.is_constructor = is_constructor;
        frame.is_async = is_async;
        if let Some(fptr) = function_ptr {
            let func = unsafe { &*(fptr as *const crate::object::function::JSFunction) };
            frame.is_strict_frame = func.is_strict();
            frame.var_name_map = func.bytecode.as_ref().map_or(std::ptr::null(), |bc| {
                std::rc::Rc::as_ptr(&bc.var_name_to_slot)
            });
            frame.has_upvalues = func
                .upvalues
                .as_ref()
                .map_or(false, |uv| !uv.upvalue_slots.is_empty());
        } else {
            frame.is_strict_frame = false;
            frame.var_name_map = std::ptr::null();
            frame.has_upvalues = false;
        }
        frame.cached_arguments = None;
        frame.uses_arguments = save_args;
        if save_args {
            frame.saved_args.clear();
            frame.saved_args.extend_from_slice(args);
        } else if !frame.saved_args.is_empty() {
            frame.saved_args.clear();
        }
        if frame.upvalue_sync_map.is_some() {
            frame.upvalue_sync_map = None;
            frame.upvalue_sync_bitset = 0;
        }
        self.pc = 0;
        self.cached_code_ptr = bytecode_ptr;
        self.cached_code_len = bytecode_len;
        self.cached_const_ptr = constants_ptr;
        self.cached_registers_base = base;
        self.cached_has_upvalue_sync = false;
        self.cached_upvalue_sync_bitset = 0;
        self.cached_registers_ptr = unsafe { self.registers.as_mut_ptr().add(base) };
        self.cached_ic_table_ptr = frame.ic_table_ptr;
        if let Some(fptr) = function_ptr {
            if frame.has_upvalues {
                let func = unsafe { JSValue::function_from_ptr(fptr) };
                if let Some(uv) = func.upvalues.as_ref() {
                    let slots = &uv.upvalue_slots;
                    self.cached_upvalue_slot_ptr = slots.as_ptr();
                    self.cached_upvalues_len = slots.len();
                } else {
                    self.cached_upvalue_slot_ptr = std::ptr::null();
                    self.cached_upvalues_len = 0;
                }
            } else {
                self.cached_upvalue_slot_ptr = std::ptr::null();
                self.cached_upvalues_len = 0;
            }
        } else {
            self.cached_upvalue_slot_ptr = std::ptr::null();
            self.cached_upvalues_len = 0;
        }
    }

    #[inline(always)]
    fn push_frame_from_arg_regs_raw(
        &mut self,
        _ctx: &mut JSContext,
        locals_count: u32,
        bytecode_ptr: *const u8,
        bytecode_len: usize,
        constants_ptr: *const JSValue,
        constants_len: usize,
        return_pc: usize,
        function_ptr: Option<usize>,
        ic_table_ptr: *mut crate::compiler::InlineCacheTable,
        this_value: JSValue,
        dst_reg: u16,
        arg_count: u16,
        is_constructor: bool,
        is_async: bool,
        _caller_base: usize,
        arg_regs: &[u16],
        save_args: bool,
    ) {
        let needed = (locals_count as usize).max(arg_count as usize + 1);
        let prev = &self.frames[self.frame_index];
        let base = prev.registers_base + prev.registers_count;
        let total = base + needed;

        if total > self.registers.len() {
            if total > self.registers.capacity() {
                let new_cap = (self.registers.capacity() * 2 + 64).max(total);
                self.registers.reserve(new_cap - self.registers.len());
            }

            unsafe {
                self.registers.set_len(total);
            }
            self.cached_registers_ptr =
                unsafe { self.registers.as_mut_ptr().add(self.cached_registers_base) };
        }

        unsafe {
            let ptr = self.registers.as_mut_ptr().add(base);
            ptr.write(this_value);

            for (i, r) in arg_regs.iter().enumerate() {
                ptr.add(i + 1).write(self.get_reg(*r));
            }
            let fill_start = arg_regs.len() + 1;
            if fill_start < needed {
                let undef = JSValue::undefined();
                for i in fill_start..needed {
                    ptr.add(i).write(undef);
                }
            }
        }

        self.frame_index += 1;
        self.clear_pending_finally_for_frame(self.frame_index);
        let is_new_frame = self.frame_index >= self.frames.len();
        if is_new_frame {
            self.frames.resize(self.frames.len() * 2, CallFrame::new());
        }

        let frame = &mut self.frames[self.frame_index];
        frame.return_pc = return_pc;
        frame.registers_base = base;
        frame.registers_count = needed;
        frame.locals_count = locals_count;
        frame.bytecode_ptr = bytecode_ptr;
        frame.bytecode_len = bytecode_len;
        frame.constants_ptr = constants_ptr;
        frame.constants_len = constants_len;
        frame.function_ptr = function_ptr;
        frame.with_env = function_ptr.map(|ptr| unsafe {
            (&*(ptr as *const crate::object::function::JSFunction)).with_env.clone()
        }).flatten();
        frame.ic_table_ptr = ic_table_ptr;
        frame.this_value = this_value;
        frame.dst_reg = dst_reg;
        frame.arg_count = arg_count;
        frame.super_ctor = JSValue::undefined();
        frame.new_target = JSValue::undefined();
        frame.this_initialized = true;
        frame.this_cell = None;
        frame.is_super_call = false;
        frame.is_constructor = is_constructor;
        frame.is_async = is_async;
        if let Some(fptr) = function_ptr {
            let func = unsafe { &*(fptr as *const crate::object::function::JSFunction) };
            frame.is_strict_frame = func.is_strict();
            frame.var_name_map = func.bytecode.as_ref().map_or(std::ptr::null(), |bc| {
                std::rc::Rc::as_ptr(&bc.var_name_to_slot)
            });
            frame.has_upvalues = func
                .upvalues
                .as_ref()
                .map_or(false, |uv| !uv.upvalue_slots.is_empty());
        } else {
            frame.is_strict_frame = false;
            frame.var_name_map = std::ptr::null();
            frame.has_upvalues = false;
        }
        frame.cached_arguments = None;
        frame.uses_arguments = save_args;
        if save_args {
            if !is_new_frame && !frame.saved_args.is_empty() {
                frame.saved_args.clear();
            }
            frame.saved_args.reserve(arg_regs.len());
            let caller_ptr = self.cached_registers_ptr;
            for r in arg_regs.iter() {
                frame
                    .saved_args
                    .push(unsafe { *caller_ptr.add(*r as usize) });
            }
        } else if !is_new_frame && !frame.saved_args.is_empty() {
            frame.saved_args.clear();
        }
        if !is_new_frame && frame.upvalue_sync_map.is_some() {
            frame.upvalue_sync_map = None;
            frame.upvalue_sync_bitset = 0;
        }
        self.pc = 0;
        self.cached_code_ptr = bytecode_ptr;
        self.cached_code_len = bytecode_len;
        self.cached_const_ptr = constants_ptr;
        self.cached_registers_base = base;
        self.cached_has_upvalue_sync = false;
        self.cached_upvalue_sync_bitset = 0;
        self.cached_registers_ptr = unsafe { self.registers.as_mut_ptr().add(base) };
        self.cached_ic_table_ptr = ic_table_ptr;
        if let Some(fptr) = function_ptr {
            if frame.has_upvalues {
                let func = unsafe { JSValue::function_from_ptr(fptr) };
                if let Some(uv) = func.upvalues.as_ref() {
                    let slots = &uv.upvalue_slots;
                    self.cached_upvalue_slot_ptr = slots.as_ptr();
                    self.cached_upvalues_len = slots.len();
                } else {
                    self.cached_upvalue_slot_ptr = std::ptr::null();
                    self.cached_upvalues_len = 0;
                }
            } else {
                self.cached_upvalue_slot_ptr = std::ptr::null();
                self.cached_upvalues_len = 0;
            }
        } else {
            self.cached_upvalue_slot_ptr = std::ptr::null();
            self.cached_upvalues_len = 0;
        }
    }

    #[inline(never)]
    fn scan_eval_bindings(
        frames: &[CallFrame],
        frame_index: usize,
        atom_id: u32,
    ) -> Option<JSValue> {
        for fi in (0..=frame_index).rev() {
            if let Some(ref eb) = frames[fi].eval_bindings {
                if let Some(val) = eb.get(&atom_id) {
                    return Some(*val);
                }
            }
        }
        None
    }

    #[inline(always)]
    fn pop_frame(&mut self, return_value: JSValue) {
        if self.frame_index == 0 {
            return;
        }

        if self.eval_binding_frames > 0 && self.frames[self.frame_index].eval_bindings.is_some() {
            self.frames[self.frame_index].eval_bindings = None;
            self.eval_binding_frames -= 1;
        }
        let frame = &self.frames[self.frame_index];
        let dst_reg = frame.dst_reg;
        let return_pc = frame.return_pc;
        let is_super_call = frame.is_super_call;

        self.frame_index -= 1;
        self.pc = return_pc;
        self.refresh_cache();
        let regs_ptr = self.cached_registers_ptr;
        unsafe {
            *regs_ptr.add(dst_reg as usize) = return_value;
        }

        let caller = &mut self.frames[self.frame_index];
        let mut pending_elements = None;
        let mut rebind_error = false;
        let mut propagate_cell = None;
        let mut propagate_value = JSValue::undefined();
        if is_super_call {
            if caller.this_initialized {
                rebind_error = true;
            } else {
                caller.this_initialized = true;
                if return_value.is_object_like() {
                    caller.this_value = return_value;
                    unsafe {
                        *regs_ptr = return_value;
                    }
                }
                if let Some(cell) = caller.this_cell.clone() {
                    cell.set((caller.this_value, true));
                    propagate_value = caller.this_value;
                    propagate_cell = Some(cell);
                }
                if let Some(fptr) = Self::private_elements_owner_fn(caller) {
                    pending_elements = Some((caller.this_value, fptr));
                }
            }
        }
        if let Some(ref sync_map) = caller.upvalue_sync_map {
            if let Some(cell) = sync_map.get(&dst_reg) {
                cell.set(return_value);
            }
        }
        if let Some(cell) = propagate_cell {
            self.propagate_super_this_to_enclosing_frames(&cell, propagate_value);
        }
        if rebind_error {
            self.pending_super_rebind_error = true;
        }
        if let Some((this_val, fptr)) = pending_elements {
            self.mark_instance_elements_pending(this_val, fptr);
        }
    }

    pub fn find_var_in_frame_stack(&self, name_atom: u32) -> Option<(usize, u16)> {
        for fi in (0..=self.frame_index).rev() {
            let frame = &self.frames[fi];
            if !frame.var_name_map.is_null() {
                let map = unsafe { &*frame.var_name_map };
                for &(an, slot, _) in map.iter().rev() {
                    if an == name_atom {
                        return Some((fi, slot));
                    }
                }
            }
        }
        None
    }

    #[inline(always)]
    pub fn get_var_in_caller_vm(&self, name_atom: u32) -> Option<JSValue> {
        if let Some(ptr) = self.caller_vm {
            let caller = unsafe { &*(ptr as *const VM) };
            for fi in (0..=caller.frame_index).rev() {
                let frame = &caller.frames[fi];
                if let Some(ref eb) = frame.eval_bindings {
                    if let Some(val) = eb.get(&name_atom) {
                        return Some(*val);
                    }
                }
                if !frame.var_name_map.is_null() {
                    let map = unsafe { &*frame.var_name_map };
                    for &(an, slot, _) in map.iter().rev() {
                        if an == name_atom {
                            let base = frame.registers_base;
                            let val = caller.registers[base + slot as usize];
                            return Some(val);
                        }
                    }
                }
            }
        }
        None
    }

    pub fn eval_private_owner_in_caller_vm(&self, name_atom: u32) -> Option<JSValue> {
        let name = crate::runtime::atom::Atom(name_atom);
        let mut vm_ptr = self.caller_vm?;
        loop {
            let vm = unsafe { &*(vm_ptr as *const VM) };
            for fi in (0..=vm.frame_index).rev() {
                let frame = &vm.frames[fi];
                if let Some(fptr) = frame.function_ptr {
                    let func =
                        unsafe { &*(fptr as *const crate::object::function::JSFunction) };
                    if let Some(uv) = func.upvalues.as_ref() {
                        if let Some(cell) = uv.upvalue_cells.get(&name) {
                            let value = cell.get();
                            if !value.is_undefined() {
                                return Some(value);
                            }
                        }
                    }
                    let home = func.home_object;
                    if home.is_object_like() {
                        let home_obj = home.as_object();
                        let owner_ptr = home.get_ptr();
                        if home_obj.has_private_kind(owner_ptr, name)
                            || home_obj.has_private_method(owner_ptr, name)
                            || home_obj.has_private_field(owner_ptr, name)
                        {
                            return Some(home);
                        }
                    }
                    if func.is_class_constructor {
                        let proto_ptr = func.cached_prototype_ptr as usize;
                        if proto_ptr != 0 {
                            let proto_obj =
                                unsafe { &*(proto_ptr as *const crate::object::object::JSObject) };
                            if proto_obj.has_private_kind(proto_ptr, name)
                                || proto_obj.has_private_method(proto_ptr, name)
                                || proto_obj.has_private_field(proto_ptr, name)
                            {
                                return Some(JSValue::new_object(proto_ptr));
                            }
                        }
                    }
                }
                if !frame.var_name_map.is_null() {
                    let map = unsafe { &*frame.var_name_map };
                    for &(an, slot, _) in map.iter() {
                        if an == name_atom {
                            let base = frame.registers_base;
                            let value = vm.registers[base + slot as usize];
                            if !value.is_undefined() {
                                return Some(value);
                            }
                        }
                    }
                }
            }
            vm_ptr = vm.caller_vm?;
        }
    }

    pub fn set_var_in_caller_vm(
        &mut self,
        ctx: &mut JSContext,
        name_atom: u32,
        value: JSValue,
    ) -> CallerVarSet {
        if let Some(ptr) = self.caller_vm {
            let caller = unsafe { &mut *(ptr as *mut VM) };
            for fi in (0..=caller.frame_index).rev() {
                let in_eb = caller.frames[fi]
                    .eval_bindings
                    .as_ref()
                    .map_or(false, |eb| eb.contains_key(&name_atom));
                if in_eb {
                    caller.frames[fi]
                        .eval_bindings
                        .as_mut()
                        .unwrap()
                        .insert(name_atom, value);

                    if fi == 0 {
                        let global = ctx.global();
                        if global.is_object() {
                            global.as_object_mut().set_cached(
                                crate::runtime::atom::Atom(name_atom),
                                value,
                                ctx.shape_cache_mut(),
                            );
                        }
                    }
                    return CallerVarSet::Stored;
                }
                if !caller.frames[fi].var_name_map.is_null() {
                    let map = unsafe { &*caller.frames[fi].var_name_map };
                    for &(an, slot, tag) in map.iter().rev() {
                        if an == name_atom {
                            if tag == 1 {
                                return CallerVarSet::Immutable;
                            }
                            let base = caller.frames[fi].registers_base;
                            if fi == caller.frame_index {
                                caller.registers[base + slot as usize] = value;
                            } else {
                                let saved = caller.frame_index;
                                caller.frame_index = fi;
                                caller.refresh_cache();
                                caller.set_reg(slot, value);
                                caller.frame_index = saved;
                                caller.refresh_cache();
                            }

                            if fi == 0 {
                                let global = ctx.global();
                                if global.is_object() {
                                    global.as_object_mut().set_cached(
                                        crate::runtime::atom::Atom(name_atom),
                                        value,
                                        ctx.shape_cache_mut(),
                                    );
                                }
                            }
                            return CallerVarSet::Stored;
                        }
                    }
                }
            }

            for fi in (0..=caller.frame_index).rev() {
                if caller.frames[fi].function_ptr.is_some() || fi == 0 {
                    if caller.frames[fi].eval_bindings.is_none() {
                        caller.frames[fi].eval_bindings =
                            Some(Box::new(std::collections::HashMap::new()));
                        caller.eval_binding_frames += 1;
                    }
                    caller.frames[fi]
                        .eval_bindings
                        .as_mut()
                        .unwrap()
                        .insert(name_atom, value);

                    if fi == 0 {
                        let global = ctx.global();
                        if global.is_object() {
                            global.as_object_mut().set_cached(
                                crate::runtime::atom::Atom(name_atom),
                                value,
                                ctx.shape_cache_mut(),
                            );
                        }
                    }
                    return CallerVarSet::Stored;
                }
            }
        }
        CallerVarSet::NotFound
    }

    pub fn init_var_in_caller_vm(
        &mut self,
        ctx: &mut JSContext,
        name_atom: u32,
        value: JSValue,
    ) -> bool {
        if let Some(ptr) = self.caller_vm {
            let caller = unsafe { &mut *(ptr as *mut VM) };
            for fi in (0..=caller.frame_index).rev() {
                if let Some(ref eb) = caller.frames[fi].eval_bindings {
                    if eb.contains_key(&name_atom) {
                        return true;
                    }
                }
                if !caller.frames[fi].var_name_map.is_null() {
                    let map = unsafe { &*caller.frames[fi].var_name_map };
                    if map.iter().any(|&(an, _, tag)| an == name_atom && tag == 0) {
                        return true;
                    }
                }
            }

            for fi in (0..=caller.frame_index).rev() {
                if caller.frames[fi].function_ptr.is_some() || fi == 0 {
                    if caller.frames[fi].eval_bindings.is_none() {
                        caller.frames[fi].eval_bindings =
                            Some(Box::new(std::collections::HashMap::new()));
                        caller.eval_binding_frames += 1;
                    }
                    caller.frames[fi]
                        .eval_bindings
                        .as_mut()
                        .unwrap()
                        .insert(name_atom, value);
                    if fi == 0 {
                        let global = ctx.global();
                        if global.is_object() {
                            global.as_object_mut().set_cached(
                                crate::runtime::atom::Atom(name_atom),
                                value,
                                ctx.shape_cache_mut(),
                            );
                        }
                    }
                    return true;
                }
            }
        }
        false
    }

    pub fn set_var_in_frame_stack(&mut self, ctx: &mut JSContext, name: &str, value: JSValue) {
        let atom = ctx.intern(name);
        for fi in (0..=self.frame_index).rev() {
            if !self.frames[fi].var_name_map.is_null() {
                let map = unsafe { &*self.frames[fi].var_name_map };
                for &(an, slot, _) in map.iter().rev() {
                    if an == atom.0 {
                        let base = self.frames[fi].registers_base;
                        if fi == self.frame_index {
                            self.registers[base + slot as usize] = value;
                        } else {
                            let saved_fi = self.frame_index;
                            self.frame_index = fi;
                            self.refresh_cache();
                            self.set_reg(slot, value);
                            self.frame_index = saved_fi;
                            self.refresh_cache();
                        }
                        return;
                    }
                }
            }
        }
        let global = ctx.global();
        if global.is_object() {
            global
                .as_object_mut()
                .set_cached(atom, value, ctx.shape_cache_mut());
        }
    }

    #[inline(always)]
    fn execute_call(
        &mut self,
        ctx: &mut JSContext,
        func_val: JSValue,
        this_val: JSValue,
        dst: u16,
        argc: u16,
        arg_regs: &[u16],
        obj_reg: u16,
        is_call_new: bool,
        is_call_method: bool,
        is_super_call: bool,
    ) -> Result<bool, String> {
        if func_val.is_function() {
            let ptr = func_val.get_ptr();
            let js_func = unsafe { JSValue::function_from_ptr(ptr) };
            if js_func.is_class_constructor && !is_call_new && !is_super_call {
                self.set_pending_type_error(
                    ctx,
                    "Class constructor cannot be invoked without 'new'",
                );
                return Ok(false);
            }
            if is_call_new
                && (js_func.is_arrow()
                    || js_func.is_async()
                    || js_func.is_generator()
                    || js_func.is_method)
            {
                let msg = format!(
                    "{} is not a constructor",
                    self.format_thrown_value(&func_val, ctx)
                );
                self.set_pending_type_error(ctx, &msg);
                return Ok(false);
            }
            if let Some(ref rb) = js_func.bytecode {
                let return_pc = self.pc;
                let caller_base = self.cached_registers_base;
                let uses_arguments = js_func.uses_arguments();

                if js_func.is_async() && !js_func.is_generator() {
                    let mut args_buf = [JSValue::undefined(); 16];
                    let mut args_vec = Vec::new();
                    let args: &[JSValue] = if argc as usize <= 16 {
                        for (i, r) in arg_regs.iter().enumerate() {
                            args_buf[i] = self.get_reg(*r);
                        }
                        &args_buf[..argc as usize]
                    } else {
                        args_vec.reserve(argc as usize);
                        for r in arg_regs {
                            args_vec.push(self.get_reg(*r));
                        }
                        &args_vec
                    };
                    let promise = start_async_call(ctx, js_func, ptr, this_val, args)?;
                    self.set_reg(dst, promise);
                    return Ok(false);
                }

                if js_func.is_generator() {
                    let mut args_buf = [JSValue::undefined(); 16];
                    let mut args_vec = Vec::new();
                    let args: &[JSValue] = if argc as usize <= 16 {
                        for (i, r) in arg_regs.iter().enumerate() {
                            args_buf[i] = self.get_reg(*r);
                        }
                        &args_buf[..argc as usize]
                    } else {
                        args_vec.reserve(argc as usize);
                        for r in arg_regs {
                            args_vec.push(self.get_reg(*r));
                        }
                        &args_vec
                    };

                    let saved_frame_index = self.frame_index;
                    let generator = self.instantiate_generator_object(ctx, rb, func_val, this_val, args)?;
                    self.frame_index = saved_frame_index;
                    if let Some(gen_val) = generator {
                        self.set_reg(dst, gen_val);
                    }
                    return Ok(false);
                }

                if is_call_new && rb.is_simple_constructor {
                    if let Some(cached_shape) = rb.cached_constructor_final_shape {
                        if this_val.is_object() {
                            let obj = unsafe { JSValue::object_from_ptr_mut(this_val.get_ptr()) };
                            obj.fast_init_from_simple_constructor(
                                rb.simple_constructor_props
                                    .iter()
                                    .map(|&(atom, arg_idx, _)| {
                                        let value = if (arg_idx as usize) < arg_regs.len() {
                                            self.get_reg(arg_regs[arg_idx as usize])
                                        } else {
                                            JSValue::undefined()
                                        };
                                        (atom, value)
                                    }),
                                cached_shape,
                            );
                        }
                        self.set_reg(dst, this_val);
                        return Ok(false);
                    }
                }
                let fn_this = if js_func.is_arrow() {
                    js_func.arrow_this
                } else if !js_func.is_strict() && (this_val.is_undefined() || this_val.is_null()) {
                    ctx.global()
                } else {
                    this_val
                };
                self.push_frame_from_arg_regs_raw(
                    ctx,
                    rb.locals_count,
                    rb.effective_code_ptr(),
                    rb.effective_code_len(),
                    rb.effective_const_ptr(),
                    rb.effective_const_len(),
                    return_pc,
                    Some(ptr),
                    rb.effective_ic_table_ptr(),
                    fn_this,
                    dst,
                    argc,
                    is_call_new,
                    js_func.is_async(),
                    caller_base,
                    arg_regs,
                    uses_arguments,
                );
                if js_func.is_arrow() {
                    self.frames[self.frame_index].this_cell = js_func.arrow_this_cell.clone();
                    self.frames[self.frame_index].super_ctor = js_func.arrow_super;
                    self.frames[self.frame_index].super_base = js_func.arrow_super_base;
                    if let Some(cell) = &js_func.arrow_this_cell {
                        let (tv, init) = cell.get();
                        self.frames[self.frame_index].this_value = tv;
                        self.frames[self.frame_index].this_initialized = init;
                    } else {
                        self.frames[self.frame_index].this_initialized =
                            js_func.arrow_this_initialized;
                    }
                } else {
                    let super_key = ctx.common_atoms.__super__;
                    if let Some(super_val) = js_func.base.get(super_key) {
                        self.frames[self.frame_index].super_ctor = super_val;
                    }
                    if js_func.is_derived_constructor {
                        self.frames[self.frame_index].this_initialized = false;
                    }
                }
                if is_call_new {
                    self.frames[self.frame_index].new_target = func_val;
                }
                if js_func.is_class_constructor && !js_func.is_derived_constructor {
                    self.mark_instance_elements_pending(fn_this, ptr);
                }
                if !js_func.home_object.is_undefined() {
                    if js_func.home_object.is_object_like() {
                        let home = js_func.home_object.as_object();
                        let base = home
                            .prototype
                            .map(|p| JSValue::new_object(p as usize))
                            .unwrap_or(JSValue::undefined());
                        self.frames[self.frame_index].super_base = base;
                    }
                } else {
                    let sc = self.frames[self.frame_index].super_ctor;
                    if sc.is_object_like() {
                        let base = sc
                            .as_object()
                            .get(ctx.common_atoms.prototype)
                            .unwrap_or(JSValue::undefined());
                        self.frames[self.frame_index].super_base = base;
                    }
                }
                Ok(true)
            } else if js_func.is_builtin() {
                if is_call_new
                    && !ctx.builtin_is_constructor(
                        &js_func
                            .builtin_atom
                            .map(|ba| ctx.get_atom_str(ba).to_string())
                            .unwrap_or_default(),
                    )
                {
                    self.set_pending_type_error(ctx, "not a constructor");
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => return Ok(false),
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(ExecutionOutcome::Complete(v)) => {
                                self.set_reg(dst, v);
                                return Ok(false);
                            }
                            ThrowDispatch::AsyncComplete(ExecutionOutcome::Yield(v)) => {
                                self.set_reg(dst, v);
                                return Ok(false);
                            }
                        }
                    }
                    return Ok(false);
                }
                let mut args_buf = [JSValue::undefined(); 17];
                let builtin_name = js_func
                    .builtin_atom
                    .map(|ba| ctx.get_atom_str(ba).to_string())
                    .unwrap_or_default();
                if is_call_new && builtin_name == "symbol_constructor" {
                    self.set_pending_type_error(ctx, "Symbol is not a constructor");
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => return Ok(false),
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            _ => {}
                        }
                    }
                    return Ok(false);
                }
                let needs_callee = Self::builtin_needs_callee(&builtin_name);
                let pass_this = is_call_method && !needs_callee && js_func.builtin_needs_this();
                let builtin_arg_count = argc as usize
                    + if pass_this { 1 } else { 0 }
                    + if needs_callee { 1 } else { 0 };
                let mut args_vec = Vec::new();
                let args: &[JSValue] = if builtin_arg_count <= 16 {
                    let mut idx = 0;
                    if needs_callee {
                        args_buf[idx] = func_val;
                        idx += 1;
                    }
                    if pass_this {
                        args_buf[0] = self.get_reg(obj_reg);
                        idx += 1;
                    }
                    for (i, r) in arg_regs.iter().enumerate() {
                        args_buf[idx + i] = self.get_reg(*r);
                    }
                    &args_buf[..builtin_arg_count]
                } else {
                    args_vec.reserve(builtin_arg_count);
                    if needs_callee {
                        args_vec.push(func_val);
                    }
                    if pass_this {
                        args_vec.push(self.get_reg(obj_reg));
                    }
                    for r in arg_regs {
                        args_vec.push(self.get_reg(*r));
                    }
                    &args_vec
                };
                let prev_call_with_new = ctx.is_call_with_new();
                ctx.set_call_with_new(is_call_new);
                let result = if let Some(bf) = js_func.builtin_func {
                    ctx.call_builtin_direct(bf, args)
                } else if let Some(ba) = js_func.builtin_atom {
                    let name = ctx.get_atom_str(ba).to_string();
                    ctx.call_builtin(&name, args)
                } else {
                    JSValue::undefined()
                };
                ctx.set_call_with_new(prev_call_with_new);
                if let Some(exc) = ctx.pending_exception.take() {
                    match self.dispatch_throw_value(ctx, exc) {
                        ThrowDispatch::Caught => return Ok(false),
                        ThrowDispatch::Uncaught(e) => return Err(e),
                        ThrowDispatch::AsyncComplete(o) => match o {
                            ExecutionOutcome::Complete(v) => {
                                self.set_reg(dst, v);
                                return Ok(false);
                            }
                            ExecutionOutcome::Yield(v) => {
                                self.set_reg(dst, v);
                                return Ok(false);
                            }
                        },
                    }
                }
                if is_call_new && result.is_function() {
                    let proto_key = ctx.common_atoms.prototype;
                    if let Some(proto_val) = func_val.as_object().get(proto_key) {
                        if proto_val.is_object_like() {
                            result.as_function_mut().base.prototype = Some(
                                proto_val.get_ptr()
                                    as *mut crate::object::object::JSObject,
                            );
                        }
                    }
                }
                if is_call_new && !result.is_object_like() {
                    if this_val.is_object() {
                        this_val
                            .as_object_mut()
                            .set(ctx.common_atoms.__value__, result);
                        if result.is_string() {
                            let s = ctx.get_atom_str(result.get_atom()).to_string();
                            let char_count = s.chars().count();
                            this_val.as_object_mut().define_property(
                                ctx.common_atoms.length,
                                crate::object::object::PropertyDescriptor {
                                    value: Some(JSValue::new_int(char_count as i64)),
                                    writable: false,
                                    enumerable: false,
                                    configurable: false,
                                    get: None,
                                    set: None,
                                },
                            );
                            if char_count > 0 {
                                let elements = this_val.as_object_mut().ensure_elements();
                                for c in s.chars() {
                                    let atom = ctx.intern(&c.to_string());
                                    elements.push(JSValue::new_string(atom));
                                }
                            }
                        }
                    }
                    self.set_reg(dst, this_val);
                } else {
                    self.set_reg(dst, result);
                }
                Ok(false)
            } else {
                Ok(false)
            }
        } else if func_val.is_object() && is_call_new {
            let obj = func_val.as_object();
            if let Some(bound_fn) = obj.get(ctx.common_atoms.__boundFn) {
                let bound_args_val = obj.get(ctx.common_atoms.__boundArgs).filter(|v| v.is_object());
                let bound_len = bound_args_val
                    .and_then(|v| v.as_object().get(ctx.common_atoms.length))
                    .map(|v| v.get_int() as usize)
                    .unwrap_or(0);
                let mut actual_args = Vec::with_capacity(bound_len + argc as usize);
                if let Some(ref bound_args_val) = bound_args_val {
                    let bound_args_obj = bound_args_val.as_object();
                    for i in 0..bound_len {
                        let key = self.int_atom(i, ctx);
                        if let Some(arg_val) = bound_args_obj.get(key) {
                            actual_args.push(arg_val);
                        }
                    }
                }
                for r in arg_regs {
                    actual_args.push(self.get_reg(*r));
                }
                if !Self::value_is_constructor(ctx, bound_fn) {
                    let msg = format!(
                        "{} is not a constructor",
                        self.format_thrown_value(&func_val, ctx)
                    );
                    self.set_pending_type_error(ctx, &msg);
                    return Ok(false);
                }
                let mut new_obj = crate::object::object::JSObject::new();
                if bound_fn.is_function() {
                    let target_func = bound_fn.as_function();
                    let proto_atom = ctx.common_atoms.prototype;
                    if let Some(proto_val) = target_func.base.get(proto_atom) {
                        if proto_val.is_object_like() {
                            let proto_ptr = proto_val.get_ptr();
                            new_obj.prototype = Some(proto_ptr as *mut crate::object::object::JSObject);
                        }
                    }
                }
                let new_ptr = Box::into_raw(Box::new(new_obj)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(new_ptr);
                let new_val = JSValue::new_object(new_ptr);
                let result =
                    self.call_function_with_this_impl(ctx, bound_fn, new_val, &actual_args, true);
                match result {
                    Ok(r) => {
                        let final_val = if r.is_object_like() { r } else { new_val };
                        self.set_reg(dst, final_val);
                    }
                    Err(_) => {
                        self.set_pending_type_error(ctx, "bound function construct failed");
                    }
                }
                Ok(false)
            } else {
                let msg = format!(
                    "{} is not a function",
                    self.format_thrown_value(&func_val, ctx)
                );
                self.set_pending_type_error(ctx, &msg);
                Ok(false)
            }
        } else if func_val.is_object() && !is_call_new {
            let mut args_buf = [JSValue::undefined(); 16];
            let mut args_vec = Vec::new();
            let call_args: &[JSValue] = if argc as usize <= 16 {
                for (i, r) in arg_regs.iter().enumerate() {
                    args_buf[i] = self.get_reg(*r);
                }
                &args_buf[..argc as usize]
            } else {
                args_vec.reserve(argc as usize);
                for r in arg_regs.iter() {
                    args_vec.push(self.get_reg(*r));
                }
                &args_vec
            };
            match self.call_function_with_this(ctx, func_val, this_val, call_args) {
                Ok(result) => {
                    self.set_reg(dst, result);
                    Ok(false)
                }
                Err(_) => {
                    let msg = format!(
                        "{} is not a function",
                        self.format_thrown_value(&func_val, ctx)
                    );
                    self.set_pending_type_error(ctx, &msg);
                    if let Some(exc) = self.pending_throw.take() {
                        let disp = self.dispatch_throw_value(ctx, exc);
                        match disp {
                            ThrowDispatch::Caught => Ok(false),
                            ThrowDispatch::Uncaught(e) => Err(e),
                            ThrowDispatch::AsyncComplete(o) => match o {
                                ExecutionOutcome::Complete(v) => {
                                    self.set_reg(dst, v);
                                    Ok(false)
                                }
                                _ => Err("call error".to_string()),
                            },
                        }
                    } else {
                        Ok(false)
                    }
                }
            }
        } else {
            let msg = format!(
                "{} is not a function",
                self.format_thrown_value(&func_val, ctx)
            );
            self.set_pending_type_error(ctx, &msg);
            Ok(false)
        }
    }

    #[inline(always)]
    fn read_u8(&mut self) -> u8 {
        let pc = self.pc;
        let val = unsafe { *self.cached_code_ptr.add(pc) };
        self.pc = pc + 1;
        val
    }

    #[inline(always)]
    fn read_i64(&mut self) -> i64 {
        let val =
            unsafe { std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const i64) };
        self.pc += 8;
        val
    }

    #[inline(always)]
    fn read_i32(&mut self) -> i32 {
        let val =
            unsafe { std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const i32) };
        self.pc += 4;
        val
    }

    #[inline(always)]
    fn read_u32(&mut self) -> u32 {
        self.read_i32() as u32
    }

    #[inline(always)]
    fn get_reg(&self, idx: u16) -> JSValue {
        if self.cached_has_upvalue_sync {
            return self.get_reg_upvalue_slow(idx);
        }
        unsafe { *self.cached_registers_ptr.add(idx as usize) }
    }

    #[cold]
    #[inline(never)]
    fn get_reg_upvalue_slow(&self, idx: u16) -> JSValue {
        let captured = if idx < 64 {
            self.cached_upvalue_sync_bitset & (1u64 << idx) != 0
        } else {
            true
        };
        if captured {
            let frame = &self.frames[self.frame_index];
            if let Some(cell) = frame.upvalue_sync_map.as_ref().and_then(|m| m.get(&idx)) {
                return cell.get();
            }
        }
        unsafe { *self.cached_registers_ptr.add(idx as usize) }
    }

    #[inline(always)]
    fn set_reg(&mut self, idx: u16, val: JSValue) {
        unsafe {
            *self.cached_registers_ptr.add(idx as usize) = val;
        }
        if self.cached_has_upvalue_sync {
            self.set_reg_upvalue_slow(idx, val);
        }
    }

    #[cold]
    #[inline(never)]
    fn set_reg_upvalue_slow(&mut self, idx: u16, val: JSValue) {
        let captured = if idx < 64 {
            self.cached_upvalue_sync_bitset & (1u64 << idx) != 0
        } else {
            true
        };
        if captured {
            let frame = &self.frames[self.frame_index];
            if let Some(cell) = frame.upvalue_sync_map.as_ref().and_then(|m| m.get(&idx)) {
                cell.set(val);
            }
        }
    }

    fn format_stack_trace(&self, ctx: &JSContext) -> String {
        let mut frames = Vec::new();
        for fi in (0..=self.frame_index).rev() {
            let frame = &self.frames[fi];
            let func_name = if let Some(fptr) = frame.function_ptr {
                let func_val = JSValue::new_function(fptr);
                let js_func = func_val.as_function();
                ctx.get_atom_str(js_func.name).to_string()
            } else {
                "<top-level>".to_string()
            };
            let mut location = crate::compiler::location::SourceLocation::unknown();
            if let Some(fptr) = frame.function_ptr {
                let func_val = JSValue::new_function(fptr);
                let js_func = func_val.as_function();
                location.filename = js_func.source_filename.clone();
                if let Some(ref table) = js_func.line_number_table {
                    if let Some(line) = table.lookup_line(frame.current_pc as u32) {
                        location.line = line;
                    }
                }
            }
            frames.push(crate::compiler::location::FrameInfo::new(
                func_name, location,
            ));
        }
        let mut result = String::new();
        for frame in frames.iter().rev() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&format!(
                "  at {} ({}:{}:{})",
                frame.function_name,
                frame.location.filename,
                frame.location.line,
                frame.location.column
            ));
        }
        result
    }

    #[cold]
    fn current_super_constructor(&self) -> JSValue {
        let frame = &self.frames[self.frame_index];
        if let Some(fptr) = frame.function_ptr {
            let func = unsafe { &*(fptr as *const crate::object::function::JSFunction) };
            if !func.is_arrow() && func.is_derived_constructor {
                return func
                    .base
                    .prototype
                    .map(|proto| {
                        let proto_obj = unsafe {
                            &*(proto as *const crate::object::object::JSObject)
                        };
                        if proto_obj.is_function_object {
                            JSValue::new_function(proto as usize)
                        } else {
                            JSValue::new_object(proto as usize)
                        }
                    })
                    .unwrap_or_else(JSValue::undefined);
            }
        }
        frame.super_ctor
    }

    fn private_elements_owner_fn(frame: &CallFrame) -> Option<usize> {
        let fptr = frame.function_ptr?;
        let func =
            unsafe { &*(fptr as *const crate::object::function::JSFunction) };
        let has_elements = func.has_private_elements || !func.instance_fields.is_empty();
        if func.is_arrow() {
            if func.arrow_enclosing_ctor_ptr != 0 {
                Some(func.arrow_enclosing_ctor_ptr)
            } else {
                None
            }
        } else if has_elements {
            Some(fptr)
        } else {
            None
        }
    }

    fn mark_instance_elements_pending(&mut self, this_val: JSValue, func_ptr: usize) {
        if !this_val.is_object_like() {
            return;
        }
        let func = unsafe { &*(func_ptr as *const crate::object::function::JSFunction) };
        if func.has_private_elements || !func.instance_fields.is_empty() {
            self.pending_instance_elements = Some((this_val, func_ptr));
        }
    }

    fn run_instance_elements(
        &mut self,
        ctx: &mut JSContext,
        this_val: JSValue,
        func_ptr: usize,
    ) -> Result<(), String> {
        if !this_val.is_object_like() {
            self.set_pending_type_error(ctx, "Cannot initialize elements on non-object");
            return Err("".to_string());
        }
        let (has_private_elements, owner, field_count) = {
            let func = unsafe { &*(func_ptr as *const crate::object::function::JSFunction) };
            (
                func.has_private_elements,
                func.cached_prototype_ptr as usize,
                func.instance_fields.len(),
            )
        };
        if has_private_elements && owner != 0 {
            let obj = this_val.as_object();
            if obj.has_private_brand(owner) {
                self.set_pending_type_error(
                    ctx,
                    "Cannot initialize private elements twice",
                );
                return Err("".to_string());
            }
            if !obj.extensible() {
                self.set_pending_type_error(ctx, "Cannot add private elements to object");
                return Err("".to_string());
            }
            this_val.as_object_mut().add_private_brand(owner);
        }
        for i in 0..field_count {
            let (key, private_owner, private_name, initializer) = {
                let func =
                    unsafe { &*(func_ptr as *const crate::object::function::JSFunction) };
                let field = &func.instance_fields[i];
                (
                    field.key,
                    field.private_owner,
                    field.private_name,
                    field.initializer,
                )
            };
            let value = if initializer.is_undefined() {
                JSValue::undefined()
            } else {
                self.call_function_with_this(ctx, initializer, this_val, &[])?
            };
            if private_name.0 != 0 {
                let owner_ptr = private_owner.get_ptr();
                let obj = this_val.as_object_mut();
                if !obj.extensible() || obj.has_private_field(owner_ptr, private_name) {
                    self.set_pending_type_error(
                        ctx,
                        "Cannot add private field to object",
                    );
                    return Err("".to_string());
                }
                obj.set_private_field(owner_ptr, private_name, value);
                if private_owner.is_object_like() {
                    private_owner.as_object_mut().define_private_kind(
                        owner_ptr,
                        private_name,
                        crate::object::object::PrivateKind::Field,
                    );
                }
            } else {
                let js_obj = this_val.as_object_mut();
                if !js_obj.extensible() {
                    self.set_pending_type_error(
                        ctx,
                        "Cannot add property to non-extensible object",
                    );
                    return Err("".to_string());
                }
                let atom = if key.is_symbol() {
                    crate::runtime::atom::Atom(0x40000000 | key.get_symbol_id())
                } else {
                    key.get_atom()
                };
                if js_obj
                    .get_own_descriptor(atom)
                    .map_or(false, |d| !d.configurable)
                {
                    self.set_pending_type_error(
                        ctx,
                        "Cannot redefine non-configurable property",
                    );
                    return Err("".to_string());
                }
                let desc = crate::object::object::PropertyDescriptor {
                    value: Some(value),
                    writable: true,
                    enumerable: true,
                    configurable: true,
                    get: None,
                    set: None,
                };
                js_obj.define_property(atom, desc);
            }
        }
        Ok(())
    }

    fn propagate_super_this_to_enclosing_frames(
        &mut self,
        cell: &std::rc::Rc<std::cell::Cell<(JSValue, bool)>>,
        this_value: JSValue,
    ) {
        for idx in (0..self.frame_index).rev() {
            let matches = self.frames[idx]
                .this_cell
                .as_ref()
                .map_or(false, |frame_cell| std::rc::Rc::ptr_eq(frame_cell, cell));
            if matches {
                self.frames[idx].this_initialized = true;
                self.frames[idx].this_value = this_value;
                let base = self.frames[idx].registers_base;
                if base < self.registers.len() {
                    self.registers[base] = this_value;
                }
            }
        }
    }

    fn bind_super_call_result(&mut self, dst: u16) {
        if self.frames[self.frame_index].this_initialized {
            self.pending_super_rebind_error = true;
            return;
        }
        let result = self.get_reg(dst);
        self.frames[self.frame_index].this_initialized = true;
        if result.is_object_like() {
            self.frames[self.frame_index].this_value = result;
            self.set_reg(0, result);
        }
        if let Some(cell) = self.frames[self.frame_index].this_cell.clone() {
            let this_value = self.frames[self.frame_index].this_value;
            let this_initialized = self.frames[self.frame_index].this_initialized;
            cell.set((this_value, this_initialized));
            self.propagate_super_this_to_enclosing_frames(&cell, this_value);
        }
        if let Some(fptr) = Self::private_elements_owner_fn(&self.frames[self.frame_index]) {
            let this_val = self.frames[self.frame_index].this_value;
            self.mark_instance_elements_pending(this_val, fptr);
        }
    }

    fn bind_super_call_builtin_result(&mut self, dst: u16) {
        if self.frames[self.frame_index].this_initialized {
            self.pending_super_rebind_error = true;
            return;
        }
        let result = self.get_reg(dst);
        let this_val = self.frames[self.frame_index].this_value;
        self.frames[self.frame_index].this_initialized = true;
        if result.is_object_like() {
            if this_val.is_object_like() {
                let proto = this_val.as_object().prototype;
                if let Some(proto_ptr) = proto {
                    result.as_object_mut().prototype = Some(proto_ptr);
                }
            }
            self.frames[self.frame_index].this_value = result;
            self.set_reg(0, result);
        }
        if let Some(cell) = self.frames[self.frame_index].this_cell.clone() {
            let this_value = self.frames[self.frame_index].this_value;
            let this_initialized = self.frames[self.frame_index].this_initialized;
            cell.set((this_value, this_initialized));
            self.propagate_super_this_to_enclosing_frames(&cell, this_value);
        }
        if let Some(fptr) = Self::private_elements_owner_fn(&self.frames[self.frame_index]) {
            let this_val = self.frames[self.frame_index].this_value;
            self.mark_instance_elements_pending(this_val, fptr);
        }
    }

    pub fn set_pending_type_error(&mut self, ctx: &mut JSContext, msg: &str) {
        use crate::object::object::JSObject;
        let mut err = JSObject::new();
        let name_atom = ctx.intern("name");
        let msg_atom = ctx.intern("message");
        let type_error_atom = ctx.intern("TypeError");
        let msg_str_atom = ctx.intern(msg);
        err.set(name_atom, JSValue::new_string(type_error_atom));
        err.set(msg_atom, JSValue::new_string(msg_str_atom));
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.pending_throw = Some(JSValue::new_object(ptr));
    }

    fn rethrow_last_caught(&mut self, ctx: &mut JSContext) {
        let caught = self.last_caught_exception.take();
        let host = ctx.pending_exception.take();
        if let Some(exc) = caught.or(host) {
            self.pending_throw = Some(exc);
        }
    }

    fn adopt_pending_host_exception(&mut self, ctx: &mut JSContext) {
        if self.pending_throw.is_none() {
            if let Some(exc) = ctx.pending_exception.take() {
                self.pending_throw = Some(exc);
            }
        }
    }

    fn is_bound_function_value(&self, ctx: &JSContext, value: JSValue) -> bool {
        value.is_object() && value.as_object().get_own(ctx.common_atoms.__boundFn).is_some()
    }

    fn is_callable_value(&self, ctx: &JSContext, value: JSValue) -> bool {
        if value.is_function() || self.is_bound_function_value(ctx, value) {
            return true;
        }
        if value.is_object() {
            if let Some(function_prototype) = ctx.get_function_prototype() {
                return value.get_ptr() as *mut crate::object::object::JSObject
                    == function_prototype;
            }
        }
        false
    }

    fn has_instance_atom(&mut self, ctx: &mut JSContext) -> crate::runtime::atom::Atom {
        if self.cached_has_instance_atom.0 != 0 {
            return self.cached_has_instance_atom;
        }
        let sym = crate::builtins::symbol::get_symbol_has_instance(ctx);
        let atom = if sym.is_symbol() {
            crate::runtime::atom::Atom(0x40000000 | sym.get_symbol_id())
        } else {
            crate::runtime::atom::Atom(0)
        };
        self.cached_has_instance_atom = atom;
        atom
    }

    fn get_property_value_full(
        &mut self,
        ctx: &mut JSContext,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
    ) -> Option<JSValue> {
        if !obj_val.is_object() && !obj_val.is_function() {
            return None;
        }
        let base_ptr = if obj_val.is_function() {
            std::ptr::addr_of!(obj_val.as_function().base)
        } else {
            obj_val.get_ptr() as *const crate::object::object::JSObject
        };
        let mut current = Some(base_ptr as *mut crate::object::object::JSObject);
        let mut depth = 0u32;
        while let Some(ptr) = current {
            if ptr.is_null() || depth > 1000 {
                break;
            }
            depth += 1;
            let pobj = unsafe { &*ptr };
            if let Some(getter) = pobj.get_own_accessor_value(atom) {
                if getter.is_function() {
                    match self.call_function_with_this(ctx, getter, obj_val, &[]) {
                        Ok(value) => return Some(value),
                        Err(_) => {
                            self.rethrow_last_caught(ctx);
                            return None;
                        }
                    }
                }
                return Some(JSValue::undefined());
            }
            if let Some(value) = pobj.get_own(atom) {
                return Some(value);
            }
            current = pobj.prototype;
        }
        None
    }

    pub fn ordinary_has_instance(
        &mut self,
        ctx: &mut JSContext,
        ctor_val: JSValue,
        obj_val: JSValue,
    ) -> Option<bool> {
        if !self.is_callable_value(ctx, ctor_val) {
            return Some(false);
        }
        if self.is_bound_function_value(ctx, ctor_val) {
            let bound_fn = ctor_val
                .as_object()
                .get_own(ctx.common_atoms.__boundFn)
                .unwrap_or_else(JSValue::undefined);
            return self.instanceof_operator(ctx, bound_fn, obj_val);
        }
        if !obj_val.is_object() && !obj_val.is_function() {
            return Some(false);
        }
        let proto_atom = ctx.common_atoms.prototype;
        let proto_val = match self.get_property_value_full(ctx, ctor_val, proto_atom) {
            Some(value) => value,
            None => {
                if self.pending_throw.is_some() {
                    return None;
                }
                JSValue::undefined()
            }
        };
        if !proto_val.is_object() && !proto_val.is_function() {
            self.set_pending_type_error(
                ctx,
                "Function has non-object prototype in instanceof check",
            );
            return None;
        }
        let proto_ptr = proto_val.get_ptr() as *mut crate::object::object::JSObject;
        let obj_ptr = obj_val.get_ptr() as *const crate::object::object::JSObject;
        let mut current = unsafe { (*obj_ptr).prototype };
        let mut depth = 0u32;
        while let Some(p) = current {
            if std::ptr::eq(p, proto_ptr) {
                return Some(true);
            }
            current = unsafe { (*p).prototype };
            depth += 1;
            if depth > 1000 {
                break;
            }
        }
        Some(false)
    }

    fn instanceof_operator(
        &mut self,
        ctx: &mut JSContext,
        ctor_val: JSValue,
        obj_val: JSValue,
    ) -> Option<bool> {
        let hi_atom = self.has_instance_atom(ctx);
        let handler = match self.get_property_value_full(ctx, ctor_val, hi_atom) {
            Some(value) => value,
            None => {
                if self.pending_throw.is_some() {
                    return None;
                }
                JSValue::undefined()
            }
        };
        if !handler.is_null_or_undefined() {
            if !self.is_callable_value(ctx, handler) {
                let msg = format!(
                    "{} is not a function",
                    self.format_thrown_value(&handler, ctx)
                );
                self.set_pending_type_error(ctx, &msg);
                return None;
            }
            match self.call_function_with_this(ctx, handler, ctor_val, &[obj_val]) {
                Ok(value) => return Some(value.is_truthy()),
                Err(_) => {
                    self.rethrow_last_caught(ctx);
                    return None;
                }
            }
        }
        if !self.is_callable_value(ctx, ctor_val) {
            self.set_pending_type_error(ctx, "Right-hand side of 'instanceof' is not callable");
            return None;
        }
        self.ordinary_has_instance(ctx, ctor_val, obj_val)
    }

    pub fn take_pending_exception(&mut self, ctx: &mut JSContext) -> Option<JSValue> {
        let caught = self.last_caught_exception.take();
        let host = ctx.pending_exception.take();
        let pending = self.pending_throw.take();
        caught.or(host).or(pending)
    }

    pub fn get_import_attributes(
        &mut self,
        ctx: &mut JSContext,
        options: JSValue,
    ) -> Result<Option<String>, JSValue> {
        if options.is_undefined() {
            return Ok(None);
        }
        if !options.is_object_like() {
            self.set_pending_type_error(ctx, "Import options must be an object");
            return Err(self
                .pending_throw
                .take()
                .unwrap_or_else(JSValue::undefined));
        }
        let with_atom = ctx.intern("with");
        let with_value = match self.get_prop_value(ctx, options, with_atom) {
            Ok(value) => value,
            Err(_) => {
                return Err(self
                    .take_pending_exception(ctx)
                    .unwrap_or_else(JSValue::undefined));
            }
        };
        if with_value.is_undefined() {
            return Ok(None);
        }
        if !with_value.is_object_like() {
            self.set_pending_type_error(ctx, "Import attributes must be an object");
            return Err(self
                .pending_throw
                .take()
                .unwrap_or_else(JSValue::undefined));
        }
        let keys = self.own_enumerable_keys_in_order(ctx, with_value);
        let mut attribute_type = None;
        for atom in keys {
            if atom.0 & 0x40000000 != 0 {
                continue;
            }
            let value = match self.get_prop_value(ctx, with_value, atom) {
                Ok(value) => value,
                Err(_) => {
                    return Err(self
                        .take_pending_exception(ctx)
                        .unwrap_or_else(JSValue::undefined));
                }
            };
            if !value.is_string() {
                self.set_pending_type_error(ctx, "Import attribute values must be strings");
                return Err(self
                    .pending_throw
                    .take()
                    .unwrap_or_else(JSValue::undefined));
            }
            let name = ctx.get_atom_str(atom).to_string();
            if name == "type" {
                attribute_type = Some(ctx.get_atom_str(value.get_atom()).to_string());
            }
        }
        Ok(attribute_type)
    }

    fn invoke_iterator_return(
        &mut self,
        ctx: &mut JSContext,
        iter_val: JSValue,
    ) -> Option<JSValue> {
        if !iter_val.is_object() {
            return None;
        }
        let iterator = match Self::iterator_record_parts(ctx, iter_val) {
            Some((iterator, _, _)) => iterator,
            None => iter_val,
        };
        let return_atom = ctx.intern("return");
        let ret_fn = match self.get_prop_value(ctx, iterator, return_atom) {
            Ok(v) => v,
            Err(_) => {
                self.rethrow_last_caught(ctx);
                return self.take_pending_exception(ctx);
            }
        };
        if ret_fn.is_undefined() || ret_fn.is_null() {
            return None;
        }
        if !ret_fn.is_function() {
            self.set_pending_type_error(ctx, "Iterator return is not a function");
            return self.take_pending_exception(ctx);
        }
        match self.call_function_with_this(ctx, ret_fn, iterator, &[]) {
            Ok(result) => {
                if result.is_object_like() {
                    None
                } else {
                    self.set_pending_type_error(ctx, "Iterator result is not an object");
                    self.take_pending_exception(ctx)
                }
            }
            Err(_) => {
                self.rethrow_last_caught(ctx);
                self.take_pending_exception(ctx)
            }
        }
    }

    fn yield_star_step(
        &mut self,
        ctx: &mut JSContext,
        iter_reg: u16,
        value_reg: u16,
        done_reg: u16,
        returning_reg: u16,
    ) -> Result<(), String> {
        let kind = self.generator_resume_kind;
        self.generator_resume_kind = 0;
        self.set_reg(returning_reg, JSValue::bool(false));
        let iter_val = self.get_reg(iter_reg);
        let received = self.get_reg(value_reg);
        if !iter_val.is_object_like() {
            self.set_pending_type_error(ctx, "yield* iterator is not an object");
            return Err("yield* iterator is not an object".to_string());
        }
        if kind == 0 {
            let arr_atom = ctx.common_atoms.__iter_arr__;
            let has_fast_path = unsafe { iter_val.as_object() }.get(arr_atom).is_some();
            if has_fast_path {
                match self.op_iterator_next(
                    ctx,
                    usize::MAX,
                    value_reg,
                    done_reg,
                    iter_val,
                    None,
                    false,
                    true,
                    None,
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                }
                if !self.get_reg(done_reg).is_truthy() {
                    let v = self.get_reg(value_reg);
                    let mut result = crate::object::object::JSObject::new();
                    result.set(ctx.intern("value"), v);
                    result.set(ctx.intern("done"), JSValue::bool(false));
                    let ptr = Box::into_raw(Box::new(result)) as usize;
                    self.set_reg(value_reg, JSValue::new_object(ptr));
                }
                return Ok(());
            }
        }
        let target = match Self::iterator_record_parts(ctx, iter_val) {
            Some((iterator, _, _)) => iterator,
            None => iter_val,
        };
        let done_atom = ctx.intern("done");
        let value_atom = ctx.intern("value");
        let result: JSValue;
        match kind {
            1 => {
                let throw_atom = ctx.intern("throw");
                let throw_fn = match self.get_prop_value(ctx, target, throw_atom) {
                    Ok(v) => v,
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                };
                if throw_fn.is_undefined() || throw_fn.is_null() {
                    if let Some(exc) = self.invoke_iterator_return(ctx, target) {
                        self.pending_throw = Some(exc);
                        return Err("yield* iterator has no throw method".to_string());
                    }
                    self.set_pending_type_error(ctx, "yield* iterator has no throw method");
                    return Err("yield* iterator has no throw method".to_string());
                }
                if !throw_fn.is_function() {
                    self.set_pending_type_error(ctx, "yield* iterator throw is not a function");
                    return Err("yield* iterator throw is not a function".to_string());
                }
                result = match self.call_function_with_this(ctx, throw_fn, target, &[received]) {
                    Ok(v) => v,
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                };
            }
            2 => {
                let return_atom = ctx.intern("return");
                let return_fn = match self.get_prop_value(ctx, target, return_atom) {
                    Ok(v) => v,
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                };
                if return_fn.is_undefined() || return_fn.is_null() {
                    self.set_reg(value_reg, received);
                    self.set_reg(done_reg, JSValue::bool(true));
                    self.set_reg(returning_reg, JSValue::bool(true));
                    return Ok(());
                }
                if !return_fn.is_function() {
                    self.set_pending_type_error(ctx, "yield* iterator return is not a function");
                    return Err("yield* iterator return is not a function".to_string());
                }
                result = match self.call_function_with_this(ctx, return_fn, target, &[received]) {
                    Ok(v) => v,
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                };
            }
            _ => {
                let next_atom = ctx.intern("next");
                let next_fn = match Self::iterator_record_parts(ctx, iter_val) {
                    Some((_, next, _)) => next,
                    None => match self.get_prop_value(ctx, target, next_atom) {
                        Ok(v) => v,
                        Err(e) => {
                            self.rethrow_last_caught(ctx);
                            return Err(e);
                        }
                    },
                };
                if !next_fn.is_function() {
                    self.set_pending_type_error(ctx, "iterator next is not a function");
                    return Err("iterator next is not a function".to_string());
                }
                result = match self.call_function_with_this(ctx, next_fn, target, &[received]) {
                    Ok(v) => v,
                    Err(e) => {
                        self.rethrow_last_caught(ctx);
                        return Err(e);
                    }
                };
            }
        }
        if !result.is_object_like() {
            self.set_pending_type_error(ctx, "iterator result is not an object");
            return Err("iterator result is not an object".to_string());
        }
        let done = match self.get_prop_value(ctx, result, done_atom) {
            Ok(v) => v.is_truthy(),
            Err(e) => {
                self.rethrow_last_caught(ctx);
                return Err(e);
            }
        };
        if done {
            let value = match self.get_prop_value(ctx, result, value_atom) {
                Ok(v) => v,
                Err(e) => {
                    self.rethrow_last_caught(ctx);
                    return Err(e);
                }
            };
            self.set_reg(value_reg, value);
            self.set_reg(done_reg, JSValue::bool(true));
            if kind == 2 {
                self.set_reg(returning_reg, JSValue::bool(true));
            }
        } else {
            self.set_reg(value_reg, result);
            self.set_reg(done_reg, JSValue::bool(false));
        }
        Ok(())
    }

    fn iterator_result_done_value(
        &mut self,
        ctx: &mut JSContext,
        result: JSValue,
        done_atom: crate::runtime::atom::Atom,
        value_atom: crate::runtime::atom::Atom,
    ) -> Result<(bool, JSValue), String> {
        if !result.is_object_like() {
            self.set_pending_type_error(ctx, "iterator result is not an object");
            return Err("iterator result is not an object".to_string());
        }
        let done = match self.get_prop_value(ctx, result, done_atom) {
            Ok(v) => v.is_truthy(),
            Err(e) => {
                self.rethrow_last_caught(ctx);
                return Err(e);
            }
        };
        let value = match self.get_prop_value(ctx, result, value_atom) {
            Ok(v) => v,
            Err(e) => {
                self.rethrow_last_caught(ctx);
                return Err(e);
            }
        };
        Ok((done, value))
    }

    fn async_yield_star_await(
        &mut self,
        ctx: &mut JSContext,
        instr_pc: usize,
        phase: i64,
        value: JSValue,
        state_reg: u16,
        await_reg: u16,
    ) -> Result<bool, String> {
        let promise = ctx.call_builtin("promise_resolve", &[value]);
        self.adopt_pending_host_exception(ctx);
        if self.pending_throw.is_some() {
            return Err("yield* await threw".to_string());
        }
        self.set_reg(state_reg, JSValue::new_int(phase));
        self.await_suspend = Some((promise, await_reg));
        self.pc = instr_pc;
        Ok(true)
    }

    fn async_yield_star_step(
        &mut self,
        ctx: &mut JSContext,
        instr_pc: usize,
        iter_reg: u16,
        value_reg: u16,
        done_reg: u16,
        returning_reg: u16,
        state_reg: u16,
        await_reg: u16,
    ) -> Result<bool, String> {
        let done_atom = ctx.intern("done");
        let value_atom = ctx.intern("value");
        let phase = self.get_reg(state_reg).get_int();
        if phase != 0 {
            self.set_reg(state_reg, JSValue::new_int(0));
            let awaited = self.get_reg(await_reg);
            let async_from_sync = Self::iterator_record_parts(ctx, self.get_reg(iter_reg))
                .map(|(_, _, from_sync)| from_sync)
                .unwrap_or(false);
            match phase {
                10 | 11 | 12 => {
                    let is_return = phase == 12;
                    let (done, value) = if async_from_sync {
                        (self.get_reg(done_reg).is_truthy(), awaited)
                    } else {
                        match self.iterator_result_done_value(ctx, awaited, done_atom, value_atom)
                        {
                            Ok(pair) => pair,
                            Err(e) => return Err(e),
                        }
                    };
                    self.set_reg(value_reg, value);
                    if done {
                        self.set_reg(returning_reg, JSValue::bool(is_return));
                        self.set_reg(done_reg, JSValue::bool(true));
                    } else {
                        self.set_reg(done_reg, JSValue::bool(false));
                    }
                    Ok(false)
                }
                13 => {
                    self.set_reg(value_reg, awaited);
                    self.set_reg(returning_reg, JSValue::bool(true));
                    self.set_reg(done_reg, JSValue::bool(true));
                    Ok(false)
                }
                15 => {
                    self.set_reg(returning_reg, JSValue::bool(true));
                    self.set_reg(done_reg, JSValue::bool(true));
                    Ok(false)
                }
                14 => {
                    self.set_pending_type_error(
                        ctx,
                        "yield* iterator does not have a throw method",
                    );
                    Err("yield* iterator does not have a throw method".to_string())
                }
                _ => {
                    self.set_pending_type_error(ctx, "invalid yield* resume state");
                    Err("invalid yield* resume state".to_string())
                }
            }
        } else {
            let kind = self.generator_resume_kind;
            self.generator_resume_kind = 0;
            let iter_val = self.get_reg(iter_reg);
            if !iter_val.is_object_like() {
                self.set_pending_type_error(ctx, "yield* iterator is not an object");
                return Err("yield* iterator is not an object".to_string());
            }
            let (target, next_fn, async_from_sync) =
                match Self::iterator_record_parts(ctx, iter_val) {
                    Some(parts) => parts,
                    None => (iter_val, JSValue::undefined(), false),
                };
            let received = self.get_reg(value_reg);
            match kind {
                0 => {
                    if !next_fn.is_function() {
                        self.set_pending_type_error(ctx, "iterator next is not a function");
                        return Err("iterator next is not a function".to_string());
                    }
                    let result =
                        match self.call_function_with_this(ctx, next_fn, target, &[received]) {
                            Ok(v) => v,
                            Err(e) => {
                                self.rethrow_last_caught(ctx);
                                return Err(e);
                            }
                        };
                    if async_from_sync {
                        let (done, value) =
                            match self.iterator_result_done_value(ctx, result, done_atom, value_atom)
                            {
                                Ok(pair) => pair,
                                Err(e) => return Err(e),
                            };
                        self.set_reg(done_reg, JSValue::bool(done));
                        self.async_yield_star_await(ctx, instr_pc, 10, value, state_reg, await_reg)
                    } else {
                        self.async_yield_star_await(ctx, instr_pc, 10, result, state_reg, await_reg)
                    }
                }
                1 => {
                    let throw_atom = ctx.intern("throw");
                    let return_atom = ctx.intern("return");
                    let throw_fn = match self.get_prop_value(ctx, target, throw_atom) {
                        Ok(v) => v,
                        Err(e) => {
                            self.rethrow_last_caught(ctx);
                            return Err(e);
                        }
                    };
                    if async_from_sync {
                        if throw_fn.is_undefined() || throw_fn.is_null() {
                            self.pending_throw = Some(received);
                            return Err("yield* iterator has no throw value".to_string());
                        }
                        if !throw_fn.is_function() {
                            self.set_pending_type_error(ctx, "iterator throw is not a function");
                            return Err("iterator throw is not a function".to_string());
                        }
                        let result =
                            match self.call_function_with_this(ctx, throw_fn, target, &[received]) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.rethrow_last_caught(ctx);
                                    return Err(e);
                                }
                            };
                        let (done, value) =
                            match self.iterator_result_done_value(ctx, result, done_atom, value_atom)
                            {
                                Ok(pair) => pair,
                                Err(e) => return Err(e),
                            };
                        self.set_reg(done_reg, JSValue::bool(done));
                        self.async_yield_star_await(ctx, instr_pc, 11, value, state_reg, await_reg)
                    } else if throw_fn.is_undefined() || throw_fn.is_null() {
                        let return_fn = match self.get_prop_value(ctx, target, return_atom) {
                            Ok(v) => v,
                            Err(e) => {
                                self.rethrow_last_caught(ctx);
                                return Err(e);
                            }
                        };
                        if return_fn.is_undefined() || return_fn.is_null() {
                            self.set_pending_type_error(
                                ctx,
                                "yield* iterator does not have a throw method",
                            );
                            return Err(
                                "yield* iterator does not have a throw method".to_string()
                            );
                        }
                        if !return_fn.is_function() {
                            self.set_pending_type_error(ctx, "iterator return is not a function");
                            return Err("iterator return is not a function".to_string());
                        }
                        let result =
                            match self.call_function_with_this(ctx, return_fn, target, &[]) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.rethrow_last_caught(ctx);
                                    return Err(e);
                                }
                            };
                        self.async_yield_star_await(ctx, instr_pc, 14, result, state_reg, await_reg)
                    } else {
                        if !throw_fn.is_function() {
                            self.set_pending_type_error(ctx, "iterator throw is not a function");
                            return Err("iterator throw is not a function".to_string());
                        }
                        let result =
                            match self.call_function_with_this(ctx, throw_fn, target, &[received]) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.rethrow_last_caught(ctx);
                                    return Err(e);
                                }
                            };
                        self.async_yield_star_await(ctx, instr_pc, 11, result, state_reg, await_reg)
                    }
                }
                2 => {
                    let return_atom = ctx.intern("return");
                    let return_fn = match self.get_prop_value(ctx, target, return_atom) {
                        Ok(v) => v,
                        Err(e) => {
                            self.rethrow_last_caught(ctx);
                            return Err(e);
                        }
                    };
                    if async_from_sync {
                        if return_fn.is_undefined() || return_fn.is_null() {
                            self.async_yield_star_await(
                                ctx,
                                instr_pc,
                                15,
                                JSValue::undefined(),
                                state_reg,
                                await_reg,
                            )
                        } else {
                            if !return_fn.is_function() {
                                self.set_pending_type_error(
                                    ctx,
                                    "iterator return is not a function",
                                );
                                return Err("iterator return is not a function".to_string());
                            }
                            let result = match self.call_function_with_this(
                                ctx,
                                return_fn,
                                target,
                                &[received],
                            ) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.rethrow_last_caught(ctx);
                                    return Err(e);
                                }
                            };
                            let (done, value) = match self.iterator_result_done_value(
                                ctx, result, done_atom, value_atom,
                            ) {
                                Ok(pair) => pair,
                                Err(e) => return Err(e),
                            };
                            self.set_reg(done_reg, JSValue::bool(done));
                            self.async_yield_star_await(
                                ctx,
                                instr_pc,
                                12,
                                value,
                                state_reg,
                                await_reg,
                            )
                        }
                    } else if return_fn.is_undefined() || return_fn.is_null() {
                        self.async_yield_star_await(
                            ctx,
                            instr_pc,
                            13,
                            received,
                            state_reg,
                            await_reg,
                        )
                    } else {
                        if !return_fn.is_function() {
                            self.set_pending_type_error(ctx, "iterator return is not a function");
                            return Err("iterator return is not a function".to_string());
                        }
                        let result = match self.call_function_with_this(
                            ctx,
                            return_fn,
                            target,
                            &[received],
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                self.rethrow_last_caught(ctx);
                                return Err(e);
                            }
                        };
                        self.async_yield_star_await(ctx, instr_pc, 12, result, state_reg, await_reg)
                    }
                }
                _ => {
                    self.set_pending_type_error(ctx, "invalid yield* resume kind");
                    Err("invalid yield* resume kind".to_string())
                }
            }
        }
    }

    #[cold]
    fn set_pending_reference_error(&mut self, ctx: &mut JSContext, msg: &str) {
        use crate::object::object::JSObject;
        let mut err = JSObject::new();
        let name_atom = ctx.intern("name");
        let msg_atom = ctx.intern("message");
        let ref_error_atom = ctx.intern("ReferenceError");
        let msg_str_atom = ctx.intern(msg);
        err.set(name_atom, JSValue::new_string(ref_error_atom));
        err.set(msg_atom, JSValue::new_string(msg_str_atom));
        if let Some(proto) = ctx.get_reference_error_prototype() {
            err.prototype = Some(proto);
        }
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.pending_throw = Some(JSValue::new_object(ptr));
    }

    fn set_function_display_name(
        &mut self,
        ctx: &mut JSContext,
        func_val: JSValue,
        key_val: JSValue,
        prefix: Option<&str>,
    ) {
        if !func_val.is_function() {
            return;
        }
        let base_atom = if key_val.is_symbol() {
            let symbol_id = key_val.get_symbol_id();
            let description = ctx
                .runtime()
                .symbol_description(symbol_id)
                .unwrap_or(crate::builtins::symbol::NO_DESCRIPTION);
            if description == crate::builtins::symbol::NO_DESCRIPTION {
                ctx.intern("")
            } else {
                let text = format!("[{}]", ctx.get_atom_str(description));
                ctx.intern(&text)
            }
        } else if key_val.is_string() {
            key_val.get_atom()
        } else if key_val.is_int() {
            let text = key_val.get_int().to_string();
            ctx.intern(&text)
        } else if key_val.is_float() {
            VM::js_to_string(&key_val, ctx).get_atom()
        } else {
            return;
        };
        let display_atom = match prefix {
            Some(prefix_text) => {
                let text = format!("{} {}", prefix_text, ctx.get_atom_str(base_atom));
                ctx.intern(&text)
            }
            None => base_atom,
        };
        let func = func_val.as_function_mut();
        func.name = display_atom;
        let name_desc = crate::object::object::PropertyDescriptor {
            value: Some(JSValue::new_string(display_atom)),
            writable: false,
            enumerable: false,
            configurable: true,
            get: None,
            set: None,
        };
        func.base.define_property(ctx.common_atoms.name, name_desc);
    }

    fn value_is_constructor(ctx: &JSContext, value: JSValue) -> bool {
        if value.is_function() {
            let f = value.as_function();
            if f.is_builtin() {
                let name = f
                    .builtin_atom
                    .map(|ba| ctx.get_atom_str(ba).to_string())
                    .unwrap_or_default();
                ctx.builtin_is_constructor(&name)
            } else {
                !f.is_arrow() && !f.is_async() && !f.is_generator() && !f.is_method
            }
        } else if value.is_object() {
            value
                .as_object()
                .get(ctx.common_atoms.__boundFn)
                .map_or(false, |target| Self::value_is_constructor(ctx, target))
        } else {
            false
        }
    }

    fn generator_prototype_for(
        ctx: &mut JSContext,
        js_func: &crate::object::function::JSFunction,
    ) -> Option<*mut crate::object::object::JSObject> {
        let intrinsic = if js_func.is_async() {
            ctx.get_async_generator_prototype()
        } else {
            ctx.get_generator_prototype()
        };
        if let Some(proto_val) = js_func.base.get(ctx.common_atoms.prototype) {
            if proto_val.is_object_like() {
                return Some(proto_val.get_ptr() as *mut crate::object::object::JSObject);
            }
        }
        intrinsic
    }

    fn frame_is_derived_constructor(frame: &CallFrame) -> bool {
        frame.function_ptr.map_or(false, |fptr| unsafe {
            (*(fptr as *const crate::object::function::JSFunction)).is_derived_constructor
        })
    }

    fn throw_return_completion_error(
        &mut self,
        ctx: &mut JSContext,
        msg: &str,
        is_reference_error: bool,
    ) -> Result<Option<ExecutionOutcome>, String> {
        if is_reference_error {
            self.set_pending_reference_error(ctx, msg);
        } else {
            self.set_pending_type_error(ctx, msg);
        }
        let Some(exc) = self.pending_throw.take() else {
            return Ok(None);
        };
        match self.dispatch_throw_value(ctx, exc) {
            ThrowDispatch::Caught => Ok(None),
            ThrowDispatch::Uncaught(e) => Err(e),
            ThrowDispatch::AsyncComplete(o) => Ok(Some(o)),
        }
    }

    fn execute_return_completion(
        &mut self,
        ctx: &mut JSContext,
        mut value: JSValue,
    ) -> Result<Option<ExecutionOutcome>, String> {
        if self
            .pending_finally_rethrow
            .map_or(false, |(frame, _)| frame == self.frame_index)
        {
            self.pending_finally_rethrow = None;
        }
        if self
            .pending_finally_return
            .map_or(false, |(frame, _)| frame == self.frame_index)
        {
            self.pending_finally_return = None;
        }
        if self
            .pending_finally_jump
            .map_or(false, |(frame, _, _)| frame == self.frame_index)
        {
            self.pending_finally_jump = None;
        }
        while let Some(handler) = self.exception_handlers.last().cloned() {
            if handler.frame_index != self.frame_index {
                break;
            }
            self.exception_handlers.pop();
            if let Some(fp) = handler.finally_pc {
                self.pending_finally_return = Some((self.frame_index, value));
                self.pending_finally_depth = self.same_frame_handler_count();
                self.pc = fp;
                self.refresh_cache();
                return Ok(None);
            }
        }
        if self.frames[self.frame_index].is_constructor {
            let (this_value, this_initialized, derived) = {
                let frame = &self.frames[self.frame_index];
                let (this_value, this_initialized) = match &frame.this_cell {
                    Some(cell) => cell.get(),
                    None => (frame.this_value, frame.this_initialized),
                };
                let derived = Self::frame_is_derived_constructor(frame);
                (this_value, this_initialized, derived)
            };
            if derived {
                if !value.is_object_like() {
                    if value.is_undefined() {
                        if this_initialized {
                            value = this_value;
                        } else {
                            return self.throw_return_completion_error(
                                ctx,
                                "this is not initialized",
                                true,
                            );
                        }
                    } else {
                        return self.throw_return_completion_error(
                            ctx,
                            "Derived constructors may only return object or undefined",
                            false,
                        );
                    }
                }
            } else if !value.is_object_like() {
                value = this_value;
            }
        }
        if self.frame_index == 0 {
            if self.frames[0].is_async {
                value = ctx.call_builtin("promise_resolve", &[value]);
            }
            return Ok(Some(ExecutionOutcome::Complete(value)));
        }
        let is_async = self.frames[self.frame_index].is_async;
        self.cleanup_handlers_for_frame();
        if is_async {
            value = ctx.call_builtin("promise_resolve", &[value]);
        }
        self.pop_frame(value);
        Ok(None)
    }

    fn execute_jump_completion(&mut self, target: usize, target_depth: usize) {
        if self
            .pending_finally_return
            .map_or(false, |(frame, _)| frame == self.frame_index)
        {
            self.pending_finally_return = None;
        }
        if self
            .pending_finally_rethrow
            .map_or(false, |(frame, _)| frame == self.frame_index)
        {
            self.pending_finally_rethrow = None;
        }
        if self
            .pending_finally_jump
            .map_or(false, |(frame, _, _)| frame == self.frame_index)
        {
            self.pending_finally_jump = None;
        }
        loop {
            let depth = self
                .exception_handlers
                .iter()
                .filter(|h| h.frame_index == self.frame_index)
                .count();
            if depth <= target_depth {
                break;
            }
            let handler = self.exception_handlers.pop().unwrap();
            if let Some(fp) = handler.finally_pc {
                self.pending_finally_jump = Some((self.frame_index, target, target_depth));
                self.pending_finally_depth = self.same_frame_handler_count();
                self.pc = fp;
                self.refresh_cache();
                return;
            }
        }
        self.pc = target;
        self.refresh_cache();
    }

    #[cold]
    fn dispatch_throw_value(&mut self, ctx: &mut JSContext, value: JSValue) -> ThrowDispatch {
        if self
            .last_caught_exception
            .map_or(false, |caught| caught.raw_bits() == value.raw_bits())
        {
            self.last_caught_exception = None;
        }
        if ctx
            .pending_exception
            .map_or(false, |pending| pending.raw_bits() == value.raw_bits())
        {
            ctx.pending_exception = None;
        }
        if self
            .pending_throw
            .map_or(false, |pending| pending.raw_bits() == value.raw_bits())
        {
            self.pending_throw = None;
        }
        let find_async_frame = |frames: &[CallFrame], frame_index: usize| {
            for i in (0..=frame_index).rev() {
                if frames[i].is_async {
                    return Some(i);
                }
            }
            None
        };

        if let Some(handler) = self.exception_handlers.last().cloned() {
            self.exception_handlers.pop();
            let handled_frame = handler.frame_index;
            let same_frame_after = self
                .exception_handlers
                .iter()
                .filter(|h| h.frame_index == self.frame_index)
                .count();
            self.clear_pending_finally_if_escaped(handled_frame, same_frame_after);

            if self.frame_index != handler.frame_index {
                if let Some(async_idx) = find_async_frame(&self.frames, self.frame_index) {
                    if async_idx > handler.frame_index {
                        while self.frame_index > async_idx {
                            if self.frame_index == 0 {
                                break;
                            }
                            self.frame_index -= 1;
                        }
                        let rejected = ctx.call_builtin("promise_reject", &[value]);
                        if self.frame_index == 0 {
                            return ThrowDispatch::AsyncComplete(ExecutionOutcome::Complete(
                                rejected,
                            ));
                        }
                        let return_pc = self.frames[self.frame_index].return_pc;
                        let dst_reg = self.frames[self.frame_index].dst_reg;
                        self.frame_index -= 1;
                        self.pc = return_pc;
                        self.refresh_cache();
                        self.set_reg(dst_reg, rejected);
                        return ThrowDispatch::Caught;
                    }
                }
                while self.frame_index > handler.frame_index {
                    if self.frame_index == 0 {
                        break;
                    }
                    self.frame_index -= 1;
                }
            }

            if handler
                .finally_pc
                .map_or(false, |fp| fp == handler.catch_pc)
            {
                self.finally_rethrow = Some(value);
            }
            self.pc = handler.catch_pc;
            self.refresh_cache();
            self.caught_exception_value = Some(value);
            ThrowDispatch::Caught
        } else {
            self.clear_pending_finally_for_frame(self.frame_index);
            if let Some(async_idx) = find_async_frame(&self.frames, self.frame_index) {
                let rejected = ctx.call_builtin("promise_reject", &[value]);
                if async_idx == 0 {
                    return ThrowDispatch::AsyncComplete(ExecutionOutcome::Complete(rejected));
                }
                while self.frame_index > async_idx {
                    self.frame_index -= 1;
                }
                let return_pc = self.frames[self.frame_index].return_pc;
                let dst_reg = self.frames[self.frame_index].dst_reg;
                self.frame_index -= 1;
                self.pc = return_pc;
                self.refresh_cache();
                self.set_reg(dst_reg, rejected);
                return ThrowDispatch::Caught;
            }
            self.pending_throw = Some(value);
            let msg = self.format_thrown_value(&value, ctx);
            let trace = self.format_stack_trace(ctx);
            ThrowDispatch::Uncaught(format!("Uncaught: {}\nStack trace:\n{}", msg, trace))
        }
    }

    fn format_thrown_value(&self, value: &JSValue, ctx: &JSContext) -> String {
        if value.is_string() {
            return ctx.get_atom_str(value.get_atom()).to_string();
        }
        if value.is_int() {
            return value.get_int().to_string();
        }
        if value.is_float() {
            return value.get_float().to_string();
        }
        if value.is_bool() {
            return value.get_bool().to_string();
        }
        if value.is_null() {
            return "null".to_string();
        }
        if value.is_undefined() {
            return "undefined".to_string();
        }
        if value.is_object() {
            let ptr = value.get_ptr();
            let obj = unsafe { &*(ptr as *const crate::object::JSObject) };
            let name = obj.get(ctx.common_atoms.name).and_then(|v| {
                if v.is_string() {
                    Some(ctx.get_atom_str(v.get_atom()).to_string())
                } else {
                    None
                }
            });
            let name = name.or_else(|| {
                obj.get(ctx.common_atoms.constructor).and_then(|ctor| {
                    if ctor.is_function() {
                        let func = ctor.as_function();
                        let name = ctx.get_atom_str(func.name);
                        if name.is_empty() {
                            None
                        } else {
                            Some(name.to_string())
                        }
                    } else if ctor.is_object() {
                        let ctor_obj = ctor.as_object();
                        ctor_obj.get(ctx.common_atoms.name).and_then(|v| {
                            if v.is_string() {
                                Some(ctx.get_atom_str(v.get_atom()).to_string())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
            });
            let message = obj.get(ctx.common_atoms.message).and_then(|v| {
                if v.is_string() {
                    Some(ctx.get_atom_str(v.get_atom()).to_string())
                } else {
                    None
                }
            });
            match (name, message) {
                (Some(n), Some(m)) => {
                    return format!("{}: {}", n, m);
                }
                (Some(n), _) => return n,
                (None, Some(m)) => {
                    if m.is_empty() {
                        return "Error".to_string();
                    }
                    return m;
                }
                _ => {}
            }
            return "[object Object]".to_string();
        }
        if value.is_function() {
            return "function".to_string();
        }
        if value.is_symbol() {
            return "Symbol".to_string();
        }
        if value.is_bigint() {
            return "BigInt".to_string();
        }
        "<value>".to_string()
    }

    #[inline(never)]
    fn fill_gc_roots(&mut self, ctx: &JSContext) {
        let roots = &mut self.gc_roots;
        roots.clear();
        let global = ctx.global();
        if global.is_object() || global.is_function() {
            roots.push(global);
        }

        for fi in 0..=self.frame_index {
            let frame = &self.frames[fi];
            let base = frame.registers_base;
            let count = frame.registers_count;
            for i in 0..count {
                let v = self.registers[base + i];
                if v.is_object() || v.is_function() {
                    roots.push(v);
                }
            }
            if let Some(ptr) = frame.function_ptr {
                roots.push(JSValue::new_function(ptr));
            }
            let tv = frame.this_value;
            if tv.is_object() || tv.is_function() {
                roots.push(tv);
            }
            let sv = frame.super_ctor;
            if sv.is_object() || sv.is_function() {
                roots.push(sv);
            }
        }

        macro_rules! push_proto {
            ($getter:ident) => {
                if let Some(ptr) = ctx.$getter() {
                    roots.push(JSValue::new_object(ptr as usize));
                }
            };
        }
        push_proto!(get_string_prototype);
        push_proto!(get_number_prototype);
        push_proto!(get_array_prototype);
        push_proto!(get_regexp_prototype);
        push_proto!(get_object_prototype);
        push_proto!(get_function_prototype);
        push_proto!(get_map_prototype);
        push_proto!(get_set_prototype);
        push_proto!(get_weakmap_prototype);
        push_proto!(get_weakset_prototype);
        push_proto!(get_error_prototype);
        push_proto!(get_type_error_prototype);
        push_proto!(get_weakref_prototype);
        push_proto!(get_finalization_registry_prototype);
        push_proto!(get_generator_prototype);
        push_proto!(get_async_generator_prototype);
        push_proto!(get_generator_function_prototype);
        push_proto!(get_async_generator_function_prototype);
        push_proto!(get_promise_prototype);

        for &v in &ctx.runtime().gc_heap().extra_roots {
            if v.is_object() || v.is_function() {
                roots.push(v);
            }
        }
    }

    pub fn collect_roots(&self, ctx: &JSContext) -> Vec<JSValue> {
        let mut roots = Vec::with_capacity(256);
        let global = ctx.global();
        if global.is_object() || global.is_function() {
            roots.push(global);
        }

        for fi in 0..=self.frame_index {
            let frame = &self.frames[fi];
            let base = frame.registers_base;
            let count = frame.registers_count;
            for i in 0..count {
                let v = self.registers[base + i];
                if v.is_object() || v.is_function() {
                    roots.push(v);
                }
            }
            if let Some(ptr) = frame.function_ptr {
                roots.push(JSValue::new_function(ptr));
            }
            let tv = frame.this_value;
            if tv.is_object() || tv.is_function() {
                roots.push(tv);
            }
            let sv = frame.super_ctor;
            if sv.is_object() || sv.is_function() {
                roots.push(sv);
            }
        }

        macro_rules! push_proto {
            ($getter:ident) => {
                if let Some(ptr) = ctx.$getter() {
                    roots.push(JSValue::new_object(ptr as usize));
                }
            };
        }
        push_proto!(get_string_prototype);
        push_proto!(get_number_prototype);
        push_proto!(get_array_prototype);
        push_proto!(get_regexp_prototype);
        push_proto!(get_object_prototype);
        push_proto!(get_function_prototype);
        push_proto!(get_map_prototype);
        push_proto!(get_set_prototype);
        push_proto!(get_weakmap_prototype);
        push_proto!(get_weakset_prototype);
        push_proto!(get_error_prototype);
        push_proto!(get_type_error_prototype);
        push_proto!(get_weakref_prototype);
        push_proto!(get_finalization_registry_prototype);
        push_proto!(get_generator_prototype);
        push_proto!(get_async_generator_prototype);
        push_proto!(get_generator_function_prototype);
        push_proto!(get_async_generator_function_prototype);
        push_proto!(get_promise_prototype);

        for &v in &ctx.runtime().gc_heap().extra_roots {
            if v.is_object() || v.is_function() {
                roots.push(v);
            }
        }
        if let Some((this_val, _)) = self.pending_instance_elements {
            if this_val.is_object() || this_val.is_function() {
                roots.push(this_val);
            }
        }
        roots
    }

    #[inline(always)]
    pub fn maybe_gc(&mut self, ctx: &mut JSContext) {
        if self.allocation_count >= GC_CHECK_INTERVAL {
            self.allocation_count = 0;
            let heap = ctx.runtime().gc_heap();
            let do_minor = heap.nursery_is_full();
            let do_full = heap.should_collect();

            if do_minor || do_full {
                // Skip minor GC if nursery has no collectible objects
                // (e.g., only pinned survivors that never die)
                let skip_minor = do_minor && heap.nursery_collectible_count() == 0
                    && heap.nursery_pinned_count() > 0;

                if do_minor && !skip_minor {
                    self.fill_gc_roots(ctx);
                    let roots_len = self.gc_roots.len();
                    let roots_ptr = self.gc_roots.as_mut_ptr();
                    let roots = unsafe { std::slice::from_raw_parts_mut(roots_ptr, roots_len) };
                    let _ = ctx.runtime_mut().minor_gc(roots);
                }

                if do_full || ctx.runtime().gc_heap().should_collect() {
                    self.run_gc(ctx);
                }
            }
        }
    }

    pub fn minor_gc(&mut self, ctx: &mut JSContext) -> usize {
        self.fill_gc_roots(ctx);
        let roots_ptr = self.gc_roots.as_ptr();
        let roots_len = self.gc_roots.len();
        let roots = unsafe { std::slice::from_raw_parts(roots_ptr, roots_len) };
        ctx.runtime_mut().minor_gc(roots)
    }

    pub fn run_gc(&mut self, ctx: &mut JSContext) -> usize {
        self.fill_gc_roots(ctx);
        let roots_ptr = self.gc_roots.as_ptr();
        let roots_len = self.gc_roots.len();
        let roots = unsafe { std::slice::from_raw_parts(roots_ptr, roots_len) };
        let freed = ctx.runtime_mut().run_gc(roots);

        if ctx.runtime().gc_heap().deleted_props_count > 0 {
            let live_objects: Vec<(usize, u8)> = {
                let heap = ctx.runtime().gc_heap();
                let mut objs = Vec::new();
                heap.for_each_live_object(|ptr, tag| objs.push((ptr, tag)));
                objs
            };
            {
                let cache = ctx.shape_cache_mut();
                for (ptr, tag) in live_objects {
                    unsafe {
                        if tag == crate::runtime::gc::TAG_ARRAY {
                            let arr = &mut *(ptr as *mut crate::object::array_obj::JSArrayObject);
                            arr.header.compact_props(cache);
                        } else {
                            let obj = &mut *(ptr as *mut crate::object::object::JSObject);
                            obj.compact_props(cache);
                        }
                    }
                }
            }

            ctx.runtime_mut().gc_heap_mut().deleted_props_count = 0;
        }
        freed
    }

    pub fn call_function(
        &mut self,
        ctx: &mut JSContext,
        func: JSValue,
        args: &[JSValue],
    ) -> Result<JSValue, String> {
        self.call_function_with_this(ctx, func, JSValue::undefined(), args)
    }

    pub fn construct(
        &mut self,
        ctx: &mut JSContext,
        ctor: JSValue,
        args: &[JSValue],
    ) -> Result<JSValue, String> {
        if !ctor.is_function() || !Self::value_is_constructor(ctx, ctor) {
            return Err("not a constructor".to_string());
        }

        let js_func = ctor.as_function();

        let mut new_obj = crate::object::object::JSObject::new();

        if !js_func.cached_prototype_ptr.is_null() {
            new_obj.prototype = Some(js_func.cached_prototype_ptr);
        } else {
            let proto_key = ctx.common_atoms.prototype;
            if let Some(proto_val) = js_func.base.get(proto_key) {
                if proto_val.is_object_like() {
                    let cptr = proto_val.get_ptr() as *mut crate::object::object::JSObject;
                    ctor.as_function_mut().cached_prototype_ptr = cptr;
                    new_obj.prototype = Some(cptr);
                }
            } else if !js_func.is_builtin() {
                let mut pobj = crate::object::object::JSObject::new();
                pobj.set(ctx.common_atoms.constructor, ctor);
                if let Some(opp) = ctx.get_object_prototype() {
                    pobj.prototype = Some(opp);
                }
                let pp = Box::into_raw(Box::new(pobj)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(pp);
                ctor.as_function_mut().base.set(proto_key, JSValue::new_object(pp));
                let cptr = pp as *mut crate::object::object::JSObject;
                ctor.as_function_mut().cached_prototype_ptr = cptr;
                new_obj.prototype = Some(cptr);
            }
        }

        let new_ptr = Box::into_raw(Box::new(new_obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(new_ptr);
        let new_val = JSValue::new_object(new_ptr);

        if js_func.is_class_constructor && !js_func.is_derived_constructor {
            self.mark_instance_elements_pending(new_val, ctor.get_ptr());
        }

        match self.call_function_with_this_impl(ctx, ctor, new_val, args, true) {
            Ok(result) => {
                if result.is_object_like() {
                    Ok(result)
                } else {
                    Ok(new_val)
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn call_function_with_this(
        &mut self,
        ctx: &mut JSContext,
        func: JSValue,
        this_value: JSValue,
        args: &[JSValue],
    ) -> Result<JSValue, String> {
        self.call_function_with_this_impl(ctx, func, this_value, args, false)
    }

    fn call_function_with_this_impl(
        &mut self,
        ctx: &mut JSContext,
        func: JSValue,
        this_value: JSValue,
        args: &[JSValue],
        constructing: bool,
    ) -> Result<JSValue, String> {
        if func.is_object() && !constructing {
            if let Some(function_prototype) = ctx.get_function_prototype() {
                if func.get_ptr() as *mut crate::object::object::JSObject == function_prototype
                {
                    return Ok(JSValue::undefined());
                }
            }
        }

        if func.is_object() {
            let obj = func.as_object();
            if let Some(bound_fn) = obj.get(ctx.common_atoms.__boundFn) {
                if bound_fn.is_function() && bound_fn.as_function().is_class_constructor {
                    let msg = "Class constructor cannot be invoked without 'new'".to_string();
                    self.set_pending_type_error(ctx, &msg);
                    self.last_caught_exception = self.pending_throw.take();
                    return Err(msg);
                }
                let bound_this = obj.get(ctx.common_atoms.__boundThis).unwrap_or(this_value);
                let bound_args_val = obj
                    .get(ctx.common_atoms.__boundArgs)
                    .filter(|val| val.is_object());

                let bound_len = bound_args_val
                    .and_then(|v| v.as_object().get(ctx.common_atoms.length))
                    .map(|v| v.get_int() as usize)
                    .unwrap_or(0);

                let mut actual_args = Vec::with_capacity(bound_len.saturating_add(args.len()));
                if let Some(bound_args_val) = bound_args_val {
                    let bound_args_obj = bound_args_val.as_object();
                    for i in 0..bound_len {
                        let key = self.int_atom(i, ctx);
                        if let Some(arg_val) = bound_args_obj.get(key) {
                            actual_args.push(arg_val);
                        }
                    }
                }
                actual_args.extend_from_slice(args);
                return self
                    .call_function_with_this_impl(ctx, bound_fn, bound_this, &actual_args, constructing);
            }
        }

        if !func.is_function() {
            let msg = format!(
                "{} is not a function",
                self.format_thrown_value(&func, ctx)
            );
            self.set_pending_type_error(ctx, &msg);
            self.last_caught_exception = self.pending_throw.take();
            return Err(msg);
        }

        if !constructing && func.as_function().is_class_constructor {
            let msg = "Class constructor cannot be invoked without 'new'".to_string();
            self.set_pending_type_error(ctx, &msg);
            self.last_caught_exception = self.pending_throw.take();
            return Err(msg);
        }

        let ptr = func.get_ptr();
        let js_func = func.as_function();

        if js_func.is_builtin() {
            let builtin_name = js_func
                .builtin_atom
                .map(|ba| ctx.get_atom_str(ba).to_string())
                .unwrap_or_default();
            let needs_this = ctx.builtin_needs_this(&builtin_name);
            let needs_callee = Self::builtin_needs_callee(&builtin_name);

            let mut call_args_vec: Vec<JSValue> = Vec::new();
            let call_args: &[JSValue] = if needs_callee {
                call_args_vec.reserve(args.len() + 1);
                call_args_vec.push(func);
                call_args_vec.extend_from_slice(args);
                &call_args_vec
            } else if needs_this {
                call_args_vec.reserve(args.len() + 1);
                call_args_vec.push(this_value);
                call_args_vec.extend_from_slice(args);
                &call_args_vec
            } else {
                args
            };

            if let Some(bf) = js_func.builtin_func {
                let result = ctx.call_builtin_direct(bf, call_args);
                if let Some(exc) = ctx.pending_exception.take() {
                    let msg = if exc.is_string() {
                        ctx.get_atom_str(exc.get_atom()).to_string()
                    } else if exc.is_object() {
                        let obj = exc.as_object();
                        let m = obj.get(ctx.common_atoms.message);
                        let n = obj.get(ctx.intern("name"));
                        match (n, m) {
                            (Some(nv), Some(mv)) if nv.is_string() && mv.is_string() => {
                                format!(
                                    "{}: {}",
                                    ctx.get_atom_str(nv.get_atom()),
                                    ctx.get_atom_str(mv.get_atom())
                                )
                            }
                            _ => "builtin error".to_string(),
                        }
                    } else {
                        "builtin error".to_string()
                    };
                    self.last_caught_exception = Some(exc);
                    return Err(msg);
                }
                return Ok(result);
            } else if let Some(ba) = js_func.builtin_atom {
                let result = ctx.call_builtin(&ctx.get_atom_str(ba).to_string(), call_args);
                if let Some(exc) = ctx.pending_exception.take() {
                    let msg = if exc.is_string() {
                        ctx.get_atom_str(exc.get_atom()).to_string()
                    } else if exc.is_object() {
                        let obj = exc.as_object();
                        let m = obj.get(ctx.common_atoms.message);
                        let n = obj.get(ctx.intern("name"));
                        match (n, m) {
                            (Some(nv), Some(mv)) if nv.is_string() && mv.is_string() => {
                                format!(
                                    "{}: {}",
                                    ctx.get_atom_str(nv.get_atom()),
                                    ctx.get_atom_str(mv.get_atom())
                                )
                            }
                            _ => "builtin error".to_string(),
                        }
                    } else {
                        "builtin error".to_string()
                    };
                    self.last_caught_exception = Some(exc);
                    return Err(msg);
                }
                return Ok(result);
            }
            return Ok(JSValue::undefined());
        }

        if let Some(ref rb) = js_func.bytecode {
            if js_func.is_async() && !js_func.is_generator() {
                let promise = start_async_call(ctx, js_func, ptr, this_value, args)?;
                return Ok(promise);
            }
            if js_func.is_generator() {
                let snapshot_this = if !js_func.is_strict()
                    && (this_value.is_undefined() || this_value.is_null())
                {
                    ctx.global()
                } else {
                    this_value
                };
                let min_slots = 1 + args.len();
                let snapshot_len = (rb.locals_count as usize).max(min_slots);
                let mut snapshot = vec![JSValue::undefined(); snapshot_len];
                snapshot[0] = snapshot_this;
                for (i, arg) in args.iter().enumerate() {
                    snapshot[i + 1] = *arg;
                }
                let (
                    result_snapshot,
                    result_pc,
                    result_done,
                    result_send_reg,
                    result_handlers,
                    result_yield_is_result,
                ) = {
                    let mut generator_vm = crate::runtime::vm::VM::new();
                    ctx.runtime_mut().gc_heap_mut().gc_suppressed = true;
                    let step = generator_vm.execute_generator_step(
                        ctx,
                        rb,
                        &snapshot,
                        0,
                        JSValue::undefined(),
                        None,
                        args,
                        Some(ptr),
                        &[],
                        0,
                        JSValue::undefined(),
                        false,
                    );
                    let uncaught = generator_vm.last_uncaught.take();
                    ctx.runtime_mut().gc_heap_mut().gc_suppressed = false;
                    drop(generator_vm);
                    match step {
                        Ok((_val, new_snapshot, new_pc, done, src, handlers, yield_is_result)) => {
                            (new_snapshot, new_pc, done, src, handlers, yield_is_result)
                        }
                        Err(_) => {
                            if let Some(exc) = uncaught {
                                self.last_caught_exception = Some(exc.clone());
                                self.pending_throw = Some(exc);
                            }
                            (snapshot, 0, false, None, Vec::new(), false)
                        }
                    }
                };
                let mut gen_obj = crate::object::object::JSObject::new();
                gen_obj.set_is_generator(true);
                if let Some(proto_ptr) = Self::generator_prototype_for(ctx, js_func) {
                    gen_obj.prototype = Some(proto_ptr);
                }
                if js_func.is_async() {
                    gen_obj.set_async_state(crate::object::object::AsyncState {
                        bytecode: Box::new(rb.clone()),
                        snapshot: result_snapshot,
                        pc: result_pc,
                        done: result_done,
                        send_reg: result_send_reg,
                        saved_args: args.to_vec(),
                        func_ptr: Some(ptr),
                        handlers: result_handlers,
                        pending_throw: None,
                        resume_kind: 0,
                        resume_value: JSValue::undefined(),
                        yield_is_result: result_yield_is_result,
                        pending_yield: false,
                    });
                    gen_obj.set_async_generator_state(crate::object::object::AsyncGeneratorState {
                        queue: Vec::new(),
                        running: false,
                        current_promise: JSValue::undefined(),
                        started: false,
                        awaiting_return: false,
                        pending_return_action: 0,
                    });
                } else {
                    gen_obj.set_generator_state(crate::object::object::GeneratorState {
                        bytecode: Box::new(rb.clone()),
                        snapshot: result_snapshot,
                        pc: result_pc,
                        done: result_done,
                        send_reg: result_send_reg,
                        argc: args.len() as u32,
                        saved_args: args.to_vec(),
                        func_ptr: Some(ptr),
                        handlers: result_handlers,
                        yield_is_result: result_yield_is_result,
                    });
                }
                let gen_ptr = Box::into_raw(Box::new(gen_obj)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(gen_ptr);
                return Ok(JSValue::new_object(gen_ptr));
            }

            let saved_frame_index = self.frame_index;
            let saved_pc = self.pc;
            let saved_r0 = self.get_reg(0);
            let saved_handlers: Vec<_> = std::mem::take(&mut self.exception_handlers);

            let fn_this =
                if !js_func.is_strict() && (this_value.is_undefined() || this_value.is_null()) {
                    ctx.global()
                } else {
                    this_value
                };
            self.push_frame_raw(
                rb.locals_count,
                rb.effective_code_ptr(),
                rb.effective_code_len(),
                rb.effective_const_ptr(),
                rb.effective_const_len(),
                self.frames[self.frame_index].bytecode_len,
                Some(ptr),
                fn_this,
                0,
                args.len() as u16,
                false,
                js_func.is_async(),
                args,
                js_func.uses_arguments(),
            );

            {
                let super_key = ctx.common_atoms.__super__;
                if let Some(super_val) = js_func.base.get(super_key) {
                    self.frames[self.frame_index].super_ctor = super_val;
                }
                if js_func.is_derived_constructor {
                    self.frames[self.frame_index].this_initialized = false;
                }
                if js_func.is_class_constructor && !js_func.is_derived_constructor {
                    self.mark_instance_elements_pending(fn_this, ptr);
                }
                if !js_func.home_object.is_undefined() && js_func.home_object.is_object_like() {
                    let home = js_func.home_object.as_object();
                    let base = home
                        .prototype
                        .map(|p| JSValue::new_object(p as usize))
                        .unwrap_or(JSValue::undefined());
                    self.frames[self.frame_index].super_base = base;
                }
            }
            if let Some(new_target) = self.pending_new_target.take() {
                let frame = &mut self.frames[self.frame_index];
                frame.new_target = new_target;
                frame.is_constructor = true;
            }

            let result = self.execute_inner(ctx, rb, false, 0, false);
            let return_value = self.get_reg(0);

            let pending_exc = result
                .as_ref()
                .err()
                .and_then(|_| self.pending_throw.take());

            while self.frame_index > saved_frame_index {
                self.pop_frame(JSValue::undefined());
            }
            self.pc = saved_pc;
            self.exception_handlers = saved_handlers;
            self.refresh_cache();
            self.set_reg(0, saved_r0);

            if let Some(exc) = pending_exc {
                self.last_caught_exception = Some(exc.clone());
                ctx.pending_exception = Some(exc);
                return Err("".to_string());
            }

            return match result {
                Ok(_) => Ok(return_value),
                Err(e) => Err(e),
            };
        }

        Ok(JSValue::undefined())
    }

    pub fn execute(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
    ) -> Result<ExecutionOutcome, String> {
        let result = self.execute_inner(ctx, bytecode, false, 0, true);

        if !self.ctx_ptr.is_null() {
            unsafe {
                let count = self.frames[0].registers_count;
                for i in 0..count {
                    self.registers[i].release_atoms_in(&mut *self.ctx_ptr);
                    self.registers[i] = JSValue::undefined();
                }
            }
        }
        self.ctx_ptr = std::ptr::null_mut();
        result
    }

    pub fn execute_preserving_registers(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
    ) -> Result<ExecutionOutcome, String> {
        let result = self.execute_inner(ctx, bytecode, true, 0, true);
        self.ctx_ptr = std::ptr::null_mut();
        result
    }

    pub fn execute_eval(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
        this_value: JSValue,
        caller_vm_ptr: Option<usize>,
    ) -> Result<ExecutionOutcome, String> {
        self.ctx_ptr = ctx;
        self.caller_vm = caller_vm_ptr;
        ctx.set_register_vm_ptr(Some(self as *mut VM as usize));

        let needed = (bytecode.locals_count as usize).max(1);
        if needed > self.registers.len() {
            self.registers.resize(needed, JSValue::undefined());
        }
        self.registers[0] = this_value;

        let frame = &mut self.frames[0];
        frame.registers_base = 0;
        frame.registers_count = needed;
        frame.locals_count = bytecode.locals_count;
        frame.bytecode_ptr = bytecode.effective_code_ptr();
        frame.bytecode_len = bytecode.effective_code_len();
        frame.constants_ptr = bytecode.effective_const_ptr();
        frame.constants_len = bytecode.effective_const_len();
        frame.return_pc = 0;
        frame.function_ptr = None;
        frame.with_env = None;
        frame.this_value = this_value;
        frame.saved_args.clear();
        frame.upvalue_sync_map = None;
        frame.upvalue_sync_bitset = 0;
        frame.uses_arguments = false;
        frame.var_name_map = std::rc::Rc::as_ptr(&bytecode.var_name_to_slot);
        frame.ic_table_ptr = bytecode.effective_ic_table_ptr();
        frame.super_ctor = JSValue::undefined();
        frame.super_base = JSValue::undefined();
        frame.new_target = JSValue::undefined();
        frame.this_initialized = true;
        frame.this_cell = None;
        frame.is_super_call = false;
        if let Some(caller_ptr) = caller_vm_ptr {
            let caller = unsafe { &*(caller_ptr as *const VM) };
            let caller_frame = &caller.frames[caller.frame_index];
            frame.super_ctor = caller_frame.super_ctor;
            frame.super_base = caller_frame.super_base;
            frame.new_target = caller_frame.new_target;
            frame.this_initialized = caller_frame.this_initialized;
        }

        self.frame_index = 0;
        self.exception_handlers.clear();
        self.frames[0].is_strict_frame = bytecode.is_strict;
        self.refresh_cache();

        let result = self.execute_inner(ctx, bytecode, true, 0, false);
        self.ctx_ptr = std::ptr::null_mut();
        result
    }

    pub fn direct_eval(&mut self, ctx: &mut JSContext, source: &str) -> Result<JSValue, String> {
        let is_strict_caller = self.frames[self.frame_index].is_strict_frame;

        if !is_strict_caller {
            let frame = &self.frames[self.frame_index];
            if frame.in_parameter_initializer
                && frame.param_env_has_arguments
                && crate::compiler::eval_source_declares_var(source, "arguments")
            {
                let mut err = crate::object::object::JSObject::new();
                err.set(
                    ctx.intern("name"),
                    JSValue::new_string(ctx.intern("SyntaxError")),
                );
                err.set(
                    ctx.intern("message"),
                    JSValue::new_string(ctx.intern(
                        "Identifier 'arguments' has already been declared",
                    )),
                );
                if let Some(proto) = ctx.get_syntax_error_prototype() {
                    err.prototype = Some(proto);
                }
                let ptr = Box::into_raw(Box::new(err)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(ptr);
                ctx.pending_exception = Some(JSValue::new_object(ptr));
                return Ok(JSValue::undefined());
            }
        }

        let in_field_initializer = self.eval_direct_flags & 1 != 0;
        let (allow_super_property, allow_super_call) = {
            let frame = &self.frames[self.frame_index];
            let mut allow_property = frame.super_base.is_object_like()
                || frame.super_ctor.is_object_like();
            let mut allow_call = false;
            if let Some(ptr) = frame.function_ptr {
                let func = unsafe { &*(ptr as *const crate::object::function::JSFunction) };
                if !func.home_object.is_undefined() {
                    allow_property = true;
                }
                if func.is_derived_constructor {
                    allow_call = true;
                }
            }
            if in_field_initializer {
                allow_call = false;
            }
            (allow_property, allow_call)
        };
        let caller_vm_ptr = self as *mut VM;
        let mut eval_private_names: Vec<crate::runtime::atom::Atom> = Vec::new();
        for fi in (0..=self.frame_index).rev() {
            let frame = &self.frames[fi];
            if let Some(ptr) = frame.function_ptr {
                let func = unsafe { &*(ptr as *const crate::object::function::JSFunction) };
                if func.home_object.is_object_like() {
                    let owner = func.home_object.get_ptr();
                    let obj = func.home_object.as_object();
                    obj.for_each_private_kind(|key, _| {
                        if key.owner == owner {
                            eval_private_names.push(key.name);
                        }
                    });
                    obj.for_each_private_method(|key, _| {
                        if key.owner == owner {
                            eval_private_names.push(key.name);
                        }
                    });
                    obj.for_each_private_field(|key, _| {
                        if key.owner == owner {
                            eval_private_names.push(key.name);
                        }
                    });
                }
                if func.is_class_constructor {
                    let proto_ptr = func.cached_prototype_ptr as usize;
                    if proto_ptr != 0 {
                        let proto =
                            unsafe { &*(proto_ptr as *const crate::object::object::JSObject) };
                        proto.for_each_private_kind(|key, _| {
                            if key.owner == proto_ptr {
                                eval_private_names.push(key.name);
                            }
                        });
                        proto.for_each_private_method(|key, _| {
                            if key.owner == proto_ptr {
                                eval_private_names.push(key.name);
                            }
                        });
                        proto.for_each_private_field(|key, _| {
                            if key.owner == proto_ptr {
                                eval_private_names.push(key.name);
                            }
                        });
                    }
                }
                if let Some(uv) = func.upvalues.as_ref() {
                    for &cell_name in uv.upvalue_cells.keys() {
                        if ctx.get_atom_str(cell_name).starts_with('#') {
                            eval_private_names.push(cell_name);
                        }
                    }
                    for &cell_name in uv.upvalues.keys() {
                        if ctx.get_atom_str(cell_name).starts_with('#') {
                            eval_private_names.push(cell_name);
                        }
                    }
                }
            }
            if !frame.var_name_map.is_null() {
                let map = unsafe { &*frame.var_name_map };
                for &(name_atom, _, _) in map.iter() {
                    let name = ctx
                        .get_atom_str(crate::runtime::atom::Atom(name_atom))
                        .to_string();
                    if name.starts_with('#') {
                        eval_private_names.push(crate::runtime::atom::Atom(name_atom));
                    }
                }
            }
        }
        let saved_eval_private_names = ctx.eval_private_names.take();
        ctx.eval_private_names = Some(eval_private_names);
        let eval_result = crate::compiler::eval_code_via_ast_with_opt_level_as_eval_with_caller(
            ctx,
            source,
            ctx.get_compiler_opt_level(),
            is_strict_caller,
            caller_vm_ptr,
            allow_super_property,
            allow_super_call,
            in_field_initializer,
        );
        ctx.eval_private_names = saved_eval_private_names;
        match eval_result {
            Ok(v) => Ok(v),
            Err(e) => self.convert_eval_error_to_exc(ctx, e),
        }
    }

    pub fn indirect_eval(&mut self, ctx: &mut JSContext, source: &str) -> Result<JSValue, String> {
        match crate::eval(ctx, source) {
            Ok(v) => Ok(v),
            Err(e) => self.convert_eval_error_to_exc(ctx, e),
        }
    }

    fn convert_eval_error_to_exc(
        &mut self,
        ctx: &mut JSContext,
        e: String,
    ) -> Result<JSValue, String> {
        if e.starts_with("Uncaught:") {
            let value_str = e[9..].split('\n').next().unwrap_or("").trim();
            let is_error = value_str.contains("Error:")
                || value_str.starts_with("[object ")
                || value_str.starts_with("function ");
            if is_error {
                let mut err = crate::object::object::JSObject::new();
                let name = if value_str.starts_with("SyntaxError") {
                    "SyntaxError"
                } else if value_str.starts_with("ReferenceError") {
                    "ReferenceError"
                } else if value_str.starts_with("TypeError") {
                    "TypeError"
                } else if value_str.starts_with("RangeError") {
                    "RangeError"
                } else {
                    "Error"
                };
                err.set(ctx.intern("name"), JSValue::new_string(ctx.intern(name)));
                err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&e)));
                match name {
                    "TypeError" => {
                        if let Some(proto) = ctx.get_type_error_prototype() {
                            err.prototype = Some(proto);
                        }
                    }
                    "SyntaxError" => {
                        if let Some(proto) = ctx.get_syntax_error_prototype() {
                            err.prototype = Some(proto);
                        }
                    }
                    "ReferenceError" => {
                        if let Some(proto) = ctx.get_reference_error_prototype() {
                            err.prototype = Some(proto);
                        }
                    }
                    _ => {
                        if let Some(proto) = ctx.get_error_prototype() {
                            err.prototype = Some(proto);
                        }
                    }
                }
                let ptr = Box::into_raw(Box::new(err)) as usize;
                ctx.runtime_mut().gc_heap_mut().track(ptr);
                let exc = JSValue::new_object(ptr);
                ctx.pending_exception = Some(exc);
                Ok(JSValue::undefined())
            } else {
                let raw_value = if let Ok(n) = value_str.parse::<i64>() {
                    JSValue::new_int(n)
                } else if let Ok(f) = value_str.parse::<f64>() {
                    JSValue::new_float(f)
                } else if value_str == "true" {
                    JSValue::bool(true)
                } else if value_str == "false" {
                    JSValue::bool(false)
                } else if value_str == "null" {
                    JSValue::null()
                } else if value_str == "undefined" {
                    JSValue::undefined()
                } else {
                    JSValue::undefined()
                };
                ctx.pending_exception = Some(raw_value);
                Ok(JSValue::undefined())
            }
        } else if e.starts_with("Parse error") || e.starts_with("SyntaxError") {
            let mut err = crate::object::object::JSObject::new();
            err.set(
                ctx.intern("name"),
                JSValue::new_string(ctx.intern("SyntaxError")),
            );
            err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&e)));
            if let Some(proto) = ctx.get_syntax_error_prototype() {
                err.prototype = Some(proto);
            }
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            let exc = JSValue::new_object(ptr);
            ctx.pending_exception = Some(exc);
            Ok(JSValue::undefined())
        } else if e.starts_with("ReferenceError") {
            let mut err = crate::object::object::JSObject::new();
            err.set(
                ctx.intern("name"),
                JSValue::new_string(ctx.intern("ReferenceError")),
            );
            err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&e)));
            if let Some(proto) = ctx.get_reference_error_prototype() {
                err.prototype = Some(proto);
            }
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            let exc = JSValue::new_object(ptr);
            ctx.pending_exception = Some(exc);
            Ok(JSValue::undefined())
        } else if e.starts_with("TypeError") {
            let mut err = crate::object::object::JSObject::new();
            err.set(
                ctx.intern("name"),
                JSValue::new_string(ctx.intern("TypeError")),
            );
            err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(&e)));
            if let Some(proto) = ctx.get_type_error_prototype() {
                err.prototype = Some(proto);
            }
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            let exc = JSValue::new_object(ptr);
            ctx.pending_exception = Some(exc);
            Ok(JSValue::undefined())
        } else {
            Err(e)
        }
    }

    fn execute_inner(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
        preserve_registers: bool,
        start_pc: usize,
        setup_frame: bool,
    ) -> Result<ExecutionOutcome, String> {
        let depth = RUST_CALL_DEPTH.with(|d| {
            let v = d.get() + 1;
            d.set(v);
            v
        });
        if depth > 2000 {
            RUST_CALL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            let mut err = crate::object::object::JSObject::new();
            err.set(
                ctx.intern("name"),
                JSValue::new_string(ctx.intern("RangeError")),
            );
            err.set(
                ctx.intern("message"),
                JSValue::new_string(ctx.intern("Maximum call stack size exceeded")),
            );
            if let Some(proto) = ctx.get_range_error_prototype() {
                err.prototype = Some(proto);
            }
            let ptr = Box::into_raw(Box::new(err)) as usize;
            ctx.runtime_mut().gc_heap_mut().track(ptr);
            self.pending_throw = Some(JSValue::new_object(ptr));
            return Err("RangeError: Maximum call stack size exceeded".to_string());
        }
        let result = self.execute_inner_impl(ctx, bytecode, preserve_registers, start_pc, setup_frame);
        RUST_CALL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        result
    }

    fn execute_inner_impl(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
        preserve_registers: bool,
        start_pc: usize,
        setup_frame: bool,
    ) -> Result<ExecutionOutcome, String> {
        self.ctx_ptr = ctx;

        if setup_frame {
            let needed = bytecode.locals_count as usize;
            if needed > self.registers.len() {
                self.registers.resize(needed, JSValue::undefined());
            }
            let global_val = ctx.global();
            let pending_this = self.pending_this.take();
            let frame_this = pending_this.unwrap_or(global_val);
            if !preserve_registers {
                for i in 0..needed {
                    self.registers[i] = if i == 0 {
                        frame_this
                    } else {
                        JSValue::undefined()
                    };
                }
            } else {
                self.registers[0] = frame_this;
            }

            let saved_args = self.pending_saved_args.take();
            let frame = &mut self.frames[0];
            frame.registers_base = 0;
            frame.registers_count = needed;
            frame.locals_count = bytecode.locals_count;
            frame.bytecode_ptr = bytecode.effective_code_ptr();
            frame.bytecode_len = bytecode.effective_code_len();
            frame.constants_ptr = bytecode.effective_const_ptr();
            frame.constants_len = bytecode.effective_const_len();
            frame.return_pc = 0;
            frame.function_ptr = self.pending_function_ptr.take();
            frame.with_env = frame.function_ptr.map(|ptr| unsafe {
                (&*(ptr as *const crate::object::function::JSFunction)).with_env.clone()
            }).flatten();
            frame.this_value = frame_this;
            frame.saved_args.clear();
            if let Some(args) = saved_args {
                frame.saved_args = args;
            }
            frame.upvalue_sync_map = None;
            frame.upvalue_sync_bitset = 0;
            frame.uses_arguments = false;
            frame.var_name_map = std::rc::Rc::as_ptr(&bytecode.var_name_to_slot);
            frame.ic_table_ptr = bytecode.effective_ic_table_ptr();
            frame.has_upvalues = match frame.function_ptr {
                Some(ptr) => {
                    let func = unsafe { JSValue::function_from_ptr(ptr) };
                    func.upvalues
                        .as_ref()
                        .map_or(false, |uv| !uv.upvalue_slots.is_empty())
                }
                None => false,
            };

            self.frame_index = 0;
            self.exception_handlers = self.pending_handlers.take().unwrap_or_default();
            self.frames[0].is_strict_frame = bytecode.is_strict;

            if let Some(ptr) = self.frames[0].function_ptr {
                let js_func = unsafe { JSValue::function_from_ptr(ptr) };
                if js_func.is_arrow() {
                    self.frames[0].this_cell = js_func.arrow_this_cell.clone();
                    self.frames[0].super_ctor = js_func.arrow_super;
                    self.frames[0].super_base = js_func.arrow_super_base;
                    if let Some(cell) = &js_func.arrow_this_cell {
                        let (tv, init) = cell.get();
                        self.frames[0].this_value = tv;
                        self.frames[0].this_initialized = init;
                        self.registers[0] = tv;
                    } else {
                        self.frames[0].this_initialized = js_func.arrow_this_initialized;
                    }
                } else {
                    let super_key = ctx.common_atoms.__super__;
                    if let Some(super_val) = js_func.base.get(super_key) {
                        self.frames[0].super_ctor = super_val;
                    }
                    if js_func.is_derived_constructor {
                        self.frames[0].this_initialized = false;
                    }
                    if !js_func.home_object.is_undefined() {
                        if js_func.home_object.is_object_like() {
                            let home = js_func.home_object.as_object();
                            let base = home
                                .prototype
                                .map(|p| JSValue::new_object(p as usize))
                                .unwrap_or(JSValue::undefined());
                            self.frames[0].super_base = base;
                        }
                    } else {
                        let sc = self.frames[0].super_ctor;
                        if sc.is_object_like() {
                            let base = sc
                                .as_object()
                                .get(ctx.common_atoms.prototype)
                                .unwrap_or(JSValue::undefined());
                            self.frames[0].super_base = base;
                        }
                    }
                }
            }
        }
        let mut start_pc = start_pc;
        if let Some(value) = self.pending_return_value.take() {
            let mut finally_pc = None;
            while let Some(handler) = self.exception_handlers.last().cloned() {
                if handler.frame_index != self.frame_index {
                    break;
                }
                self.exception_handlers.pop();
                if let Some(fp) = handler.finally_pc {
                    finally_pc = Some(fp);
                    break;
                }
            }
            match finally_pc {
                Some(fp) => {
                    self.pending_finally_return = Some((self.frame_index, value));
                    self.pending_finally_depth = self.same_frame_handler_count();
                    start_pc = fp;
                }
                None => {
                    if self.frame_index == 0 {
                        return Ok(ExecutionOutcome::Complete(value));
                    }
                    self.pop_frame(value);
                }
            }
        }
        self.pc = start_pc;
        self.refresh_cache();
        ctx.reset_interrupt_counter();

        loop {
            if self.pending_throw.is_some() {
                let exc = unsafe { self.pending_throw.take().unwrap_unchecked() };
                match self.dispatch_throw_value(ctx, exc) {
                    ThrowDispatch::Caught => {}
                    ThrowDispatch::Uncaught(e) => return Err(e),
                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                }
            }
            if let Some((this_val, fptr)) = self.pending_instance_elements.take() {
                if self.run_instance_elements(ctx, this_val, fptr).is_err() {
                    let caught = self.last_caught_exception.take();
                    let host = ctx.pending_exception.take();
                    let exc = caught.or(host).or_else(|| self.pending_throw.take());
                    if let Some(exc) = exc {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => {}
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
            }
            if self.pending_super_rebind_error {
                self.pending_super_rebind_error = false;
                self.set_pending_reference_error(ctx, "Super constructor already called");
                if let Some(exc) = self.pending_throw.take() {
                    match self.dispatch_throw_value(ctx, exc) {
                        ThrowDispatch::Caught => {}
                        ThrowDispatch::Uncaught(e) => return Err(e),
                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                    }
                }
            }
            if (self.pc & INTERRUPT_POLL_EVERY_MASK) == 0 {
                ctx.check_interrupt()?;
            }
            if self.pc >= self.cached_code_len {
                return Ok(ExecutionOutcome::Complete(JSValue::undefined()));
            }

            let instr_pc = self.pc;
            let op_val = unsafe { *self.cached_code_ptr.add(instr_pc) };
            self.pc = instr_pc + 1;
            if op_val == 90 {
                let dst = unsafe {
                    std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const u16)
                };
                self.pc += 2;
                let src = unsafe {
                    std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const u16)
                };
                self.pc += 2;
                let val = self.get_reg(src);
                self.set_reg(dst, val);
                continue;
            }
            let op = Opcode::from_u8_unchecked(op_val);

            match op {
                Opcode::Nop => {}
                Opcode::ParamInitMark => {
                    let mark = self.read_u8();
                    let frame = &mut self.frames[self.frame_index];
                    frame.in_parameter_initializer = mark != 0;
                    frame.param_env_has_arguments = mark == 2;
                }
                Opcode::End => {
                    if self.frame_index == 0 {
                        return Ok(ExecutionOutcome::Complete(self.get_reg(0)));
                    }
                    self.cleanup_handlers_for_frame();
                    self.pop_frame(self.get_reg(0));
                }
                Opcode::Return => {
                    let src = self.read_u16_pc();
                    let mut ret = self.get_reg(src);
                    {
                        let frame = &self.frames[self.frame_index];
                        if frame.is_constructor
                            && !Self::frame_is_derived_constructor(frame)
                            && !ret.is_object_like()
                        {
                            ret = frame.this_value;
                        }
                    }
                    let frame = &self.frames[self.frame_index];

                    if frame.is_constructor {
                        if let Some(fptr) = frame.function_ptr {
                            let func =
                                unsafe { &*(fptr as *const crate::object::function::JSFunction) };
                            if let Some(ref bc) = func.bytecode {
                                if bc.is_simple_constructor
                                    && bc.cached_constructor_final_shape.is_none()
                                {
                                    if ret.is_object() {
                                        let obj =
                                            unsafe { JSValue::object_from_ptr(ret.get_ptr()) };
                                        if let Some(shape) = obj.shape_ptr() {
                                            let func_mut = unsafe {
                                                &mut *(fptr
                                                    as *mut crate::object::function::JSFunction)
                                            };
                                            if let Some(ref mut bc_mut) = func_mut.bytecode {
                                                bc_mut.cached_constructor_final_shape = Some(shape);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    match self.execute_return_completion(ctx, ret) {
                        Ok(Some(outcome)) => return Ok(outcome),
                        Ok(None) => {}
                        Err(e) => return Err(e),
                    }
                    continue;
                }
                Opcode::Yield => {
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    self.last_yield_src = Some(src);
                    self.last_yield_is_result = false;
                    return Ok(ExecutionOutcome::Yield(val));
                }
                Opcode::YieldResult => {
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    self.last_yield_src = Some(src);
                    self.last_yield_is_result = true;
                    return Ok(ExecutionOutcome::Yield(val));
                }
                Opcode::YieldStarStep => {
                    let iter_reg = self.read_u16_pc();
                    let value_reg = self.read_u16_pc();
                    let done_reg = self.read_u16_pc();
                    let returning_reg = self.read_u16_pc();
                    match self.yield_star_step(
                        ctx,
                        iter_reg,
                        value_reg,
                        done_reg,
                        returning_reg,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            if self.pending_throw.is_some() {
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Opcode::AsyncYieldStarStep => {
                    let instr_pc = self.pc - 1;
                    let iter_reg = self.read_u16_pc();
                    let value_reg = self.read_u16_pc();
                    let done_reg = self.read_u16_pc();
                    let returning_reg = self.read_u16_pc();
                    let state_reg = self.read_u16_pc();
                    let await_reg = self.read_u16_pc();
                    match self.async_yield_star_step(
                        ctx,
                        instr_pc,
                        iter_reg,
                        value_reg,
                        done_reg,
                        returning_reg,
                        state_reg,
                        await_reg,
                    ) {
                        Ok(true) => return Ok(ExecutionOutcome::Yield(JSValue::undefined())),
                        Ok(false) => {}
                        Err(e) => {
                            if self.pending_throw.is_some() {
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Opcode::Await => {
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    if self.async_step {
                        let promise = if crate::builtins::promise::is_promise(&val) {
                            val
                        } else {
                            ctx.call_builtin("promise_resolve", &[val])
                        };
                        self.await_suspend = Some((promise, src));
                        return Ok(ExecutionOutcome::Yield(JSValue::undefined()));
                    }
                    if crate::builtins::promise::is_promise(&val) {
                        let ptr = val.get_ptr();
                        let promise_obj =
                            unsafe { &*(ptr as *mut crate::object::object::JSObject) };
                        let state_atom = ctx.common_atoms.__promise_state__;
                        let result_atom = ctx.common_atoms.__promise_result__;
                        let state = promise_obj
                            .get(state_atom)
                            .unwrap_or(JSValue::new_int(0))
                            .get_int();
                        let result = promise_obj.get(result_atom).unwrap_or(JSValue::undefined());
                        match state {
                            1 => {
                                self.set_reg(src, result);
                            }
                            2 => {
                                if self.frame_index == 0 {
                                    let rejected = ctx.call_builtin("promise_reject", &[result]);
                                    return Ok(ExecutionOutcome::Complete(rejected));
                                }
                                let mut found_async = false;
                                for i in (0..=self.frame_index).rev() {
                                    if self.frames[i].is_async {
                                        found_async = true;
                                        while self.frame_index > i {
                                            self.pop_frame(JSValue::undefined());
                                        }
                                        let rejected =
                                            ctx.call_builtin("promise_reject", &[result]);
                                        self.set_reg(0, rejected);
                                        break;
                                    }
                                }
                                if !found_async {
                                    return Err(format!("Uncaught (in promise): {:?}", result));
                                }
                            }
                            _ => {
                                self.set_reg(src, JSValue::undefined());
                            }
                        }
                    } else {
                        self.set_reg(src, val);
                    }
                }

                Opcode::LoadConst => {
                    let dst = self.read_u16_pc();
                    let idx = self.read_u32() as usize;
                    let val = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    self.set_reg(dst, val);
                }
                Opcode::LoadInt => {
                    let dst = self.read_u16_pc();
                    let val = self.read_i32() as i64;
                    self.set_reg(dst, JSValue::new_int(val));
                }
                Opcode::LoadInt8 => {
                    let dst = self.read_u16_pc();
                    let val = self.read_u8() as i8 as i64;
                    self.set_reg(dst, JSValue::new_int(val));
                }
                Opcode::LoadTrue => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, JSValue::bool(true));
                }
                Opcode::LoadFalse => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, JSValue::bool(false));
                }
                Opcode::LoadNull => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, JSValue::null());
                }
                Opcode::LoadUndefined => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, JSValue::undefined());
                }

                Opcode::Move => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    self.set_reg(dst, val);
                }

                Opcode::Add => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    if JSValue::both_int(&a, &b) {
                        self.set_reg(dst, JSValue::new_int(a.get_int() + b.get_int()));
                    } else if JSValue::both_raw_float(&a, &b) {
                        self.set_reg(dst, JSValue::new_float_raw(a.get_float() + b.get_float()));
                    } else {
                        let result = self.add_slow(&a, &b, ctx);
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => match o {
                                    ExecutionOutcome::Complete(v) => {
                                        self.set_reg(dst, v);
                                        return Ok(ExecutionOutcome::Complete(v));
                                    }
                                    _ => {
                                        self.set_reg(dst, JSValue::undefined());
                                        return Ok(
                                            ExecutionOutcome::Complete(JSValue::undefined()),
                                        );
                                    }
                                },
                            }
                        }
                        self.set_reg(dst, result);
                    }
                }
                Opcode::AddNum => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        JSValue::new_int(a.get_int() + b.get_int())
                    } else {
                        let fa = if a.is_float() {
                            a.get_float()
                        } else if a.is_int() {
                            a.get_int() as f64
                        } else {
                            Self::js_to_number(&a, ctx)
                        };
                        let fb = if b.is_float() {
                            b.get_float()
                        } else if b.is_int() {
                            b.get_int() as f64
                        } else {
                            Self::js_to_number(&b, ctx)
                        };
                        JSValue::new_float(fa + fb)
                    };
                    self.set_reg(dst, result);
                }
                Opcode::AddImm8 => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let imm = self.read_u8() as i8 as i64;
                    let val = self.get_reg(src);
                    let result = if val.is_int() {
                        JSValue::new_int(val.get_int() + imm)
                    } else if val.is_float() {
                        JSValue::new_float_raw(val.get_float() + imm as f64)
                    } else {
                        let b = JSValue::new_int(imm);
                        self.add_slow(&val, &b, ctx)
                    };
                    self.set_reg(dst, result);
                }
                Opcode::SubNum => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        JSValue::new_int(a.get_int() - b.get_int())
                    } else {
                        let fa = if a.is_float() {
                            a.get_float()
                        } else if a.is_int() {
                            a.get_int() as f64
                        } else {
                            Self::js_to_number(&a, ctx)
                        };
                        let fb = if b.is_float() {
                            b.get_float()
                        } else if b.is_int() {
                            b.get_int() as f64
                        } else {
                            Self::js_to_number(&b, ctx)
                        };
                        JSValue::new_float(fa - fb)
                    };
                    self.set_reg(dst, result);
                }
                Opcode::MulNum => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let ai = a.get_int();
                        let bi = b.get_int();
                        match ai.checked_mul(bi) {
                            Some(prod) if prod >= -(1i64 << 46) && prod < (1i64 << 46) => {
                                JSValue::new_int(prod)
                            }
                            _ => JSValue::new_float(ai as f64 * bi as f64),
                        }
                    } else if a.is_float() && b.is_float() {
                        JSValue::new_float_raw(a.get_float() * b.get_float())
                    } else {
                        let fa = if a.is_float() {
                            a.get_float()
                        } else if a.is_int() {
                            a.get_int() as f64
                        } else {
                            Self::js_to_number(&a, ctx)
                        };
                        let fb = if b.is_float() {
                            b.get_float()
                        } else if b.is_int() {
                            b.get_int() as f64
                        } else {
                            Self::js_to_number(&b, ctx)
                        };
                        JSValue::new_float(fa * fb)
                    };
                    self.set_reg(dst, result);
                }
                Opcode::DivNum => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let ai = a.get_int();
                        let bi = b.get_int();
                        if bi != 0 && ai % bi == 0 {
                            JSValue::new_int(ai / bi)
                        } else if bi != 0 {
                            JSValue::new_float(ai as f64 / bi as f64)
                        } else {
                            JSValue::new_float(f64::NAN)
                        }
                    } else if a.is_float() && b.is_float() {
                        JSValue::new_float_raw(a.get_float() / b.get_float())
                    } else {
                        let fa = if a.is_float() {
                            a.get_float()
                        } else if a.is_int() {
                            a.get_int() as f64
                        } else {
                            Self::js_to_number(&a, ctx)
                        };
                        let fb = if b.is_float() {
                            b.get_float()
                        } else if b.is_int() {
                            b.get_int() as f64
                        } else {
                            Self::js_to_number(&b, ctx)
                        };
                        if fb != 0.0 {
                            JSValue::new_float(fa / fb)
                        } else {
                            JSValue::new_float(f64::NAN)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Sub => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let v = a.get_int() - b.get_int();
                        if v == 0 && a.get_int() < b.get_int() {
                            JSValue::new_float_raw(-0.0f64)
                        } else {
                            JSValue::new_int(v)
                        }
                    } else if JSValue::both_raw_float(&a, &b) {
                        JSValue::new_float_raw(a.get_float() - b.get_float())
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        Self::create_bigint(a_int - b_int)
                    } else if a.is_int() && b.is_float() {
                        JSValue::new_float_raw(a.get_int() as f64 - b.get_float())
                    } else if a.is_float() && b.is_int() {
                        JSValue::new_float_raw(a.get_float() - b.get_int() as f64)
                    } else {
                        match self.arithmetic_binary_op(&a, &b, ArithOp::Sub, ctx) {
                            Some(v) => v,
                            None => continue,
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::SubImm8 => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let imm = self.read_u8() as i8 as i64;
                    let a = self.get_reg(a_reg);
                    let result = if a.is_int() {
                        JSValue::new_int(a.get_int() - imm)
                    } else if a.is_float() {
                        JSValue::new_float(a.get_float() - imm as f64)
                    } else {
                        let to_num = self.to_numeric(&a, ctx);
                        match to_num {
                            Some(v) => {
                                if v.is_bigint() {
                                    self.set_pending_type_error(
                                        ctx,
                                        "Cannot mix BigInt and other types",
                                    );
                                    continue;
                                }
                                JSValue::new_float(
                                    Self::numeric_to_f64(&v, ctx) - imm as f64,
                                )
                            }
                            None => continue,
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Mul => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let ai = a.get_int();
                        let bi = b.get_int();
                        match ai.checked_mul(bi) {
                            Some(prod) if prod >= -(1i64 << 46) && prod < (1i64 << 46) => {
                                if prod == 0 && (ai < 0) != (bi < 0) {
                                    JSValue::new_float_raw(-0.0f64)
                                } else {
                                    JSValue::new_int(prod)
                                }
                            }
                            _ => JSValue::new_float(ai as f64 * bi as f64),
                        }
                    } else if JSValue::both_raw_float(&a, &b) {
                        JSValue::new_float_raw(a.get_float() * b.get_float())
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        Self::create_bigint(a_int * b_int)
                    } else if a.is_int() && b.is_float() {
                        JSValue::new_float(a.get_int() as f64 * b.get_float())
                    } else if a.is_float() && b.is_int() {
                        JSValue::new_float(a.get_float() * b.get_int() as f64)
                    } else {
                        match self.arithmetic_binary_op(&a, &b, ArithOp::Mul, ctx) {
                            Some(v) => v,
                            None => continue,
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Div => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let ai = a.get_int();
                        let bi = b.get_int();
                        if bi != 0 && ai % bi == 0 {
                            if ai == 0 && bi < 0 {
                                JSValue::new_float_raw(-0.0f64)
                            } else {
                                JSValue::new_int(ai / bi)
                            }
                        } else if bi != 0 {
                            JSValue::new_float(ai as f64 / bi as f64)
                        } else {
                            JSValue::new_float(ai as f64 / 0.0f64)
                        }
                    } else if JSValue::both_raw_float(&a, &b) {
                        JSValue::new_float_raw(a.get_float() / b.get_float())
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        if b_int != 0 {
                            Self::create_bigint(a_int / b_int)
                        } else {
                            self.set_pending_range_error(ctx, "Division by zero");
                            continue;
                        }
                    } else if a.is_int() && b.is_float() {
                        JSValue::new_float(a.get_int() as f64 / b.get_float())
                    } else if a.is_float() && b.is_int() {
                        let bi = b.get_int();
                        if bi != 0 {
                            JSValue::new_float(a.get_float() / bi as f64)
                        } else {
                            JSValue::new_float(a.get_float() / 0.0f64)
                        }
                    } else {
                        match self.arithmetic_binary_op(&a, &b, ArithOp::Div, ctx) {
                            Some(v) => v,
                            None => continue,
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Mod => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = if JSValue::both_int(&a, &b) {
                        let bi = b.get_int();
                        if bi != 0 {
                            let r = a.get_int() % bi;
                            if r == 0 && a.get_int() < 0 {
                                JSValue::new_float_raw(-0.0f64)
                            } else {
                                JSValue::new_int(r)
                            }
                        } else {
                            JSValue::new_float(f64::NAN)
                        }
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        if b_int != 0 {
                            Self::create_bigint(a_int % b_int)
                        } else {
                            self.set_pending_range_error(ctx, "Division by zero");
                            continue;
                        }
                    } else {
                        match self.arithmetic_binary_op(&a, &b, ArithOp::Mod, ctx) {
                            Some(v) => v,
                            None => continue,
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Pow => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.pow_op(&a, &b, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::BitAnd => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::And, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::BitOr => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::Or, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::BitXor => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::Xor, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::BitNot => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let a = match self.to_numeric(&a, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    let result = if a.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        Self::create_bigint(!a_int)
                    } else {
                        let na = if a.is_int() {
                            a.get_int() as f64
                        } else {
                            Self::js_to_number(&a, ctx)
                        };
                        let i32_val = Self::to_int32(na);
                        JSValue::new_int(!(i32_val as i64))
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Shl => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::Shl, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Shr => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::Shr, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::UShr => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.numeric_binary_op(&a, &b, BitwiseOp::UShr, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Neg => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let a = match self.to_numeric(&a, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    let result = if a.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        Self::create_bigint(-a_int)
                    } else if a.is_int() {
                        let v = a.get_int();
                        if v == 0 {
                            JSValue::new_float_raw(-0.0f64)
                        } else {
                            JSValue::new_int(-v)
                        }
                    } else if a.is_float() {
                        JSValue::new_float_raw(-a.get_float())
                    } else {
                        JSValue::new_float_raw(-Self::js_to_number(&a, ctx))
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Pos => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let val = match self.to_numeric(&val, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };

                    if val.is_bigint() {
                        self.set_pending_type_error(ctx, "Cannot convert a BigInt value to a number");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if val.is_int() || val.is_float() {
                        self.set_reg(dst, val);
                    } else {
                        let n = Self::js_to_number(&val, ctx);
                        self.set_reg(dst, JSValue::new_float(n));
                    }
                }

                Opcode::ToNumeric => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let val = match self.to_numeric(&val, ctx) {
                        Some(v) => v,
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    };
                    if val.is_int() || val.is_float() || val.is_bigint() {
                        self.set_reg(dst, val);
                    } else {
                        let n = Self::js_to_number(&val, ctx);
                        self.set_reg(dst, JSValue::new_float(n));
                    }
                }

                Opcode::Lt => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.abstract_less_than(&a, &b, ctx, true) {
                        Some(v) => JSValue::bool(v),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            JSValue::bool(false)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Lte => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.abstract_less_than(&b, &a, ctx, false) {
                        Some(v) => JSValue::bool(!v),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            JSValue::bool(false)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::LteImm8 => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let imm = self.read_u8() as i8 as i64;
                    let a = self.get_reg(a_reg);
                    let imm_val = JSValue::new_int(imm);
                    let result = match self.abstract_less_than(&imm_val, &a, ctx, false) {
                        Some(v) => JSValue::bool(!v),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            JSValue::bool(false)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Gt => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.abstract_less_than(&b, &a, ctx, false) {
                        Some(v) => JSValue::bool(v),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            JSValue::bool(false)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Gte => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.abstract_less_than(&a, &b, ctx, true) {
                        Some(v) => JSValue::bool(!v),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            JSValue::bool(false)
                        }
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Eq => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => JSValue::bool(eq),
                        None => continue,
                    };
                    self.set_reg(dst, result);
                }
                Opcode::Neq => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let result = match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => JSValue::bool(!eq),
                        None => continue,
                    };
                    self.set_reg(dst, result);
                }
                Opcode::StrictEq => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    self.set_reg(dst, JSValue::bool(a.strict_eq(&b)));
                }
                Opcode::StrictNeq => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    self.set_reg(dst, JSValue::bool(!a.strict_eq(&b)));
                }
                Opcode::Not => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    self.set_reg(dst, JSValue::bool(!a.is_truthy()));
                }
                Opcode::TypeOf => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let ca = &ctx.common_atoms;
                    let atom = if a.is_undefined() {
                        ca.typeof_undefined
                    } else if a.is_null() {
                        ca.typeof_object
                    } else if a.is_bool() {
                        ca.typeof_boolean
                    } else if a.is_int() || a.is_float() {
                        ca.typeof_number
                    } else if a.is_string() {
                        ca.typeof_string
                    } else if a.is_symbol() {
                        ca.typeof_symbol
                    } else if a.is_bigint() {
                        ca.typeof_bigint
                    } else if a.is_function() {
                        ca.typeof_function
                    } else if a.is_object() {
                        let obj = a.as_object();
                        let is_function_prototype =
                            ctx.get_function_prototype().map_or(false, |function_prototype| {
                                a.get_ptr() as *mut crate::object::object::JSObject
                                    == function_prototype
                            });
                        if obj.get(ctx.common_atoms.__boundFn).is_some() || is_function_prototype {
                            ca.typeof_function
                        } else {
                            ca.typeof_object
                        }
                    } else {
                        ca.typeof_object
                    };
                    self.set_reg(dst, JSValue::new_string(atom));
                }

                Opcode::MathSin
                | Opcode::MathCos
                | Opcode::MathSqrt
                | Opcode::MathAbs
                | Opcode::MathFloor
                | Opcode::MathCeil
                | Opcode::MathRound => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let a = self.get_reg(src);

                    if a.is_int()
                        && matches!(
                            op,
                            Opcode::MathFloor
                                | Opcode::MathCeil
                                | Opcode::MathRound
                                | Opcode::MathAbs
                        )
                    {
                        let result = if matches!(op, Opcode::MathAbs) {
                            let i = a.get_int();
                            if i == i64::MIN {
                                JSValue::new_float(-(i64::MIN as f64))
                            } else if i < 0 {
                                JSValue::new_int(-i)
                            } else {
                                a
                            }
                        } else {
                            a
                        };
                        self.set_reg(dst, result);
                        continue;
                    }
                    let f = if a.is_int() {
                        a.get_int() as f64
                    } else if a.is_float() {
                        a.get_float()
                    } else {
                        Self::js_to_number(&a, ctx)
                    };
                    let result = match op {
                        Opcode::MathSin => f.sin(),
                        Opcode::MathCos => f.cos(),
                        Opcode::MathSqrt => f.sqrt(),
                        Opcode::MathAbs => f.abs(),
                        Opcode::MathFloor => {
                            let floored = f.floor();
                            if floored == 0.0 && floored.is_sign_negative() {
                                self.set_reg(dst, JSValue::new_float(-0.0));
                                continue;
                            }
                            const MAX_SAFE: f64 = (1i64 << 47) as f64;
                            if floored >= -MAX_SAFE && floored <= MAX_SAFE {
                                self.set_reg(dst, JSValue::new_int(floored as i64));
                                continue;
                            }
                            floored
                        }
                        Opcode::MathCeil => {
                            let ceiled = f.ceil();
                            if ceiled == 0.0 && ceiled.is_sign_negative() {
                                self.set_reg(dst, JSValue::new_float(-0.0));
                                continue;
                            }
                            const MAX_SAFE: f64 = (1i64 << 47) as f64;
                            if ceiled >= -MAX_SAFE && ceiled <= MAX_SAFE {
                                self.set_reg(dst, JSValue::new_int(ceiled as i64));
                                continue;
                            }
                            ceiled
                        }
                        Opcode::MathRound => {
                            if f.is_nan() || f.is_infinite() {
                                self.set_reg(dst, JSValue::new_float(f));
                                continue;
                            }
                            if f == 0.0 {
                                self.set_reg(dst, JSValue::new_float(if f.is_sign_negative() { -0.0 } else { f }));
                                continue;
                            }
                            let r = (f + 0.5).floor();
                            let rounded = if f < r - 0.5 { r - 1.0 } else { r };
                            if rounded == 0.0 && f < 0.0 {
                                self.set_reg(dst, JSValue::new_float(-0.0));
                                continue;
                            }
                            const MAX_SAFE: f64 = (1i64 << 47) as f64;
                            if rounded >= -MAX_SAFE && rounded <= MAX_SAFE {
                                self.set_reg(dst, JSValue::new_int(rounded as i64));
                                continue;
                            }
                            rounded
                        }
                        _ => unreachable!("unhandled math op"),
                    };
                    self.set_reg(dst, JSValue::new_float(result));
                }
                Opcode::MathPow => {
                    let dst = self.read_u16_pc();
                    let base_reg = self.read_u16_pc();
                    let exp_reg = self.read_u16_pc();
                    let base = self.get_reg(base_reg);
                    let exp = self.get_reg(exp_reg);
                    let b = if base.is_int() {
                        base.get_int() as f64
                    } else if base.is_float() {
                        base.get_float()
                    } else {
                        Self::js_to_number(&base, ctx)
                    };
                    let e = if exp.is_int() {
                        exp.get_int() as f64
                    } else if exp.is_float() {
                        exp.get_float()
                    } else {
                        Self::js_to_number(&exp, ctx)
                    };
                    let result = if e.is_nan() {
                        f64::NAN
                    } else if b.abs() == 1.0 && e.is_infinite() {
                        f64::NAN
                    } else if e == 2.0 {
                        b * b
                    } else if e == 0.5 {
                        b.sqrt()
                    } else if e >= 2.0 && e <= 8.0 && e == e.floor() {
                        let mut r = b;
                        let mut n = e as i32 - 1;
                        while n > 0 {
                            r *= b;
                            n -= 1;
                        }
                        r
                    } else if e == -1.0 {
                        1.0 / b
                    } else if e == 1.0 {
                        b
                    } else {
                        b.powf(e)
                    };
                    self.set_reg(dst, JSValue::new_float(result));
                }
                Opcode::MathMin | Opcode::MathMax => {
                    let dst = self.read_u16_pc();
                    let a_reg = self.read_u16_pc();
                    let b_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b = self.get_reg(b_reg);
                    let fa = if a.is_int() {
                        a.get_int() as f64
                    } else if a.is_float() {
                        a.get_float()
                    } else if a.is_object() || a.is_function() {
                        match crate::builtins::global::js_to_number_value(ctx, &a) {
                            Ok(v) => v,
                            Err(()) => f64::NAN,
                        }
                    } else {
                        Self::js_to_number(&a, ctx)
                    };
                    let fb = if b.is_int() {
                        b.get_int() as f64
                    } else if b.is_float() {
                        b.get_float()
                    } else if b.is_object() || b.is_function() {
                        match crate::builtins::global::js_to_number_value(ctx, &b) {
                            Ok(v) => v,
                            Err(()) => f64::NAN,
                        }
                    } else {
                        Self::js_to_number(&b, ctx)
                    };
                    let result = if fa.is_nan() || fb.is_nan() {
                        f64::NAN
                    } else if op == Opcode::MathMin {
                        if fa == 0.0 && fb == 0.0 {
                            if fa.is_sign_negative() { fa } else { fb }
                        } else {
                            fa.min(fb)
                        }
                    } else {
                        if fa == 0.0 && fb == 0.0 {
                            if fb.is_sign_positive() { fb } else { fa }
                        } else {
                            fa.max(fb)
                        }
                    };
                    self.set_reg(dst, JSValue::new_float(result));
                }

                Opcode::Jump => {
                    let offset = self.read_i32();
                    self.pc = (self.pc as i64 + offset as i64) as usize;
                }
                Opcode::JumpIf => {
                    let src = self.read_u16_pc();
                    let offset = self.read_i32();
                    if self.get_reg(src).is_truthy() {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::JumpIfNot => {
                    let src = self.read_u16_pc();
                    let offset = self.read_i32();
                    if !self.get_reg(src).is_truthy() {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::JumpIfNullish => {
                    let src = self.read_u16_pc();
                    let offset = self.read_i32();
                    let v = self.get_reg(src);
                    if v.is_null() || v.is_undefined() {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::Jump8 => {
                    let offset = self.read_u8() as i8;
                    self.pc = (self.pc as i64 + offset as i64) as usize;
                }
                Opcode::JumpIf8 => {
                    let src = self.read_u16_pc();
                    let offset = self.read_u8() as i8;
                    if self.get_reg(src).is_truthy() {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::JumpIfNot8 => {
                    let src = self.read_u16_pc();
                    let offset = self.read_u8() as i8;
                    if !self.get_reg(src).is_truthy() {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::EndFinally => {
                    if self
                        .pending_finally_rethrow
                        .map_or(false, |(frame, _)| frame == self.frame_index)
                    {
                        let (_, value) = self.pending_finally_rethrow.take().unwrap();
                        match self.dispatch_throw_value(ctx, value) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }

                    if self
                        .pending_finally_return
                        .map_or(false, |(frame, _)| frame == self.frame_index)
                    {
                        let (_, value) = self.pending_finally_return.take().unwrap();
                        match self.execute_return_completion(ctx, value) {
                            Ok(Some(outcome)) => return Ok(outcome),
                            Ok(None) => {}
                            Err(e) => return Err(e),
                        }
                        continue;
                    }

                    if self
                        .pending_finally_jump
                        .map_or(false, |(frame, _, _)| frame == self.frame_index)
                    {
                        let (_, target, target_depth) =
                            self.pending_finally_jump.take().unwrap();
                        self.execute_jump_completion(target, target_depth);
                        continue;
                    }
                }
                Opcode::ResetPerIterVar => {
                    let slot = self.read_u16_pc();
                    if self.frame_index < self.frames.len() {
                        let frame = &mut self.frames[self.frame_index];
                        if let Some(ref mut map) = frame.upvalue_sync_map {
                            map.remove(&slot);
                            if slot < 64 {
                                frame.upvalue_sync_bitset &= !(1u64 << slot);
                            }
                            if map.is_empty() {
                                frame.upvalue_sync_map = None;
                                frame.upvalue_sync_bitset = 0;
                            }
                        }
                        self.refresh_cache();
                    }
                }
                Opcode::JumpBreak => {
                    let target_depth = self.read_u16_pc() as usize;
                    let offset = self.read_i32();
                    let target = (self.pc as i32 + offset) as usize;
                    self.execute_jump_completion(target, target_depth);
                }
                Opcode::Throw => {
                    let src = self.read_u16_pc();
                    let value = self.get_reg(src);

                    let find_async_frame = |frames: &[CallFrame], frame_index: usize| {
                        for i in (0..=frame_index).rev() {
                            if frames[i].is_async {
                                return Some(i);
                            }
                        }
                        None
                    };

                    if let Some(handler) = self.exception_handlers.last().cloned() {
                        self.exception_handlers.pop();
                        let handled_frame = handler.frame_index;
                        let same_frame_after = self
                            .exception_handlers
                            .iter()
                            .filter(|h| h.frame_index == self.frame_index)
                            .count();
                        self.clear_pending_finally_if_escaped(handled_frame, same_frame_after);

                        if self.frame_index != handler.frame_index {
                            if let Some(async_idx) =
                                find_async_frame(&self.frames, self.frame_index)
                            {
                                if async_idx > handler.frame_index {
                                    while self.frame_index > async_idx {
                                        if self.frame_index == 0 {
                                            break;
                                        }
                                        self.frame_index -= 1;
                                    }

                                    let rejected = ctx.call_builtin("promise_reject", &[value]);
                                    if self.frame_index == 0 {
                                        return Ok(ExecutionOutcome::Complete(rejected));
                                    }
                                    let return_pc = self.frames[self.frame_index].return_pc;
                                    let dst_reg = self.frames[self.frame_index].dst_reg;
                                    self.frame_index -= 1;
                                    self.pc = return_pc;
                                    self.refresh_cache();
                                    self.set_reg(dst_reg, rejected);
                                    continue;
                                }
                            }
                            while self.frame_index > handler.frame_index {
                                if self.frame_index == 0 {
                                    break;
                                }
                                let return_pc = self.frames[self.frame_index].return_pc;
                                let _ = return_pc;
                                self.frame_index -= 1;
                            }
                        }

                        // If catch_pc equals finally_pc, the exception should run the
                        // finally body first (then rethrow)
                        if handler
                            .finally_pc
                            .map_or(false, |fp| fp == handler.catch_pc)
                        {
                            self.finally_rethrow = Some(value);
                            self.pc = handler.catch_pc;
                        } else {
                            self.pc = handler.catch_pc;
                        }
                        self.refresh_cache();
                        self.caught_exception_value = Some(value);
                    } else {
                        self.clear_pending_finally_for_frame(self.frame_index);
                        if let Some(async_idx) = find_async_frame(&self.frames, self.frame_index) {
                            let rejected = ctx.call_builtin("promise_reject", &[value]);
                            if async_idx == 0 {
                                return Ok(ExecutionOutcome::Complete(rejected));
                            }
                            while self.frame_index > async_idx {
                                self.frame_index -= 1;
                            }
                            let return_pc = self.frames[self.frame_index].return_pc;
                            let dst_reg = self.frames[self.frame_index].dst_reg;
                            self.frame_index -= 1;
                            self.pc = return_pc;
                            self.refresh_cache();
                            self.set_reg(dst_reg, rejected);
                            continue;
                        }
                        self.pending_throw = Some(value);
                        let msg = self.format_thrown_value(&value, ctx);
                        let trace = self.format_stack_trace(ctx);
                        return Err(format!("Uncaught: {}\nStack trace:\n{}", msg, trace));
                    }
                }
                Opcode::Try => {
                    let catch_pc = self.read_i32() as usize;
                    let finally_pc = self.read_i32() as usize;
                    self.exception_handlers.push(ExceptionHandler {
                        frame_index: self.frame_index,
                        catch_pc,
                        finally_pc: if finally_pc > 0 {
                            Some(finally_pc)
                        } else {
                            None
                        },
                    });
                }
                Opcode::Catch => {
                    self.exception_handlers.pop();
                }
                Opcode::Finally => {
                    // Move finally_rethrow to pending so it can be rethrown
                    // AFTER the finally body executes
                    if let Some(exc) = self.finally_rethrow.take() {
                        self.pending_finally_rethrow = Some((self.frame_index, exc));
                        self.pending_finally_depth = self.same_frame_handler_count();
                    }
                }

                Opcode::GetLocal => {
                    let dst = self.read_u16_pc();
                    let idx = self.read_u32() as u16;
                    self.set_reg(dst, self.get_reg(idx));
                }
                Opcode::SetLocal => {
                    let idx = self.read_u32() as u16;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    self.set_reg(idx, val);
                }
                Opcode::IncLocal | Opcode::DecLocal => {
                    let slot = self.read_u16_pc();
                    let val = self.get_reg(slot);
                    let delta: i128 = if op == Opcode::IncLocal { 1 } else { -1 };
                    let result = if val.is_int() {
                        JSValue::new_int(val.get_int() + delta as i64)
                    } else if val.is_bigint() {
                        let big = Self::get_bigint_int(&val).unwrap_or(0);
                        Self::create_bigint(big + delta)
                    } else if val.is_float() {
                        JSValue::new_float_raw(val.get_float() + delta as f64)
                    } else {
                        match self.to_numeric(&val, ctx) {
                            Some(n) => {
                                if n.is_bigint() {
                                    let big = Self::get_bigint_int(&n).unwrap_or(0);
                                    Self::create_bigint(big + delta)
                                } else if n.is_int() {
                                    JSValue::new_int(n.get_int() + delta as i64)
                                } else {
                                    JSValue::new_float_raw(
                                        Self::js_to_number(&n, ctx) + delta as f64,
                                    )
                                }
                            }
                            None => {
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                                continue;
                            }
                        }
                    };
                    self.set_reg(slot, result);
                }
                Opcode::GetGlobal => {
                    let ic_pc = self.pc - 1;
                    let dst = self.read_u16_pc();
                    let idx = self.read_u32() as usize;
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let mut resolved = false;
                    if self.eval_binding_frames > 0 {
                        if let Some(val) =
                            Self::scan_eval_bindings(&self.frames, self.frame_index, atom.0)
                        {
                            self.set_reg(dst, val);
                            resolved = true;
                        }
                    }
                    if !resolved {
                        if let Some(val) = self.get_var_in_caller_vm(atom.0) {
                            self.set_reg(dst, val);
                        } else {
                            let global = ctx.global();
                            let mut result = JSValue::undefined();
                            if global.is_object() {
                                let global_obj = global.as_object();

                                let global_shape_id = global_obj.shape_id_cache;
                                let ic_table_ptr = self.cached_ic_table_ptr;
                                let mut ic_hit = false;
                                if !ic_table_ptr.is_null() {
                                    if let Some(offset) = unsafe {
                                        (*ic_table_ptr).get_global_cache(
                                            ic_pc,
                                            global_shape_id,
                                            atom.0,
                                        )
                                    } {
                                        result = global_obj
                                            .get_by_offset(offset as usize)
                                            .unwrap_or(JSValue::undefined());
                                        ic_hit = true;
                                    }
                                }
                                if !ic_hit {
                                    if let Some(accessor) = global_obj.get_own_accessor_entry(atom)
                                    {
                                        if let Some(getter) = accessor.get {
                                            if getter.is_function() {
                                                match self.call_function_with_this(
                                                    ctx,
                                                    getter,
                                                    global,
                                                    &[],
                                                ) {
                                                    Ok(v) => result = v,
                                                    Err(msg) => {
                                                        self.set_pending_type_error(ctx, &msg);
                                                        if let Some(exc) = self.pending_throw.take()
                                                        {
                                                            match self
                                                                .dispatch_throw_value(ctx, exc)
                                                            {
                                                                ThrowDispatch::Caught => {}
                                                                ThrowDispatch::Uncaught(e) => {
                                                                    return Err(e);
                                                                }
                                                                ThrowDispatch::AsyncComplete(o) => {
                                                                    return Ok(o);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else if let Some(value) = global_obj.get(atom) {
                                        result = value;

                                        if !ic_table_ptr.is_null() {
                                            if let Some(offset) = global_obj.find_offset(atom) {
                                                unsafe {
                                                    (*ic_table_ptr).insert_global_cache(
                                                        ic_pc,
                                                        global_shape_id,
                                                        offset as u32,
                                                        atom.0,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            self.set_reg(dst, result);
                        }
                    }
                }
                Opcode::SetGlobal => {
                    let idx = self.read_u32() as usize;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let is_strict = self.frames[self.frame_index].is_strict_frame;

                    if self.caller_vm.is_some() {
                        let caller_var_set = self.set_var_in_caller_vm(ctx, atom.0, val);
                        if matches!(caller_var_set, CallerVarSet::Immutable) {
                            if is_strict {
                                self.set_pending_type_error(
                                    ctx,
                                    "Assignment to constant variable.",
                                );
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                        } else if matches!(caller_var_set, CallerVarSet::NotFound) {
                            if is_strict {
                                let name_str = ctx.get_atom_str(atom).to_string();
                                let err_msg = format!("{} is not defined", name_str);
                                let mut err = crate::object::object::JSObject::new();
                                err.set(
                                    ctx.intern("name"),
                                    JSValue::new_string(ctx.intern("ReferenceError")),
                                );
                                err.set(
                                    ctx.intern("message"),
                                    JSValue::new_string(ctx.intern(&err_msg)),
                                );
                                if let Some(proto) = ctx.get_reference_error_prototype() {
                                    err.prototype = Some(proto);
                                }
                                let ptr = Box::into_raw(Box::new(err)) as usize;
                                ctx.runtime_mut().gc_heap_mut().track(ptr);
                                self.pending_throw = Some(JSValue::new_object(ptr));
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                            let global = ctx.global();
                            if global.is_object() {
                                let global_obj = global.as_object_mut();
                                global_obj.set_cached(atom, val, ctx.shape_cache_mut());
                            }
                        }
                    } else {
                        if is_strict {
                            let mut found = false;

                            if self.eval_binding_frames == 0 && self.caller_vm.is_none() {
                                let global = ctx.global();
                                if global.is_object() {
                                    found = global.as_object().get_own(atom).is_some();
                                }

                                if !found && !self.frames[0].var_name_map.is_null() {
                                    let vnm = unsafe { &*self.frames[0].var_name_map };
                                    found = vnm.iter().any(|&(an, _, _)| an == atom.0);
                                }
                            } else if self.eval_binding_frames > 0 {
                                for fi in (0..=self.frame_index).rev() {
                                    if let Some(ref eb) = self.frames[fi].eval_bindings {
                                        if eb.contains_key(&atom.0) {
                                            found = true;
                                            break;
                                        }
                                    }
                                    if !self.frames[fi].var_name_map.is_null() {
                                        let vnm = unsafe { &*self.frames[fi].var_name_map };
                                        if vnm.iter().any(|&(an, _, _)| an == atom.0) {
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                                if !found {
                                    let global = ctx.global();
                                    if global.is_object() {
                                        found = global.as_object().get_own(atom).is_some();
                                    }
                                }
                            } else {
                                if self.get_var_in_caller_vm(atom.0).is_some() {
                                    found = true;
                                }
                                if !found {
                                    let global = ctx.global();
                                    if global.is_object() {
                                        found = global.as_object().get_own(atom).is_some();
                                    }
                                }
                            }
                            if !found {
                                let name_str = ctx.get_atom_str(atom).to_string();
                                let err_msg = format!("{} is not defined", name_str);
                                let mut err = crate::object::object::JSObject::new();
                                err.set(
                                    ctx.intern("name"),
                                    JSValue::new_string(ctx.intern("ReferenceError")),
                                );
                                err.set(
                                    ctx.intern("message"),
                                    JSValue::new_string(ctx.intern(&err_msg)),
                                );
                                if let Some(proto) = ctx.get_reference_error_prototype() {
                                    err.prototype = Some(proto);
                                }
                                let ptr = Box::into_raw(Box::new(err)) as usize;
                                ctx.runtime_mut().gc_heap_mut().track(ptr);
                                self.pending_throw = Some(JSValue::new_object(ptr));
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                        }
                        let global = ctx.global();
                        if global.is_object() {
                            let global_obj_ref = global.as_object();
                            if is_strict {
                                if let Some(desc) = global_obj_ref.get_own_descriptor(atom) {
                                    if !desc.writable && desc.get.is_none() && desc.set.is_none() {
                                        self.set_pending_type_error(
                                            ctx,
                                            "Cannot assign to read only property",
                                        );
                                        if let Some(exc) = self.pending_throw.take() {
                                            match self.dispatch_throw_value(ctx, exc) {
                                                ThrowDispatch::Caught => continue,
                                                ThrowDispatch::Uncaught(e) => return Err(e),
                                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                            }
                                        }
                                        continue;
                                    }
                                }
                            }
                            let global_obj = global.as_object_mut();
                            global_obj.set_cached(atom, val, ctx.shape_cache_mut());
                        }
                    }
                }
                Opcode::SetGlobalVar => {
                    let idx = self.read_u32() as usize;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let is_strict = self.frames[self.frame_index].is_strict_frame;
                    if self.caller_vm.is_some() {
                        let caller_var_set = self.set_var_in_caller_vm(ctx, atom.0, val);
                        if matches!(caller_var_set, CallerVarSet::Immutable) {
                            if is_strict {
                                self.set_pending_type_error(
                                    ctx,
                                    "Assignment to constant variable.",
                                );
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                        } else if matches!(caller_var_set, CallerVarSet::NotFound) {
                            if is_strict {
                                let name_str = ctx.get_atom_str(atom).to_string();
                                let err_msg = format!("{} is not defined", name_str);
                                let mut err = crate::object::object::JSObject::new();
                                err.set(
                                    ctx.intern("name"),
                                    JSValue::new_string(ctx.intern("ReferenceError")),
                                );
                                err.set(
                                    ctx.intern("message"),
                                    JSValue::new_string(ctx.intern(&err_msg)),
                                );
                                if let Some(proto) = ctx.get_reference_error_prototype() {
                                    err.prototype = Some(proto);
                                }
                                let ptr = Box::into_raw(Box::new(err)) as usize;
                                ctx.runtime_mut().gc_heap_mut().track(ptr);
                                self.pending_throw = Some(JSValue::new_object(ptr));
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                            let global = ctx.global();
                            if global.is_object() {
                                let global_obj = global.as_object_mut();
                                global_obj.set_cached_non_configurable(
                                    atom,
                                    val,
                                    ctx.shape_cache_mut(),
                                );
                            }
                        }
                    } else {
                        if is_strict {
                            let mut found = false;

                            if self.eval_binding_frames == 0 && self.caller_vm.is_none() {
                                let global = ctx.global();
                                if global.is_object() {
                                    found = global.as_object().get_own(atom).is_some();
                                }
                                if !found && !self.frames[0].var_name_map.is_null() {
                                    let vnm = unsafe { &*self.frames[0].var_name_map };
                                    found = vnm.iter().any(|&(an, _, _)| an == atom.0);
                                }
                            } else if self.eval_binding_frames > 0 {
                                for fi in (0..=self.frame_index).rev() {
                                    if let Some(ref eb) = self.frames[fi].eval_bindings {
                                        if eb.contains_key(&atom.0) {
                                            found = true;
                                            break;
                                        }
                                    }
                                    if !self.frames[fi].var_name_map.is_null() {
                                        let vnm = unsafe { &*self.frames[fi].var_name_map };
                                        if vnm.iter().any(|&(an, _, _)| an == atom.0) {
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                                if !found {
                                    let global = ctx.global();
                                    if global.is_object() {
                                        found = global.as_object().get_own(atom).is_some();
                                    }
                                }
                            } else {
                                if self.get_var_in_caller_vm(atom.0).is_some() {
                                    found = true;
                                }
                                if !found {
                                    let global = ctx.global();
                                    if global.is_object() {
                                        found = global.as_object().get_own(atom).is_some();
                                    }
                                }
                            }
                            if !found {
                                let name_str = ctx.get_atom_str(atom).to_string();
                                let err_msg = format!("{} is not defined", name_str);
                                let mut err = crate::object::object::JSObject::new();
                                err.set(
                                    ctx.intern("name"),
                                    JSValue::new_string(ctx.intern("ReferenceError")),
                                );
                                err.set(
                                    ctx.intern("message"),
                                    JSValue::new_string(ctx.intern(&err_msg)),
                                );
                                if let Some(proto) = ctx.get_reference_error_prototype() {
                                    err.prototype = Some(proto);
                                }
                                let ptr = Box::into_raw(Box::new(err)) as usize;
                                ctx.runtime_mut().gc_heap_mut().track(ptr);
                                self.pending_throw = Some(JSValue::new_object(ptr));
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                        }
                        let global = ctx.global();
                        if global.is_object() {
                            let global_obj = global.as_object_mut();
                            global_obj.set_cached_non_configurable(
                                atom,
                                val,
                                ctx.shape_cache_mut(),
                            );
                        }
                    }
                }
                Opcode::InitGlobalVar => {
                    let idx = self.read_u32() as usize;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    if self.caller_vm.is_some() {
                        self.init_var_in_caller_vm(ctx, atom.0, val);
                    } else {
                        let global = ctx.global();
                        if global.is_object() {
                            let global_obj = global.as_object_mut();
                            if !global_obj.has_own(atom) {
                                global_obj.define_cached(atom, val, ctx.shape_cache_mut());
                            }
                        }
                    }
                }
                Opcode::DefineGlobal => {
                    let idx = self.read_u32() as usize;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let global = ctx.global();
                    if global.is_object() {
                        let global_obj = global.as_object_mut();
                        global_obj.define_cached(atom, val, ctx.shape_cache_mut());
                    }
                }
                Opcode::DeleteGlobal => {
                    let dst = self.read_u16_pc();
                    let idx = self.read_u32() as usize;
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let global = ctx.global();
                    let result = if global.is_object() {
                        let global_obj = global.as_object_mut();
                        let deleted = global_obj.delete(atom);
                        if deleted {
                            ctx.runtime_mut().gc_heap_mut().deleted_props_count += 1;
                        }
                        deleted
                    } else {
                        true
                    };
                    let is_strict = self.frames[self.frame_index].is_strict_frame;
                    if is_strict && !result {
                        self.set_pending_type_error(
                            ctx,
                            "Delete of unqualified identifier in strict mode",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => {}
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    self.set_reg(dst, JSValue::bool(result));
                }
                Opcode::ThrowConstAssignment => {
                    let msg_idx = self.read_u32() as usize;
                    let msg_const = if msg_idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(msg_idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let name = if msg_const.is_string() {
                        ctx.get_atom_str(msg_const.get_atom()).to_string()
                    } else {
                        String::new()
                    };
                    let msg = format!("Assignment to constant variable '{}'", name);
                    self.set_pending_type_error(ctx, &msg);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    continue;
                }
                Opcode::ThrowReferenceError => {
                    let msg_idx = self.read_u32() as usize;
                    let msg_const = if msg_idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(msg_idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let err_msg = if msg_const.is_string() {
                        ctx.get_atom_str(msg_const.get_atom()).to_string()
                    } else {
                        String::new()
                    };
                    let mut err = crate::object::object::JSObject::new();
                    err.set(
                        ctx.intern("name"),
                        JSValue::new_string(ctx.intern("ReferenceError")),
                    );
                    err.set(
                        ctx.intern("message"),
                        JSValue::new_string(ctx.intern(&err_msg)),
                    );
                    if let Some(proto) = ctx.get_reference_error_prototype() {
                        err.prototype = Some(proto);
                    }
                    let ptr = Box::into_raw(Box::new(err)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track(ptr);
                    self.pending_throw = Some(JSValue::new_object(ptr));
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    continue;
                }
                Opcode::GetUpvalue => {
                    let dst = self.read_u16_pc();
                    let slot = self.read_u16_pc() as usize;
                    let result = if !self.cached_upvalue_slot_ptr.is_null()
                        && slot < self.cached_upvalues_len
                    {
                        let rc = unsafe { &*self.cached_upvalue_slot_ptr.add(slot) };
                        unsafe { *(*std::rc::Rc::as_ptr(rc)).as_ptr() }
                    } else {
                        JSValue::undefined()
                    };
                    self.set_reg(dst, result);
                }
                Opcode::SetUpvalue => {
                    let slot = self.read_u16_pc() as usize;
                    let src = self.read_u16_pc();
                    let val = self.get_reg(src);
                    if !self.cached_upvalue_slot_ptr.is_null() && slot < self.cached_upvalues_len {
                        let rc = unsafe { &*self.cached_upvalue_slot_ptr.add(slot) };
                        unsafe { (*std::rc::Rc::as_ptr(rc)).as_ptr().write(val) };
                    }
                }

                Opcode::NewObject => {
                    let dst = self.read_u16_pc();
                    let ptr = if let Some(proto_ptr) = ctx.get_object_prototype() {
                        if let Some(nursery_ptr) = ctx.runtime_mut().gc_heap_mut().alloc_object() {
                            unsafe {
                                (*nursery_ptr).prototype = Some(proto_ptr);
                                (*nursery_ptr).ensure_shape(ctx.shape_cache_mut());
                            }
                            nursery_ptr as usize
                        } else {
                            let mut obj = crate::object::object::JSObject::new();
                            obj.prototype = Some(proto_ptr);
                            obj.ensure_shape(ctx.shape_cache_mut());
                            let heap_ptr = Box::into_raw(Box::new(obj)) as usize;
                            ctx.runtime_mut().gc_heap_mut().track(heap_ptr);
                            heap_ptr
                        }
                    } else {
                        let mut obj = crate::object::object::JSObject::new();
                        obj.ensure_shape(ctx.shape_cache_mut());
                        let heap_ptr = Box::into_raw(Box::new(obj)) as usize;
                        ctx.runtime_mut().gc_heap_mut().track(heap_ptr);
                        heap_ptr
                    };
                    self.allocation_count += 1;
                    self.set_reg(dst, JSValue::new_object(ptr));
                    self.maybe_gc(ctx);
                }
                Opcode::NewArray => {
                    let dst = self.read_u16_pc();
                    let count = self.read_u16_pc();
                    let ptr = if let Some(proto_ptr) = ctx.get_array_prototype() {
                        if let Some(nursery_ptr) = ctx.runtime_mut().gc_heap_mut().alloc_array() {
                            unsafe {
                                (*nursery_ptr).header.set_prototype_raw(proto_ptr);
                                (*nursery_ptr).elements.reserve(count as usize);
                                (*nursery_ptr).header.ensure_shape(ctx.shape_cache_mut());
                            }
                            nursery_ptr as usize
                        } else {
                            let mut arr = crate::object::array_obj::JSArrayObject::with_capacity(
                                count as usize,
                            );
                            arr.header.set_prototype_raw(proto_ptr);
                            arr.header.ensure_shape(ctx.shape_cache_mut());
                            let heap_ptr = Box::into_raw(Box::new(arr)) as usize;
                            ctx.runtime_mut().gc_heap_mut().track_array(heap_ptr);
                            heap_ptr
                        }
                    } else {
                        let mut arr =
                            crate::object::array_obj::JSArrayObject::with_capacity(count as usize);
                        arr.header.ensure_shape(ctx.shape_cache_mut());
                        let heap_ptr = Box::into_raw(Box::new(arr)) as usize;
                        ctx.runtime_mut().gc_heap_mut().track_array(heap_ptr);
                        heap_ptr
                    };
                    self.allocation_count += 1;
                    self.set_reg(dst, JSValue::new_object(ptr));
                    self.maybe_gc(ctx);
                }
                Opcode::GetField => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    if obj_val.is_null_or_undefined() {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot read property of null or undefined",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if !key_val.is_string()
                        && !key_val.is_symbol()
                        && !key_val.is_int()
                        && !key_val.is_float()
                    {
                        key_val = self.to_property_key_value(ctx, key_val);
                        if let Some(exc) = self.take_pending_exception(ctx) {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        self.set_reg(key_reg, key_val);
                    }
                    let result = if obj_val.is_object_like() && key_val.is_int() {
                        let ptr = obj_val.get_ptr();
                        let js_obj = unsafe { JSValue::object_from_ptr(ptr) };
                        if js_obj.is_dense_array() {
                            let idx = key_val.get_int() as usize;
                            let index_atom = self.int_atom(idx, ctx);
                            if let Some(entry) = js_obj.get_own_accessor_entry(index_atom) {
                                if let Some(getter) = entry.get {
                                    if getter.is_function() {
                                        match self.call_function_with_this(ctx, getter, obj_val, &[]) {
                                            Ok(v) => v,
                                            Err(e) => {
                                                let exc = self
                                                    .last_caught_exception
                                                    .take()
                                                    .unwrap_or(JSValue::undefined());
                                                match self.dispatch_throw_value(ctx, exc) {
                                                    ThrowDispatch::Caught => continue,
                                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                                }
                                            }
                                        }
                                    } else {
                                        JSValue::undefined()
                                    }
                                } else {
                                    JSValue::undefined()
                                }
                            } else {
                                let arr = unsafe {
                                    &*(ptr as *const crate::object::array_obj::JSArrayObject)
                                };
                                if idx < arr.elements.len() {
                                    arr.elements[idx]
                                } else {
                                    JSValue::undefined()
                                }
                            }
                        } else if js_obj.obj_type() == crate::object::object::ObjectType::TypedArray {
                            let idx = key_val.get_int() as usize;
                            if let Some(val) = crate::builtins::typedarray::ta_get_element(ctx, js_obj, idx) {
                                self.set_reg(dst, val);
                                continue;
                            }
                            JSValue::undefined()
                        } else {
                            let idx = key_val.get_int();
                            if idx >= 0 {
                                if let Some(val) = js_obj.get_indexed(idx as usize) {
                                    self.set_reg(dst, val);
                                    continue;
                                }
                            }
                            let atom = self.int_atom(idx as usize, ctx);
                            if let Some(v) =
                                self.get_named_prop_result(ctx, dst, obj_val, atom, usize::MAX)
                            {
                                v
                            } else {
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                                continue;
                            }
                        }
                    } else if obj_val.is_string() && key_val.is_int() {
                        let s = ctx.get_atom_str(obj_val.get_atom());
                        let idx = key_val.get_int();
                        if idx < 0 {
                            JSValue::undefined()
                        } else if let Some(ch) = s.chars().nth(idx as usize) {
                            let chs = ch.to_string();
                            JSValue::new_string(ctx.intern(&chs))
                        } else {
                            JSValue::undefined()
                        }
                    } else if obj_val.is_string() && key_val.is_string() {
                        let str_atom = obj_val.get_atom();
                        let key = ctx.get_atom_str(key_val.get_atom());
                        if key == "length" {
                            JSValue::new_int(ctx.string_char_count(str_atom) as i64)
                        } else if let Ok(idx) = key.parse::<usize>() {
                            let s = ctx.get_atom_str(str_atom);
                            if let Some(ch) = s.chars().nth(idx) {
                                let chs = ch.to_string();
                                JSValue::new_string(ctx.intern(&chs))
                            } else {
                                JSValue::undefined()
                            }
                        } else if let Some(proto_ptr) = ctx.get_string_prototype() {
                            let proto_obj = unsafe { &*proto_ptr };
                            proto_obj
                                .get(key_val.get_atom())
                                .unwrap_or(JSValue::undefined())
                        } else {
                            JSValue::undefined()
                        }
                    } else if obj_val.is_object_like() && key_val.is_string() {
                        let js_obj = unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };
                        let atom = key_val.get_atom();

                        if let Some(getter) = js_obj.get_own_accessor_value(atom) {
                            if getter.is_function() {
                                match self.call_function_with_this(ctx, getter, obj_val, &[]) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        let exc = self.last_caught_exception.take().unwrap_or(JSValue::undefined());
                                        match self.dispatch_throw_value(ctx, exc) {
                                            ThrowDispatch::Caught => JSValue::undefined(),
                                            ThrowDispatch::Uncaught(_) => {
                                                return Err(e);
                                            }
                                            ThrowDispatch::AsyncComplete(o) => {
                                                return Ok(o);
                                            }
                                        }
                                    }
                                }
                            } else {
                                JSValue::undefined()
                            }
                        } else if js_obj.is_dense_array() {
                            if let Some(index) = ctx.atom_table().get_array_index(atom) {
                                let arr = unsafe {
                                    &*(obj_val.get_ptr()
                                        as *const crate::object::array_obj::JSArrayObject)
                                };
                                arr.elements.get(index).copied().unwrap_or(JSValue::undefined())
                            } else {
                                js_obj.get(atom).unwrap_or(JSValue::undefined())
                            }
                        } else {
                            if let Some(v) =
                                self.get_named_prop_result(ctx, dst, obj_val, atom, usize::MAX)
                            {
                                v
                            } else {
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                                continue;
                            }
                        }
                    } else if obj_val.is_object_like() && key_val.is_symbol() {
                        let sym_key =
                            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id());
                        if let Some(v) =
                            self.get_named_prop_result(ctx, dst, obj_val, sym_key, usize::MAX)
                        {
                            v
                        } else {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    } else if obj_val.is_object_like() && key_val.is_float() {
                        let s = VM::js_to_string(&key_val, ctx);
                        let atom = s.get_atom();
                        if let Some(v) =
                            self.get_named_prop_result(ctx, dst, obj_val, atom, usize::MAX)
                        {
                            v
                        } else {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    } else if obj_val.is_symbol() && key_val.is_string() {
                        if let Some(proto_ptr) = ctx.get_symbol_prototype() {
                            let proto_obj = unsafe { &*proto_ptr };
                            let atom = key_val.get_atom();
                            if let Some(getter) = proto_obj.get_own_accessor_value(atom) {
                                if getter.is_function() {
                                    match self.call_function_with_this(ctx, getter, obj_val, &[]) {
                                        Ok(v) => v,
                                        Err(_) => JSValue::undefined(),
                                    }
                                } else {
                                    JSValue::undefined()
                                }
                            } else {
                                proto_obj.get(atom).unwrap_or(JSValue::undefined())
                            }
                        } else {
                            JSValue::undefined()
                        }
                    } else if obj_val.is_symbol() && key_val.is_symbol() {
                        if let Some(proto_ptr) = ctx.get_symbol_prototype() {
                            let proto_obj = unsafe { &*proto_ptr };
                            let sym_key =
                                crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id());
                            proto_obj.get(sym_key).unwrap_or(JSValue::undefined())
                        } else {
                            JSValue::undefined()
                        }
                    } else if obj_val.is_bigint() && key_val.is_string() {
                        if let Some(proto_ptr) = ctx.get_bigint_prototype() {
                            let proto_obj = unsafe { &*proto_ptr };
                            let atom = key_val.get_atom();
                            if let Some(getter) = proto_obj.get_own_accessor_value(atom) {
                                if getter.is_function() {
                                    match self.call_function_with_this(ctx, getter, obj_val, &[]) {
                                        Ok(v) => v,
                                        Err(_) => JSValue::undefined(),
                                    }
                                } else {
                                    JSValue::undefined()
                                }
                            } else {
                                proto_obj.get(atom).unwrap_or(JSValue::undefined())
                            }
                        } else {
                            JSValue::undefined()
                        }
                    } else {
                        JSValue::undefined()
                    };
                    self.set_reg(dst, result);
                }
                Opcode::ToPropertyKey => {
                    let reg = self.read_u16_pc();
                    let value = self.get_reg(reg);
                    if !value.is_string()
                        && !value.is_symbol()
                        && !value.is_int()
                        && !value.is_float()
                    {
                        let key = self.to_property_key_value(ctx, value);
                        if let Some(exc) = self.take_pending_exception(ctx) {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        self.set_reg(reg, key);
                    }
                }
                Opcode::SetField => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    let value = self.get_reg(val_reg);
                    if obj_val.is_null_or_undefined() {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot set property of null or undefined",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if !key_val.is_string()
                        && !key_val.is_symbol()
                        && !key_val.is_int()
                        && !key_val.is_float()
                    {
                        key_val = self.to_property_key_value(ctx, key_val);
                        if let Some(exc) = self.take_pending_exception(ctx) {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    if obj_val.is_object_like() && key_val.is_string() {
                        if let Some(index) = ctx.atom_table().get_array_index(key_val.get_atom()) {
                            key_val = JSValue::new_int(index as i64);
                        }
                    }
                    if obj_val.is_object_like() && key_val.is_int() {
                        let ptr = obj_val.get_ptr();
                        let js_obj = unsafe { JSValue::object_from_ptr_mut(ptr) };
                        if js_obj.is_dense_array() {
                            let idx = key_val.get_int() as usize;
                            let arr = unsafe {
                                &mut *(ptr as *mut crate::object::array_obj::JSArrayObject)
                            };
                            if idx < arr.elements.len() {
                                if js_obj.frozen() {
                                    if self.frames[self.frame_index].is_strict_frame {
                                        self.set_pending_type_error(
                                            ctx,
                                            "Cannot assign to read only property",
                                        );
                                    }
                                } else {
                                    arr.elements[idx] = value;
                                }
                            } else {
                                // Check prototype chain for setters when EXTENDING the array
                                let mut proto = js_obj.prototype;
                                let mut setter_found = None;
                                while let Some(p) = proto {
                                    let pobj = unsafe { &*p };
                                    let key_atom = self.int_atom(idx, ctx);
                                    if let Some(entry) = pobj.get_own_accessor_entry(key_atom) {
                                        setter_found = entry.set;
                                        break;
                                    }
                                    proto = pobj.prototype;
                                }
                                if let Some(setter) = setter_found {
                                    if self
                                        .call_function_with_this(ctx, setter, obj_val, &[value])
                                        .is_err()
                                    {
                                        let caught = self.last_caught_exception.take();
                                        let host = ctx.pending_exception.take();
                                        let exc = caught
                                            .or(host)
                                            .or_else(|| self.pending_throw.take());
                                        if let Some(exc) = exc {
                                            match self.dispatch_throw_value(ctx, exc) {
                                                ThrowDispatch::Caught => continue,
                                                ThrowDispatch::Uncaught(e) => return Err(e),
                                                ThrowDispatch::AsyncComplete(o) => {
                                                    return Ok(o)
                                                }
                                            }
                                        }
                                    }
                                } else if !js_obj.extensible() {
                                    if self.frames[self.frame_index].is_strict_frame {
                                        self.set_pending_type_error(
                                            ctx,
                                            "Cannot define property, object is not extensible",
                                        );
                                    }
                                } else if idx > 100_000 {
                                    let len_atom = ctx.common_atoms.length;
                                    let old_len = arr
                                        .header
                                        .get(len_atom)
                                        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
                                        .unwrap_or(0);
                                    if idx + 1 > old_len {
                                        arr.header.set_length_ic(
                                            len_atom,
                                            JSValue::new_int((idx + 1) as i64),
                                            ctx.shape_cache_mut(),
                                        );
                                    }
                                    let key_atom = self.int_atom(idx, ctx);
                                    arr.header
                                        .set_cached(key_atom, value, ctx.shape_cache_mut());
                                    arr.header.clear_dense_array_flag();
                                } else {
                                    while arr.elements.len() < idx {
                                        arr.elements.push(JSValue::undefined());
                                    }
                                    arr.elements.push(value);
                                    let len_atom = ctx.common_atoms.length;

                                    let new_elements_len = arr.elements.len();
                                    let old_len = arr
                                        .header
                                        .get(len_atom)
                                        .map(|v| if v.is_int() { v.get_int() as usize } else { 0 })
                                        .unwrap_or(0);
                                    let new_len = new_elements_len.max(old_len);
                                    if new_len != old_len {
                                        arr.header.set_length_ic(
                                            len_atom,
                                            JSValue::new_int(new_len as i64),
                                            ctx.shape_cache_mut(),
                                        );
                                    }
                                }
                            }
                        } else if js_obj.obj_type() == crate::object::object::ObjectType::TypedArray {
                            let idx = key_val.get_int() as usize;
                            crate::builtins::typedarray::ta_set_element(ctx, js_obj, idx, value);
                        } else {
                            let idx = key_val.get_int();
                            let atom = self.int_atom(idx as usize, ctx);
                            let setter = {
                                let mut found =
                                    js_obj.get_own_accessor_entry(atom).and_then(|e| e.set);
                                if found.is_none() {
                                    let mut proto = js_obj.prototype;
                                    while let Some(p) = proto {
                                        let pobj = unsafe { &*p };
                                        if let Some(e) = pobj.get_own_accessor_entry(atom) {
                                            found = e.set;
                                            break;
                                        }
                                        proto = pobj.prototype;
                                    }
                                }
                                found
                            };
                            if let Some(s) = setter {
                                if s.is_function() {
                                    match self.call_function_with_this(ctx, s, obj_val, &[value]) {
                                        Ok(_) => {}
                                        Err(_) => {}
                                    }
                                }
                            } else if idx >= 0 && js_obj.maybe_set_indexed(idx as usize, value) {
                            } else {
                                let pre_offset = js_obj.find_offset(atom);
                                let own_accessor =
                                    js_obj.get_own_accessor_entry(atom).is_some();
                                if !own_accessor
                                    && pre_offset.is_some()
                                    && js_obj.is_prop_writable_at(pre_offset)
                                {
                                    js_obj.set_cached(atom, value, ctx.shape_cache_mut());
                                } else {
                                    self.set_named_prop(ctx, obj_val, value, atom, usize::MAX);
                                }
                            }
                        }
                    } else if obj_val.is_object_like()
                        && (key_val.is_string()
                            || key_val.is_float()
                            || key_val.is_symbol())
                    {
                        let js_obj = unsafe { JSValue::object_from_ptr_mut(obj_val.get_ptr()) };
                        let atom = if key_val.is_string() {
                            key_val.get_atom()
                        } else if key_val.is_symbol() {
                            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id())
                        } else {
                            let s = VM::js_to_string(&key_val, ctx);
                            s.get_atom()
                        };
                        let pre_offset = js_obj.find_offset(atom);
                        let own_accessor = js_obj.get_own_accessor_entry(atom).is_some();
                        if !own_accessor
                            && pre_offset.is_some()
                            && js_obj.is_prop_writable_at(pre_offset)
                        {
                            js_obj.set_cached(atom, value, ctx.shape_cache_mut());
                        } else {
                            self.set_named_prop(ctx, obj_val, value, atom, usize::MAX);
                        }
                    } else if !obj_val.is_object_like() && self.frames[self.frame_index].is_strict_frame {
                        self.set_pending_type_error(ctx, "Cannot assign to a primitive value");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => {}
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                _ => {}
                            }
                        }
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    self.set_reg(obj_reg, obj_val);
                }
                Opcode::GetProp => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    if obj_val.is_null_or_undefined() {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot read property of null or undefined",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if !key_val.is_string()
                        && !key_val.is_symbol()
                        && !key_val.is_int()
                        && !key_val.is_float()
                    {
                        key_val = self.to_property_key_value(ctx, key_val);
                        if let Some(exc) = self.take_pending_exception(ctx) {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }

                    if !obj_val.is_object_like()
                        && !obj_val.is_function()
                        && (obj_val.is_int()
                            || obj_val.is_float()
                            || obj_val.is_string()
                            || obj_val.is_bool()
                            || obj_val.is_symbol()
                            || obj_val.is_bigint())
                        && key_val.is_string()
                    {
                        let atom = key_val.get_atom();
                        let start_proto = if obj_val.is_string() {
                            ctx.get_string_prototype()
                        } else if obj_val.is_int() || obj_val.is_float() {
                            ctx.get_number_prototype()
                        } else if obj_val.is_symbol() {
                            ctx.get_symbol_prototype()
                        } else if obj_val.is_bigint() {
                            ctx.get_bigint_prototype()
                        } else if obj_val.is_bool() {
                            ctx.get_bool_prototype()
                        } else {
                            ctx.get_object_prototype()
                        };
                        let mut current = start_proto;
                        let mut found = false;
                        while let Some(ptr) = current {
                            let pobj = unsafe { &*ptr };
                            if let Some(getter) = pobj.get_own_accessor_value(atom) {
                                if getter.is_function() {
                                    let r = self
                                        .call_function_with_this(ctx, getter, obj_val, &[])
                                        .unwrap_or(JSValue::undefined());
                                    self.set_reg(dst, r);
                                } else {
                                    self.set_reg(dst, JSValue::undefined());
                                }
                                found = true;
                                break;
                            }
                            if let Some(val) = pobj.get_own(atom) {
                                self.set_reg(dst, val);
                                found = true;
                                break;
                            }
                            current = pobj.prototype;
                        }
                        if !found {
                            self.set_reg(dst, JSValue::undefined());
                        }
                        continue;
                    }
                    let result = if key_val.is_string() {
                        if let Some(result) = self.get_named_prop_result(
                            ctx,
                            dst,
                            obj_val,
                            key_val.get_atom(),
                            instr_pc,
                        ) {
                            result
                        } else {
                            continue;
                        }
                    } else if key_val.is_symbol() {
                        if obj_val.is_object_like() {
                            let js_obj = unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };
                            let sym_key =
                                crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id());
                            js_obj.get(sym_key).unwrap_or(JSValue::undefined())
                        } else {
                            JSValue::undefined()
                        }
                    } else if key_val.is_int() && obj_val.is_object_like() {
                        if obj_val.is_object() {
                            let idx = key_val.get_int();
                            let js_obj_check =
                                unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };

                            if idx >= 0 {
                                if let Some(val) = js_obj_check.get_indexed(idx as usize) {
                                    self.set_reg(dst, val);
                                    continue;
                                }
                            }
                        }
                        let atom = self.int_atom(key_val.get_int() as usize, ctx);
                        if let Some(result) =
                            self.get_named_prop_result(ctx, dst, obj_val, atom, instr_pc)
                        {
                            result
                        } else {
                            continue;
                        }
                    } else if key_val.is_float() && obj_val.is_object_like() {
                        let s = VM::js_to_string(&key_val, ctx);
                        let atom = s.get_atom();
                        if let Some(result) =
                            self.get_named_prop_result(ctx, dst, obj_val, atom, instr_pc)
                        {
                            result
                        } else {
                            continue;
                        }
                    } else {
                        JSValue::undefined()
                    };
                    self.set_reg(dst, result);
                }
                Opcode::GetNamedProp => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    if !self.get_named_prop_fast(ctx, dst, obj_reg, atom, instr_pc) {
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                }
                Opcode::SetProp => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    let value = self.get_reg(val_reg);
                    if obj_val.is_null_or_undefined() {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot set property of null or undefined",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if !key_val.is_string()
                        && !key_val.is_symbol()
                        && !key_val.is_int()
                        && !key_val.is_float()
                    {
                        key_val = self.to_property_key_value(ctx, key_val);
                        if let Some(exc) = self.take_pending_exception(ctx) {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }

                    if key_val.is_string() {
                        self.set_named_prop(ctx, obj_val, value, key_val.get_atom(), usize::MAX);
                    } else if key_val.is_symbol() && obj_val.is_object_like() {
                        let sym_key =
                            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id());
                        self.set_named_prop(ctx, obj_val, value, sym_key, usize::MAX);
                    } else if key_val.is_int() && obj_val.is_object_like() {
                        let atom = self.int_atom(key_val.get_int() as usize, ctx);
                        self.set_named_prop(ctx, obj_val, value, atom, usize::MAX);
                    } else if key_val.is_float() && obj_val.is_object_like() {
                        let s = VM::js_to_string(&key_val, ctx);
                        let atom = s.get_atom();
                        self.set_named_prop(ctx, obj_val, value, atom, usize::MAX);
                    }
                    self.set_reg(obj_reg, obj_val);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(_) => continue,
                        }
                    }
                }
                Opcode::SetNamedProp => {
                    let obj_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let fast =
                        self.set_named_prop_fast(ctx, obj_reg, val_reg, atom, instr_pc);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    if fast {
                        continue;
                    }
                }

                Opcode::LtJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() < b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() < b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int < b_int
                    } else {
                        match self.abstract_less_than(&a, &b, ctx, true) {
                            Some(v) => v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if !cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::LtJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() < b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() < b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int < b_int
                    } else {
                        match self.abstract_less_than(&a, &b, ctx, true) {
                            Some(v) => v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::LteJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() <= b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() <= b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int <= b_int
                    } else {
                        match self.abstract_less_than(&b, &a, ctx, false) {
                            Some(v) => !v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if !cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::LteJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() <= b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() <= b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int <= b_int
                    } else {
                        match self.abstract_less_than(&b, &a, ctx, false) {
                            Some(v) => !v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::GtJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() > b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() > b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int > b_int
                    } else {
                        match self.abstract_less_than(&b, &a, ctx, false) {
                            Some(v) => v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if !cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::GtJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() > b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() > b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int > b_int
                    } else {
                        match self.abstract_less_than(&b, &a, ctx, false) {
                            Some(v) => v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::GteJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() >= b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() >= b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int >= b_int
                    } else {
                        match self.abstract_less_than(&a, &b, ctx, true) {
                            Some(v) => !v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if !cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::GteJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    let cmp = if JSValue::both_int(&a, &b) {
                        a.get_int() >= b.get_int()
                    } else if a.is_float() && b.is_float() {
                        a.get_float() >= b.get_float()
                    } else if a.is_bigint() && b.is_bigint() {
                        let a_int = Self::get_bigint_int(&a).unwrap_or(0);
                        let b_int = Self::get_bigint_int(&b).unwrap_or(0);
                        a_int >= b_int
                    } else {
                        match self.abstract_less_than(&a, &b, ctx, true) {
                            Some(v) => !v,
                            None => {
                                if self.pending_throw.is_some() {
                                    continue;
                                }
                                false
                            }
                        }
                    };
                    if cmp {
                        self.pc = (self.pc as i64 + offset as i64) as usize;
                    }
                }
                Opcode::EqJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => {
                            if !eq {
                                self.pc = (self.pc as i32 + offset) as usize;
                            }
                        }
                        None => continue,
                    }
                }
                Opcode::EqJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => {
                            if eq {
                                self.pc = (self.pc as i32 + offset) as usize;
                            }
                        }
                        None => continue,
                    }
                }
                Opcode::NeqJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => {
                            if eq {
                                self.pc = (self.pc as i32 + offset) as usize;
                            }
                        }
                        None => continue,
                    }
                }
                Opcode::NeqJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    match self.loose_equal_value(ctx, a, b) {
                        Some(eq) => {
                            if !eq {
                                self.pc = (self.pc as i32 + offset) as usize;
                            }
                        }
                        None => continue,
                    }
                }
                Opcode::StrictEqJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    if !a.strict_eq(&b) {
                        self.pc = (self.pc as i32 + offset) as usize;
                    }
                }
                Opcode::StrictEqJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    if a.strict_eq(&b) {
                        self.pc = (self.pc as i32 + offset) as usize;
                    }
                }
                Opcode::StrictNeqJumpIfNot => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    if a.strict_eq(&b) {
                        self.pc = (self.pc as i32 + offset) as usize;
                    }
                }
                Opcode::StrictNeqJumpIf => {
                    let a_reg = self.read_u16_pc();
                    let a = self.get_reg(a_reg);
                    let b_reg = self.read_u16_pc();
                    let b = self.get_reg(b_reg);
                    let offset = self.read_i32_pc();
                    if !a.strict_eq(&b) {
                        self.pc = (self.pc as i32 + offset) as usize;
                    }
                }

                Opcode::DeleteProp => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    if obj_val.is_null() || obj_val.is_undefined() {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot delete property from null or undefined",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => {}
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    let atom = if key_val.is_string() {
                        Some(key_val.get_atom())
                    } else if key_val.is_symbol() {
                        let sym_id = key_val.get_symbol_id();
                        Some(crate::runtime::atom::Atom(0x40000000 | sym_id))
                    } else if key_val.is_int() && (obj_val.is_object() || obj_val.is_function()) {
                        Some(self.int_atom(key_val.get_int() as usize, ctx))
                    } else if key_val.is_float() && (obj_val.is_object() || obj_val.is_function()) {
                        let s = VM::js_to_string(&key_val, ctx);
                        Some(s.get_atom())
                    } else {
                        None
                    };
                    let result = if let Some(atom) = atom {
                        if obj_val.is_object() || obj_val.is_function() {
                            let js_obj = obj_val.as_object_mut();
                            let mut deleted = false;
                            if js_obj.is_dense_array() && key_val.is_int() {
                                let idx = key_val.get_int();
                                if idx >= 0 {
                                    let ptr = obj_val.get_ptr();
                                    let arr = unsafe {
                                        &mut *(ptr as *mut crate::object::array_obj::JSArrayObject)
                                    };
                                    if (idx as usize) < arr.elements.len() {
                                        arr.elements[idx as usize] = JSValue::undefined();
                                        deleted = true;
                                    }
                                }
                            }
                            if !deleted {
                                deleted = js_obj.delete(atom);
                            }
                            if deleted {
                                ctx.runtime_mut().gc_heap_mut().deleted_props_count += 1;
                            }
                            deleted
                        } else {
                            true
                        }
                    } else {
                        true
                    };
                    let is_strict = self.frames[self.frame_index].is_strict_frame;
                    if is_strict && !result {
                        self.set_pending_type_error(ctx, "Cannot delete property");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => {}
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    self.set_reg(dst, JSValue::bool(result));
                }
                Opcode::HasProperty => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    if !(obj_val.is_object() || obj_val.is_function()) {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot use 'in' operator to search for a property in a non-object",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    let js_obj = obj_val.as_object();
                    let result = if key_val.is_string() {
                        let atom = key_val.get_atom();
                        js_obj.has_property(atom) || ctx.array_index_has_own(&obj_val, atom)
                    } else if key_val.is_int() {
                        let atom = self.int_atom(key_val.get_int() as usize, ctx);
                        js_obj.has_property(atom) || ctx.array_index_has_own(&obj_val, atom)
                    } else if key_val.is_symbol() {
                        let sym_key =
                            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id());
                        js_obj.has_property(sym_key)
                    } else {
                        let s = VM::js_to_string(&key_val, ctx);
                        let atom = s.get_atom();
                        js_obj.has_property(atom) || ctx.array_index_has_own(&obj_val, atom)
                    };
                    self.set_reg(dst, JSValue::bool(result));
                }
                Opcode::InstanceOf => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let ctor_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let ctor_val = self.get_reg(ctor_reg);
                    if !ctor_val.is_object() && !ctor_val.is_function() {
                        self.set_pending_type_error(
                            ctx,
                            "Right-hand side of 'instanceof' is not an object",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => {
                                    self.set_reg(dst, JSValue::undefined());
                                }
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }

                    if ctor_val.is_function() {
                        let js_func = ctor_val.as_function();
                        let ctor_proto = js_func.cached_prototype_ptr as usize;
                        if ctor_proto != 0
                            && !js_func.has_symbol_on_base()
                            && (obj_val.is_object() || obj_val.is_function())
                        {
                            let obj_proto = unsafe {
                                (*(obj_val.get_ptr()
                                    as *const crate::object::object::JSObject))
                                    .prototype
                            };
                            let obj_proto_key = obj_proto.map(|p| p as usize).unwrap_or(0);
                            let ctor_ptr = ctor_val.get_ptr();
                            if self.instanceof_cache_ctor == ctor_ptr
                                && self.instanceof_cache_proto == obj_proto_key
                                && self.instanceof_cache_ctor_proto == ctor_proto
                            {
                                self.set_reg(dst, JSValue::bool(self.instanceof_cache_result));
                                continue;
                            }
                            let mut proto_opt = obj_proto;
                            let mut found = false;
                            let mut limit = 0;
                            while let Some(proto_ptr) = proto_opt {
                                if proto_ptr as usize == ctor_proto {
                                    found = true;
                                    break;
                                }
                                proto_opt = unsafe { (*proto_ptr).prototype };
                                limit += 1;
                                if limit > 1000 {
                                    break;
                                }
                            }
                            self.instanceof_cache_ctor = ctor_ptr;
                            self.instanceof_cache_proto = obj_proto_key;
                            self.instanceof_cache_ctor_proto = ctor_proto;
                            self.instanceof_cache_result = found;
                            self.set_reg(dst, JSValue::bool(found));
                            continue;
                        }
                    }

                    match self.instanceof_operator(ctx, ctor_val, obj_val) {
                        Some(result) => self.set_reg(dst, JSValue::bool(result)),
                        None => {
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => {
                                        self.set_reg(dst, JSValue::undefined());
                                    }
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            } else {
                                self.set_reg(dst, JSValue::undefined());
                            }
                        }
                    }
                }
                Opcode::NewRegExp => {
                    let cache_key = self.cached_code_ptr as usize + self.pc;
                    let dst = self.read_u16_pc();
                    let pattern_idx = self.read_u32() as usize;
                    let flags_idx = self.read_u32() as usize;
                    let constants_len = self.frames[self.frame_index].constants_len;
                    let pattern_val = if pattern_idx < constants_len {
                        unsafe { *self.cached_const_ptr.add(pattern_idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let flags_val = if flags_idx < constants_len {
                        unsafe { *self.cached_const_ptr.add(flags_idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let pattern = if pattern_val.is_string() {
                        ctx.get_atom_str(pattern_val.get_atom()).to_string()
                    } else {
                        String::new()
                    };
                    let flags = if flags_val.is_string() {
                        ctx.get_atom_str(flags_val.get_atom()).to_string()
                    } else {
                        String::new()
                    };

                    if let Some(cached_re) = self.regex_lit_cache.get(&cache_key) {
                        let cloned_re = cached_re.clone();
                        let re_val = crate::builtins::regexp::create_regexp_object_precompiled(
                            ctx, &pattern, &flags, cloned_re,
                        );
                        self.set_reg(dst, re_val);
                    } else {
                        let pattern_atom = ctx.intern(&pattern);
                        let flags_atom = ctx.intern(&flags);
                        let re_val = crate::builtins::regexp::regexp_constructor(
                            ctx,
                            &[
                                JSValue::new_string(pattern_atom),
                                JSValue::new_string(flags_atom),
                            ],
                        );

                        if re_val.is_object() {
                            if let Some(compiled) = re_val.as_object().get_compiled_regex() {
                                self.regex_lit_cache.insert(cache_key, compiled.clone());
                            }
                        }
                        self.set_reg(dst, re_val);
                    }
                }
                Opcode::GetPrivate => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let owner_val = self.get_reg(owner_reg);
                    let mut result = JSValue::undefined();
                    let mut failed = false;
                    if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        let owner_obj = owner_val.as_object();
                        match owner_obj.get_private_kind(owner_ptr, atom) {
                            Some(crate::object::object::PrivateKind::Method) => {
                                let js_obj = obj_val.as_object();
                                if !js_obj.has_private_brand(owner_ptr) {
                                    failed = true;
                                } else {
                                    match owner_obj.get_private_method(owner_ptr, atom) {
                                        Some(v) => result = v,
                                        None => failed = true,
                                    }
                                }
                            }
                            Some(crate::object::object::PrivateKind::Accessor) => {
                                let js_obj = obj_val.as_object();
                                if !js_obj.has_private_brand(owner_ptr) {
                                    failed = true;
                                } else {
                                    let getter = owner_obj
                                        .get_own_private_accessor_entry(owner_ptr, atom)
                                        .and_then(|e| e.get);
                                    match getter {
                                        Some(getter) => {
                                            match self.call_function_with_this(
                                                ctx, getter, obj_val, &[],
                                            ) {
                                                Ok(v) => result = v,
                                                Err(e) => {
                                                    let exc = self
                                                        .last_caught_exception
                                                        .take()
                                                        .or_else(|| ctx.pending_exception.take())
                                                        .or_else(|| self.pending_throw.take());
                                                    if let Some(exc) = exc {
                                                        match self.dispatch_throw_value(ctx, exc) {
                                                            ThrowDispatch::Caught => continue,
                                                            ThrowDispatch::Uncaught(e) => {
                                                                return Err(e)
                                                            }
                                                            ThrowDispatch::AsyncComplete(o) => {
                                                                return Ok(o)
                                                            }
                                                        }
                                                    }
                                                    return Err(e);
                                                }
                                            }
                                        }
                                        None => failed = true,
                                    }
                                }
                            }
                            _ => {
                                let js_obj = obj_val.as_object();
                                match js_obj.get_private_field(owner_ptr, atom) {
                                    Some(v) => result = v,
                                    None => failed = true,
                                }
                            }
                        }
                    } else {
                        failed = true;
                    }
                    if failed {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot read private member from an object whose class did not declare it",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    self.set_reg(dst, result);
                }
                Opcode::SetPrivate => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let val = self.get_reg(val_reg);
                    let owner_val = self.get_reg(owner_reg);
                    let mut failed = false;
                    if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        let owner_obj = owner_val.as_object();
                        match owner_obj.get_private_kind(owner_ptr, atom) {
                            Some(crate::object::object::PrivateKind::Accessor) => {
                                let js_obj = obj_val.as_object();
                                if !js_obj.has_private_brand(owner_ptr) {
                                    failed = true;
                                } else {
                                    let setter = owner_obj
                                        .get_own_private_accessor_entry(owner_ptr, atom)
                                        .and_then(|e| e.set);
                                    match setter {
                                        Some(setter) => {
                                            match self.call_function_with_this(
                                                ctx, setter, obj_val, &[val],
                                            ) {
                                                Ok(_) => {}
                                                Err(e) => {
                                                    let exc = self
                                                        .last_caught_exception
                                                        .take()
                                                        .or_else(|| ctx.pending_exception.take())
                                                        .or_else(|| self.pending_throw.take());
                                                    if let Some(exc) = exc {
                                                        match self.dispatch_throw_value(ctx, exc) {
                                                            ThrowDispatch::Caught => continue,
                                                            ThrowDispatch::Uncaught(e) => {
                                                                return Err(e)
                                                            }
                                                            ThrowDispatch::AsyncComplete(o) => {
                                                                return Ok(o)
                                                            }
                                                        }
                                                    }
                                                    return Err(e);
                                                }
                                            }
                                        }
                                        None => failed = true,
                                    }
                                }
                            }
                            _ => {
                                let js_obj = obj_val.as_object_mut();
                                if js_obj.has_private_field(owner_ptr, atom) {
                                    js_obj.set_private_field(owner_ptr, atom, val);
                                } else {
                                    failed = true;
                                }
                            }
                        }
                    } else {
                        failed = true;
                    }
                    if failed {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot write private member to an object whose class did not declare it",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::DefinePrivateField => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let val = self.get_reg(val_reg);
                    let owner_val = self.get_reg(owner_reg);
                    let mut failed = false;
                    if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        let js_obj = obj_val.as_object_mut();
                        if !js_obj.extensible() || js_obj.has_private_field(owner_ptr, atom) {
                            failed = true;
                        } else {
                            js_obj.set_private_field(owner_ptr, atom, val);
                            js_obj.add_private_brand(owner_ptr);
                            js_obj.define_private_kind(
                                owner_ptr,
                                atom,
                                crate::object::object::PrivateKind::Field,
                            );
                        }
                    } else {
                        failed = true;
                    }
                    if failed {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot initialize private field twice",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::DefinePrivateMethod => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let val = self.get_reg(val_reg);
                    let owner_val = self.get_reg(owner_reg);
                    if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        if val.is_function() {
                            val.as_function_mut().home_object = obj_val;
                        }
                        let js_obj = obj_val.as_object_mut();
                        js_obj.define_private_method(owner_ptr, atom, val);
                        js_obj.define_private_kind(
                            owner_ptr,
                            atom,
                            crate::object::object::PrivateKind::Method,
                        );
                    }
                }
                Opcode::AddPrivateBrand => {
                    let obj_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let owner_val = self.get_reg(owner_reg);
                    if obj_val.is_object_like() && owner_val.is_object_like() {
                        let owner_ptr = owner_val.get_ptr();
                        obj_val.as_object_mut().add_private_brand(owner_ptr);
                    }
                }
                Opcode::AddInstanceField => {
                    let class_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let init_reg = self.read_u16_pc();
                    let class_val = self.get_reg(class_reg);
                    let mut key_val = self.get_reg(key_reg);
                    let init_val = self.get_reg(init_reg);
                    if class_val.is_function() {
                        let Some(key_val) = self.canonical_property_key_value(ctx, key_val) else {
                            if let Some(exc) = self.take_pending_exception(ctx) {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        };
                        let class_func = class_val.as_function_mut();
                        class_func.instance_fields.push(
                            crate::object::function::InstanceFieldDefinition {
                                key: key_val,
                                private_owner: JSValue::undefined(),
                                private_name: crate::runtime::atom::Atom(0),
                                initializer: init_val,
                            },
                        );
                    }
                }
                Opcode::AddInstancePrivateField => {
                    let class_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let name = self.read_u32();
                    let init_reg = self.read_u16_pc();
                    let class_val = self.get_reg(class_reg);
                    let owner_val = self.get_reg(owner_reg);
                    let init_val = self.get_reg(init_reg);
                    if class_val.is_function() && owner_val.is_object_like() {
                        let class_func = class_val.as_function_mut();
                        class_func.instance_fields.push(
                            crate::object::function::InstanceFieldDefinition {
                                key: JSValue::undefined(),
                                private_owner: owner_val,
                                private_name: crate::runtime::atom::Atom(name),
                                initializer: init_val,
                            },
                        );
                        owner_val.as_object_mut().define_private_kind(
                            owner_val.get_ptr(),
                            crate::runtime::atom::Atom(name),
                            crate::object::object::PrivateKind::Field,
                        );
                    }
                }
                Opcode::DefineField => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    let value = self.get_reg(val_reg);
                    if obj_val.is_object_like() {
                        let Some(key_val) = self.canonical_property_key_value(ctx, key_val) else {
                            if let Some(exc) = self.take_pending_exception(ctx) {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        };
                        let mut failed = false;
                        if !obj_val.as_object().extensible() {
                            failed = true;
                        } else if key_val.is_string() {
                            let atom = key_val.get_atom();
                            if obj_val
                                .as_object()
                                .get_own_descriptor(atom)
                                .map_or(false, |d| !d.configurable)
                            {
                                failed = true;
                            } else {
                                let js_obj =
                                    unsafe { JSValue::object_from_ptr_mut(obj_val.get_ptr()) };
                                let desc = crate::object::object::PropertyDescriptor {
                                    value: Some(value),
                                    writable: true,
                                    enumerable: true,
                                    configurable: true,
                                    get: None,
                                    set: None,
                                };
                                js_obj.define_property(atom, desc);
                            }
                        } else if key_val.is_symbol() {
                            let atom = key_val.get_atom();
                            let js_obj = unsafe { JSValue::object_from_ptr_mut(obj_val.get_ptr()) };
                            let desc = crate::object::object::PropertyDescriptor {
                                value: Some(value),
                                writable: true,
                                enumerable: true,
                                configurable: true,
                                get: None,
                                set: None,
                            };
                            js_obj.define_property(atom, desc);
                        } else {
                            failed = true;
                        }
                        if failed {
                            self.set_pending_type_error(
                                ctx,
                                "Cannot define property on non-extensible object",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                        }
                    } else {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot define property on non-object",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::GetEvalPrivateOwner => {
                    let dst = self.read_u16_pc();
                    let name = self.read_u32();
                    if let Some(owner) = self.eval_private_owner_in_caller_vm(name) {
                        self.set_reg(dst, owner);
                    } else {
                        self.set_pending_reference_error(ctx, "Private name is not defined");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::CheckObjectOrNull => {
                    let reg = self.read_u16_pc();
                    let value = self.get_reg(reg);
                    if !value.is_object_like() && !value.is_null() {
                        self.set_pending_type_error(
                            ctx,
                            "Class extends value does not have a valid prototype property",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::HasPrivate => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let owner_val = self.get_reg(owner_reg);
                    let result = if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        let owner_obj = owner_val.as_object();
                        match owner_obj.get_private_kind(owner_ptr, atom) {
                            Some(crate::object::object::PrivateKind::Method)
                            | Some(crate::object::object::PrivateKind::Accessor) => {
                                obj_val.as_object().has_private_brand(owner_ptr)
                            }
                            _ => obj_val.as_object().has_private_field(owner_ptr, atom),
                        }
                    } else {
                        false
                    };
                    self.set_reg(dst, JSValue::bool(result));
                }
                Opcode::GatherRest => {
                    let dst = self.read_u16_pc();
                    let base = self.frames[self.frame_index].registers_base;
                    let arg_count = self.frames[self.frame_index].arg_count as usize;
                    let rest_start = dst as usize;

                    let rest_count = if arg_count + 1 > rest_start {
                        arg_count + 1 - rest_start
                    } else {
                        0
                    };
                    let mut arr =
                        crate::object::array_obj::JSArrayObject::with_capacity(rest_count);
                    if let Some(proto_ptr) = ctx.get_array_prototype() {
                        arr.header.set_prototype_raw(proto_ptr);
                    }
                    for i in 0..rest_count {
                        let val = self.registers[base + rest_start + i];
                        arr.push(val);
                    }
                    let len_atom = ctx.common_atoms.length;
                    arr.header
                        .set(len_atom, JSValue::new_int(rest_count as i64));
                    let ptr = Box::into_raw(Box::new(arr)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track_array(ptr);
                    self.set_reg(dst, JSValue::new_object(ptr));
                }
                Opcode::ObjectSpread => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let dst_val = self.get_reg(dst);
                    let src_val = self.get_reg(src);
                    if dst_val.is_object() || dst_val.is_function() {
                        let keys = self.own_enumerable_keys_in_order(ctx, src_val);
                        for atom in keys {
                            let value = match self.get_copy_value(ctx, src_val, atom) {
                                Ok(value) => value,
                                Err(e) => {
                                    let caught = self.last_caught_exception.take();
                                    let host = ctx.pending_exception.take();
                                    let exc = caught
                                        .or(host)
                                        .or_else(|| self.pending_throw.take());
                                    if let Some(exc) = exc {
                                        match self.dispatch_throw_value(ctx, exc) {
                                            ThrowDispatch::Caught => continue,
                                            ThrowDispatch::Uncaught(e) => return Err(e),
                                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                        }
                                    }
                                    return Err(e);
                                }
                            };
                            let dst_obj = dst_val.as_object_mut();
                            dst_obj.set_cached(atom, value, ctx.shape_cache_mut());
                        }
                    }
                }
                Opcode::ObjectRest => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let keys_reg = self.read_u16_pc();
                    let dst_val = self.get_reg(dst);
                    let src_val = self.get_reg(src);
                    let keys_val = self.get_reg(keys_reg);
                    if dst_val.is_object() || dst_val.is_function() {
                        let mut excluded = std::collections::HashSet::new();
                        if keys_val.is_object() {
                            let keys_obj = keys_val.as_object();
                            let (len, dense) = if keys_obj.is_dense_array() {
                                let arr = unsafe {
                                    &*(keys_val.get_ptr()
                                        as *const crate::object::array_obj::JSArrayObject)
                                };
                                (arr.elements.len(), true)
                            } else {
                                (
                                    keys_obj
                                        .get(ctx.common_atoms.length)
                                        .map(|v| v.get_int() as usize)
                                        .unwrap_or(0),
                                    false,
                                )
                            };
                            for i in 0..len {
                                let k = if dense {
                                    let arr = unsafe {
                                        &*(keys_val.get_ptr()
                                            as *const crate::object::array_obj::JSArrayObject)
                                    };
                                    arr.get(i).unwrap_or(JSValue::undefined())
                                } else {
                                    let key = self.int_atom(i, ctx);
                                    keys_obj.get(key).unwrap_or(JSValue::undefined())
                                };
                                let atom = if k.is_string() {
                                    Some(k.get_atom())
                                } else if k.is_symbol() {
                                    Some(crate::runtime::atom::Atom(
                                        0x40000000 | k.get_symbol_id(),
                                    ))
                                } else if k.is_int() {
                                    let s = k.get_int().to_string();
                                    Some(ctx.atom_table_mut().intern(&s))
                                } else if k.is_float() {
                                    let f = k.get_float();
                                    let s = if f.fract() == 0.0
                                        && f.is_finite()
                                        && f.abs() < 9.007_199_254_740_992e15
                                    {
                                        (f as i64).to_string()
                                    } else {
                                        format!("{}", f)
                                    };
                                    Some(ctx.atom_table_mut().intern(&s))
                                } else {
                                    None
                                };
                                if let Some(atom) = atom {
                                    excluded.insert(atom);
                                }
                            }
                        }
                        let entries = self.own_enumerable_keys_in_order(ctx, src_val);
                        for atom in entries {
                            if excluded.contains(&atom) {
                                continue;
                            }
                            let value = match self.get_copy_value(ctx, src_val, atom) {
                                Ok(value) => value,
                                Err(e) => {
                                    let caught = self.last_caught_exception.take();
                                    let host = ctx.pending_exception.take();
                                    let exc = caught
                                        .or(host)
                                        .or_else(|| self.pending_throw.take());
                                    if let Some(exc) = exc {
                                        match self.dispatch_throw_value(ctx, exc) {
                                            ThrowDispatch::Caught => continue,
                                            ThrowDispatch::Uncaught(e) => return Err(e),
                                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                        }
                                    }
                                    return Err(e);
                                }
                            };
                            let dst_obj = dst_val.as_object_mut();
                            dst_obj.set_cached(atom, value, ctx.shape_cache_mut());
                        }
                    }
                }
                Opcode::GetPropertyNames => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut arr = crate::object::array_obj::JSArrayObject::new();
                    if let Some(proto_ptr) = ctx.get_array_prototype() {
                        arr.header.set_prototype_raw(proto_ptr);
                    }
                    // Collect all enumerable properties from the object and its prototype chain
                    let mut seen = Vec::new();
                    let obj_proto_ptr = ctx.get_object_prototype().map(|p| p as usize);
                    let arr_proto_ptr = ctx.get_array_prototype().map(|p| p as usize);
                    let str_proto_ptr = ctx.get_string_prototype().map(|p| p as usize);
                    let mut current_ptr = if obj_val.is_object() || obj_val.is_function() {
                        Some(obj_val.get_ptr() as usize)
                    } else {
                        None
                    };
                    while let Some(ptr) = current_ptr {
                        // Stop at built-in prototypes (their methods should be non-enumerable)
                        if let Some(obj_proto) = obj_proto_ptr {
                            if ptr == obj_proto {
                                break;
                            }
                        }
                        if let Some(arr_proto) = arr_proto_ptr {
                            if ptr == arr_proto {
                                break;
                            }
                        }
                        if let Some(str_proto) = str_proto_ptr {
                            if ptr == str_proto {
                                break;
                            }
                        }
                        let js_obj = unsafe { &*(ptr as *const crate::object::object::JSObject) };
                        let mut int_keys: Vec<i64> = Vec::new();
                        let mut str_keys: Vec<crate::runtime::atom::Atom> = Vec::new();
                        // Handle dense array indices
                        if js_obj.is_dense_array() {
                            let arr_obj =
                                unsafe { &*(ptr as *mut crate::object::array_obj::JSArrayObject) };
                            for i in 0..arr_obj.elements.len() {
                                let idx_atom = ctx.intern(i.to_string().as_str());
                                if !seen.contains(&idx_atom.0) {
                                    seen.push(idx_atom.0);
                                    int_keys.push(i as i64);
                                }
                            }
                        }
                        if js_obj.is_mapped_arguments() {
                            if let Some(entries) = js_obj.argument_entries() {
                                for entry in entries {
                                    if !entry.present || seen.contains(&entry.atom.0) {
                                        continue;
                                    }
                                    seen.push(entry.atom.0);
                                    if !entry.enumerable() {
                                        continue;
                                    }
                                    let key_str = ctx.get_atom_str(entry.atom);
                                    if let Ok(n) = key_str.parse::<i64>() {
                                        if n >= 0 && key_str == n.to_string() {
                                            int_keys.push(n);
                                            continue;
                                        }
                                    }
                                    str_keys.push(entry.atom);
                                }
                            }
                        }
                        for (atom, _value) in js_obj.own_properties() {
                            // Track ALL own properties (including non-enumerable) for shadowing
                            if seen.contains(&atom.0) {
                                continue;
                            }
                            seen.push(atom.0);
                            let key_str = ctx.get_atom_str(atom);
                            if let Ok(n) = key_str.parse::<i64>() {
                                if n >= 0 && key_str == n.to_string() {
                                    int_keys.push(n);
                                    continue;
                                }
                            }
                            str_keys.push(atom);
                        }
                        // Also track non-enumerable own properties for shadowing
                        for atom in js_obj.non_enumerable_property_atoms() {
                            if !seen.contains(&atom.0) {
                                seen.push(atom.0);
                            }
                        }
                        int_keys.sort();
                        for k in &int_keys {
                            let atom = ctx.intern(&k.to_string());
                            arr.push(JSValue::new_string(atom));
                        }
                        for atom in str_keys {
                            arr.push(JSValue::new_string(atom));
                        }
                        current_ptr = js_obj.prototype.map(|p| p as usize);
                    }
                    let len_atom = ctx.common_atoms.length;
                    arr.header
                        .set(len_atom, JSValue::new_int(arr.elements.len() as i64));
                    let ptr = Box::into_raw(Box::new(arr)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track_array(ptr);
                    self.allocation_count += 1;
                    self.set_reg(dst, JSValue::new_object(ptr));
                    self.maybe_gc(ctx);
                }
                Opcode::ArrayExtend => {
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let dst_val = self.get_reg(dst);
                    let src_val = self.get_reg(src);
                    if dst_val.is_object() && src_val.is_object() {
                        let dst_ptr = dst_val.get_ptr();
                        let src_ptr = src_val.get_ptr();
                        let is_src_array = src_val.as_object().is_dense_array();
                        let is_dst_array = dst_val.as_object().is_dense_array();
                        if is_src_array && is_dst_array {
                            let dst_arr = unsafe {
                                &mut *(dst_ptr as *mut crate::object::array_obj::JSArrayObject)
                            };
                            let src_arr = unsafe {
                                &*(src_ptr as *const crate::object::array_obj::JSArrayObject)
                            };
                            for val in src_arr.elements.iter() {
                                dst_arr.push(*val);
                            }
                            let len_atom = ctx.common_atoms.length;
                            dst_arr
                                .header
                                .set(len_atom, JSValue::new_int(dst_arr.elements.len() as i64));
                        } else if is_dst_array {
                            let dst_arr = unsafe {
                                &mut *(dst_ptr as *mut crate::object::array_obj::JSArrayObject)
                            };
                            let src_obj =
                                unsafe { &*(src_ptr as *const crate::object::object::JSObject) };
                            let len_atom = ctx.common_atoms.length;
                            let arr_len = src_obj
                                .get(len_atom)
                                .map(|v| v.get_int() as usize)
                                .unwrap_or(0);
                            if let Some(elems) = src_obj.get_array_elements() {
                                for val in elems.iter() {
                                    dst_arr.push(*val);
                                }
                            } else {
                                for i in 0..arr_len {
                                    let key = ctx.intern(&i.to_string());
                                    if let Some(val) = src_obj.get(key) {
                                        dst_arr.push(val);
                                    }
                                }
                            }
                            dst_arr
                                .header
                                .set(len_atom, JSValue::new_int(dst_arr.elements.len() as i64));
                        }
                    }
                }
                Opcode::ArrayPush => {
                    let arr_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let arr_val = self.get_reg(arr_reg);
                    let val = self.get_reg(val_reg);
                    if arr_val.is_object() {
                        let arr_ptr = arr_val.get_ptr();
                        let arr = unsafe {
                            &mut *(arr_ptr as *mut crate::object::array_obj::JSArrayObject)
                        };
                        arr.push(val);
                        let len_atom = ctx.common_atoms.length;
                        let new_len = arr.elements.len() as i64;
                        if !arr.header.has_own(len_atom) {
                            arr.header.define_property(
                                len_atom,
                                crate::object::object::PropertyDescriptor {
                                    value: Some(JSValue::new_int(new_len)),
                                    writable: true,
                                    enumerable: false,
                                    configurable: false,
                                    get: None,
                                    set: None,
                                },
                            );
                        } else {
                            arr.header.set(len_atom, JSValue::new_int(new_len));
                        }
                    }
                }
                Opcode::ArrayPushAll => {
                    let arr_reg = self.read_u16_pc();
                    let src_reg = self.read_u16_pc();
                    let arr_val = self.get_reg(arr_reg);
                    let src_val = self.get_reg(src_reg);
                    if arr_val.is_object() && src_val.is_object() {
                        let src_values = {
                            let src_ptr = src_val.get_ptr();
                            let src = unsafe {
                                &*(src_ptr as *const crate::object::array_obj::JSArrayObject)
                            };
                            src.elements.clone()
                        };
                        let arr_ptr = arr_val.get_ptr();
                        let arr = unsafe {
                            &mut *(arr_ptr as *mut crate::object::array_obj::JSArrayObject)
                        };
                        for val in src_values {
                            arr.push(val);
                        }
                        let len_atom = ctx.common_atoms.length;
                        let new_len = arr.elements.len() as i64;
                        if !arr.header.has_own(len_atom) {
                            arr.header.define_property(
                                len_atom,
                                crate::object::object::PropertyDescriptor {
                                    value: Some(JSValue::new_int(new_len)),
                                    writable: true,
                                    enumerable: false,
                                    configurable: false,
                                    get: None,
                                    set: None,
                                },
                            );
                        } else {
                            arr.header.set(len_atom, JSValue::new_int(new_len));
                        }
                    }
                }
                Opcode::LoadCaughtException => {
                    let dst = self.read_u16_pc();
                    let value = self
                        .caught_exception_value
                        .take()
                        .unwrap_or_else(JSValue::undefined);
                    self.set_reg(dst, value);
                }
                Opcode::SetFunctionName => {
                    let fn_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let fn_val = self.get_reg(fn_reg);
                    let key_val = self.get_reg(key_reg);
                    self.set_function_display_name(ctx, fn_val, key_val, None);
                }
                Opcode::SetProto => {
                    let obj_reg = self.read_u16_pc();
                    let proto_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let proto_val = self.get_reg(proto_reg);
                    if obj_val.is_object_like() {
                        let obj_ref = obj_val.as_object_mut();
                        if proto_val.is_object_like() {
                            obj_ref.prototype =
                                Some(proto_val.get_ptr() as *mut crate::object::object::JSObject);
                        } else if proto_val.is_null() {
                            obj_ref.prototype = None;
                        }
                    }
                }
                Opcode::GetSuper => {
                    let dst = self.read_u16_pc();
                    let value = self.current_super_constructor();
                    self.set_reg(dst, value);
                }
                Opcode::GetNewTarget => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, self.frames[self.frame_index].new_target);
                }
                Opcode::MarkDirectEval => {
                    self.eval_direct_flags = self.read_u8();
                    self.eval_direct_call = true;
                }
                Opcode::CheckThisInitialized => {
                    if !self.frames[self.frame_index].this_initialized {
                        self.set_pending_reference_error(ctx, "this is not initialized");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::GetSuperBase => {
                    if !self.frames[self.frame_index].this_initialized {
                        self.set_pending_reference_error(ctx, "this is not initialized");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, self.frames[self.frame_index].super_base);
                }
                Opcode::SetHomeObject => {
                    let func_reg = self.read_u16_pc();
                    let home_reg = self.read_u16_pc();
                    let func_val = self.get_reg(func_reg);
                    let home_val = self.get_reg(home_reg);
                    if func_val.is_function() && home_val.is_object_like() {
                        func_val.as_function_mut().home_object = home_val;
                    }
                }
                Opcode::GetSuperProp => {
                    if !self.frames[self.frame_index].this_initialized {
                        self.set_pending_reference_error(ctx, "this is not initialized");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    let dst = self.read_u16_pc();
                    let base_reg = self.read_u16_pc();
                    let this_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let base = self.get_reg(base_reg);
                    let this_val = self.get_reg(this_reg);
                    let key_val = self.get_reg(key_reg);
                    let atom = self.super_key_atom(ctx, key_val);
                    if let Some(exc) = self.take_pending_exception(ctx) {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    if let Some(result) = self.get_named_prop_result_with_receiver(
                        ctx,
                        dst,
                        base,
                        atom,
                        usize::MAX,
                        Some(this_val),
                    ) {
                        self.set_reg(dst, result);
                    } else if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::SetSuperProp => {
                    if !self.frames[self.frame_index].this_initialized {
                        self.set_pending_reference_error(ctx, "this is not initialized");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                    let base_reg = self.read_u16_pc();
                    let this_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let base = self.get_reg(base_reg);
                    let receiver = self.get_reg(this_reg);
                    let key_val = self.get_reg(key_reg);
                    let value = self.get_reg(val_reg);
                    self.set_super_prop(ctx, base, receiver, key_val, value);
                    if let Some(exc) = self.take_pending_exception(ctx) {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }

                Opcode::DefineAccessor | Opcode::DefineClassAccessor => {
                    let class_accessor = op == Opcode::DefineClassAccessor;
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let getter_reg = self.read_u16_pc();
                    let setter_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    if obj_val.is_object_like() {
                        let prop_key = if key_val.is_string() {
                            key_val
                        } else if key_val.is_symbol() {
                            JSValue::new_string(crate::runtime::atom::Atom(
                                0x40000000 | key_val.get_symbol_id(),
                            ))
                        } else {
                            let prim = self.ordinary_to_primitive(&key_val, "string", ctx);
                            if self.pending_throw.is_some() {
                                if let Some(exc) = self.pending_throw.take() {
                                    let disp = self.dispatch_throw_value(ctx, exc);
                                    match disp {
                                        ThrowDispatch::Caught => {}
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => match o {
                                            ExecutionOutcome::Complete(v) => {
                                                self.set_reg(obj_reg, v);
                                            }
                                            _ => {}
                                        },
                                    }
                                }
                                continue;
                            }
                            if prim.is_string() {
                                prim
                            } else {
                                VM::js_to_string(&prim, ctx)
                            }
                        };
                        if prop_key.is_string() {
                            let atom = prop_key.get_atom();
                            let getter = if getter_reg != u16::MAX {
                                Some(self.get_reg(getter_reg))
                            } else {
                                None
                            };
                            let setter = if setter_reg != u16::MAX {
                                Some(self.get_reg(setter_reg))
                            } else {
                                None
                            };
                            if let Some(g) = getter {
                                if g.is_function() {
                                    g.as_function_mut().home_object = obj_val;
                                }
                                self.set_function_display_name(ctx, g, key_val, Some("get"));
                            }
                            if let Some(s) = setter {
                                if s.is_function() {
                                    s.as_function_mut().home_object = obj_val;
                                }
                                self.set_function_display_name(ctx, s, key_val, Some("set"));
                            }
                            let obj = obj_val.as_object_mut();
                            if class_accessor
                                && obj_val.is_function()
                                && atom == ctx.common_atoms.prototype
                                && obj
                                    .get_own_descriptor(atom)
                                    .map_or(false, |d| !d.configurable)
                            {
                                self.set_pending_type_error(
                                    ctx,
                                    "Classes may not have a static property named 'prototype'",
                                );
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                                continue;
                            }
                            let existing = obj.get_own_accessor_entry(atom);
                            let final_getter = getter.or_else(|| existing.and_then(|e| e.get));
                            let final_setter = setter.or_else(|| existing.and_then(|e| e.set));
                            if class_accessor {
                                obj.define_accessor_flags(
                                    atom, final_getter, final_setter, false, true,
                                );
                            } else {
                                obj.define_accessor(atom, final_getter, final_setter);
                            }
                        } else if self.pending_throw.is_none() {
                            self.set_pending_type_error(ctx, "Invalid property key");
                        }
                    }
                }

                Opcode::DefinePrivateAccessor => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let getter_reg = self.read_u16_pc();
                    let setter_reg = self.read_u16_pc();
                    let owner_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let key_val = self.get_reg(key_reg);
                    let owner_val = self.get_reg(owner_reg);
                    if obj_val.is_object_like()
                        && key_val.is_string()
                        && owner_val.is_object_like()
                    {
                        let atom = key_val.get_atom();
                        let owner_ptr = owner_val.get_ptr();
                        let getter = if getter_reg != u16::MAX {
                            Some(self.get_reg(getter_reg))
                        } else {
                            None
                        };
                        let setter = if setter_reg != u16::MAX {
                            Some(self.get_reg(setter_reg))
                        } else {
                            None
                        };
                        if let Some(g) = getter {
                            if g.is_function() {
                                g.as_function_mut().home_object = obj_val;
                            }
                        }
                        if let Some(s) = setter {
                            if s.is_function() {
                                s.as_function_mut().home_object = obj_val;
                            }
                        }
                        let obj = obj_val.as_object_mut();
                        obj.define_private_accessor(owner_ptr, atom, getter, setter);
                        obj.define_private_kind(
                            owner_ptr,
                            atom,
                            crate::object::object::PrivateKind::Accessor,
                        );
                    }
                }

                Opcode::DefineClassPrototype => {
                    let ctor_reg = self.read_u16_pc();
                    let proto_reg = self.read_u16_pc();
                    let ctor_val = self.get_reg(ctor_reg);
                    let proto_val = self.get_reg(proto_reg);
                    if ctor_val.is_object_like() && proto_val.is_object_like() {
                        let ctor_obj =
                            unsafe { JSValue::object_from_ptr_mut(ctor_val.get_ptr()) };
                        let proto_obj =
                            unsafe { JSValue::object_from_ptr_mut(proto_val.get_ptr()) };
                        let proto_desc = crate::object::object::PropertyDescriptor {
                            value: Some(proto_val),
                            writable: false,
                            enumerable: false,
                            configurable: false,
                            get: None,
                            set: None,
                        };
                        ctor_obj.define_property(ctx.common_atoms.prototype, proto_desc);
                        let ctor_desc = crate::object::object::PropertyDescriptor {
                            value: Some(ctor_val),
                            writable: true,
                            enumerable: false,
                            configurable: true,
                            get: None,
                            set: None,
                        };
                        proto_obj.define_property(ctx.common_atoms.constructor, ctor_desc);
                        let func = unsafe {
                            &mut *(ctor_val.get_ptr()
                                as *mut crate::object::function::JSFunction)
                        };
                        func.cached_prototype_ptr =
                            proto_val.get_ptr() as *mut crate::object::object::JSObject;
                    }
                }

                Opcode::SetMethodProp => {
                    let obj_reg = self.read_u16_pc();
                    let key_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let obj_val = self.get_reg(obj_reg);
                    let mut key_val = self.get_reg(key_reg);
                    let value = self.get_reg(val_reg);
                    if obj_val.is_object_like() {
                        if !key_val.is_string()
                            && !key_val.is_symbol()
                            && !key_val.is_int()
                            && !key_val.is_float()
                        {
                            key_val = self.to_property_key_value(ctx, key_val);
                        }
                        let js_obj = unsafe { JSValue::object_from_ptr_mut(obj_val.get_ptr()) };
                        let atom = if key_val.is_string() {
                            key_val.get_atom()
                        } else if key_val.is_symbol() {
                            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id())
                        } else if key_val.is_float() {
                            let s = VM::js_to_string(&key_val, ctx);
                            s.get_atom()
                        } else {
                            self.int_atom(key_val.get_int() as usize, ctx)
                        };
                        let static_prototype = obj_val.is_function()
                            && atom == ctx.common_atoms.prototype
                            && js_obj
                                .get_own_descriptor(atom)
                                .map_or(false, |d| !d.configurable);
                        if static_prototype {
                            self.set_pending_type_error(
                                ctx,
                                "Classes may not have a static property named 'prototype'",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                        let desc = crate::object::object::PropertyDescriptor {
                            value: Some(value),
                            writable: true,
                            enumerable: false,
                            configurable: true,
                            get: None,
                            set: None,
                        };
                        if value.is_function() {
                            value.as_function_mut().home_object = obj_val;
                            self.set_function_display_name(ctx, value, key_val, None);
                        }
                        js_obj.define_property(atom, desc);
                    }
                    self.set_reg(obj_reg, obj_val);
                }

                Opcode::WithEnter => {
                    let src = self.read_u16_pc();
                    let mut value = self.get_reg(src);
                    if value.is_null() || value.is_undefined() {
                        self.set_pending_type_error(ctx, "Cannot convert null or undefined to object");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    if !value.is_object_like() {
                        let boxed = crate::builtins::object::object_to_object(ctx, &value);
                        if boxed.is_object_like() {
                            value = boxed;
                        } else {
                            self.set_pending_type_error(
                                ctx,
                                "Cannot convert value to object",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    }
                    let parent = self.frames[self.frame_index]
                        .with_env
                        .clone()
                        .unwrap_or_else(JSValue::undefined);
                    let object_atom = ctx.intern("\u{1}with_object");
                    let parent_atom = ctx.intern("\u{1}with_parent");
                    let mut env = crate::object::object::JSObject::new();
                    env.set(object_atom, value);
                    env.set(parent_atom, parent);
                    let ptr = Box::into_raw(Box::new(env)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track(ptr);
                    self.frames[self.frame_index].with_env = Some(JSValue::new_object(ptr));
                }
                Opcode::WithExit => {
                    let current = self.frames[self.frame_index].with_env.clone();
                    if let Some(env_val) = current {
                        let parent_atom = ctx.intern("\u{1}with_parent");
                        let env_obj =
                            unsafe { JSValue::object_from_ptr(env_val.get_ptr()) };
                        let parent = env_obj.get_own(parent_atom);
                        self.frames[self.frame_index].with_env =
                            parent.filter(|p| p.is_object_like());
                    }
                }
                Opcode::WithHas => {
                    let dst = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let found = self.with_find_object(ctx, atom, instr_pc, dst).is_some();
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                        continue;
                    }
                    self.set_reg(dst, JSValue::bool(found));
                }
                Opcode::WithGet => {
                    let dst = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    if let Some(object) = self.with_find_object(ctx, atom, instr_pc, dst) {
                        if self.pending_throw.is_none() {
                            self.get_named_prop_result(ctx, dst, object, atom, instr_pc);
                        }
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::WithSet => {
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let src = self.read_u16_pc();
                    let value = self.get_reg(src);
                    if let Some(object) = self.with_find_object(ctx, atom, instr_pc, u16::MAX) {
                        if self.pending_throw.is_none() {
                            self.set_named_prop(ctx, object, value, atom, usize::MAX);
                        }
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::WithGetVar => {
                    let dst = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let kind = self.read_u8();
                    let index = self.read_u16_pc();
                    let flags = self.read_u8();
                    self.with_get_var(ctx, dst, atom, kind, index, flags, instr_pc);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::WithSetVar => {
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let src = self.read_u16_pc();
                    let kind = self.read_u8();
                    let index = self.read_u16_pc();
                    let _flags = self.read_u8();
                    let value = self.get_reg(src);
                    self.with_set_var(ctx, atom, value, kind, index, instr_pc);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::WithDelete => {
                    let dst = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let mut result = true;
                    if let Some(object) = self.with_find_object(ctx, atom, instr_pc, u16::MAX) {
                        if self.pending_throw.is_none() {
                            result = unsafe { JSValue::object_from_ptr_mut(object.get_ptr()) }
                                .delete(atom);
                        }
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    self.set_reg(dst, JSValue::bool(result));
                }

                Opcode::WithResolve => {
                    let dst = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let found = self.with_find_object(ctx, atom, instr_pc, u16::MAX);
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                    self.set_reg(dst, found.unwrap_or_else(JSValue::undefined));
                }
                Opcode::WithGetResolved => {
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let object = self.get_reg(obj_reg);
                    if object.is_object_like() {
                        let exists = unsafe { JSValue::object_from_ptr(object.get_ptr()) }
                            .has_property(atom);
                        if exists {
                            self.get_named_prop_result(ctx, dst, object, atom, instr_pc);
                        } else if self.frames[self.frame_index].is_strict_frame {
                            self.set_pending_reference_error(
                                ctx,
                                "Cannot access variable before initialization",
                            );
                        } else {
                            self.set_reg(dst, JSValue::undefined());
                        }
                    } else {
                        self.set_reg(dst, JSValue::undefined());
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::WithSetResolved => {
                    let obj_reg = self.read_u16_pc();
                    let val_reg = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let object = self.get_reg(obj_reg);
                    let value = self.get_reg(val_reg);
                    if object.is_object_like() {
                        let exists = unsafe { JSValue::object_from_ptr(object.get_ptr()) }
                            .has_property(atom);
                        if !exists && self.frames[self.frame_index].is_strict_frame {
                            self.set_pending_reference_error(
                                ctx,
                                "Cannot assign to undeclared variable",
                            );
                        } else if !exists {
                            if self.prototype_rejects_typed_array_set(ctx, object, atom) {
                                if self.frames[self.frame_index].is_strict_frame {
                                    self.set_pending_type_error(
                                        ctx,
                                        "Cannot create property on typed array prototype",
                                    );
                                }
                            } else {
                                unsafe { JSValue::object_from_ptr_mut(object.get_ptr()) }
                                    .set(atom, value);
                            }
                        } else {
                            self.set_named_prop(ctx, object, value, atom, usize::MAX);
                        }
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }

                Opcode::CallCurrent1 => {
                    unsafe {
                        (*self.frames.as_mut_ptr().add(self.frame_index)).current_pc = instr_pc;
                    }
                    let dst = self.read_u16_pc();
                    let arg_reg = self.read_u16_pc();
                    let caller = &self.frames[self.frame_index];
                    let function_ptr = caller.function_ptr;
                    if function_ptr.is_none() {
                        self.set_pending_type_error(ctx, "undefined is not a function");
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                        continue;
                    }
                    let one_arg = [arg_reg];
                    self.push_frame_from_arg_regs_raw(
                        ctx,
                        caller.locals_count,
                        caller.bytecode_ptr,
                        caller.bytecode_len,
                        caller.constants_ptr,
                        caller.constants_len,
                        self.pc,
                        function_ptr,
                        caller.ic_table_ptr,
                        JSValue::undefined(),
                        dst,
                        1,
                        false,
                        caller.is_async,
                        self.cached_registers_base,
                        &one_arg,
                        caller.uses_arguments,
                    );
                    continue;
                }

                Opcode::Call
                | Opcode::Call0
                | Opcode::Call1
                | Opcode::Call2
                | Opcode::Call3
                | Opcode::CallMethod
                | Opcode::CallNew
                | Opcode::CallCurrent => {
                    unsafe {
                        (*self.frames.as_mut_ptr().add(self.frame_index)).current_pc = instr_pc;
                    }
                    let is_call_current = op == Opcode::CallCurrent;
                    let is_call0 = op == Opcode::Call0;
                    let is_call1 = op == Opcode::Call1;
                    let is_call2 = op == Opcode::Call2;
                    let is_call3 = op == Opcode::Call3;
                    let is_call_method = op == Opcode::CallMethod;
                    let is_call_new = op == Opcode::CallNew;
                    let dst = self.read_u16_pc();
                    let obj_reg = if is_call_method {
                        self.read_u16_pc()
                    } else {
                        0
                    };
                    let func_reg = if is_call_current {
                        0
                    } else {
                        self.read_u16_pc()
                    };
                    let mut one_arg = [0u16; 1];
                    let mut two_args = [0u16; 2];
                    let mut three_args = [0u16; 3];
                    let argc = if is_call0 {
                        0
                    } else if is_call1 {
                        one_arg[0] = self.read_u16_pc();
                        1
                    } else if is_call2 {
                        two_args[0] = self.read_u16_pc();
                        two_args[1] = self.read_u16_pc();
                        2
                    } else if is_call3 {
                        three_args[0] = self.read_u16_pc();
                        three_args[1] = self.read_u16_pc();
                        three_args[2] = self.read_u16_pc();
                        3
                    } else {
                        self.read_u16_pc()
                    };

                    let mut arg_regs_buf = [0u16; 16];
                    let mut arg_regs_vec = Vec::new();
                    let arg_regs: &[u16] = if is_call1 {
                        &one_arg
                    } else if is_call2 {
                        &two_args
                    } else if is_call3 {
                        &three_args
                    } else if argc as usize <= 16 {
                        for i in 0..argc {
                            arg_regs_buf[i as usize] = self.read_u16_pc();
                        }
                        &arg_regs_buf[..argc as usize]
                    } else {
                        arg_regs_vec.reserve(argc as usize);
                        for _ in 0..argc {
                            arg_regs_vec.push(self.read_u16_pc());
                        }
                        &arg_regs_vec
                    };

                    if is_call_current {
                        let caller = &self.frames[self.frame_index];
                        let function_ptr = caller.function_ptr;
                        if function_ptr.is_none() {
                            self.set_pending_type_error(ctx, "undefined is not a function");
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }

                        self.push_frame_from_arg_regs_raw(
                            ctx,
                            caller.locals_count,
                            caller.bytecode_ptr,
                            caller.bytecode_len,
                            caller.constants_ptr,
                            caller.constants_len,
                            self.pc,
                            function_ptr,
                            caller.ic_table_ptr,
                            JSValue::undefined(),
                            dst,
                            argc,
                            false,
                            caller.is_async,
                            self.cached_registers_base,
                            arg_regs,
                            caller.uses_arguments,
                        );
                        continue;
                    }

                    let func_val = self.get_reg(func_reg);
                    let is_super_call = !is_call_method
                        && !is_call_new
                        && {
                            let s = self.current_super_constructor();
                            (s.is_function() || s.is_object_like())
                                && s.raw_bits() == func_val.raw_bits()
                        };
                    let caller_new_target = self.frames[self.frame_index].new_target;
                    if is_super_call {
                        let is_constructor = Self::value_is_constructor(ctx, func_val);
                        if !is_constructor {
                            self.set_pending_type_error(
                                ctx,
                                "Super constructor is not a constructor",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    }
                    let this_val = if is_call_method {
                        self.get_reg(obj_reg)
                    } else if is_call_new {
                        let props = ctx.runtime_mut().gc_heap_mut().take_prop_vec();
                        let mut obj = crate::object::object::JSObject::new_typed_from_pool(
                            crate::object::object::ObjectType::Ordinary,
                            props,
                        );
                        obj.ensure_shape(ctx.shape_cache_mut());
                        if let Some(proto_ptr) = ctx.get_object_prototype() {
                            obj.prototype = Some(proto_ptr);
                        }
                        let func_val = if is_call_current {
                            if let Some(ptr) = self.frames[self.frame_index].function_ptr {
                                JSValue::new_function(ptr)
                            } else {
                                JSValue::undefined()
                            }
                        } else {
                            self.get_reg(func_reg)
                        };
                        if func_val.is_function() {
                            let js_func = func_val.as_function();

                            if !js_func.cached_prototype_ptr.is_null() {
                                obj.prototype = Some(js_func.cached_prototype_ptr);
                            } else {
                                let proto_key = ctx.common_atoms.prototype;
                                if let Some(proto_val) = js_func.base.get(proto_key) {
                                    if proto_val.is_object_like() {
                                        let cptr = proto_val.get_ptr()
                                            as *mut crate::object::object::JSObject;
                                        func_val.as_function_mut().cached_prototype_ptr = cptr;
                                        obj.prototype = Some(cptr);
                                    }
                                } else if !js_func.is_builtin() {
                                    let mut pobj = crate::object::object::JSObject::new();
                                    pobj.set(ctx.common_atoms.constructor, func_val);
                                    if let Some(opp) = ctx.get_object_prototype() {
                                        pobj.prototype = Some(opp);
                                    }
                                    let pp = Box::into_raw(Box::new(pobj)) as usize;
                                    ctx.runtime_mut().gc_heap_mut().track(pp);
                                    let func_mut = func_val.as_function_mut();
                                    func_mut.base.set(proto_key, JSValue::new_object(pp));
                                    let cptr = pp as *mut crate::object::object::JSObject;
                                    func_mut.cached_prototype_ptr = cptr;
                                    obj.prototype = Some(cptr);
                                }
                            }
                        }
                        let ptr = if ctx.runtime().gc_heap().nursery_enabled_and_can_fit_object() {
                            ctx.runtime_mut()
                                .gc_heap_mut()
                                .alloc_object_with_value(obj)
                                .map(|p| p as usize)
                                .unwrap_or_else(|| {
                                    let hp = Box::into_raw(Box::new(
                                        crate::object::object::JSObject::new(),
                                    )) as usize;
                                    ctx.runtime_mut().gc_heap_mut().track(hp);
                                    hp
                                })
                        } else {
                            let heap_ptr = Box::into_raw(Box::new(obj)) as usize;
                            ctx.runtime_mut().gc_heap_mut().track(heap_ptr);
                            heap_ptr
                        };
                        self.allocation_count += 1;
                        JSValue::new_object(ptr)
                    } else {
                        if is_super_call {
                            self.frames[self.frame_index].this_value
                        } else {
                            JSValue::undefined()
                        }
                    };

                    let direct_eval = self.eval_direct_call;
                    self.eval_direct_call = false;
                    self.eval_direct_active = direct_eval;
                    let pushed = self.execute_call(
                        ctx,
                        func_val,
                        this_val,
                        dst,
                        argc,
                        arg_regs,
                        obj_reg,
                        is_call_new,
                        is_call_method,
                        is_super_call,
                    )?;
                    self.eval_direct_active = false;
                    if is_super_call {
                        if pushed {
                            let frame = &mut self.frames[self.frame_index];
                            frame.is_super_call = true;
                            frame.is_constructor = true;
                            frame.new_target = caller_new_target;
                        } else {
                            let is_builtin = func_val.is_function() && func_val.as_function().is_builtin();
                            if is_builtin {
                                self.bind_super_call_builtin_result(dst);
                            } else {
                                self.bind_super_call_result(dst);
                            }
                        }
                    }
                    if pushed {
                        continue;
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }

                Opcode::CallNamedMethod => {
                    unsafe {
                        (*self.frames.as_mut_ptr().add(self.frame_index)).current_pc = instr_pc;
                    }
                    let dst = self.read_u16_pc();
                    let obj_reg = self.read_u16_pc();
                    let atom = crate::runtime::atom::Atom(self.read_u32_pc());
                    let argc = self.read_u16_pc();

                    let mut arg_regs_buf = [0u16; 16];
                    let mut arg_regs_vec = Vec::new();
                    let arg_regs: &[u16] = if argc as usize <= 16 {
                        for i in 0..argc {
                            arg_regs_buf[i as usize] = self.read_u16_pc();
                        }
                        &arg_regs_buf[..argc as usize]
                    } else {
                        arg_regs_vec.reserve(argc as usize);
                        for _ in 0..argc {
                            arg_regs_vec.push(self.read_u16_pc());
                        }
                        &arg_regs_vec
                    };

                    let obj_val = self.get_reg(obj_reg);
                    if !self.get_named_prop_fast(ctx, obj_reg, obj_reg, atom, instr_pc) {}
                    let func_val = self.get_reg(obj_reg);

                    self.set_reg(obj_reg, obj_val);
                    let this_val = obj_val;

                    if self.execute_call(
                        ctx, func_val, this_val, dst, argc, arg_regs, obj_reg, false, true, false,
                    )? {
                        continue;
                    }
                    if let Some(exc) = self.pending_throw.take() {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }

                Opcode::CallSpread | Opcode::CallMethodSpread | Opcode::CallNewSpread => {
                    let is_call_method = op == Opcode::CallMethodSpread;
                    let is_call_new = op == Opcode::CallNewSpread;
                    let dst = self.read_u16_pc();
                    let obj_reg = if is_call_method {
                        self.read_u16_pc()
                    } else {
                        0
                    };
                    let func_reg = self.read_u16_pc();
                    let arr_reg = self.read_u16_pc();

                    let func_val = self.get_reg(func_reg);
                    let arr_val = self.get_reg(arr_reg);
                    let mut args = Vec::new();
                    if arr_val.is_object() {
                        let arr_ptr = arr_val.get_ptr();
                        let is_jsarray = arr_val.as_object().is_dense_array();
                        if is_jsarray {
                            let arr = unsafe {
                                &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                            };
                            for val in arr.elements.iter() {
                                args.push(*val);
                            }
                        } else {
                            let arr = arr_val.as_object();
                            let len_atom = ctx.common_atoms.length;
                            let arr_len =
                                arr.get(len_atom).map(|v| v.get_int() as usize).unwrap_or(0);
                            if let Some(elems) = arr.get_array_elements() {
                                for val in elems.iter() {
                                    args.push(*val);
                                }
                            } else {
                                for i in 0..arr_len {
                                    let key = ctx.intern(&i.to_string());
                                    if let Some(val) = arr.get(key) {
                                        args.push(val);
                                    }
                                }
                            }
                        }
                    }
                    let argc = args.len() as u16;
                    let is_super_call = !is_call_method
                        && !is_call_new
                        && {
                            let s = self.current_super_constructor();
                            (s.is_function() || s.is_object_like())
                                && s.raw_bits() == func_val.raw_bits()
                        };
                    let caller_new_target = self.frames[self.frame_index].new_target;
                    if is_super_call {
                        let is_constructor = Self::value_is_constructor(ctx, func_val);
                        if !is_constructor {
                            self.set_pending_type_error(
                                ctx,
                                "Super constructor is not a constructor",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            continue;
                        }
                    }

                    let this_val = if is_call_method {
                        self.get_reg(obj_reg)
                    } else if is_call_new {
                        let props = ctx.runtime_mut().gc_heap_mut().take_prop_vec();
                        let mut obj = crate::object::object::JSObject::new_typed_from_pool(
                            crate::object::object::ObjectType::Ordinary,
                            props,
                        );
                        obj.ensure_shape(ctx.shape_cache_mut());
                        if let Some(proto_ptr) = ctx.get_object_prototype() {
                            obj.prototype = Some(proto_ptr);
                        }
                        if func_val.is_function() {
                            let js_func = func_val.as_function();

                            if !js_func.cached_prototype_ptr.is_null() {
                                obj.prototype = Some(js_func.cached_prototype_ptr);
                            } else {
                                let proto_key = ctx.common_atoms.prototype;
                                if let Some(proto_val) = js_func.base.get(proto_key) {
                                    if proto_val.is_object_like() {
                                        let cptr = proto_val.get_ptr()
                                            as *mut crate::object::object::JSObject;
                                        func_val.as_function_mut().cached_prototype_ptr = cptr;
                                        obj.prototype = Some(cptr);
                                    }
                                } else if !js_func.is_builtin() {
                                    let mut pobj = crate::object::object::JSObject::new();
                                    pobj.set(ctx.common_atoms.constructor, func_val);
                                    if let Some(opp) = ctx.get_object_prototype() {
                                        pobj.prototype = Some(opp);
                                    }
                                    let pp = Box::into_raw(Box::new(pobj)) as usize;
                                    ctx.runtime_mut().gc_heap_mut().track(pp);
                                    let func_mut = func_val.as_function_mut();
                                    func_mut.base.set(proto_key, JSValue::new_object(pp));
                                    let cptr = pp as *mut crate::object::object::JSObject;
                                    func_mut.cached_prototype_ptr = cptr;
                                    obj.prototype = Some(cptr);
                                }
                            }
                        }
                        let ptr = if ctx.runtime().gc_heap().nursery_enabled_and_can_fit_object() {
                            ctx.runtime_mut()
                                .gc_heap_mut()
                                .alloc_object_with_value(obj)
                                .map(|p| p as usize)
                                .unwrap_or_else(|| {
                                    let hp = Box::into_raw(Box::new(
                                        crate::object::object::JSObject::new(),
                                    )) as usize;
                                    ctx.runtime_mut().gc_heap_mut().track(hp);
                                    hp
                                })
                        } else {
                            let heap_ptr = Box::into_raw(Box::new(obj)) as usize;
                            ctx.runtime_mut().gc_heap_mut().track(heap_ptr);
                            heap_ptr
                        };
                        self.allocation_count += 1;
                        JSValue::new_object(ptr)
                    } else {
                        if is_super_call {
                            self.frames[self.frame_index].this_value
                        } else {
                            JSValue::undefined()
                        }
                    };

                    if func_val.is_function() {
                        let ptr = func_val.get_ptr();
                        let js_func = func_val.as_function();
                        if js_func.is_class_constructor && !is_call_new && !is_super_call {
                            self.set_pending_type_error(
                                ctx,
                                "Class constructor cannot be invoked without 'new'",
                            );
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            self.eval_direct_active = false;
                            continue;
                        }
                        if let Some(ref rb) = js_func.bytecode {
                            if js_func.is_generator() {
                                match self.instantiate_generator_object(
                                    ctx, rb, func_val, this_val, &args,
                                )? {
                                    Some(gen_val) => self.set_reg(dst, gen_val),
                                    None => {
                                        if let Some(exc) = self.pending_throw.take() {
                                            match self.dispatch_throw_value(ctx, exc) {
                                                ThrowDispatch::Caught => continue,
                                                ThrowDispatch::Uncaught(e) => return Err(e),
                                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                            }
                                        }
                                    }
                                }
                                self.eval_direct_active = false;
                                continue;
                            }
                            let return_pc = self.pc;
                            self.push_frame(
                                rb,
                                return_pc,
                                Some(ptr),
                                this_val,
                                dst,
                                argc,
                                is_call_new,
                                js_func.is_async(),
                                &args,
                                js_func.uses_arguments(),
                            );
                            if js_func.is_arrow() {
                                self.frames[self.frame_index].this_cell = js_func.arrow_this_cell.clone();
                                self.frames[self.frame_index].super_ctor = js_func.arrow_super;
                                self.frames[self.frame_index].super_base = js_func.arrow_super_base;
                                if let Some(cell) = &js_func.arrow_this_cell {
                                    let (tv, init) = cell.get();
                                    self.frames[self.frame_index].this_value = tv;
                                    self.frames[self.frame_index].this_initialized = init;
                                } else {
                                    self.frames[self.frame_index].this_initialized =
                                        js_func.arrow_this_initialized;
                                }
                            } else {
                                let super_key = ctx.common_atoms.__super__;
                                if let Some(super_val) = js_func.base.get(super_key) {
                                    self.frames[self.frame_index].super_ctor = super_val;
                                }
                                if js_func.is_derived_constructor {
                                    self.frames[self.frame_index].this_initialized = false;
                                }
                            }
                            if is_call_new {
                                self.frames[self.frame_index].new_target = func_val;
                            }
                            if js_func.is_class_constructor && !js_func.is_derived_constructor {
                                self.mark_instance_elements_pending(this_val, ptr);
                            }
                            if is_super_call {
                                let frame = &mut self.frames[self.frame_index];
                                frame.is_super_call = true;
                                frame.is_constructor = true;
                                frame.new_target = caller_new_target;
                            }
                            self.eval_direct_active = false;
                            continue;
                        } else if js_func.is_builtin() {
                            let mut builtin_args = Vec::with_capacity(args.len() + 1);
                            if is_call_method {
                                builtin_args.push(self.get_reg(obj_reg));
                            }
                            builtin_args.extend(args);
                            let direct_eval = self.eval_direct_call;
                            self.eval_direct_call = false;
                            self.eval_direct_active = direct_eval;
                            let result = if let Some(bf) = js_func.builtin_func {
                                ctx.call_builtin_direct(bf, &builtin_args)
                            } else if let Some(ba) = js_func.builtin_atom {
                                let name = ctx.get_atom_str(ba).to_string();
                                ctx.call_builtin(&name, &builtin_args)
                            } else {
                                JSValue::undefined()
                            };
                            self.set_reg(dst, result);
                            self.eval_direct_active = false;
                            if is_super_call {
                                self.bind_super_call_builtin_result(dst);
                            }
            }
        } else if func_val.is_object() && !is_call_new {
                        match self.call_function_with_this(ctx, func_val, this_val, &args) {
                            Ok(result) => {
                                self.set_reg(dst, result);
                                if is_super_call {
                                    self.bind_super_call_result(dst);
                                }
                            }
                            Err(_) => {
                                let msg = format!(
                                    "{} is not a function",
                                    self.format_thrown_value(&func_val, ctx)
                                );
                                self.set_pending_type_error(ctx, &msg);
                                if let Some(exc) = self.pending_throw.take() {
                                    match self.dispatch_throw_value(ctx, exc) {
                                        ThrowDispatch::Caught => continue,
                                        ThrowDispatch::Uncaught(e) => return Err(e),
                                        ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                    }
                                }
                            }
                        }
                    } else {
                        let msg = format!(
                            "{} is not a function",
                            self.format_thrown_value(&func_val, ctx)
                        );
                        self.set_pending_type_error(ctx, &msg);
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }

                Opcode::NewFunction
                | Opcode::NewGeneratorFunction
                | Opcode::NewAsyncFunction
                | Opcode::NewAsyncGeneratorFunction => {
                    let is_generator = op == Opcode::NewGeneratorFunction
                        || op == Opcode::NewAsyncGeneratorFunction;
                    let is_async =
                        op == Opcode::NewAsyncFunction || op == Opcode::NewAsyncGeneratorFunction;

                    let newfunc_start_pc = self.pc;

                    let cached: Option<std::sync::Arc<crate::compiler::opcode::NestedBytecode>> =
                        if let Some(parent_ptr) = self.frames[self.frame_index].function_ptr {
                            let parent_func = unsafe {
                                &*(parent_ptr as *const crate::object::function::JSFunction)
                            };
                            if let Some(bc) = parent_func.bytecode.as_ref() {
                                // First check the instance's own nested_bytecodes
                                let found = bc
                                    .nested_bytecodes
                                    .iter()
                                    .find(|(k, _)| *k == newfunc_start_pc as u32)
                                    .map(|(_, v)| v.clone());
                                if found.is_some() {
                                    found
                                } else if !bc.shared_nested_bytecodes_ptr.is_null() {
                                    // Fall back to the shared NestedBytecode's nested_bytecodes
                                    // (persists across re-creations of the parent closure)
                                    unsafe { &*(*bc.shared_nested_bytecodes_ptr).get() }
                                        .iter()
                                        .find(|(k, _)| *k == newfunc_start_pc as u32)
                                        .map(|(_, v)| v.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                    let mut nb_for_ic: Option<
                        std::sync::Arc<crate::compiler::opcode::NestedBytecode>,
                    > = None;

                    let (
                        fb_code,
                        fb_constants,
                        locals_count,
                        param_count,
                        uses_arguments,
                        has_simple_parameters,
                        is_strict,
                        function_kind,
                        func_var_name_to_slot,
                        upvalue_descs,
                        line_number_table_opt,
                        func_name_atom,
                    ) = if let Some(ref nb) = cached {
                        self.pc = newfunc_start_pc + nb.parent_bytecode_span as usize;
                        let uvdescs: Vec<(u32, u32)> = nb.upvalue_descs.clone();

                        (
                            Vec::new(),
                            Vec::new(),
                            nb.locals_count,
                            nb.param_count,
                            nb.uses_arguments,
                            nb.has_simple_parameters,
                            nb.is_strict,
                            nb.function_kind,
                            nb.var_name_to_slot.clone(),
                            uvdescs,
                            nb.line_number_table.clone(),
                            nb.func_name_atom,
                        )
                    } else {
                        let param_count = self.read_u16_pc();
                        let uses_arguments = self.read_u8() != 0;
                        let is_strict = self.read_u8() != 0;
                        let has_simple_parameters = self.read_u8() != 0;
                        let function_kind = self.read_u8();
                        let locals_count = self.read_u16_pc() as u32;
                        let bytecode_len = self.read_u32();
                        let mut fb_code = Vec::with_capacity(bytecode_len as usize);

                        unsafe {
                            let src = self.cached_code_ptr.add(self.pc);
                            fb_code.set_len(bytecode_len as usize);
                            std::ptr::copy_nonoverlapping(
                                src,
                                fb_code.as_mut_ptr(),
                                bytecode_len as usize,
                            );
                        }
                        self.pc += bytecode_len as usize;
                        let constants_len = self.read_u32();
                        let mut fb_constants = Vec::with_capacity(constants_len as usize);
                        for _ in 0..constants_len {
                            let const_type = self.read_u8();
                            match const_type {
                                0 => fb_constants.push(JSValue::undefined()),
                                1 => fb_constants.push(JSValue::null()),
                                2 => fb_constants.push(JSValue::bool(true)),
                                3 => fb_constants.push(JSValue::bool(false)),
                                4 => {
                                    let v = self.read_i64();
                                    fb_constants.push(JSValue::new_int(v));
                                }
                                5 => {
                                    let bytes = [
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                        self.read_u8(),
                                    ];
                                    let v = f64::from_le_bytes(bytes);
                                    fb_constants.push(JSValue::new_float(v));
                                }
                                6 => {
                                    let atom_id = self.read_u32() as u32;
                                    fb_constants.push(JSValue::new_string(
                                        crate::runtime::atom::Atom(atom_id),
                                    ));
                                }
                                _ => fb_constants.push(JSValue::undefined()),
                            }
                        }
                        let upvalue_count = self.read_u32();
                        let mut upvalue_descs = Vec::with_capacity(upvalue_count as usize);
                        for _ in 0..upvalue_count {
                            let atom_id = self.read_u32();
                            let local_idx = self.read_u32();
                            upvalue_descs.push((atom_id, local_idx));
                        }

                        let line_table_entry_count = self.read_u32();
                        let mut line_table = crate::compiler::location::LineNumberTable::new();
                        for _ in 0..line_table_entry_count {
                            let off = self.read_u32();
                            let line = self.read_u32();
                            line_table.add_entry(off, line);
                        }
                        let line_number_table_opt = if line_table_entry_count > 0 {
                            Some(std::sync::Arc::new(line_table))
                        } else {
                            None
                        };
                        let func_name_atom = self.read_u32() as u32;
                        let var_count = self.read_u32();
                        let func_var_name_to_slot = std::rc::Rc::new({
                            let mut v = Vec::new();
                            for _ in 0..var_count {
                                let atom_id = self.read_u32();
                                let slot = self.read_u16_pc();
                                let tag = self.read_u8();
                                v.push((atom_id, slot, tag));
                            }
                            v
                        });

                        let parent_bytecode_span = (self.pc - newfunc_start_pc) as u32;

                        if let Some(parent_ptr) = self.frames[self.frame_index].function_ptr {
                            let parent_func = unsafe {
                                &mut *(parent_ptr as *mut crate::object::function::JSFunction)
                            };
                            if let Some(bc) = parent_func.bytecode.as_mut() {
                                let key = newfunc_start_pc as u32;
                                // Check instance's own nested_bytecodes first
                                let found_nb = bc
                                    .nested_bytecodes
                                    .iter()
                                    .find(|(k, _)| *k == key)
                                    .map(|(_, v)| v.clone())
                                    .or_else(|| {
                                        if !bc.shared_nested_bytecodes_ptr.is_null() {
                                            // Check shared NestedBytecode's nested_bytecodes
                                            unsafe { &*(*bc.shared_nested_bytecodes_ptr).get() }
                                                .iter()
                                                .find(|(k, _)| *k == key)
                                                .map(|(_, v)| v.clone())
                                        } else {
                                            None
                                        }
                                    });
                                if let Some(existing) = found_nb {
                                    nb_for_ic = Some(existing.clone());
                                } else {
                                    // Create new NestedBytecode
                                    let nb = std::sync::Arc::new(
                                        crate::compiler::opcode::NestedBytecode {
                                            code: fb_code.clone(),
                                            constants: fb_constants.clone(),
                                            locals_count,
                                            param_count,
                                            uses_arguments,
                                            has_simple_parameters,
                                            is_strict,
                                            function_kind,
                                            var_name_to_slot: func_var_name_to_slot.clone(),
                                            line_number_table: line_number_table_opt.clone(),
                                            parent_bytecode_span,
                                            upvalue_count: upvalue_descs.len() as u32,
                                            upvalue_descs: upvalue_descs.clone(),
                                            func_name_atom,
                                            source_text: None,
                                            ic_table: std::cell::UnsafeCell::new(
                                                crate::compiler::InlineCacheTable::new(),
                                            ),
                                            nested_bytecodes: std::cell::UnsafeCell::new(Vec::new()),
                                        },
                                    );
                                    nb_for_ic = Some(nb.clone());
                                    // Store in instance's nested_bytecodes
                                    bc.nested_bytecodes.push((key, nb.clone()));
                                    // Also store in shared NestedBytecode's nested_bytecodes
                                    // so sub-sub-closures persist across re-creations of this closure
                                    if !bc.shared_nested_bytecodes_ptr.is_null() {
                                        let shared = unsafe {
                                            &mut *(*bc.shared_nested_bytecodes_ptr).get()
                                        };
                                        if !shared.iter().any(|(k, _)| *k == key) {
                                            shared.push((key, nb.clone()));
                                        }
                                    }
                                }
                            }
                        }
                        (
                            fb_code,
                            fb_constants,
                            locals_count,
                            param_count,
                            uses_arguments,
                            has_simple_parameters,
                            is_strict,
                            function_kind,
                            func_var_name_to_slot,
                            upvalue_descs,
                            line_number_table_opt,
                            func_name_atom,
                        )
                    };

                    let mut func = crate::object::function::JSFunction::new();

                    if let Some(fn_proto_ptr) = ctx.get_function_prototype() {
                        func.base.prototype = Some(fn_proto_ptr);
                    }
                    func.param_count = param_count as u32;
                    func.arity = param_count as u32;
                    func.locals_count = locals_count;
                    func.set_is_generator(is_generator);
                    func.set_is_async(is_async);
                    func.set_uses_arguments(uses_arguments);
                    func.set_is_strict(is_strict);
                    func.set_is_arrow(function_kind & 4 != 0);
                    func.is_derived_constructor = function_kind & 8 != 0;
                    func.is_class_constructor = function_kind & 2 != 0;
                    func.is_method = function_kind & 32 != 0;
                    func.has_private_elements = function_kind & 16 != 0;
                    if function_kind & 4 != 0 {
                        if let Some(parent_ptr) = self.frames[self.frame_index].function_ptr {
                            let parent_func = unsafe {
                                &*(parent_ptr as *const crate::object::function::JSFunction)
                            };
                            func.arrow_enclosing_ctor_ptr = if parent_func.is_arrow() {
                                parent_func.arrow_enclosing_ctor_ptr
                            } else if parent_func.is_class_constructor
                                && parent_func.has_private_elements
                            {
                                parent_ptr
                            } else {
                                0
                            };
                        }
                        let parent = &mut self.frames[self.frame_index];
                        if parent.this_cell.is_none() {
                            parent.this_cell = Some(std::rc::Rc::new(std::cell::Cell::new((
                                parent.this_value,
                                parent.this_initialized,
                            ))));
                        }
                        func.arrow_this_cell = parent.this_cell.clone();
                        func.arrow_this = parent.this_value;
                        func.arrow_this_initialized = parent.this_initialized;
                        func.arrow_super = parent.super_ctor;
                        func.arrow_super_base = parent.super_base;
                    }
                    func.line_number_table = line_number_table_opt.clone();
                    func.bytecode = Some(Bytecode {
                        code: fb_code,
                        constants: fb_constants,
                        locals_count,
                        param_count,
                        line_number_table: line_number_table_opt,
                        ic_table: crate::compiler::InlineCacheTable::new(),
                        shared_ic_table_ptr: std::ptr::null_mut(),
                        shared_nested_bytecodes_ptr: std::ptr::null_mut(),
                        shared_code_ptr: std::ptr::null(),
                        shared_code_len: 0,
                        shared_const_ptr: std::ptr::null(),
                        shared_const_len: 0,
                        uses_arguments,
                        has_simple_parameters,
                        is_strict,
                        var_name_to_slot: func_var_name_to_slot,
                        nested_bytecodes: Vec::new(),
                        is_simple_constructor: false,
                        simple_constructor_props: Vec::new(),
                        cached_constructor_final_shape: None,
                        cached_constructor_atoms: Vec::new(),
                    });

                    let effective_nb = nb_for_ic.or_else(|| cached.clone());
                    if let Some(ref nb_arc) = effective_nb {
                        if let Some(bc) = func.bytecode.as_mut() {
                            bc.shared_ic_table_ptr = nb_arc.ic_table.get();
                            bc.shared_nested_bytecodes_ptr =
                                &nb_arc.nested_bytecodes as *const _ as *mut _;

                            if bc.code.is_empty() {
                                bc.shared_code_ptr = nb_arc.code.as_ptr();
                                bc.shared_code_len = nb_arc.code.len();
                            }
                            if bc.constants.is_empty() {
                                bc.shared_const_ptr = nb_arc.constants.as_ptr();
                                bc.shared_const_len = nb_arc.constants.len();
                            }
                        }
                    }
                    func.shared_nb_for_ic = effective_nb;

                    let inherited_sentinel = u16::MAX as usize;
                    let current_frame_base = self.frames[self.frame_index].registers_base;
                    if !upvalue_descs.is_empty() {
                        let n = upvalue_descs.len();
                        let uv = func.upvalues_mut();
                        uv.upvalue_slot_atoms.reserve(n);
                        uv.upvalue_slots.reserve(n);
                        uv.upvalue_cells.reserve(n);
                        uv.upvalue_local_indices.reserve(n);
                    }
                    for (atom_id, local_idx_raw) in &upvalue_descs {
                        let atom = crate::runtime::atom::Atom(*atom_id);
                        let local_idx = *local_idx_raw as usize;
                        func.upvalues_mut().upvalue_slot_atoms.push(atom);

                        let cell = if local_idx != inherited_sentinel {
                            func.upvalues_mut()
                                .upvalue_local_indices
                                .insert(atom, local_idx);
                            let initial_value =
                                if local_idx < self.frames[self.frame_index].registers_count {
                                    self.registers[current_frame_base + local_idx]
                                } else {
                                    JSValue::undefined()
                                };
                            let current_frame = &mut self.frames[self.frame_index];
                            let local_idx_u16 = local_idx as u16;
                            if let Some(existing) = current_frame
                                .upvalue_sync_map
                                .as_ref()
                                .and_then(|m| m.get(&local_idx_u16))
                            {
                                existing.clone()
                            } else {
                                let new_cell =
                                    std::rc::Rc::new(std::cell::Cell::new(initial_value));
                                // Always create the sync map entry so that
                                // later assignments to the register also
                                // update the upvalue cell. This is needed
                                // for closures that capture let variables
                                // before initialization (function hoisting).
                                current_frame
                                    .upvalue_sync_map
                                    .get_or_insert_with(|| Box::new(FxHashMap::default()))
                                    .insert(local_idx_u16, new_cell.clone());
                                if local_idx_u16 < 64 {
                                    current_frame.upvalue_sync_bitset |= 1u64 << local_idx_u16;
                                    self.cached_upvalue_sync_bitset |= 1u64 << local_idx_u16;
                                }
                                self.cached_has_upvalue_sync = true;
                                new_cell
                            }
                        } else if let Some(parent_ptr) = self.frames[self.frame_index].function_ptr
                        {
                            let parent_func = unsafe {
                                &*(parent_ptr as *const crate::object::function::JSFunction)
                            };

                            if let Some(parent_cell) = parent_func
                                .upvalues_ref()
                                .and_then(|u| u.upvalue_cells.get(&atom))
                            {
                                parent_cell.clone()
                            } else {
                                std::rc::Rc::new(std::cell::Cell::new(JSValue::undefined()))
                            }
                        } else {
                            std::rc::Rc::new(std::cell::Cell::new(JSValue::undefined()))
                        };
                        func.upvalues_mut().upvalue_slots.push(cell.clone());
                        func.upvalues_mut().upvalue_cells.insert(atom, cell);
                    }

                    let display_name = if func_name_atom != 0 {
                        crate::runtime::atom::Atom(func_name_atom)
                    } else {
                        ctx.intern("")
                    };
                    func.name = display_name;
                    {
                        let length_desc = crate::object::object::PropertyDescriptor {
                            value: Some(JSValue::new_int(param_count as i64)),
                            writable: false,
                            enumerable: false,
                            configurable: true,
                            get: None,
                            set: None,
                        };
                        func.base.define_property(ctx.common_atoms.length, length_desc);
                    }

                    {
                        let name_desc = crate::object::object::PropertyDescriptor {
                            value: Some(JSValue::new_string(display_name)),
                            writable: false,
                            enumerable: false,
                            configurable: true,
                            get: None,
                            set: None,
                        };
                        func.base.define_property(ctx.common_atoms.name, name_desc);
                    }

                    let has_prototype = function_kind & 1 != 0;
                    let is_class_constructor = function_kind & 2 != 0;

                    func.with_env = self.frames[self.frame_index].with_env.clone();
                    let proto_atom = ctx.common_atoms.prototype;
                    let func_ptr = Box::into_raw(Box::new(func)) as usize;
                    ctx.runtime_mut().gc_heap_mut().track_function(func_ptr);
                    let unboxed_func =
                        unsafe { &mut *(func_ptr as *mut crate::object::function::JSFunction) };
                    if has_prototype && !is_class_constructor {
                        let mut proto_obj = crate::object::object::JSObject::new();
                        let parent_proto = if is_generator && is_async {
                            ctx.get_async_generator_prototype()
                        } else if is_generator {
                            ctx.get_generator_prototype()
                        } else {
                            ctx.get_object_prototype()
                        };
                        if let Some(pp) = parent_proto {
                            proto_obj.prototype = Some(pp);
                        }
                        let proto_ptr = Box::into_raw(Box::new(proto_obj)) as usize;
                        ctx.runtime_mut().gc_heap_mut().track(proto_ptr);
                        let func_value = JSValue::new_function(func_ptr);
                        if !is_generator {
                            unsafe {
                                let proto_obj_mut =
                                    &mut *(proto_ptr as *mut crate::object::object::JSObject);
                                let ctor_desc = crate::object::object::PropertyDescriptor {
                                    value: Some(func_value),
                                    writable: true,
                                    enumerable: false,
                                    configurable: true,
                                    get: None,
                                    set: None,
                                };
                                proto_obj_mut
                                    .define_property(ctx.common_atoms.constructor, ctor_desc);
                            }
                        }
                        let proto_desc = crate::object::object::PropertyDescriptor {
                            value: Some(JSValue::new_object(proto_ptr)),
                            writable: true,
                            enumerable: false,
                            configurable: false,
                            get: None,
                            set: None,
                        };
                        unboxed_func.base.define_property(proto_atom, proto_desc);
                        unboxed_func.cached_prototype_ptr =
                            proto_ptr as *mut crate::object::object::JSObject;
                    }
                    let function_parent_proto = if is_generator && is_async {
                        ctx.get_async_generator_function_prototype()
                    } else if is_generator {
                        ctx.get_generator_function_prototype()
                    } else {
                        ctx.get_function_prototype()
                    };
                    if let Some(fn_pp) = function_parent_proto {
                        unboxed_func.base.prototype = Some(fn_pp);
                    } else if let Some(obj_pp) = ctx.get_object_prototype() {
                        unboxed_func.base.prototype = Some(obj_pp);
                    }
                    self.allocation_count += 1;
                    let dst_reg = self.read_u16_pc();
                    self.set_reg(dst_reg, JSValue::new_function(func_ptr));
                    self.maybe_gc(ctx);
                }
                Opcode::LoadTdz => {
                    let dst = self.read_u16_pc();
                    self.set_reg(dst, JSValue::new_tdz());
                }
                Opcode::CheckTdz => {
                    let reg = self.read_u16_pc();
                    if self.get_reg(reg).is_tdz() {
                        self.set_pending_reference_error(
                            ctx,
                            "Cannot access variable before initialization",
                        );
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(_) => continue,
                            }
                        }
                    }
                }
                Opcode::CheckRef => {
                    let reg = self.read_u16_pc();
                    let idx = self.read_u32() as usize;

                    let reg_val = self.get_reg(reg);
                    if !reg_val.is_undefined()
                        && !reg_val.is_tdz()
                        && self.eval_binding_frames == 0
                        && self.caller_vm.is_none()
                    {
                        continue;
                    }
                    let name = if idx < self.frames[self.frame_index].constants_len {
                        unsafe { *self.cached_const_ptr.add(idx) }
                    } else {
                        JSValue::undefined()
                    };
                    let atom = name.get_atom();
                    let mut exists = false;
                    let has_eval = self.eval_binding_frames > 0 || self.caller_vm.is_some();
                    if has_eval {
                        for fi in (0..=self.frame_index).rev() {
                            if let Some(ref eb) = self.frames[fi].eval_bindings {
                                if eb.contains_key(&atom.0) {
                                    exists = true;
                                    break;
                                }
                            }
                        }
                        if !exists && self.get_var_in_caller_vm(atom.0).is_some() {
                            exists = true;
                        }
                    }
                    if !exists {
                        let global = ctx.global();
                        if global.is_object() {
                            exists = global.as_object().get_own(atom).is_some();
                        }
                    }
                    if !exists {
                        let ref_err_atom = ctx.intern("ReferenceError");
                        let name_str = ctx.get_atom_str(atom).to_string();
                        let err_msg = format!("{} is not defined", name_str);
                        let msg_atom = ctx.intern(&err_msg);
                        let mut err = crate::object::object::JSObject::new();
                        err.set(ctx.intern("name"), JSValue::new_string(ref_err_atom));
                        err.set(ctx.intern("message"), JSValue::new_string(msg_atom));
                        if let Some(proto) = ctx.get_reference_error_prototype() {
                            err.prototype = Some(proto as *mut _);
                        } else if let Some(proto) = ctx.get_error_prototype() {
                            err.prototype = Some(proto as *mut _);
                        }
                        let ptr = Box::into_raw(Box::new(err)) as usize;
                        ctx.runtime_mut().gc_heap_mut().track(ptr);
                        self.pending_throw = Some(JSValue::new_object(ptr));
                        if let Some(exc) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(_) => continue,
                            }
                        }
                    }
                }
                Opcode::CheckObjectCoercible => {
                    let reg = self.read_u16_pc();
                    let val = self.get_reg(reg);
                    if val.is_null() || val.is_undefined() {
                        let type_err_atom = ctx.intern("TypeError");
                        let msg = "Cannot destructure 'undefined' or 'null'.";
                        let msg_atom = ctx.intern(msg);
                        let mut err = crate::object::object::JSObject::new();
                        err.set(ctx.intern("name"), JSValue::new_string(type_err_atom));
                        err.set(ctx.intern("message"), JSValue::new_string(msg_atom));
                        if let Some(proto) = ctx.get_type_error_prototype() {
                            err.prototype = Some(proto as *mut _);
                        } else if let Some(proto) = ctx.get_error_prototype() {
                            err.prototype = Some(proto as *mut _);
                        }
                        let ptr = Box::into_raw(Box::new(err)) as usize;
                        ctx.runtime_mut().gc_heap_mut().track(ptr);
                        let exc = JSValue::new_object(ptr);
                        self.pending_throw = Some(exc);
                        if let Some(thrown) = self.pending_throw.take() {
                            match self.dispatch_throw_value(ctx, thrown) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => {
                                    self.pending_throw = Some(exc);
                                    return Err(e);
                                }
                                ThrowDispatch::AsyncComplete(_) => continue,
                            }
                        }
                    }
                }
                Opcode::GetIterator | Opcode::GetAsyncIterator => {
                    let is_async = op == Opcode::GetAsyncIterator;
                    let dst = self.read_u16_pc();
                    let src = self.read_u16_pc();
                    let iterable = self.get_reg(src);
                    match self.get_iterator(ctx, iterable, is_async) {
                        Ok(record) => self.set_reg(dst, record),
                        Err(e) => {
                            if self.pending_throw.is_none() {
                                let host_exc = ctx.pending_exception.take();
                                if let Some(exc) =
                                    self.last_caught_exception.take().or(host_exc)
                                {
                                    self.pending_throw = Some(exc);
                                }
                            }
                            if let Some(exc) = self.pending_throw.take() {
                                match self.dispatch_throw_value(ctx, exc) {
                                    ThrowDispatch::Caught => continue,
                                    ThrowDispatch::Uncaught(e) => return Err(e),
                                    ThrowDispatch::AsyncComplete(o) => return Ok(o),
                                }
                            }
                            return Err(e);
                        }
                    }
                }
                Opcode::IteratorNext => {
                    let instr_pc = self.pc - 1;
                    let dst_val = self.read_u16_pc();
                    let dst_done = self.read_u16_pc();
                    let iter_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    if let Err(e) = self.op_iterator_next(
                        ctx, instr_pc, dst_val, dst_done, iter_val, None, false, true, None,
                    ) {
                        if self.pending_throw.is_some() {
                            continue;
                        }
                        return Err(e);
                    }
                }
                Opcode::IteratorStep => {
                    let instr_pc = self.pc - 1;
                    let dst_done = self.read_u16_pc();
                    let iter_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    if let Err(e) = self.op_iterator_next(
                        ctx,
                        instr_pc,
                        dst_done,
                        dst_done,
                        iter_val,
                        None,
                        false,
                        false,
                        None,
                    ) {
                        if self.pending_throw.is_some() {
                            continue;
                        }
                        return Err(e);
                    }
                }
                Opcode::IteratorNextAwait => {
                    let instr_pc = self.pc - 1;
                    let dst_val = self.read_u16_pc();
                    let dst_done = self.read_u16_pc();
                    let iter_reg = self.read_u16_pc();
                    let state_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    match self.op_iterator_next(
                        ctx,
                        instr_pc,
                        dst_val,
                        dst_done,
                        iter_val,
                        None,
                        true,
                        true,
                        Some(state_reg),
                    ) {
                        Ok(true) => return Ok(ExecutionOutcome::Yield(JSValue::undefined())),
                        Ok(false) => {}
                        Err(e) => {
                            if self.pending_throw.is_some() {
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Opcode::IteratorNextSend => {
                    let instr_pc = self.pc - 1;
                    let dst_val = self.read_u16_pc();
                    let dst_done = self.read_u16_pc();
                    let iter_reg = self.read_u16_pc();
                    let send_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    let send_val = self.get_reg(send_reg);
                    if let Err(e) = self.op_iterator_next(
                        ctx,
                        instr_pc,
                        dst_val,
                        dst_done,
                        iter_val,
                        Some(send_val),
                        false,
                        true,
                        None,
                    ) {
                        if self.pending_throw.is_some() {
                            continue;
                        }
                        return Err(e);
                    }
                }
                Opcode::IteratorNextSendAwait => {
                    let instr_pc = self.pc - 1;
                    let dst_val = self.read_u16_pc();
                    let dst_done = self.read_u16_pc();
                    let iter_reg = self.read_u16_pc();
                    let send_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    let send_val = self.get_reg(send_reg);
                    match self.op_iterator_next(
                        ctx,
                        instr_pc,
                        dst_val,
                        dst_done,
                        iter_val,
                        Some(send_val),
                        true,
                        true,
                        None,
                    ) {
                        Ok(true) => return Ok(ExecutionOutcome::Yield(JSValue::undefined())),
                        Ok(false) => {}
                        Err(e) => {
                            if self.pending_throw.is_some() {
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Opcode::IteratorClose => {
                    let iter_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    if let Some(exc) = self.invoke_iterator_return(ctx, iter_val) {
                        match self.dispatch_throw_value(ctx, exc) {
                            ThrowDispatch::Caught => continue,
                            ThrowDispatch::Uncaught(e) => return Err(e),
                            ThrowDispatch::AsyncComplete(o) => return Ok(o),
                        }
                    }
                }
                Opcode::IteratorCloseFinally => {
                    let iter_reg = self.read_u16_pc();
                    let iter_val = self.get_reg(iter_reg);
                    let throw_completion = self
                        .pending_finally_rethrow
                        .map_or(false, |(frame, _)| frame == self.frame_index);
                    if let Some(exc) = self.invoke_iterator_return(ctx, iter_val) {
                        if throw_completion {
                            self.last_caught_exception = None;
                            ctx.pending_exception = None;
                            self.pending_throw = None;
                        } else {
                            if self
                                .pending_finally_return
                                .map_or(false, |(frame, _)| frame == self.frame_index)
                            {
                                self.pending_finally_return = None;
                            }
                            if self
                                .pending_finally_jump
                                .map_or(false, |(frame, _, _)| frame == self.frame_index)
                            {
                                self.pending_finally_jump = None;
                            }
                            match self.dispatch_throw_value(ctx, exc) {
                                ThrowDispatch::Caught => continue,
                                ThrowDispatch::Uncaught(e) => return Err(e),
                                ThrowDispatch::AsyncComplete(o) => return Ok(o),
                            }
                        }
                    }
                }
                Opcode::GetArguments => {
                    let dst = self.read_u16_pc();
                    let fi = self.frame_index;
                    if let Some(cached_ptr) = self.frames[fi].cached_arguments {
                        self.set_reg(dst, JSValue::new_object(cached_ptr));
                    } else {
                        let func_ptr = self.frames[fi].function_ptr;
                        let arg_count = self.frames[fi].saved_args.len();
                        let is_strict = self.frames[fi].is_strict_frame;
                        use crate::object::object::{
                            ATTR_CONFIGURABLE, ATTR_WRITABLE, PropertyDescriptor,
                        };
                        let mut args_obj = crate::object::object::JSObject::new();
                        if let Some(obj_proto) = ctx.get_object_prototype() {
                            args_obj.prototype = Some(obj_proto);
                        }

                        let args_shape = ctx.get_or_create_args_length_shape();
                        args_obj.set_first_prop_with_shape(
                            ctx.common_atoms.length,
                            JSValue::new_int(arg_count as i64),
                            ATTR_WRITABLE | ATTR_CONFIGURABLE,
                            args_shape,
                        );
                        let declared_param_count: usize = if !is_strict {
                            match func_ptr {
                                Some(ptr) => {
                                    let js_func =
                                        unsafe { JSValue::function_from_ptr(ptr) };
                                    if js_func
                                        .bytecode
                                        .as_ref()
                                        .map_or(false, |bc| bc.has_simple_parameters)
                                    {
                                        js_func.param_count as usize
                                    } else {
                                        0
                                    }
                                }
                                None => 0,
                            }
                        } else {
                            0
                        };
                        let mapped_limit = declared_param_count.min(arg_count);
                        if !is_strict {
                            args_obj
                                .set_obj_type(crate::object::object::ObjectType::MappedArguments);
                        }
                        let mut entries: Vec<crate::object::object::ArgumentEntry> =
                            Vec::with_capacity(mapped_limit);
                        for i in 0..arg_count {
                            let val = self.frames[fi].saved_args[i];
                            let atom = ctx.int_atom_mut(i);
                            if i < mapped_limit {
                                let local_idx = (1 + declared_param_count + i) as u16;
                                let registers_base = self.frames[fi].registers_base;
                                let register_index = registers_base + local_idx as usize;
                                let initial = if register_index < self.registers.len() {
                                    self.registers[register_index]
                                } else {
                                    val
                                };
                                let existing = self.frames[fi]
                                    .upvalue_sync_map
                                    .as_ref()
                                    .and_then(|m| m.get(&local_idx))
                                    .cloned();
                                let cell = match existing {
                                    Some(cell) => cell,
                                    None => {
                                        let cell =
                                            std::rc::Rc::new(std::cell::Cell::new(initial));
                                        let frame = &mut self.frames[fi];
                                        frame
                                            .upvalue_sync_map
                                            .get_or_insert_with(|| {
                                                Box::new(crate::util::FxHashMap::default())
                                            })
                                            .insert(local_idx, cell.clone());
                                        if local_idx < 64 {
                                            frame.upvalue_sync_bitset |= 1u64 << local_idx;
                                            self.cached_upvalue_sync_bitset |= 1u64 << local_idx;
                                        }
                                        self.cached_has_upvalue_sync = true;
                                        cell
                                    }
                                };
                                let mut entry =
                                    crate::object::object::ArgumentEntry::new(atom, initial);
                                entry.cell = Some(cell);
                                entries.push(entry);
                            } else {
                                args_obj.set(atom, val);
                            }
                        }
                        args_obj.set_argument_entries(entries);
                        {
                            let extra = args_obj.ensure_extra();
                            extra.mapped_args_frame_index = fi;
                            extra.mapped_args_param_count = declared_param_count as u32;
                        }
                        let callee_atom = ctx.common_atoms.callee;
                        if let Some(ptr) = func_ptr {
                            if is_strict {
                                let mut thrower = crate::object::function::JSFunction::new_builtin(
                                    ctx.intern("callee"),
                                    0,
                                );
                                thrower.builtin_atom = Some(ctx.intern("throw_type_error_callee"));
                                thrower.builtin_func =
                                    ctx.get_builtin_func("throw_type_error_callee");
                                let thrower_ptr = Box::into_raw(Box::new(thrower)) as usize;
                                ctx.runtime_mut().gc_heap_mut().track_function(thrower_ptr);
                                args_obj.define_accessor_flags(
                                    callee_atom,
                                    Some(JSValue::new_function(thrower_ptr)),
                                    None,
                                    false,
                                    false,
                                );
                            } else {
                                args_obj.define_property(
                                    callee_atom,
                                    PropertyDescriptor {
                                        value: Some(JSValue::new_function(ptr)),
                                        writable: true,
                                        enumerable: false,
                                        configurable: true,
                                        get: None,
                                        set: None,
                                    },
                                );
                            }
                        }

                        let sym_iter_atom =
                            crate::builtins::symbol::get_symbol_iterator_atom(ctx);
                        if sym_iter_atom.0 != 0 {
                            let iter_fn = ctx
                                .get_array_prototype()
                                .and_then(|p| unsafe { (*p).get(sym_iter_atom) });
                            if let Some(f) = iter_fn {
                                args_obj.define_property(
                                    sym_iter_atom,
                                    crate::object::object::PropertyDescriptor {
                                        value: Some(f),
                                        writable: true,
                                        enumerable: false,
                                        configurable: true,
                                        get: None,
                                        set: None,
                                    },
                                );
                            }
                        }

                        let ptr = if ctx
                            .runtime_mut()
                            .gc_heap_mut()
                            .nursery_enabled_and_can_fit_object()
                        {
                            ctx.runtime_mut()
                                .gc_heap_mut()
                                .alloc_object_with_value(args_obj)
                                .expect("nursery alloc failed despite can_fit")
                                as usize
                        } else {
                            let hp = Box::into_raw(Box::new(args_obj)) as usize;
                            ctx.runtime_mut().gc_heap_mut().track(hp);
                            hp
                        };
                        self.frames[fi].cached_arguments = Some(ptr);
                        self.set_reg(dst, JSValue::new_object(ptr));
                        self.allocation_count += 1;
                        self.maybe_gc(ctx);
                    }
                }
                Opcode::GetCurrentFunction => {
                    let dst = self.read_u16_pc();
                    let value = if let Some(ptr) = self.frames[self.frame_index].function_ptr {
                        JSValue::new_function(ptr)
                    } else {
                        JSValue::undefined()
                    };
                    self.set_reg(dst, value);
                }
            }
        }
    }

    pub fn execute_async_step(
        &mut self,
        ctx: &mut JSContext,
        state: &crate::object::object::AsyncState,
        is_generator: bool,
    ) -> Result<AsyncStepResult, String> {
        self.ctx_ptr = ctx;
        self.async_step = true;
        self.await_suspend = None;
        self.last_yield_src = None;
        self.last_yield_is_result = false;
        self.generator_resume_kind = 0;
        self.last_uncaught = None;
        self.pending_saved_args = Some(state.saved_args.clone());
        self.pending_function_ptr = state.func_ptr;
        self.pending_this = state.snapshot.first().copied();
        self.pending_handlers = Some(state.handlers.clone());

        let needed = state.bytecode.locals_count as usize;
        if needed > self.registers.len() {
            self.registers.resize(needed, JSValue::undefined());
        }
        for (i, val) in state.snapshot.iter().enumerate() {
            if i < self.registers.len() {
                self.registers[i] = *val;
            }
        }
        for i in state.snapshot.len()..needed {
            if i < self.registers.len() {
                self.registers[i] = JSValue::undefined();
            }
        }
        if state.yield_is_result {
            self.generator_resume_kind = state.resume_kind;
            if state.resume_kind == 2 {
                self.pending_return_value = None;
            }
        } else {
            match state.resume_kind {
                1 => {
                    self.pending_throw = Some(state.resume_value);
                }
                2 => {
                    self.pending_return_value = Some(state.resume_value);
                }
                _ => {}
            }
        }
        if let Some(exc) = state.pending_throw.clone() {
            self.pending_throw = Some(exc);
        }

        let saved_vm_ptr = ctx.get_register_vm_ptr();
        ctx.set_register_vm_ptr(Some(self as *mut VM as usize));
        let result = self.execute_inner(ctx, &state.bytecode, true, state.pc, true);
        ctx.set_register_vm_ptr(saved_vm_ptr);
        self.async_step = false;
        let stale_exc = ctx.pending_exception.take();

        let snapshot = self.registers.clone();
        let pc = self.pc;
        let handlers = self.exception_handlers.clone();

        match result {
            Ok(ExecutionOutcome::Complete(val)) => {
                Ok(AsyncStepResult::Complete(val, snapshot, pc, handlers))
            }
            Ok(ExecutionOutcome::Yield(_)) => match self.await_suspend.take() {
                Some((promise, src)) => {
                    Ok(AsyncStepResult::Suspended(promise, src, snapshot, pc, handlers))
                }
                None => {
                    if is_generator {
                        let value = self
                            .last_yield_src
                            .and_then(|src| snapshot.get(src as usize).copied())
                            .unwrap_or_else(JSValue::undefined);
                        if self.last_yield_is_result {
                            Ok(AsyncStepResult::Yielded(
                                value,
                                true,
                                self.last_yield_src,
                                snapshot,
                                pc,
                                handlers,
                            ))
                        } else {
                            let promise = if crate::builtins::promise::is_promise(&value) {
                                value
                            } else {
                                ctx.call_builtin("promise_resolve", &[value])
                            };
                            self.adopt_pending_host_exception(ctx);
                            if let Some(exc) = self.pending_throw.take() {
                                Ok(AsyncStepResult::Threw(exc))
                            } else {
                                Ok(AsyncStepResult::YieldSuspended(
                                    promise,
                                    self.last_yield_src,
                                    false,
                                    snapshot,
                                    pc,
                                    handlers,
                                ))
                            }
                        }
                    } else {
                        Ok(AsyncStepResult::Complete(
                            JSValue::undefined(),
                            snapshot,
                            pc,
                            handlers,
                        ))
                    }
                }
            },
            Err(e) => {
                let uncaught = self
                    .last_uncaught
                    .take()
                    .or_else(|| self.pending_throw.take())
                    .or_else(|| self.last_caught_exception.take())
                    .or(stale_exc);
                match uncaught {
                    Some(v) => Ok(AsyncStepResult::Threw(v)),
                    None => Ok(AsyncStepResult::Fatal(e)),
                }
            }
        }
    }

    fn instantiate_generator_object(
        &mut self,
        ctx: &mut JSContext,
        rb: &Bytecode,
        func_val: JSValue,
        this_val: JSValue,
        args: &[JSValue],
    ) -> Result<Option<JSValue>, String> {
        let js_func = unsafe { JSValue::function_from_ptr(func_val.get_ptr()) };

        let snapshot_this = if !js_func.is_strict() && (this_val.is_undefined() || this_val.is_null())
        {
            ctx.global()
        } else {
            this_val
        };
        let min_slots = 1 + args.len();
        let snapshot_len = (rb.locals_count as usize).max(min_slots);
        let mut snapshot = vec![JSValue::undefined(); snapshot_len];
        snapshot[0] = snapshot_this;
        for (i, arg) in args.iter().enumerate() {
            snapshot[i + 1] = *arg;
        }

        let (
            result_snapshot,
            result_pc,
            result_done,
            result_send_reg,
            result_handlers,
            result_yield_is_result,
        ) = {
            let mut generator_vm = crate::runtime::vm::VM::new();
            ctx.runtime_mut().gc_heap_mut().gc_suppressed = true;
            let step = generator_vm.execute_generator_step(
                ctx,
                rb,
                &snapshot,
                0,
                JSValue::undefined(),
                None,
                args,
                Some(func_val.get_ptr()),
                &[],
                0,
                JSValue::undefined(),
                false,
            );
            let uncaught = generator_vm.last_uncaught.take();
            ctx.runtime_mut().gc_heap_mut().gc_suppressed = false;
            drop(generator_vm);
            match step {
                Ok((_val, new_snapshot, new_pc, done, src, handlers, yield_is_result)) => {
                    (new_snapshot, new_pc, done, src, handlers, yield_is_result)
                }
                Err(e) => {
                    if let Some(exc) = uncaught {
                        self.last_caught_exception = Some(exc.clone());
                        self.pending_throw = Some(exc);
                        return Ok(None);
                    }
                    if e.contains("SyntaxError") || e.contains("Uncaught") {
                        return Err(e);
                    }
                    (snapshot, 0, false, None, Vec::new(), false)
                }
            }
        };

        let mut gen_obj = crate::object::object::JSObject::new();
        gen_obj.set_is_generator(true);
        if let Some(proto_ptr) = Self::generator_prototype_for(ctx, js_func) {
            gen_obj.prototype = Some(proto_ptr);
        }
        if js_func.is_async() {
            gen_obj.set_async_state(crate::object::object::AsyncState {
                bytecode: Box::new(rb.clone()),
                snapshot: result_snapshot,
                pc: result_pc,
                done: result_done,
                send_reg: result_send_reg,
                saved_args: args.to_vec(),
                func_ptr: Some(func_val.get_ptr()),
                handlers: result_handlers,
                pending_throw: None,
                resume_kind: 0,
                resume_value: JSValue::undefined(),
                yield_is_result: result_yield_is_result,
                pending_yield: false,
            });
            gen_obj.set_async_generator_state(crate::object::object::AsyncGeneratorState {
                queue: Vec::new(),
                running: false,
                current_promise: JSValue::undefined(),
                started: false,
                awaiting_return: false,
                pending_return_action: 0,
            });
        } else {
            gen_obj.set_generator_state(crate::object::object::GeneratorState {
                bytecode: Box::new(rb.clone()),
                snapshot: result_snapshot,
                pc: result_pc,
                done: result_done,
                send_reg: result_send_reg,
                argc: args.len() as u32,
                saved_args: args.to_vec(),
                func_ptr: Some(func_val.get_ptr()),
                handlers: result_handlers,
                yield_is_result: result_yield_is_result,
            });
        }
        let gen_ptr = Box::into_raw(Box::new(gen_obj)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(gen_ptr);
        Ok(Some(JSValue::new_object(gen_ptr)))
    }

    pub fn execute_generator_step(
        &mut self,
        ctx: &mut JSContext,
        bytecode: &Bytecode,
        registers_snapshot: &[JSValue],
        start_pc: usize,
        send_value: JSValue,
        send_reg: Option<u16>,
        saved_args: &[JSValue],
        func_ptr: Option<usize>,
        handlers: &[ExceptionHandler],
        resume_kind: u8,
        resume_value: JSValue,
        yield_is_result: bool,
    ) -> Result<
        (
            JSValue,
            Vec<JSValue>,
            usize,
            bool,
            Option<u16>,
            Vec<ExceptionHandler>,
            bool,
        ),
        String,
    > {
        self.ctx_ptr = ctx;
        self.last_yield_src = None;
        self.last_yield_is_result = false;
        self.generator_resume_kind = 0;
        self.last_uncaught = None;

        self.pending_saved_args = Some(saved_args.to_vec());
        self.pending_function_ptr = func_ptr;
        self.pending_this = registers_snapshot.first().copied();
        self.pending_handlers = Some(handlers.to_vec());
        if yield_is_result {
            self.generator_resume_kind = resume_kind;
            if resume_kind == 2 {
                self.pending_return_value = None;
            }
        } else {
            match resume_kind {
                1 => {
                    self.pending_throw = Some(resume_value);
                }
                2 => {
                    self.pending_return_value = Some(resume_value);
                }
                _ => {}
            }
        }

        let needed = bytecode.locals_count as usize;
        if needed > self.registers.len() {
            self.registers.resize(needed, JSValue::undefined());
        }
        for (i, val) in registers_snapshot.iter().enumerate() {
            if i < self.registers.len() {
                self.registers[i] = *val;
            }
        }
        for i in registers_snapshot.len()..needed {
            if i < self.registers.len() {
                self.registers[i] = JSValue::undefined();
            }
        }

        if let Some(r) = send_reg {
            let idx = r as usize;
            if idx < self.registers.len() {
                self.registers[idx] = send_value;
            }
        }

        let saved_vm_ptr = ctx.get_register_vm_ptr();
        ctx.set_register_vm_ptr(Some(self as *mut VM as usize));
        let result = match self.execute_inner(ctx, bytecode, true, start_pc, true) {
            Ok(r) => r,
            Err(e) => {
                ctx.set_register_vm_ptr(saved_vm_ptr);
                self.last_uncaught = self.pending_throw.take();
                return Err(e);
            }
        };
        ctx.set_register_vm_ptr(saved_vm_ptr);
        let handlers = self.exception_handlers.clone();
        match result {
            ExecutionOutcome::Complete(val) => Ok((
                val,
                self.registers.clone(),
                self.pc,
                true,
                self.last_yield_src,
                handlers,
                false,
            )),
            ExecutionOutcome::Yield(val) => Ok((
                val,
                self.registers.clone(),
                self.pc,
                false,
                self.last_yield_src,
                handlers,
                self.last_yield_is_result,
            )),
        }
    }

    /// Reads a property, invoking accessor getters and propagating their
    /// exceptions through the pending-throw machinery.
    fn property_key_index(name: &str) -> Option<u64> {
        if name.is_empty() || (name.len() > 1 && name.starts_with('0')) {
            return None;
        }
        let value: u64 = name.parse().ok()?;
        if value < 4_294_967_295 && value.to_string() == name {
            Some(value)
        } else {
            None
        }
    }

    fn own_enumerable_keys_in_order(
        &mut self,
        ctx: &mut JSContext,
        src_val: JSValue,
    ) -> Vec<crate::runtime::atom::Atom> {
        let mut integer_keys: Vec<(u64, crate::runtime::atom::Atom)> = Vec::new();
        let mut string_keys = Vec::new();
        let mut symbol_keys = Vec::new();

        if src_val.is_string() {
            let text = ctx.get_atom_str(src_val.get_atom()).to_string();
            for index in 0..text.chars().count() {
                integer_keys.push((index as u64, ctx.int_atom_mut(index)));
            }
            return integer_keys.into_iter().map(|(_, atom)| atom).collect();
        }

        if !(src_val.is_object() || src_val.is_function()) {
            return Vec::new();
        }

        let js_obj = unsafe { JSValue::object_from_ptr(src_val.get_ptr()) };
        if js_obj.is_dense_array() {
            let array = unsafe {
                &*(src_val.get_ptr() as *const crate::object::array_obj::JSArrayObject)
            };
            for index in 0..array.elements.len() {
                integer_keys.push((index as u64, ctx.int_atom_mut(index)));
            }
        }

        let mut sequence: Vec<crate::runtime::atom::Atom> = Vec::new();
        if let Some(entries) = js_obj.argument_entries() {
            for entry in entries {
                if entry.present {
                    sequence.push(entry.atom);
                }
            }
        }
        if js_obj.has_property_order() {
            for atom in js_obj.property_order_keys() {
                if !sequence.contains(&atom) {
                    sequence.push(atom);
                }
            }
        }
        js_obj.for_each_property(|atom, _, _| {
            if !sequence.contains(&atom) {
                sequence.push(atom);
            }
        });
        for (atom, _) in js_obj.accessor_keys() {
            if !sequence.contains(&atom) {
                sequence.push(atom);
            }
        }

        for atom in sequence {
            let Some(descriptor) = js_obj.get_own_descriptor(atom) else {
                continue;
            };
            if !descriptor.enumerable {
                continue;
            }
            if atom.0 & 0x40000000 != 0 {
                if !symbol_keys.contains(&atom) {
                    symbol_keys.push(atom);
                }
                continue;
            }
            let name = ctx.get_atom_str(atom);
            if let Some(index) = VM::property_key_index(name) {
                if !integer_keys.iter().any(|(i, _)| *i == index) {
                    integer_keys.push((index, atom));
                }
            } else if !string_keys.contains(&atom) {
                string_keys.push(atom);
            }
        }

        integer_keys.sort_by_key(|(index, _)| *index);
        let mut result: Vec<crate::runtime::atom::Atom> =
            integer_keys.into_iter().map(|(_, atom)| atom).collect();
        result.extend(string_keys);
        result.extend(symbol_keys);
        result
    }

    fn get_copy_value(
        &mut self,
        ctx: &mut JSContext,
        src_val: JSValue,
        atom: crate::runtime::atom::Atom,
    ) -> Result<JSValue, String> {
        if src_val.is_string() {
            let name = ctx.get_atom_str(atom).to_string();
            if let Ok(index) = name.parse::<usize>() {
                let text = ctx.get_atom_str(src_val.get_atom()).to_string();
                if let Some(ch) = text.chars().nth(index) {
                    return Ok(JSValue::new_string(ctx.intern(&ch.to_string())));
                }
            }
            return Ok(JSValue::undefined());
        }
        self.get_prop_value(ctx, src_val, atom)
    }

    pub fn get_prop_value(
        &mut self,
        ctx: &mut JSContext,
        obj: JSValue,
        atom: crate::runtime::atom::Atom,
    ) -> Result<JSValue, String> {
        if !obj.is_object_like() {
            if obj.is_string()
                || obj.is_int()
                || obj.is_float()
                || obj.is_bool()
                || obj.is_symbol()
                || obj.is_bigint()
            {
                if obj.is_string() && atom == ctx.common_atoms.length {
                    return Ok(JSValue::new_int(ctx.string_char_count(obj.get_atom()) as i64));
                }
                let mut current = if obj.is_string() {
                    ctx.get_string_prototype()
                } else if obj.is_int() || obj.is_float() {
                    ctx.get_number_prototype()
                } else if obj.is_symbol() {
                    ctx.get_symbol_prototype()
                } else if obj.is_bigint() {
                    ctx.get_bigint_prototype()
                } else {
                    ctx.get_bool_prototype()
                };
                while let Some(ptr) = current {
                    let proto = unsafe { &*ptr };
                    if let Some(entry) = proto.get_own_accessor_entry(atom) {
                        if let Some(getter) = entry.get {
                            if getter.is_function() {
                                return self.call_function_with_this(ctx, getter, obj, &[]);
                            }
                        }
                        return Ok(JSValue::undefined());
                    }
                    if let Some(v) = proto.get(atom) {
                        return Ok(v);
                    }
                    current = proto.prototype;
                }
            }
            return Ok(JSValue::undefined());
        }
        let js_obj = unsafe { obj.as_object() };
        if let Some(entry) = js_obj.get_own_accessor_entry(atom) {
            if let Some(getter) = entry.get {
                if getter.is_function() {
                    return self.call_function_with_this(ctx, getter, obj, &[]);
                }
            }
            return Ok(JSValue::undefined());
        }
        if let Some(v) = js_obj.get(atom) {
            return Ok(v);
        }
        if js_obj.is_dense_array() {
            if let Some(index) = ctx.atom_table().get_array_index(atom) {
                let array = unsafe {
                    &*(obj.get_ptr() as *const crate::object::array_obj::JSArrayObject)
                };
                if let Some(v) = array.elements.get(index).copied() {
                    return Ok(v);
                }
            }
        }
        let mut current = js_obj.prototype;
        let mut depth = 0u32;
        while let Some(proto_ptr) = current {
            if proto_ptr.is_null() || depth > 1000 {
                break;
            }
            depth += 1;
            let proto = unsafe { &*proto_ptr };
            if let Some(entry) = proto.get_own_accessor_entry(atom) {
                if let Some(getter) = entry.get {
                    if getter.is_function() {
                        return self.call_function_with_this(ctx, getter, obj, &[]);
                    }
                }
                return Ok(JSValue::undefined());
            }
            if let Some(v) = proto.get(atom) {
                return Ok(v);
            }
            current = proto.prototype;
        }
        Ok(JSValue::undefined())
    }

    fn await_settled_value(
        &mut self,
        ctx: &mut JSContext,
        val: JSValue,
    ) -> Result<Option<JSValue>, String> {
        if !crate::builtins::promise::is_promise(&val) {
            return Ok(Some(val));
        }
        let ptr = val.get_ptr();
        let promise_obj = unsafe { &*(ptr as *mut crate::object::object::JSObject) };
        let state_atom = ctx.common_atoms.__promise_state__;
        let result_atom = ctx.common_atoms.__promise_result__;
        let state = promise_obj
            .get(state_atom)
            .unwrap_or(JSValue::new_int(0))
            .get_int();
        let result = promise_obj.get(result_atom).unwrap_or(JSValue::undefined());
        match state {
            1 => Ok(Some(result)),
            2 => {
                self.last_caught_exception = Some(result.clone());
                self.pending_throw = Some(result);
                Err("awaited promise rejected".to_string())
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn op_iterator_next(
        &mut self,
        ctx: &mut JSContext,
        instr_pc: usize,
        dst_val: u16,
        dst_done: u16,
        iter_val: JSValue,
        send: Option<JSValue>,
        await_result: bool,
        want_value: bool,
        state_reg: Option<u16>,
    ) -> Result<bool, String> {
        let phase = state_reg
            .map(|r| {
                let v = self.get_reg(r);
                if v.is_int() { v.get_int() } else { 0 }
            })
            .unwrap_or(0);
        if !iter_val.is_object_like() {
            self.set_reg(dst_val, JSValue::undefined());
            self.set_reg(dst_done, JSValue::bool(true));
            return Ok(false);
        }

        let iter_ptr = iter_val.get_ptr();
        let iter_obj = unsafe { &mut *(iter_ptr as *mut crate::object::object::JSObject) };
        let arr_atom = ctx.common_atoms.__iter_arr__;

        if phase == 0
            && let Some(arr_val) = iter_obj.get(arr_atom)
        {
            let idx_atom = ctx.common_atoms.__iter_idx__;
            let idx = iter_obj.get(idx_atom).map(|v| v.get_int()).unwrap_or(0) as usize;

            if arr_val.is_string() {
                let atom = arr_val.get_atom();
                let s = ctx.atom_table().get(atom);
                let chars: Vec<char> = s.chars().collect();
                if idx < chars.len() {
                    let ch = chars[idx].to_string();
                    let ch_atom = ctx.atom_table_mut().intern(&ch);
                    let ch_val = JSValue::new_string(ch_atom);
                    iter_obj.set_cached(
                        idx_atom,
                        JSValue::new_int((idx + 1) as i64),
                        ctx.shape_cache_mut(),
                    );
                    self.set_reg(dst_val, ch_val);
                    self.set_reg(dst_done, JSValue::bool(false));
                } else {
                    self.set_reg(dst_val, JSValue::undefined());
                    self.set_reg(dst_done, JSValue::bool(true));
                }
                return Ok(false);
            } else if arr_val.is_object() {
                let arr_ptr = arr_val.get_ptr();
                let is_jsarray =
                    unsafe { &*(arr_ptr as *const crate::object::object::JSObject) }
                        .is_dense_array();
                if is_jsarray {
                    let arr = unsafe {
                        &*(arr_ptr as *const crate::object::array_obj::JSArrayObject)
                    };
                    let length = arr
                        .header
                        .get(ctx.common_atoms.length)
                        .map(|v| v.get_int().max(0) as usize)
                        .unwrap_or(0)
                        .max(arr.len());
                    if idx < length {
                        let index_atom = self.int_atom(idx, ctx);
                        let value = if let Some(entry) = arr.header.get_own_accessor_entry(index_atom)
                        {
                            if let Some(getter) = entry.get {
                                if getter.is_function() {
                                    match self.call_function_with_this(ctx, getter, arr_val, &[]) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            if let Some(exc) = self.last_caught_exception.take() {
                                                self.pending_throw = Some(exc);
                                            }
                                            return Err(e);
                                        }
                                    }
                                } else {
                                    JSValue::undefined()
                                }
                            } else {
                                JSValue::undefined()
                            }
                        } else {
                            arr.get(idx).unwrap_or(JSValue::undefined())
                        };
                        iter_obj.set_cached(
                            idx_atom,
                            JSValue::new_int((idx + 1) as i64),
                            ctx.shape_cache_mut(),
                        );
                        self.set_reg(dst_val, value);
                        self.set_reg(dst_done, JSValue::bool(false));
                    } else {
                        self.set_reg(dst_val, JSValue::undefined());
                        self.set_reg(dst_done, JSValue::bool(true));
                    }
                    return Ok(false);
                }
            } else {
                self.set_reg(dst_val, JSValue::undefined());
                self.set_reg(dst_done, JSValue::bool(true));
                return Ok(false);
            }
        }

        let done_atom = ctx.intern("done");
        let value_atom = ctx.intern("value");
        let record = Self::iterator_record_parts(ctx, iter_val);
        let (target, next_fn, async_from_sync) = match record {
            Some(parts) => parts,
            None => {
                let next_atom = ctx.intern("next");
                let next = self.get_prop_value(ctx, iter_val, next_atom)?;
                (iter_val, next, false)
            }
        };

        let mut result;
        if phase == 0 {
            if !next_fn.is_function() {
                let msg = "iterator next is not a function";
                self.set_pending_type_error(ctx, msg);
                return Err(msg.to_string());
            }
            let args: Vec<JSValue> = match send {
                Some(v) => vec![v],
                None => Vec::new(),
            };
            result = match self.call_function_with_this(ctx, next_fn, target, &args) {
                Ok(v) => v,
                Err(e) => {
                    self.rethrow_last_caught(ctx);
                    return Err(e);
                }
            };
            if await_result {
                if async_from_sync {
                    if result.is_null() || result.is_undefined() {
                        let msg = "iterator result is not an object";
                        self.set_pending_type_error(ctx, msg);
                        return Err(msg.to_string());
                    }
                    let value = match self.get_prop_value(ctx, result, value_atom) {
                        Ok(v) => v,
                        Err(e) => {
                            self.rethrow_last_caught(ctx);
                            return Err(e);
                        }
                    };
                    let done_val = match self.get_prop_value(ctx, result, done_atom) {
                        Ok(v) => v,
                        Err(e) => {
                            self.rethrow_last_caught(ctx);
                            return Err(e);
                        }
                    };
                    if done_val.is_truthy() {
                        self.set_reg(dst_val, value);
                        self.set_reg(dst_done, JSValue::bool(true));
                        return Ok(false);
                    }
                    let promise = ctx.call_builtin("promise_resolve", &[value]);
                    match self.promise_settlement(ctx, promise) {
                        PromiseSettlement::Fulfilled(v) => {
                            self.set_reg(dst_val, v);
                            self.set_reg(dst_done, JSValue::bool(false));
                            return Ok(false);
                        }
                        PromiseSettlement::Rejected(reason) => {
                            self.set_reg(dst_done, JSValue::bool(true));
                            self.last_caught_exception = Some(reason);
                            self.pending_throw = Some(reason);
                            return Err("awaited promise rejected".to_string());
                        }
                        PromiseSettlement::Pending => {
                            if let Some(r) = state_reg {
                                self.set_reg(r, JSValue::new_int(2));
                            }
                            self.await_suspend = Some((promise, dst_val));
                            self.pc = instr_pc;
                            return Ok(true);
                        }
                    }
                } else {
                    let promise = ctx.call_builtin("promise_resolve", &[result]);
                    match self.promise_settlement(ctx, promise) {
                        PromiseSettlement::Fulfilled(v) => result = v,
                        PromiseSettlement::Rejected(reason) => {
                            self.set_reg(dst_done, JSValue::bool(true));
                            self.last_caught_exception = Some(reason);
                            self.pending_throw = Some(reason);
                            return Err("awaited promise rejected".to_string());
                        }
                        PromiseSettlement::Pending => {
                            if let Some(r) = state_reg {
                                self.set_reg(r, JSValue::new_int(1));
                            }
                            self.await_suspend = Some((promise, dst_val));
                            self.pc = instr_pc;
                            return Ok(true);
                        }
                    }
                }
            }
        } else if phase == 1 {
            result = self.get_reg(dst_val);
            if let Some(r) = state_reg {
                self.set_reg(r, JSValue::new_int(0));
            }
        } else {
            if let Some(r) = state_reg {
                self.set_reg(r, JSValue::new_int(0));
            }
            self.set_reg(dst_done, JSValue::bool(false));
            return Ok(false);
        }

        if !result.is_object_like() {
            let msg = "iterator result is not an object";
            self.set_pending_type_error(ctx, msg);
            return Err(msg.to_string());
        }
        let done = match self.get_prop_value(ctx, result, done_atom) {
            Ok(v) => v.is_truthy(),
            Err(e) => {
                self.rethrow_last_caught(ctx);
                return Err(e);
            }
        };
        let mut value = JSValue::undefined();
        if want_value && !done {
            value = match self.get_prop_value(ctx, result, value_atom) {
                Ok(v) => v,
                Err(e) => {
                    self.rethrow_last_caught(ctx);
                    return Err(e);
                }
            };
        }
        self.set_reg(dst_val, value);
        self.set_reg(dst_done, JSValue::bool(done));
        Ok(false)
    }

    fn promise_settlement(&mut self, ctx: &mut JSContext, promise: JSValue) -> PromiseSettlement {
        if !promise.is_object() || !crate::builtins::promise::is_promise(&promise) {
            return PromiseSettlement::Fulfilled(promise);
        }
        let obj = promise.as_object();
        let state = obj
            .get(ctx.common_atoms.__promise_state__)
            .map(|v| v.get_int())
            .unwrap_or(0);
        let result = obj
            .get(ctx.common_atoms.__promise_result__)
            .unwrap_or_else(JSValue::undefined);
        match state {
            1 => PromiseSettlement::Fulfilled(result),
            2 => PromiseSettlement::Rejected(result),
            _ => PromiseSettlement::Pending,
        }
    }

    #[inline(always)]
    fn read_u16_pc(&mut self) -> u16 {
        let val =
            unsafe { std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const u16) };
        self.pc += 2;
        val
    }

    #[inline(always)]
    fn read_u32_pc(&mut self) -> u32 {
        let val =
            unsafe { std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const u32) };
        self.pc += 4;
        val
    }

    #[inline(always)]
    fn read_i32_pc(&mut self) -> i32 {
        let val =
            unsafe { std::ptr::read_unaligned(self.cached_code_ptr.add(self.pc) as *const i32) };
        self.pc += 4;
        val
    }

    #[inline(always)]
    fn with_env_object(
        &self,
        env: &JSValue,
        ctx: &mut JSContext,
    ) -> Option<JSValue> {
        if !env.is_object_like() {
            return None;
        }
        let atom = ctx.intern("\u{1}with_object");
        unsafe { JSValue::object_from_ptr(env.get_ptr()) }.get_own(atom)
    }

    fn with_env_parent(
        &self,
        env: &JSValue,
        ctx: &mut JSContext,
    ) -> Option<JSValue> {
        if !env.is_object_like() {
            return None;
        }
        let atom = ctx.intern("\u{1}with_parent");
        unsafe { JSValue::object_from_ptr(env.get_ptr()) }.get_own(atom)
    }

    fn with_is_unscopable(
        &mut self,
        ctx: &mut JSContext,
        object: JSValue,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
        scratch: u16,
    ) -> bool {
        let unscopables_atom = crate::builtins::symbol::get_symbol_unscopables_atom(ctx);
        let unscopables =
            self.get_named_prop_result(ctx, scratch, object, unscopables_atom, usize::MAX);
        if self.pending_throw.is_some() {
            return false;
        }
        if let Some(value) = unscopables {
            if value.is_object_like() {
                let blocked = self.get_named_prop_result(ctx, scratch, value, atom, usize::MAX);
                if self.pending_throw.is_some() {
                    return false;
                }
                return blocked.map_or(false, |v| v.is_truthy());
            }
        }
        false
    }

    fn with_find_object(
        &mut self,
        ctx: &mut JSContext,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
        scratch: u16,
    ) -> Option<JSValue> {
        let saved_scratch = if scratch == u16::MAX {
            Some((0u16, self.get_reg(0)))
        } else {
            None
        };
        let scratch = if scratch == u16::MAX { 0 } else { scratch };
        let object_atom = ctx.intern("\u{1}with_object");
        let parent_atom = ctx.intern("\u{1}with_parent");
        let mut env = self.frames[self.frame_index].with_env.clone();
        let mut result = None;
        while let Some(env_val) = env {
            if !env_val.is_object_like() {
                break;
            }
            let env_obj = unsafe { JSValue::object_from_ptr(env_val.get_ptr()) };
            let object = env_obj.get_own(object_atom);
            let parent = env_obj.get_own(parent_atom);
            if let Some(object_val) = object {
                if object_val.is_object_like() {
                    let has = unsafe { JSValue::object_from_ptr(object_val.get_ptr()) }
                        .has_property(atom);
                    if has
                        && !self.with_is_unscopable(ctx, object_val, atom, instr_pc, scratch)
                    {
                        if self.pending_throw.is_none() {
                            result = Some(object_val);
                        }
                        break;
                    }
                    if self.pending_throw.is_some() {
                        break;
                    }
                }
            }
            env = match parent {
                Some(p) if p.is_object_like() => Some(p),
                _ => None,
            };
        }
        if let Some((slot, value)) = saved_scratch {
            self.set_reg(slot, value);
        }
        result
    }

    fn with_upvalue_get(&self, slot: usize) -> JSValue {
        if !self.cached_upvalue_slot_ptr.is_null() && slot < self.cached_upvalues_len {
            let rc = unsafe { &*self.cached_upvalue_slot_ptr.add(slot) };
            unsafe { *(*std::rc::Rc::as_ptr(rc)).as_ptr() }
        } else {
            JSValue::undefined()
        }
    }

    fn with_upvalue_set(&mut self, slot: usize, val: JSValue) {
        if !self.cached_upvalue_slot_ptr.is_null() && slot < self.cached_upvalues_len {
            let rc = unsafe { &*self.cached_upvalue_slot_ptr.add(slot) };
            unsafe { (*std::rc::Rc::as_ptr(rc)).as_ptr().write(val) };
        }
    }

    fn with_get_var(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        atom: crate::runtime::atom::Atom,
        kind: u8,
        index: u16,
        flags: u8,
        instr_pc: usize,
    ) {
        if let Some(object) = self.with_find_object(ctx, atom, instr_pc, dst) {
            if self.pending_throw.is_some() {
                return;
            }
            let exists = unsafe { JSValue::object_from_ptr(object.get_ptr()) }
                .has_property(atom);
            if !exists && self.frames[self.frame_index].is_strict_frame {
                self.set_pending_reference_error(ctx, "Cannot access variable before initialization");
                return;
            }
            if exists {
                self.get_named_prop_result(ctx, dst, object, atom, instr_pc);
            } else {
                self.set_reg(dst, JSValue::undefined());
            }
            return;
        }
        if self.pending_throw.is_some() {
            return;
        }
        if kind == 0 {
            let global = ctx.global();
            let mut found = false;
            if global.is_object_like() {
                let global_obj = unsafe { JSValue::object_from_ptr(global.get_ptr()) };
                if global_obj.has_property(atom) {
                    self.get_named_prop_result(ctx, dst, global, atom, instr_pc);
                    found = true;
                }
            }
            if !found {
                if flags & 2 != 0 {
                    self.set_pending_reference_error(
                        ctx,
                        "Cannot access variable before initialization",
                    );
                } else {
                    self.set_reg(dst, JSValue::undefined());
                }
            }
            return;
        }
        let value = if kind == 2 {
            self.with_upvalue_get(index as usize)
        } else {
            self.get_reg(index)
        };
        if flags & 1 != 0 && value.is_tdz() {
            self.set_pending_reference_error(
                ctx,
                "Cannot access variable before initialization",
            );
            return;
        }
        self.set_reg(dst, value);
    }

    fn with_set_var(
        &mut self,
        ctx: &mut JSContext,
        atom: crate::runtime::atom::Atom,
        value: JSValue,
        kind: u8,
        index: u16,
        instr_pc: usize,
    ) {
        if let Some(object) = self.with_find_object(ctx, atom, instr_pc, u16::MAX) {
            if self.pending_throw.is_none() {
                self.set_named_prop(ctx, object, value, atom, usize::MAX);
            }
            return;
        }
        if self.pending_throw.is_some() {
            return;
        }
        if kind == 0 {
            let global = ctx.global();
            if global.is_object_like() {
                let global_obj = unsafe { JSValue::object_from_ptr(global.get_ptr()) };
                if self.frames[self.frame_index].is_strict_frame && !global_obj.has_property(atom)
                {
                    self.set_pending_reference_error(
                        ctx,
                        "Cannot assign to undeclared variable",
                    );
                    return;
                }
                self.set_named_prop(ctx, global, value, atom, usize::MAX);
            }
        } else if kind == 2 {
            self.with_upvalue_set(index as usize, value);
        } else {
            self.set_reg(index, value);
        }
    }

    fn get_named_prop_result(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
        ic_pc: usize,
    ) -> Option<JSValue> {
        self.get_named_prop_result_with_receiver(ctx, dst, obj_val, atom, ic_pc, None)
    }

    fn get_named_prop_result_with_receiver(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
        ic_pc: usize,
        receiver: Option<JSValue>,
    ) -> Option<JSValue> {
        let this_val = receiver.unwrap_or(obj_val);
        if obj_val.is_null_or_undefined() {
            self.set_pending_type_error(ctx, "Cannot read property of null or undefined");
            return None;
        }
        if !obj_val.is_object_like()
            && !obj_val.is_function()
            && (obj_val.is_int()
                || obj_val.is_float()
                || obj_val.is_string()
                || obj_val.is_bool()
                || obj_val.is_symbol()
                || obj_val.is_bigint())
        {
            if obj_val.is_string() && atom == ctx.common_atoms.length {
                let len = ctx.string_char_count(obj_val.get_atom()) as i64;
                let v = JSValue::new_int(len);
                self.set_reg(dst, v);
                return Some(v);
            }
            let start_proto = if obj_val.is_string() {
                ctx.get_string_prototype()
            } else if obj_val.is_int() || obj_val.is_float() {
                ctx.get_number_prototype()
            } else if obj_val.is_symbol() {
                ctx.get_symbol_prototype()
            } else if obj_val.is_bigint() {
                ctx.get_bigint_prototype()
            } else if obj_val.is_bool() {
                ctx.get_bool_prototype()
            } else {
                ctx.get_object_prototype()
            };
            let mut current = start_proto;
            while let Some(ptr) = current {
                let pobj = unsafe { &*ptr };
                if let Some(getter) = pobj.get_own_accessor_value(atom) {
                    if getter.is_function() {
                        let js_func = getter.as_function();
                        let fn_this = if !js_func.is_strict()
                            && (obj_val.is_int()
                                || obj_val.is_float()
                                || obj_val.is_string()
                                || obj_val.is_bool())
                        {
                            let mut wrapper = crate::object::object::JSObject::new();
                            if let Some(opp) = ctx.get_object_prototype() {
                                wrapper.set_prototype_raw(opp);
                            }
                            if obj_val.is_string() || obj_val.is_int() || obj_val.is_float() {
                                wrapper.set_cached(
                                    crate::runtime::atom::Atom(0),
                                    obj_val,
                                    ctx.shape_cache_mut(),
                                );
                            } else if obj_val.is_symbol() {
                                if let Some(sp) = ctx.get_symbol_prototype() {
                                    wrapper.set_prototype_raw(sp);
                                }
                                wrapper.set_cached(
                                    crate::runtime::atom::Atom(0),
                                    obj_val,
                                    ctx.shape_cache_mut(),
                                );
                            }
                            JSValue::new_object(Box::into_raw(Box::new(wrapper)) as usize)
                        } else {
                            obj_val
                        };
                        match self.call_function_with_this(ctx, getter, fn_this, &[]) {
                            Ok(v) => {
                                self.set_reg(dst, v);
                                return Some(v);
                            }
                            Err(_) => {
                                self.rethrow_last_caught(ctx);
                                return None;
                            }
                        }
                    }
                    self.set_reg(dst, JSValue::undefined());
                    return Some(JSValue::undefined());
                }
                if let Some(val) = pobj.get_own(atom) {
                    self.set_reg(dst, val);

                    let ic_table_ptr = if ic_pc == usize::MAX {
                std::ptr::null_mut()
            } else {
                self.cached_ic_table_ptr
            };
                    if !ic_table_ptr.is_null() {
                        let pc = if ic_pc == usize::MAX { 0 } else { ic_pc };
                        let pseudo_id = if obj_val.is_string() {
                            PRIM_STRING_SHAPE_ID
                        } else if obj_val.is_bool() {
                            PRIM_BOOL_SHAPE_ID
                        } else {
                            PRIM_NUMBER_SHAPE_ID
                        };
                        if let Some(offset) = pobj.find_offset(atom) {
                            unsafe {
                                (*ic_table_ptr).ensure_capacity(pc + 1);
                                if let Some(ic) = (*ic_table_ptr).get_mut(pc) {
                                    ic.insert(
                                        crate::object::shape::ShapeId(pseudo_id),
                                        offset as u32,
                                        Some(ptr as usize),
                                    );
                                }
                            }
                        }
                    }
                    return Some(val);
                }
                current = pobj.prototype;
            }
            self.set_reg(dst, JSValue::undefined());
            return Some(JSValue::undefined());
        }
        if obj_val.is_object_like() {
            let ptr = obj_val.get_ptr();
            let js_obj = unsafe { JSValue::object_from_ptr(ptr) };
            if js_obj.is_mapped_arguments() {
                if let Some(entry) = js_obj.argument_entry(atom) {
                    if entry.is_accessor() {
                        let getter = entry.accessor.get;
                        if let Some(getter) = getter {
                            if getter.is_function() {
                                match self
                                    .call_function_with_this(ctx, getter, this_val, &[])
                                {
                                    Ok(v) => {
                                        self.set_reg(dst, v);
                                        return Some(v);
                                    }
                                    Err(_) => {
                                        self.rethrow_last_caught(ctx);
                                        return None;
                                    }
                                }
                            }
                        }
                        self.set_reg(dst, JSValue::undefined());
                        return Some(JSValue::undefined());
                    }
                    let v = entry.current_value();
                    self.set_reg(dst, v);
                    return Some(v);
                }
            }
            if js_obj.is_dense_array() {
                if let Some(index) = ctx.atom_table().get_array_index(atom) {
                    let array = unsafe {
                        &*(ptr as *const crate::object::array_obj::JSArrayObject)
                    };
                    if let Some(v) = array.elements.get(index).copied() {
                        self.set_reg(dst, v);
                        return Some(v);
                    }
                }
            }
            let ic_table_ptr = if ic_pc == usize::MAX {
                std::ptr::null_mut()
            } else {
                self.cached_ic_table_ptr
            };

            let pc = if ic_pc == usize::MAX { 0 } else { ic_pc };
            let mut ic_hit = false;
            if let Some(shape_id) = js_obj.get_shape_id() {
                if !ic_table_ptr.is_null() {
                    let ic_table = unsafe { &*ic_table_ptr };
                    if let Some(ic) = ic_table.get(pc) {
                        if let Some((offset, proto_ptr)) = ic.get(shape_id) {
                            if offset == u32::MAX && proto_ptr.is_none() {
                                self.set_reg(dst, JSValue::undefined());
                                ic_hit = true;
                            } else {
                                let val = if let Some(proto) = proto_ptr {
                                    let mut proto_matches = js_obj
                                        .prototype
                                        .map(|p| !p.is_null() && p as usize == proto)
                                        .unwrap_or(false);
                                    if !proto_matches {
                                        let mut cur = js_obj.prototype;
                                        let mut depth = 0u32;
                                        while let Some(p) = cur {
                                            if p.is_null() || depth > 1000 {
                                                break;
                                            }
                                            if p as usize == proto {
                                                proto_matches = true;
                                                break;
                                            }
                                            depth += 1;
                                            unsafe {
                                                cur = (*p).prototype;
                                            }
                                        }
                                    }
                                    if proto_matches {
                                        let proto_obj = unsafe {
                                            &*(proto as *const crate::object::object::JSObject)
                                        };
                                        proto_obj.get_by_offset(offset as usize)
                                    } else {
                                        None
                                    }
                                } else {
                                    js_obj.get_by_offset(offset as usize)
                                };
                                if let Some(v) = val {
                                    self.set_reg(dst, v);
                                    ic_hit = true;
                                }
                            }
                        }
                    }
                }
            }

            if !ic_hit {
                if let Some(shape_id) = js_obj.get_shape_id() {
                    if let Some(offset) = js_obj.find_offset(atom) {
                        let v = js_obj.get_by_offset(offset).unwrap_or(JSValue::undefined());
                        self.set_reg(dst, v);
                        if !ic_table_ptr.is_null() {
                            unsafe {
                                (*ic_table_ptr).ensure_capacity(pc + 1);
                                if let Some(ic) = (*ic_table_ptr).get_mut(pc) {
                                    ic.insert(shape_id, offset as u32, None);
                                }
                            }
                        }
                    } else {
                        if let Some(getter) = js_obj.get_own_accessor_value(atom) {
                            if getter.is_function() {
                                match self.call_function_with_this(ctx, getter, this_val, &[]) {
                                    Ok(ret) => {
                                        self.set_reg(dst, ret);
                                    }
                                    Err(msg) => {
                                        self.rethrow_last_caught(ctx);
                                        return None;
                                    }
                                }
                            } else if getter.is_undefined() {
                                self.set_reg(dst, JSValue::undefined());
                            } else {
                                self.set_pending_type_error(
                                    ctx,
                                    "Property getter is not a function",
                                );
                            }
                        } else {
                            let mut current = js_obj.prototype;
                            let mut depth = 0u32;
                            let mut found = false;
                            while let Some(proto_ptr) = current {
                                if proto_ptr.is_null() || depth > 1000 {
                                    break;
                                }
                                depth += 1;
                                unsafe {
                                    let proto = &*proto_ptr;
                                    if let Some(offset) = proto.find_offset(atom) {
                                        let v = proto
                                            .get_by_offset(offset)
                                            .unwrap_or(JSValue::undefined());
                                        self.set_reg(dst, v);
                                        if !ic_table_ptr.is_null() {
                                            (*ic_table_ptr).ensure_capacity(pc + 1);
                                            if let Some(ic) = (*ic_table_ptr).get_mut(pc) {
                                                ic.insert(
                                                    shape_id,
                                                    offset as u32,
                                                    Some(proto_ptr as usize),
                                                );
                                            }
                                        }
                                        found = true;
                                        break;
                                    }
                                    if let Some(getter) = proto.get_own_accessor_value(atom) {
                                        if getter.is_function() {
                                            match self.call_function_with_this(
                                                ctx,
                                                getter,
                                                this_val,
                                                &[],
                                            ) {
                                                Ok(ret) => {
                                                    self.set_reg(dst, ret);
                                                }
                                                Err(_msg) => {
                                                    self.rethrow_last_caught(ctx);
                                                    return None;
                                                }
                                            }
                                        } else if getter.is_undefined() {
                                            self.set_reg(dst, JSValue::undefined());
                                        } else {
                                            self.set_pending_type_error(
                                                ctx,
                                                "Property getter is not a function",
                                            );
                                        }
                                        found = true;
                                        break;
                                    }
                                    current = proto.prototype;
                                }
                            }
                            if !found && obj_val.is_function() && js_obj.prototype.is_none() {
                                if let Some(fn_proto_ptr) = ctx.get_function_prototype() {
                                    let fn_proto = unsafe { &*fn_proto_ptr };
                                    if let Some(offset) = fn_proto.find_offset(atom) {
                                        let v = fn_proto
                                            .get_by_offset(offset)
                                            .unwrap_or(JSValue::undefined());
                                        self.set_reg(dst, v);
                                        found = true;
                                    } else if let Some(getter) =
                                        fn_proto.get_own_accessor_value(atom)
                                    {
                                        if getter.is_function() {
                                            match self.call_function_with_this(
                                                ctx,
                                                getter,
                                                this_val,
                                                &[],
                                            ) {
                                                Ok(ret) => {
                                                    self.set_reg(dst, ret);
                                                }
                                                Err(msg) => {
                                                    let mut err =
                                                        crate::object::object::JSObject::new();
                                                    err.set(
                                                        ctx.intern("name"),
                                                        JSValue::new_string(
                                                            ctx.intern("ReferenceError"),
                                                        ),
                                                    );
                                                    err.set(
                                                        ctx.intern("message"),
                                                        JSValue::new_string(ctx.intern(&msg)),
                                                    );
                                                    if let Some(proto) =
                                                        ctx.get_reference_error_prototype()
                                                    {
                                                        err.prototype = Some(proto);
                                                    }
                                                    let ptr = Box::into_raw(Box::new(err)) as usize;
                                                    ctx.runtime_mut().gc_heap_mut().track(ptr);
                                                    self.pending_throw =
                                                        Some(JSValue::new_object(ptr));
                                                    return None;
                                                }
                                            }
                                        } else if getter.is_undefined() {
                                            self.set_reg(dst, JSValue::undefined());
                                        } else {
                                            self.set_pending_type_error(
                                                ctx,
                                                "Property getter is not a function",
                                            );
                                        }
                                        found = true;
                                    }
                                }
                            }
                            if !found {
                                self.set_reg(dst, JSValue::undefined());
                                if !ic_table_ptr.is_null() {
                                    unsafe {
                                        (*ic_table_ptr).ensure_capacity(pc + 1);
                                        if let Some(ic) = (*ic_table_ptr).get_mut(pc) {
                                            ic.insert(shape_id, u32::MAX, None);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    let mut value = JSValue::undefined();
                    let mut found = false;

                    if let Some(offset) = js_obj.find_offset(atom) {
                        value = js_obj.get_by_offset(offset).unwrap_or(JSValue::undefined());
                        found = true;
                    } else if let Some(getter) = js_obj.get_own_accessor_value(atom) {
                        value = if getter.is_function() {
                            match self.call_function_with_this(ctx, getter, this_val, &[]) {
                                Ok(ret) => ret,
                                Err(_) => {
                                    self.rethrow_last_caught(ctx);
                                    return None;
                                }
                            }
                        } else if getter.is_undefined() {
                            JSValue::undefined()
                        } else {
                            self.set_pending_type_error(ctx, "Property getter is not a function");
                            JSValue::undefined()
                        };
                        found = true;
                    } else {
                        let mut current = js_obj.prototype;
                        let mut depth = 0u32;
                        while let Some(proto_ptr) = current {
                            if proto_ptr.is_null() || depth > 1000 {
                                break;
                            }
                            depth += 1;
                            unsafe {
                                let proto = &*proto_ptr;
                                if let Some(offset) = proto.find_offset(atom) {
                                    value =
                                        proto.get_by_offset(offset).unwrap_or(JSValue::undefined());
                                    found = true;
                                    break;
                                }
                                if let Some(getter) = proto.get_own_accessor_value(atom) {
                                    value = if getter.is_function() {
                                        match self.call_function_with_this(
                                            ctx,
                                            getter,
                                            this_val,
                                            &[],
                                        ) {
                                            Ok(ret) => ret,
                                            Err(_) => {
                                                self.rethrow_last_caught(ctx);
                                                return None;
                                            }
                                        }
                                    } else if getter.is_undefined() {
                                        JSValue::undefined()
                                    } else {
                                        self.set_pending_type_error(
                                            ctx,
                                            "Property getter is not a function",
                                        );
                                        JSValue::undefined()
                                    };
                                    found = true;
                                    break;
                                }
                                current = proto.prototype;
                            }
                        }
                    }

                    if !found && obj_val.is_function() && js_obj.prototype.is_none() {
                        if let Some(fn_proto_ptr) = ctx.get_function_prototype() {
                            let fn_proto = unsafe { &*fn_proto_ptr };
                            if let Some(offset) = fn_proto.find_offset(atom) {
                                value = fn_proto
                                    .get_by_offset(offset)
                                    .unwrap_or(JSValue::undefined());
                                found = true;
                            } else if let Some(getter) = fn_proto.get_own_accessor_value(atom) {
                                value = if getter.is_function() {
                                    match self.call_function_with_this(ctx, getter, this_val, &[]) {
                                        Ok(ret) => ret,
                                        Err(_) => {
                                            self.rethrow_last_caught(ctx);
                                            return None;
                                        }
                                    }
                                } else if getter.is_undefined() {
                                    JSValue::undefined()
                                } else {
                                    self.set_pending_type_error(
                                        ctx,
                                        "Property getter is not a function",
                                    );
                                    JSValue::undefined()
                                };
                                found = true;
                            }
                        }
                    }

                    if found {
                        self.set_reg(dst, value);
                    } else {
                        self.set_reg(dst, JSValue::undefined());
                    }
                }
            }

            Some(self.get_reg(dst))
        } else if obj_val.is_string() {
            if atom == ctx.common_atoms.length {
                Some(JSValue::new_int(
                    ctx.string_char_count(obj_val.get_atom()) as i64
                ))
            } else if let Some(proto_ptr) = ctx.get_string_prototype() {
                let proto_obj = unsafe { &*proto_ptr };
                Some(proto_obj.get(atom).unwrap_or(JSValue::undefined()))
            } else {
                Some(JSValue::undefined())
            }
        } else if obj_val.is_int() || obj_val.is_float() {
            if let Some(proto_ptr) = ctx.get_number_prototype() {
                let proto_obj = unsafe { &*proto_ptr };
                Some(proto_obj.get(atom).unwrap_or(JSValue::undefined()))
            } else {
                Some(JSValue::undefined())
            }
        } else if obj_val.is_bool() {
            if let Some(proto_ptr) = ctx.get_bool_prototype() {
                let proto_obj = unsafe { &*proto_ptr };
                Some(proto_obj.get(atom).unwrap_or(JSValue::undefined()))
            } else {
                Some(JSValue::undefined())
            }
        } else if obj_val.is_symbol() {
            let prop_str = ctx.get_atom_str(atom);
            if prop_str == "description" {
                let desc_atom = obj_val.get_atom();
                if desc_atom.0 == ctx.common_atoms.empty.0 {
                    Some(JSValue::undefined())
                } else {
                    Some(JSValue::new_string(desc_atom))
                }
            } else if let Some(proto_ptr) = ctx.get_symbol_prototype() {
                let proto = unsafe { &*proto_ptr };
                Some(proto.get(atom).unwrap_or(JSValue::undefined()))
            } else {
                Some(JSValue::undefined())
            }
        } else {
            Some(JSValue::undefined())
        }
    }

    #[inline(always)]
    fn get_named_prop_fast(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_reg: u16,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
    ) -> bool {
        let obj_val = self.get_reg(obj_reg);

        if obj_val.is_object_like() {
            let js_obj = unsafe { crate::value::JSValue::object_from_ptr(obj_val.get_ptr()) };

            if js_obj.is_dense_array() && atom == ctx.common_atoms.length {
                if js_obj.props_len() > 0 {
                    self.set_reg(dst, js_obj.get_by_offset_fast(0));
                } else {
                    self.set_reg(dst, crate::value::JSValue::new_int(0));
                }
                return true;
            }
            let shape_id = js_obj.shape_id_cache;
            if shape_id != usize::MAX {
                let ic_table_ptr = self.cached_ic_table_ptr;
                if !ic_table_ptr.is_null() {
                    let (ic_hit, r0_offset, r0_proto) =
                        unsafe { (*ic_table_ptr).get_reads0_values(instr_pc, shape_id) };
                    if ic_hit {
                        if r0_proto == 0 {
                            let val = if r0_offset == u32::MAX {
                                crate::value::JSValue::undefined()
                            } else {
                                let off = r0_offset as usize;

                                if off < crate::object::object::INLINE_PROPS
                                    && js_obj.has_no_deleted_props()
                                {
                                    js_obj.get_by_offset_fast(off)
                                } else if let Some(v) = js_obj.get_by_offset(off) {
                                    v
                                } else {
                                    return self
                                        .get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc);
                                }
                            };
                            self.set_reg(dst, val);
                            return true;
                        } else {
                            if let Some(proto_raw) = js_obj.prototype {
                                if proto_raw as usize == r0_proto {
                                    let proto_obj = unsafe { &*proto_raw };
                                    let off = r0_offset as usize;
                                    let v = if off < crate::object::object::INLINE_PROPS
                                        && proto_obj.has_no_deleted_props()
                                    {
                                        Some(proto_obj.get_by_offset_fast(off))
                                    } else {
                                        proto_obj.get_by_offset(off)
                                    };
                                    if let Some(v) = v {
                                        self.set_reg(dst, v);
                                        return true;
                                    }
                                    return self
                                        .get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc);
                                }
                            }
                            return self.get_inherited_fast(
                                ctx, dst, obj_val, atom, instr_pc, r0_offset, r0_proto,
                            );
                        }
                    } else {
                        return self
                            .get_named_prop_poly_hit(ctx, dst, obj_val, atom, instr_pc, shape_id);
                    }
                }
            }
        }
        self.get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc)
    }

    #[inline(never)]
    fn get_named_prop_poly_hit(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
        shape_id: usize,
    ) -> bool {
        let ic_table_ptr = self.cached_ic_table_ptr;
        if !ic_table_ptr.is_null() {
            let (ic_hit, r_offset, r_proto) =
                unsafe { (*ic_table_ptr).get_reads123_values(instr_pc, shape_id) };
            if ic_hit {
                let js_obj = unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };
                if r_proto == 0 {
                    let val = if r_offset == u32::MAX {
                        JSValue::undefined()
                    } else {
                        let off = r_offset as usize;
                        if off < crate::object::object::INLINE_PROPS
                            && js_obj.has_no_deleted_props()
                        {
                            js_obj.get_by_offset_fast(off)
                        } else if let Some(v) = js_obj.get_by_offset(off) {
                            v
                        } else {
                            return self.get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc);
                        }
                    };
                    self.set_reg(dst, val);
                    return true;
                } else {
                    if let Some(p) = js_obj.prototype {
                        if p as usize == r_proto {
                            let proto_obj = unsafe { &*p };
                            let off = r_offset as usize;
                            let v = if off < crate::object::object::INLINE_PROPS
                                && proto_obj.has_no_deleted_props()
                            {
                                Some(proto_obj.get_by_offset_fast(off))
                            } else {
                                proto_obj.get_by_offset(off)
                            };
                            if let Some(v) = v {
                                self.set_reg(dst, v);
                                return true;
                            }
                        }
                    }

                    return self
                        .get_inherited_fast(ctx, dst, obj_val, atom, instr_pc, r_offset, r_proto);
                }
            }
        }
        self.get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc)
    }

    #[inline(never)]
    fn get_inherited_fast(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_val: crate::value::JSValue,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
        offset: u32,
        proto_ptr: usize,
    ) -> bool {
        let js_obj = unsafe { crate::value::JSValue::object_from_ptr(obj_val.get_ptr()) };
        let mut cur = js_obj.prototype;
        let mut depth = 0u32;
        while let Some(p) = cur {
            if depth > 1000 {
                break;
            }
            if p as usize == proto_ptr {
                let proto_obj = unsafe { &*p };
                if let Some(v) = proto_obj.get_by_offset(offset as usize) {
                    self.set_reg(dst, v);
                    return true;
                }

                break;
            }
            depth += 1;
            cur = unsafe { (*p).prototype };
        }

        self.get_named_prop_slow(ctx, dst, obj_val, atom, instr_pc)
    }

    #[inline(never)]
    fn get_named_prop_slow(
        &mut self,
        ctx: &mut JSContext,
        dst: u16,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
    ) -> bool {
        if (obj_val.is_string() && atom != ctx.common_atoms.length)
            || obj_val.is_int()
            || obj_val.is_float()
            || obj_val.is_bool()
        {
            let pseudo_id = if obj_val.is_string() {
                PRIM_STRING_SHAPE_ID
            } else if obj_val.is_bool() {
                PRIM_BOOL_SHAPE_ID
            } else {
                PRIM_NUMBER_SHAPE_ID
            };
            let ic_table_ptr = self.cached_ic_table_ptr;
            if !ic_table_ptr.is_null() {
                let (ic_hit, r0_offset, r0_proto) =
                    unsafe { (*ic_table_ptr).get_reads0_values(instr_pc, pseudo_id) };
                if ic_hit {
                    if r0_proto != 0 {
                        let proto_obj =
                            unsafe { &*(r0_proto as *const crate::object::object::JSObject) };
                        if let Some(v) = proto_obj.get_by_offset(r0_offset as usize) {
                            self.set_reg(dst, v);
                            return true;
                        }
                    } else if r0_offset == u32::MAX {
                        self.set_reg(dst, JSValue::undefined());
                        return true;
                    }
                }
            }
        }
        if let Some(result) = self.get_named_prop_result(ctx, dst, obj_val, atom, instr_pc) {
            self.set_reg(dst, result);
            return true;
        }
        false
    }

    #[inline(always)]
    fn to_property_key_value(&mut self, ctx: &mut JSContext, value: JSValue) -> JSValue {
        if value.is_string() || value.is_symbol() || value.is_int() || value.is_float() {
            return value;
        }
        if value.is_object() || value.is_function() {
            let prim = self.ordinary_to_primitive(&value, "string", ctx);
            if self.pending_throw.is_some() || ctx.pending_exception.is_some() {
                return JSValue::undefined();
            }
            if prim.is_symbol() {
                return prim;
            }
            let s = VM::js_to_string(&prim, ctx);
            if s.is_string() {
                return s;
            }
            let text = crate::builtins::global::to_property_key_string(ctx, &s);
            return JSValue::new_string(ctx.intern(&text));
        }
        let s = crate::builtins::global::to_property_key_string(ctx, &value);
        JSValue::new_string(ctx.intern(&s))
    }

    fn canonical_property_key_value(
        &mut self,
        ctx: &mut JSContext,
        value: JSValue,
    ) -> Option<JSValue> {
        let key = self.to_property_key_value(ctx, value);
        if self.pending_throw.is_some() || ctx.pending_exception.is_some() {
            return None;
        }
        if key.is_string() || key.is_symbol() {
            Some(key)
        } else if key.is_int() {
            let atom = self.int_atom(key.get_int() as usize, ctx);
            Some(JSValue::new_string(atom))
        } else if key.is_float() {
            let s = VM::js_to_string(&key, ctx);
            Some(JSValue::new_string(s.get_atom()))
        } else {
            let s = VM::js_to_string(&key, ctx);
            Some(JSValue::new_string(s.get_atom()))
        }
    }

    fn set_named_prop_fast(
        &mut self,
        ctx: &mut JSContext,
        obj_reg: u16,
        val_reg: u16,
        atom: crate::runtime::atom::Atom,
        instr_pc: usize,
    ) -> bool {
        let obj_val = self.get_reg(obj_reg);
        let value = self.get_reg(val_reg);
        self.set_named_prop(ctx, obj_val, value, atom, instr_pc)
    }

    #[inline(always)]
    fn proto_chain_has_accessors(
        &self,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
    ) -> (bool, Option<JSValue>) {
        if obj_val.is_object_like() {
            let js_obj = unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };
            if let Some(entry) = js_obj.get_own_accessor_entry(atom) {
                return (true, entry.set);
            }
            let mut proto = js_obj.prototype;
            while let Some(proto_ptr) = proto {
                if proto_ptr.is_null() {
                    break;
                }
                let proto_obj = unsafe { &*proto_ptr };
                if let Some(entry) = proto_obj.get_own_accessor_entry(atom) {
                    return (true, entry.set);
                }
                proto = proto_obj.prototype;
            }
        }
        (false, None)
    }

    fn canonical_numeric_index_string(name: &str) -> Option<f64> {
        if name == "-0" {
            return Some(-0.0);
        }
        if name == "NaN" {
            return Some(f64::NAN);
        }
        if name == "Infinity" {
            return Some(f64::INFINITY);
        }
        if name == "-Infinity" {
            return Some(f64::NEG_INFINITY);
        }
        let value: f64 = name.parse().ok()?;
        if value.fract() == 0.0 && value.abs() < 1e21 && format!("{}", value) == name {
            Some(value)
        } else if format!("{}", value) == name {
            Some(value)
        } else {
            None
        }
    }

    fn prototype_rejects_typed_array_set(
        &mut self,
        ctx: &mut JSContext,
        obj_val: JSValue,
        atom: crate::runtime::atom::Atom,
    ) -> bool {
        let js_obj = unsafe { JSValue::object_from_ptr(obj_val.get_ptr()) };
        let mut current = js_obj.prototype;
        while let Some(proto_ptr) = current {
            if proto_ptr.is_null() {
                return false;
            }
            let proto = unsafe { &*proto_ptr };
            if proto.obj_type() == crate::object::object::ObjectType::TypedArray {
                let length_atom = ctx.intern("length");
                let name = ctx.get_atom_str(atom);
                let Some(index) = Self::canonical_numeric_index_string(&name) else {
                    return false;
                };
                let length = proto
                    .get(length_atom)
                    .and_then(|v| if v.is_int() { Some(v.get_int()) } else { None })
                    .unwrap_or(0);
                let valid = index.is_finite()
                    && index.fract() == 0.0
                    && !(index == 0.0 && name == "-0")
                    && index >= 0.0
                    && index < length as f64;
                return !valid;
            }
            current = proto.prototype;
        }
        false
    }

    #[inline(always)]
    fn super_key_atom(
        &mut self,
        ctx: &mut JSContext,
        key_val: JSValue,
    ) -> crate::runtime::atom::Atom {
        if key_val.is_string() {
            key_val.get_atom()
        } else if key_val.is_symbol() {
            crate::runtime::atom::Atom(0x40000000 | key_val.get_symbol_id())
        } else if key_val.is_int() {
            let idx = key_val.get_int();
            if idx >= 0 {
                self.int_atom(idx as usize, ctx)
            } else {
                let s = VM::js_to_string(&key_val, ctx);
                s.get_atom()
            }
        } else if key_val.is_float() {
            let s = VM::js_to_string(&key_val, ctx);
            s.get_atom()
        } else {
            let key = self.to_property_key_value(ctx, key_val);
            key.get_atom()
        }
    }

    #[inline(always)]
    fn set_super_prop(
        &mut self,
        ctx: &mut JSContext,
        base: JSValue,
        receiver: JSValue,
        key_val: JSValue,
        value: JSValue,
    ) -> bool {
        if base.is_null_or_undefined() {
            self.set_pending_type_error(ctx, "Cannot set property of null or undefined");
            return false;
        }
        let atom = self.super_key_atom(ctx, key_val);
        if self.pending_throw.is_some() || ctx.pending_exception.is_some() {
            return false;
        }
        let mut found: Option<(bool, Option<JSValue>, bool)> = None;
        if base.is_object_like() {
            let mut current = base;
            let mut depth = 0u32;
            while current.is_object_like() {
                let obj = current.as_object();
                if let Some(entry) = obj.get_own_accessor_entry(atom) {
                    found = Some((true, entry.set, false));
                    break;
                }
                if let Some(offset) = obj.find_offset(atom) {
                    let writable = obj.is_prop_writable_at(Some(offset));
                    found = Some((false, None, writable));
                    break;
                }
                match obj.prototype {
                    Some(p) if !p.is_null() => {
                        current = JSValue::new_object(p as usize);
                    }
                    _ => break,
                }
                depth += 1;
                if depth > 1000 {
                    break;
                }
            }
        }
        match found {
            Some((true, setter, _)) => {
                if let Some(setter_fn) = setter {
                    if setter_fn.is_function()
                        && self
                            .call_function_with_this(ctx, setter_fn, receiver, &[value])
                            .is_err()
                    {
                        let caught = self.last_caught_exception.take();
                        let host = ctx.pending_exception.take();
                        let exc = caught.or(host).or_else(|| self.pending_throw.take());
                        if let Some(exc) = exc {
                            self.pending_throw = Some(exc);
                        }
                    }
                    return true;
                }
                if self.frames[self.frame_index].is_strict_frame {
                    self.set_pending_type_error(
                        ctx,
                        "Cannot set property which has only a getter",
                    );
                }
                false
            }
            Some((false, _, writable)) => {
                if !writable {
                    if self.frames[self.frame_index].is_strict_frame {
                        let msg = format!(
                            "Cannot assign to read only property '{}'",
                            ctx.get_atom_str(atom)
                        );
                        self.set_pending_type_error(ctx, &msg);
                    }
                    return false;
                }
                if !receiver.is_object_like() {
                    return false;
                }
                let recv_obj = receiver.as_object();
                if recv_obj.get_own_accessor_entry(atom).is_some() {
                    if self.frames[self.frame_index].is_strict_frame {
                        self.set_pending_type_error(
                            ctx,
                            "Cannot set property which has only a getter",
                        );
                    }
                    return false;
                }
                if let Some(offset) = recv_obj.find_offset(atom) {
                    if !recv_obj.is_prop_writable_at(Some(offset)) {
                        if self.frames[self.frame_index].is_strict_frame {
                            let msg = format!(
                                "Cannot assign to read only property '{}'",
                                ctx.get_atom_str(atom)
                            );
                            self.set_pending_type_error(ctx, &msg);
                        }
                        return false;
                    }
                }
                self.set_named_prop(ctx, receiver, value, atom, usize::MAX);
                true
            }
            None => {
                if !receiver.is_object_like() {
                    return false;
                }
                self.set_named_prop(ctx, receiver, value, atom, usize::MAX);
                true
            }
        }
    }

    #[inline(always)]
    fn set_named_prop(
        &mut self,
        ctx: &mut JSContext,
        obj_val: JSValue,
        value: JSValue,
        atom: crate::runtime::atom::Atom,
        ic_pc: usize,
    ) -> bool {
        if obj_val.is_null_or_undefined() {
            self.set_pending_type_error(ctx, "Cannot set property of null or undefined");
            return false;
        }
        if !obj_val.is_object_like() {
            if self.frames[self.frame_index].is_strict_frame {
                self.set_pending_type_error(ctx, "Cannot assign to a primitive value");
            }
            return false;
        }
        if obj_val.is_object_like() {
            let ptr = obj_val.get_ptr();
            let js_obj = unsafe { JSValue::object_from_ptr_mut(ptr) };
            if js_obj.is_mapped_arguments() {
                let entry_state = js_obj.argument_entry(atom).map(|entry| {
                    (
                        entry.is_accessor(),
                        entry.accessor.set,
                        entry.writable(),
                    )
                });
                if let Some((is_accessor, set, writable)) = entry_state {
                    if is_accessor {
                        if let Some(setter_fn) = set {
                            if self
                                .call_function_with_this(ctx, setter_fn, obj_val, &[value])
                                .is_err()
                            {
                                let caught = self.last_caught_exception.take();
                                let host = ctx.pending_exception.take();
                                let exc =
                                    caught.or(host).or_else(|| self.pending_throw.take());
                                if let Some(exc) = exc {
                                    self.pending_throw = Some(exc);
                                }
                            }
                        } else if self.frames[self.frame_index].is_strict_frame {
                            let msg = format!(
                                "Cannot set property '{}' which has only a getter",
                                ctx.get_atom_str(atom)
                            );
                            self.set_pending_type_error(ctx, &msg);
                        }
                        return true;
                    }
                    if !writable {
                        if self.frames[self.frame_index].is_strict_frame {
                            let msg = format!(
                                "Cannot assign to read only property '{}'",
                                ctx.get_atom_str(atom)
                            );
                            self.set_pending_type_error(ctx, &msg);
                        }
                        return false;
                    }
                    if let Some(entry) = js_obj.argument_entry_mut(atom) {
                        match &entry.cell {
                            Some(cell) => cell.set(value),
                            None => entry.value = value,
                        }
                    }
                    return true;
                }
            }
            let ic_table_ptr = self.cached_ic_table_ptr;

            // For shapeless objects with no properties yet, treat as root shape for IC
            // (all new objects start at root shape before any property is assigned)
            let effective_shape_id = js_obj.get_shape_id().or_else(|| {
                if js_obj.props_len() == 0 {
                    Some(crate::object::shape::ShapeId::root())
                } else {
                    None
                }
            });
            if let Some(shape_id) = effective_shape_id {
                if !ic_table_ptr.is_null() {
                    let ic_table = unsafe { &*ic_table_ptr };
                    if let Some(ic) = ic_table.get(ic_pc) {
                        if let Some((offset, new_shape_ptr)) = ic.get_transition(shape_id) {
                            if new_shape_ptr.is_null() {
                                let off = offset as usize;
                                if off < crate::object::object::INLINE_PROPS
                                    && js_obj.has_no_deleted_props()
                                {
                                    js_obj.set_by_offset_fast(off, value);
                                } else {
                                    js_obj.set_by_offset(off, value);
                                }
                            } else {
                                let new_shape = unsafe {
                                    std::ptr::NonNull::new_unchecked(
                                        new_shape_ptr as *mut crate::object::shape::Shape,
                                    )
                                };
                                js_obj.push_prop_with_shape(
                                    offset as usize,
                                    atom,
                                    value,
                                    new_shape,
                                );
                            }
                            return true;
                        }
                    }
                }
            }

            let pre_offset = js_obj.find_offset(atom);

            if !js_obj.is_prop_writable_at(pre_offset) {
                if self.frames[self.frame_index].is_strict_frame {
                    let msg = format!(
                        "Cannot assign to read only property '{}'",
                        ctx.get_atom_str(atom)
                    );
                    self.set_pending_type_error(ctx, &msg);
                }
                return false;
            }

            let (has_accessor, setter) = if !ic_table_ptr.is_null() {
                let ic_table = unsafe { &*ic_table_ptr };
                if let Some(shape_id) = js_obj.get_shape_id() {
                    if ic_table.get_write_no_accessor(ic_pc, shape_id) {
                        (false, None)
                    } else {
                        let r = self.proto_chain_has_accessors(obj_val, atom);
                        if !r.0 {
                            unsafe {
                                (*ic_table_ptr).set_write_no_accessor(ic_pc);
                            }
                        }
                        r
                    }
                } else {
                    self.proto_chain_has_accessors(obj_val, atom)
                }
            } else {
                self.proto_chain_has_accessors(obj_val, atom)
            };
            if has_accessor {
                if let Some(setter_fn) = setter {
                    if self
                        .call_function_with_this(ctx, setter_fn, obj_val, &[value])
                        .is_err()
                    {
                        let caught = self.last_caught_exception.take();
                        let host = ctx.pending_exception.take();
                        let exc = caught
                            .or(host)
                            .or_else(|| self.pending_throw.take());
                        if let Some(exc) = exc {
                            self.pending_throw = Some(exc);
                        }
                    }
                } else if self.frames[self.frame_index].is_strict_frame {
                    let msg = format!(
                        "Cannot set property '{}' which has only a getter",
                        ctx.get_atom_str(atom)
                    );
                    self.set_pending_type_error(ctx, &msg);
                }
                return true;
            }

            if pre_offset.is_none() {
                let mut proto = js_obj.prototype;
                while let Some(proto_ptr) = proto {
                    if proto_ptr.is_null() {
                        break;
                    }
                    let proto_obj = unsafe { &*proto_ptr };
                    if let Some(desc) = proto_obj.get_own_descriptor(atom) {
                        if desc.get.is_none() && desc.set.is_none() && !desc.writable {
                            if self.frames[self.frame_index].is_strict_frame {
                                let msg = format!(
                                    "Cannot assign to read only property '{}'",
                                    ctx.get_atom_str(atom)
                                );
                                self.set_pending_type_error(ctx, &msg);
                            }
                            return false;
                        }
                        break;
                    }
                    proto = proto_obj.prototype;
                }
            }

            if pre_offset.is_none() && !js_obj.extensible() {
                if self.frames[self.frame_index].is_strict_frame {
                    let msg = format!(
                        "Cannot define property '{}', object is not extensible",
                        ctx.get_atom_str(atom)
                    );
                    self.set_pending_type_error(ctx, &msg);
                }
                return false;
            }

            if pre_offset.is_none()
                && self.prototype_rejects_typed_array_set(ctx, obj_val, atom)
            {
                if self.frames[self.frame_index].is_strict_frame {
                    self.set_pending_type_error(
                        ctx,
                        "Cannot create property on typed array prototype",
                    );
                }
                return false;
            }

            self.set_named_prop_slow(ctx, obj_val, ptr, js_obj, value, atom, ic_pc);
        }
        false
    }

    #[cold]
    fn set_named_prop_slow(
        &mut self,
        ctx: &mut JSContext,
        obj_val: JSValue,
        ptr: usize,
        js_obj: &mut crate::object::object::JSObject,
        value: JSValue,
        atom: crate::runtime::atom::Atom,
        ic_pc: usize,
    ) {
        let ic_table_ptr = self.cached_ic_table_ptr;

        let pre_shape_id = js_obj.get_shape_id();
        let pre_props_len = js_obj.props_len();
        let pre_offset = js_obj.find_offset(atom);
        js_obj.set_cached_with_offset(atom, value, ctx.shape_cache_mut(), pre_offset);
        if let Some(shape_id) = js_obj.get_shape_id() {
            let offset = pre_offset.or_else(|| js_obj.find_offset(atom));
            if let Some(offset) = offset {
                if !ic_table_ptr.is_null() && ic_pc != usize::MAX {
                    unsafe {
                        (*ic_table_ptr).ensure_capacity((ic_pc) + 1);
                        if let Some(ic) = (*ic_table_ptr).get_mut(ic_pc) {
                            let was_transition = offset == pre_props_len;
                            if was_transition {
                                if let Some(pre_id) = pre_shape_id {
                                    if let Some(new_shape_ptr) = js_obj.get_shape_ptr() {
                                        ic.insert_transition(pre_id, offset as u32, new_shape_ptr);
                                    } else {
                                        ic.insert(shape_id, offset as u32, None);
                                    }
                                } else if pre_props_len == 0 {
                                    // Object was shapeless with no props (at root state)
                                    // Record transition from root shape so future IC lookups hit
                                    if let Some(new_shape_ptr) = js_obj.get_shape_ptr() {
                                        ic.insert_transition(
                                            crate::object::shape::ShapeId::root(),
                                            offset as u32,
                                            new_shape_ptr,
                                        );
                                    }
                                } else {
                                    ic.insert(shape_id, offset as u32, None);
                                }
                            } else {
                                ic.insert_transition_null(shape_id, offset as u32);
                            }
                        }
                    }
                }
            }
        }

        if obj_val.is_function() && atom == ctx.common_atoms.prototype {
            let js_func = unsafe { JSValue::function_from_ptr_mut(ptr) };
            if value.is_object() {
                js_func.cached_prototype_ptr =
                    value.get_ptr() as *mut crate::object::object::JSObject;
            } else {
                js_func.cached_prototype_ptr = std::ptr::null_mut();
            }
        }

        if obj_val.is_function() && atom.0 & 0x40000000 != 0 {
            let js_func = unsafe { JSValue::function_from_ptr_mut(ptr) };
            js_func.mark_has_symbol_prop();
        }
    }

    fn int_atom(&self, idx: usize, ctx: &mut JSContext) -> crate::runtime::atom::Atom {
        ctx.int_atom_mut(idx)
    }

    #[cold]
    fn add_slow(&mut self, a: &JSValue, b: &JSValue, ctx: &mut JSContext) -> JSValue {
        if a.is_int() && b.is_float() {
            return JSValue::new_float_raw(a.get_int() as f64 + b.get_float());
        }
        if a.is_float() && b.is_int() {
            return JSValue::new_float_raw(a.get_float() + b.get_int() as f64);
        }

        let a = self.ordinary_to_primitive(a, "default", ctx);
        if self.pending_throw.is_some() {
            return JSValue::undefined();
        }
        let b = self.ordinary_to_primitive(b, "default", ctx);
        if self.pending_throw.is_some() {
            return JSValue::undefined();
        }

        if a.is_bigint() && b.is_bigint() {
            let a_int = Self::get_bigint_int(&a).unwrap_or(0);
            let b_int = Self::get_bigint_int(&b).unwrap_or(0);
            Self::create_bigint(a_int + b_int)
        } else if a.is_symbol() || b.is_symbol() {
            self.set_pending_type_error(ctx, "Cannot convert a Symbol value to a string");
            JSValue::undefined()
        } else if b.is_string() || a.is_string() {
            let a_str = if a.is_object() || a.is_function() {
                self.object_to_string(&a, ctx)
            } else {
                Self::js_to_string(&a, ctx)
            };
            let b_str = if b.is_object() || b.is_function() {
                self.object_to_string(&b, ctx)
            } else {
                Self::js_to_string(&b, ctx)
            };
            Self::js_add_string(&a_str, &b_str, ctx)
        } else if a.is_bigint() || b.is_bigint() {
            self.set_pending_type_error(ctx, "Cannot mix BigInt and other types");
            JSValue::undefined()
        } else if a.is_float() && b.is_float() {
            JSValue::new_float_raw(a.get_float() + b.get_float())
        } else if a.is_int() && b.is_float() {
            JSValue::new_float_raw(a.get_int() as f64 + b.get_float())
        } else if a.is_float() && b.is_int() {
            JSValue::new_float_raw(a.get_float() + b.get_int() as f64)
        } else {
            let fa = Self::js_to_number(&a, ctx);
            let fb = Self::js_to_number(&b, ctx);
            JSValue::new_float_raw(fa + fb)
        }
    }

    fn get_symbol_to_primitive_atom(
        &self,
        ctx: &mut JSContext,
    ) -> Option<crate::runtime::atom::Atom> {
        let global = ctx.global();
        if !global.is_object() {
            return None;
        }
        let sym_val = global.as_object().get(ctx.intern("Symbol.toPrimitive"))?;
        if !sym_val.is_symbol() {
            return None;
        }
        let sym_id = sym_val.get_symbol_id();
        Some(crate::runtime::atom::Atom(0x40000000 | sym_id))
    }

    fn call_function_safe(
        &mut self,
        ctx: &mut JSContext,
        func: JSValue,
        this_val: JSValue,
        args: &[JSValue],
    ) -> Result<JSValue, String> {
        let saved_handlers = std::mem::take(&mut self.exception_handlers);
        let result = self.call_function_with_this(ctx, func, this_val, args);
        self.exception_handlers = saved_handlers;
        if let Some(exc) = ctx.pending_exception.take() {
            self.pending_throw = Some(exc);
            return Err("exception propagated".to_string());
        }
        result
    }

    pub fn ordinary_to_primitive(&mut self, v: &JSValue, hint: &str, ctx: &mut JSContext) -> JSValue {
        if !v.is_object() && !v.is_function() {
            return *v;
        }
        let obj: &crate::object::JSObject = if v.is_object() {
            v.as_object()
        } else if v.is_function() {
            let func = v.as_function();

            if let Some(custom_fn) = func.base.get(ctx.common_atoms.value_of) {
                if custom_fn.is_function() {
                    if let Ok(r) = self.call_function_with_this(ctx, custom_fn, *v, &[]) {
                        if !r.is_object() && !r.is_function() {
                            return r;
                        }
                    }
                }
            }

            if let Some(custom_fn) = func.base.get(ctx.common_atoms.to_string) {
                if custom_fn.is_function() {
                    if let Ok(r) = self.call_function_with_this(ctx, custom_fn, *v, &[]) {
                        if !r.is_object() && !r.is_function() {
                            return r;
                        }
                    }
                }
            }
            let name = ctx.get_atom_str(func.name);
            let params: Vec<String> = (0..func.arity as usize)
                .map(|i| format!("a{}", i))
                .collect();
            let param_str = params.join(", ");
            let s = if name.is_empty() {
                format!("function({}) {{ [user code] }}", param_str)
            } else {
                format!("function {}({}) {{ [user code] }}", name, param_str)
            };
            return JSValue::new_string(ctx.intern(&s));
        } else {
            return *v;
        };

        let mut has_tp = false;

        let has_own_tp = self.get_symbol_to_primitive_atom(ctx).map_or(false, |a| {
            let mut cur: Option<*mut crate::object::object::JSObject> = Some(v.get_ptr() as *mut _);
            while let Some(p) = cur {
                unsafe {
                    if let Some(v) = (*p).get_own_value(a) {
                        return v.is_function();
                    }
                    cur = (*p).prototype;
                }
            }
            false
        });
        if has_own_tp {
            has_tp = true;
        } else if let Some(tp_atom) = self.get_symbol_to_primitive_atom(ctx) {
            let tp_val = {
                let saved_tp = std::mem::take(&mut self.exception_handlers);
                let result = obj
                    .get_own_descriptor(tp_atom)
                    .and_then(|desc| {
                        if let Some(getter) = desc.get {
                            self.call_function_with_this(ctx, getter, *v, &[]).ok()
                        } else {
                            desc.value
                        }
                    })
                    .or_else(|| {
                        let mut current = obj.prototype;
                        while let Some(proto_ptr) = current {
                            unsafe {
                                let proto = &*proto_ptr;
                                if let Some(desc) = proto.get_own_descriptor(tp_atom) {
                                    if let Some(getter) = desc.get {
                                        return self
                                            .call_function_with_this(ctx, getter, *v, &[])
                                            .ok();
                                    }
                                    return desc.value;
                                }
                                current = proto.prototype;
                            }
                        }
                        None
                    });
                self.exception_handlers = saved_tp;
                result
            };
            self.adopt_pending_host_exception(ctx);
            if self.pending_throw.is_some() {
                return JSValue::undefined();
            }
            has_tp = match tp_val {
                Some(t) if t.is_function() => true,
                Some(t) if t.is_undefined() || t.is_null() => false,
                Some(_) => {
                    self.set_pending_type_error(
                        ctx,
                        "Symbol.toPrimitive is not a function",
                    );
                    return JSValue::undefined();
                }
                None => false,
            };
        }
        if !has_tp {
            if let Some(prim) = obj.get(ctx.common_atoms.__value__) {
                if !prim.is_object() && !prim.is_function() {
                    return prim;
                }
            }
        }

        if let Some(tp_atom) = self.get_symbol_to_primitive_atom(ctx) {
            let tp_method = (|| -> Option<JSValue> {
                let from_own = obj.get_own_descriptor(tp_atom).and_then(|desc| {
                    if let Some(getter) = desc.get {
                        let saved = std::mem::take(&mut self.exception_handlers);
                        let result = self.call_function_with_this(ctx, getter, *v, &[]).ok();
                        self.exception_handlers = saved;
                        result
                    } else {
                        desc.value
                    }
                });
                if from_own.is_some() {
                    return from_own;
                }

                let mut current = obj.prototype;
                while let Some(proto_ptr) = current {
                    unsafe {
                        let proto = &*proto_ptr;
                        if let Some(desc) = proto.get_own_descriptor(tp_atom) {
                            if let Some(getter) = desc.get {
                                let saved = std::mem::take(&mut self.exception_handlers);
                                let result =
                                    self.call_function_with_this(ctx, getter, *v, &[]).ok();
                                self.exception_handlers = saved;
                                return result;
                            }
                            return desc.value;
                        }
                        current = proto.prototype;
                    }
                }
                None
            })();
            self.adopt_pending_host_exception(ctx);
            if self.pending_throw.is_some() {
                return JSValue::undefined();
            }
            if let Some(tp_fn) = tp_method {
                if tp_fn.is_function() {
                    let hint_atom = ctx.intern(hint);
                    let result =
                        self.call_function_safe(ctx, tp_fn, *v, &[JSValue::new_string(hint_atom)]);
                    match result {
                        Ok(r) if !r.is_object() && !r.is_function() => return r,
                        Ok(_) => {
                            self.set_pending_type_error(
                                ctx,
                                "Cannot convert object to primitive value",
                            );
                            return JSValue::undefined();
                        }
                        Err(_) => {
                            return JSValue::undefined();
                        }
                    }
                }
            }
        }

        let hint = if hint == "default" { "number" } else { hint };
        let to_string_atom = ctx.common_atoms.to_string;
        let value_of_atom = ctx.common_atoms.value_of;
        let (first_try, second_try) = if hint == "string" {
            (to_string_atom, value_of_atom)
        } else {
            (value_of_atom, to_string_atom)
        };
        for &method_atom in &[first_try, second_try] {
            let method = match self.get_prop_value(ctx, *v, method_atom) {
                Ok(value) => value,
                Err(_) => return JSValue::undefined(),
            };
            if method.is_function() {
                let result = self.call_function_safe(ctx, method, *v, &[]);
                match result {
                    Ok(r) if !r.is_object() && !r.is_function() => return r,
                    Ok(_) => {}
                    Err(_) => {
                        return JSValue::undefined();
                    }
                }
            }
        }

        if v.is_function() {
            let func = v.as_function();
            let name = ctx.get_atom_str(func.name);
            if !name.is_empty() {
                return JSValue::new_string(
                    ctx.intern(&format!("function {}() {{ [native code] }}", name)),
                );
            }
            return JSValue::new_string(ctx.intern(&format!("function() {{ [native code] }}")));
        }

        let name_atom = ctx.common_atoms.name;
        let message_atom = ctx.common_atoms.message;
        let mut err = crate::object::object::JSObject::new();
        err.set(name_atom, JSValue::new_string(ctx.intern("TypeError")));
        err.set(
            message_atom,
            JSValue::new_string(ctx.intern("Cannot convert object to primitive value")),
        );
        if let Some(proto) = ctx.get_type_error_prototype() {
            err.prototype = Some(proto);
        }
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.pending_throw = Some(JSValue::new_object(ptr));
        JSValue::undefined()
    }

    fn js_to_string(v: &JSValue, ctx: &mut JSContext) -> JSValue {
        if v.is_string() {
            return *v;
        }
        if v.is_int() {
            return JSValue::new_string(ctx.intern(&v.get_int().to_string()));
        }
        if v.is_float() {
            let number = v.get_float();
            let text = if number.is_nan() {
                "NaN".to_string()
            } else if number.is_infinite() {
                if number.is_sign_negative() {
                    "-Infinity".to_string()
                } else {
                    "Infinity".to_string()
                }
            } else if number == number.floor() && number.abs() < 1e15 {
                format!("{}", number as i64)
            } else {
                crate::builtins::string::js_float_to_string(number)
            };
            return JSValue::new_string(ctx.intern(&text));
        }
        if v.is_bool() {
            return JSValue::new_string(ctx.intern(&v.get_bool().to_string()));
        }
        if v.is_null() {
            return JSValue::new_string(ctx.common_atoms.null);
        }
        if v.is_undefined() {
            return JSValue::new_string(ctx.common_atoms.undefined);
        }
        if v.is_bigint() {
            let val = VM::get_bigint_int(v).unwrap_or(0);
            return JSValue::new_string(ctx.intern(&val.to_string()));
        }
        if v.is_symbol() {
            return JSValue::new_string(ctx.intern("Symbol()"));
        }
        JSValue::new_string(ctx.intern(""))
    }

    fn object_to_string(&mut self, v: &JSValue, ctx: &mut JSContext) -> JSValue {
        if !v.is_object() && !v.is_function() {
            return Self::js_to_string(v, ctx);
        }
        let obj: &crate::object::JSObject = if v.is_object() {
            v.as_object()
        } else if v.is_function() {
            &v.as_function().base
        } else {
            return Self::js_to_string(v, ctx);
        };
        let to_str_atom = ctx.common_atoms.to_string;

        if v.is_function() {
            let fn_builtin = ctx.get_builtin_func("function_toString");
            if let Some(f) = fn_builtin {
                let result = f(ctx, &[*v]);
                if result.is_string() {
                    return result;
                }
            }

            let func = v.as_function();
            let name = ctx.get_atom_str(func.name);
            if name.is_empty() {
                return JSValue::new_string(ctx.intern("function () { [native code] }"));
            } else {
                return JSValue::new_string(
                    ctx.intern(&format!("function {}() {{ [native code] }}", name)),
                );
            }
        }
        if let Some(to_str_fn) = obj.get(to_str_atom) {
            if to_str_fn.is_function() {
                match self.call_function_with_this(ctx, to_str_fn, *v, &[]) {
                    Ok(r) if r.is_string() => return r,
                    Ok(r) => return Self::js_to_string(&r, ctx),
                    Err(_) => {}
                }
            }
        }

        let obj_proto = if let Some(p) = ctx.get_object_prototype() {
            p
        } else {
            return Self::js_to_string(v, ctx);
        };
        let to_str = unsafe { (*obj_proto).get(to_str_atom) };
        if let Some(f) = to_str {
            if f.is_function() {
                match self.call_function_with_this(ctx, f, *v, &[]) {
                    Ok(r) => return r,
                    Err(_) => {}
                }
            }
        }
        Self::js_to_string(v, ctx)
    }

    fn set_pending_range_error(&mut self, ctx: &mut JSContext, msg: &str) {
        use crate::object::object::JSObject;
        let mut err = JSObject::new();
        err.set(ctx.intern("name"), JSValue::new_string(ctx.intern("RangeError")));
        err.set(ctx.intern("message"), JSValue::new_string(ctx.intern(msg)));
        if let Some(proto) = ctx.get_range_error_prototype() {
            err.prototype = Some(proto);
        }
        let ptr = Box::into_raw(Box::new(err)) as usize;
        ctx.runtime_mut().gc_heap_mut().track(ptr);
        self.pending_throw = Some(JSValue::new_object(ptr));
    }

    fn pow_op(&mut self, a: &JSValue, b: &JSValue, ctx: &mut JSContext) -> Option<JSValue> {
        let a = self.to_numeric(a, ctx)?;
        let b = self.to_numeric(b, ctx)?;
        if a.is_bigint() != b.is_bigint() {
            self.set_pending_type_error(ctx, "Cannot mix BigInt and other types");
            return None;
        }
        if a.is_bigint() {
            let ai = Self::get_bigint_int(&a).unwrap_or(0);
            let bi = Self::get_bigint_int(&b).unwrap_or(0);
            if bi < 0 {
                self.set_pending_range_error(ctx, "BigInt exponent must be non-negative");
                return None;
            }
            return Some(Self::create_bigint(ai.pow(bi as u32)));
        }
        let bv = Self::js_to_number(&a, ctx);
        let ev = Self::js_to_number(&b, ctx);
        if ev.is_infinite() && bv.abs() == 1.0 {
            return Some(JSValue::new_float(f64::NAN));
        }
        let r = if ev == 2.0 {
            bv * bv
        } else if ev == 0.5 {
            bv.sqrt()
        } else if ev >= 2.0 && ev <= 8.0 && ev == ev.floor() {
            let mut r = bv;
            let mut n = ev as i32 - 1;
            while n > 0 {
                r *= bv;
                n -= 1;
            }
            r
        } else if ev == -1.0 {
            1.0 / bv
        } else if ev == 1.0 {
            bv
        } else {
            bv.powf(ev)
        };
        Some(JSValue::new_float(r))
    }

    fn to_numeric(&mut self, v: &JSValue, ctx: &mut JSContext) -> Option<JSValue> {
        let mut v = v.clone();
        if v.is_object_like() {
            v = self.ordinary_to_primitive(&v, "number", ctx);
            if self.pending_throw.is_some() {
                return None;
            }
        }
        if v.is_symbol() {
            self.set_pending_type_error(ctx, "Cannot convert a Symbol value to a number");
            return None;
        }
        Some(v)
    }

    fn arithmetic_binary_op(
        &mut self,
        a: &JSValue,
        b: &JSValue,
        op: ArithOp,
        ctx: &mut JSContext,
    ) -> Option<JSValue> {
        let av = self.to_numeric(a, ctx)?;
        let bv = self.to_numeric(b, ctx)?;
        if av.is_bigint() && bv.is_bigint() {
            let ai = Self::get_bigint_int(&av).unwrap_or(0);
            let bi = Self::get_bigint_int(&bv).unwrap_or(0);
            return match op {
                ArithOp::Sub => Some(Self::create_bigint(ai - bi)),
                ArithOp::Mul => Some(Self::create_bigint(ai * bi)),
                ArithOp::Div => {
                    if bi == 0 {
                        self.set_pending_range_error(ctx, "Division by zero");
                        None
                    } else {
                        Some(Self::create_bigint(ai / bi))
                    }
                }
                ArithOp::Mod => {
                    if bi == 0 {
                        self.set_pending_range_error(ctx, "Division by zero");
                        None
                    } else {
                        Some(Self::create_bigint(ai % bi))
                    }
                }
            };
        }
        if av.is_bigint() || bv.is_bigint() {
            self.set_pending_type_error(ctx, "Cannot mix BigInt and other types");
            return None;
        }
        let af = Self::numeric_to_f64(&av, ctx);
        let bf = Self::numeric_to_f64(&bv, ctx);
        Some(match op {
            ArithOp::Sub => JSValue::new_float_raw(af - bf),
            ArithOp::Mul => JSValue::new_float(af * bf),
            ArithOp::Div => JSValue::new_float(af / bf),
            ArithOp::Mod => {
                if bf != 0.0 {
                    JSValue::new_float(af % bf)
                } else {
                    JSValue::new_float(f64::NAN)
                }
            }
        })
    }

    fn numeric_to_f64(v: &JSValue, ctx: &mut JSContext) -> f64 {
        if v.is_int() {
            v.get_int() as f64
        } else if v.is_float() {
            v.get_float()
        } else {
            Self::js_to_number(v, ctx)
        }
    }

    fn numeric_binary_op(
        &mut self,
        a: &JSValue,
        b: &JSValue,
        op: BitwiseOp,
        ctx: &mut JSContext,
    ) -> Option<JSValue> {
        let a = self.to_numeric(a, ctx)?;
        let b = self.to_numeric(b, ctx)?;
        if a.is_bigint() != b.is_bigint() {
            self.set_pending_type_error(ctx, "Cannot mix BigInt and other types");
            return None;
        }
        if a.is_bigint() && b.is_bigint() {
            let ai = Self::get_bigint_int(&a).unwrap_or(0);
            let bi = Self::get_bigint_int(&b).unwrap_or(0);
            return match op {
                BitwiseOp::And => Some(Self::create_bigint(ai & bi)),
                BitwiseOp::Or => Some(Self::create_bigint(ai | bi)),
                BitwiseOp::Xor => Some(Self::create_bigint(ai ^ bi)),
                BitwiseOp::Shl => {
                    if bi < 0 {
                        Some(Self::create_bigint(ai >> (-bi)))
                    } else {
                        Some(Self::create_bigint(ai << bi))
                    }
                }
                BitwiseOp::Shr => {
                    if bi < 0 {
                        Some(Self::create_bigint(ai << (-bi)))
                    } else {
                        Some(Self::create_bigint(ai >> bi))
                    }
                }
                BitwiseOp::UShr => {
                    self.set_pending_type_error(ctx, "BigInts have no unsigned right shift");
                    None
                }
            };
        }
        let na = if a.is_int() {
            a.get_int() as f64
        } else {
            Self::js_to_number(&a, ctx)
        };
        let nb = if b.is_int() {
            b.get_int() as f64
        } else {
            Self::js_to_number(&b, ctx)
        };
        let ia = Self::to_int32(na) as i64;
        let ib = Self::to_int32(nb) as i64;
        Some(match op {
            BitwiseOp::And => JSValue::new_int(ia & ib),
            BitwiseOp::Or => JSValue::new_int(ia | ib),
            BitwiseOp::Xor => JSValue::new_int(ia ^ ib),
            BitwiseOp::Shl => JSValue::new_int(((ia as i32) << (ib & 0x1f)) as i64),
            BitwiseOp::Shr => JSValue::new_int(((ia as i32) >> (ib & 0x1f)) as i64),
            BitwiseOp::UShr => {
                let ua = (ia as u64 & 0xffffffff) as u32;
                let sh = (ib & 0x1f) as u32;
                JSValue::new_int((ua >> sh) as i64)
            }
        })
    }

    fn utf16_less_than(a: &str, b: &str) -> bool {
        let a: Vec<u16> = a.encode_utf16().collect();
        let b: Vec<u16> = b.encode_utf16().collect();
        a < b
    }

    fn bigint_less_than_number(bigint: i128, number: f64) -> Option<bool> {
        if number.is_nan() {
            return None;
        }
        if number == f64::INFINITY {
            return Some(true);
        }
        if number == f64::NEG_INFINITY {
            return Some(false);
        }
        if number.fract() == 0.0 {
            let other = number as i128;
            return Some(bigint < other);
        }
        let floor = number.floor();
        if floor >= i128::MAX as f64 {
            return Some(true);
        }
        if floor <= i128::MIN as f64 {
            return Some(false);
        }
        Some(bigint <= floor as i128)
    }

    fn number_less_than_bigint(number: f64, bigint: i128) -> Option<bool> {
        if number.is_nan() {
            return None;
        }
        if number == f64::INFINITY {
            return Some(false);
        }
        if number == f64::NEG_INFINITY {
            return Some(true);
        }
        if number.fract() == 0.0 {
            let other = number as i128;
            return Some(other < bigint);
        }
        let floor = number.floor();
        if floor >= i128::MAX as f64 {
            return Some(false);
        }
        if floor <= i128::MIN as f64 {
            return Some(true);
        }
        Some((floor as i128) < bigint)
    }

    fn parse_bigint_string(value: &str) -> Option<i128> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Some(0);
        }
        if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
            return i128::from_str_radix(hex, 16).ok();
        }
        if let Some(oct) = trimmed.strip_prefix("0o").or_else(|| trimmed.strip_prefix("0O")) {
            return i128::from_str_radix(oct, 8).ok();
        }
        if let Some(bin) = trimmed.strip_prefix("0b").or_else(|| trimmed.strip_prefix("0B")) {
            return i128::from_str_radix(bin, 2).ok();
        }
        trimmed.parse::<i128>().ok()
    }

    fn abstract_less_than(
        &mut self,
        a: &JSValue,
        b: &JSValue,
        ctx: &mut JSContext,
        left_first: bool,
    ) -> Option<bool> {
        let mut a = a.clone();
        let mut b = b.clone();
        if left_first {
            if a.is_object_like() {
                a = self.ordinary_to_primitive(&a, "number", ctx);
                if self.pending_throw.is_some() {
                    return None;
                }
            }
            if b.is_object_like() {
                b = self.ordinary_to_primitive(&b, "number", ctx);
                if self.pending_throw.is_some() {
                    return None;
                }
            }
        } else {
            if b.is_object_like() {
                b = self.ordinary_to_primitive(&b, "number", ctx);
                if self.pending_throw.is_some() {
                    return None;
                }
            }
            if a.is_object_like() {
                a = self.ordinary_to_primitive(&a, "number", ctx);
                if self.pending_throw.is_some() {
                    return None;
                }
            }
        }
        if a.is_symbol() || b.is_symbol() {
            self.set_pending_type_error(ctx, "Cannot convert a Symbol value to a number");
            return None;
        }
        if a.is_string() && b.is_string() {
            return Some(Self::utf16_less_than(
                ctx.get_atom_str(a.get_atom()),
                ctx.get_atom_str(b.get_atom()),
            ));
        }
        if a.is_bigint() && b.is_bigint() {
            return Some(
                Self::get_bigint_int(&a).unwrap_or(0) < Self::get_bigint_int(&b).unwrap_or(0),
            );
        }
        if a.is_bigint() {
            let a_int = Self::get_bigint_int(&a).unwrap_or(0);
            if b.is_string() {
                return Self::parse_bigint_string(ctx.get_atom_str(b.get_atom()))
                    .map(|b_int| a_int < b_int);
            }
            if b.is_int() {
                return Some(a_int < b.get_int() as i128);
            }
            if b.is_float() {
                return Self::bigint_less_than_number(a_int, b.get_float());
            }
            if b.is_bool() {
                return Some(a_int < if b.get_bool() { 1 } else { 0 });
            }
            if b.is_null() {
                return Some(a_int < 0);
            }
            return None;
        }
        if b.is_bigint() {
            let b_int = Self::get_bigint_int(&b).unwrap_or(0);
            if a.is_string() {
                return Self::parse_bigint_string(ctx.get_atom_str(a.get_atom()))
                    .map(|a_int| a_int < b_int);
            }
            if a.is_int() {
                return Some((a.get_int() as i128) < b_int);
            }
            if a.is_float() {
                return Self::number_less_than_bigint(a.get_float(), b_int);
            }
            if a.is_bool() {
                return Some((if a.get_bool() { 1 } else { 0 }) < b_int);
            }
            if a.is_null() {
                return Some(0 < b_int);
            }
            return None;
        }
        let na = Self::js_to_number(&a, ctx);
        let nb = Self::js_to_number(&b, ctx);
        if na.is_nan() || nb.is_nan() {
            return None;
        }
        Some(na < nb)
    }

    fn js_to_number(v: &JSValue, ctx: &mut JSContext) -> f64 {
        if v.is_int() {
            return v.get_int() as f64;
        } else if v.is_float() {
            return v.get_float();
        } else if v.is_bool() {
            return if v.get_bool() { 1.0 } else { 0.0 };
        } else if v.is_null() {
            return 0.0;
        } else if v.is_undefined() {
            return f64::NAN;
        } else if v.is_string() {
            let s = ctx.get_atom_str(v.get_atom());
            return Self::string_to_number(s);
        } else if v.is_bigint() {
            let val = VM::get_bigint_int(v).unwrap_or(0);
            return val as f64;
        } else if v.is_symbol() {
            return f64::NAN;
        } else if v.is_object() {
            if let Some(prim) = v.as_object().get(ctx.common_atoms.__value__) {
                return Self::js_to_number(&prim, ctx);
            }
        }
        f64::NAN
    }

    pub fn string_to_number(s: &str) -> f64 {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return 0.0;
        }
        if trimmed == "Infinity" || trimmed == "+Infinity" {
            return f64::INFINITY;
        }
        if trimmed == "-Infinity" {
            return f64::NEG_INFINITY;
        }
        if (trimmed.starts_with("0x") || trimmed.starts_with("0X"))
            && trimmed.len() > 2
        {
            let hex_part = &trimmed[2..];
            if let Ok(n) = u64::from_str_radix(hex_part, 16) {
                return n as f64;
            }
            return f64::NAN;
        }
        if (trimmed.starts_with("0o") || trimmed.starts_with("0O"))
            && trimmed.len() > 2
        {
            let oct_part = &trimmed[2..];
            if let Ok(n) = u64::from_str_radix(oct_part, 8) {
                return n as f64;
            }
            return f64::NAN;
        }
        if (trimmed.starts_with("0b") || trimmed.starts_with("0B"))
            && trimmed.len() > 2
        {
            let bin_part = &trimmed[2..];
            if let Ok(n) = u64::from_str_radix(bin_part, 2) {
                return n as f64;
            }
            return f64::NAN;
        }
        let lower = trimmed.to_lowercase();
        if lower.contains("inf") || lower.contains("nan") {
            return f64::NAN;
        }
        trimmed.parse::<f64>().unwrap_or(f64::NAN)
    }

    fn to_int32(na: f64) -> i32 {
        if na.is_nan() || na.is_infinite() || na == 0.0 {
            return 0;
        }
        let pos = na.abs().floor() as u64;
        let u32_val = if na > 0.0 {
            pos % 0x1_0000_0000
        } else {
            let rem = pos % 0x1_0000_0000;
            if rem == 0 { 0 } else { 0x1_0000_0000 - rem }
        };
        u32_val as i32
    }

    fn get_bigint_int(v: &JSValue) -> Option<i128> {
        if v.is_bigint() {
            Some(v.as_object().get_bigint_value())
        } else {
            None
        }
    }

    fn create_bigint(value: i128) -> JSValue {
        let mut bigint_obj = crate::object::object::JSObject::new_bigint();
        bigint_obj.set_bigint_value(value);
        let ptr = Box::into_raw(Box::new(bigint_obj)) as usize;
        JSValue::new_bigint(ptr)
    }

    fn js_add_string(b: &JSValue, a: &JSValue, ctx: &mut JSContext) -> JSValue {
        if b.is_string() && a.is_string() {
            let atom = ctx.intern_concat_atoms(b.get_atom(), a.get_atom());
            return JSValue::new_string(atom);
        }

        fn stringify(v: &JSValue, ctx: &JSContext) -> (Option<[u8; 24]>, usize, Option<String>) {
            if v.is_string() {
                let s = ctx.get_atom_str(v.get_atom());
                let bytes = s.as_bytes();
                let len = bytes.len();
                if len <= 24 {
                    let mut buf = [0u8; 24];
                    buf[..len].copy_from_slice(bytes);
                    return (Some(buf), len, None);
                } else {
                    return (None, len, Some(s.to_string()));
                }
            } else if v.is_int() {
                let n = v.get_int();
                let mut buf = [0u8; 24];
                let mut tmp = n;
                let mut len = 0;
                let negative = tmp < 0;
                if negative {
                    tmp = -tmp;
                }
                if tmp == 0 {
                    buf[0] = b'0';
                    len = 1;
                } else {
                    while tmp > 0 {
                        buf[len] = (tmp % 10) as u8 + b'0';
                        len += 1;
                        tmp /= 10;
                    }
                }
                if negative {
                    buf[len] = b'-';
                    len += 1;
                }

                for i in 0..len / 2 {
                    buf.swap(i, len - 1 - i);
                }
                return (Some(buf), len, None);
            } else if v.is_float() {
                let f = v.get_float();
                let s = if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{}", f as i64)
                } else {
                    crate::builtins::string::js_float_to_string(f)
                };
                let bytes = s.as_bytes();
                let len = bytes.len();
                if len <= 24 {
                    let mut buf = [0u8; 24];
                    buf[..len].copy_from_slice(bytes);
                    return (Some(buf), len, None);
                } else {
                    return (None, len, Some(s));
                }
            } else if v.is_bool() {
                if v.get_bool() {
                    let mut buf = [0u8; 24];
                    buf[..4].copy_from_slice(b"true");
                    return (Some(buf), 4, None);
                } else {
                    let mut buf = [0u8; 24];
                    buf[..5].copy_from_slice(b"false");
                    return (Some(buf), 5, None);
                }
            } else if v.is_null() {
                let mut buf = [0u8; 24];
                buf[..4].copy_from_slice(b"null");
                return (Some(buf), 4, None);
            } else if v.is_object() || v.is_function() {
                let mut buf = [0u8; 24];
                let s = b"[object Object]";
                buf[..s.len()].copy_from_slice(s);
                return (Some(buf), s.len(), None);
            } else if v.is_bigint() {
                let val = crate::runtime::vm::VM::get_bigint_int(v);
                let n = val.unwrap_or(0);
                let mut buf = [0u8; 24];
                let s = n.to_string();
                let bytes = s.as_bytes();
                let len = bytes.len();
                if len <= 24 {
                    buf[..len].copy_from_slice(bytes);
                    (Some(buf), len, None)
                } else {
                    (None, len, Some(s))
                }
            } else {
                let mut buf = [0u8; 24];
                let s = b"undefined";
                buf[..s.len()].copy_from_slice(s);
                return (Some(buf), s.len(), None);
            }
        }

        let (buf_b, len_b, str_b) = stringify(b, ctx);
        let (buf_a, len_a, str_a) = stringify(a, ctx);

        let total = len_b + len_a;
        if total <= 128 {
            let mut combined = [0u8; 128];
            if let Some(buf) = buf_b {
                combined[..len_b].copy_from_slice(&buf[..len_b]);
            } else if let Some(ref s) = str_b {
                combined[..len_b].copy_from_slice(s.as_bytes());
            }
            if let Some(buf) = buf_a {
                combined[len_b..total].copy_from_slice(&buf[..len_a]);
            } else if let Some(ref s) = str_a {
                combined[len_b..total].copy_from_slice(s.as_bytes());
            }
            let atom = ctx.intern(unsafe { std::str::from_utf8_unchecked(&combined[..total]) });
            JSValue::new_string(atom)
        } else {
            let mut combined = String::with_capacity(total);
            if let Some(buf) = buf_b {
                combined.push_str(unsafe { std::str::from_utf8_unchecked(&buf[..len_b]) });
            } else if let Some(ref s) = str_b {
                combined.push_str(s);
            }
            if let Some(buf) = buf_a {
                combined.push_str(unsafe { std::str::from_utf8_unchecked(&buf[..len_a]) });
            } else if let Some(ref s) = str_a {
                combined.push_str(s);
            }
            let atom = ctx.intern(&combined);
            JSValue::new_string(atom)
        }
    }

    #[inline(always)]
    fn loose_equal_value(&mut self, ctx: &mut JSContext, a: JSValue, b: JSValue) -> Option<bool> {
        if a.raw_bits() == b.raw_bits() {
            return Some(if a.is_float() {
                !a.get_float().is_nan()
            } else {
                true
            });
        }

        if JSValue::both_int(&a, &b) {
            return Some(false);
        }

        if (a.is_object() || a.is_function()) && b.is_null_or_undefined() {
            return Some(false);
        }
        if (b.is_object() || b.is_function()) && a.is_null_or_undefined() {
            return Some(false);
        }

        if a.is_null_or_undefined() {
            return Some(b.is_null_or_undefined());
        }
        if b.is_null() || b.is_undefined() {
            return Some(false);
        }

        self.loose_equal_value_slow(ctx, a, b)
    }

    fn loose_equal_value_slow(
        &mut self,
        ctx: &mut JSContext,
        a: JSValue,
        b: JSValue,
    ) -> Option<bool> {
        if a.raw_bits() == b.raw_bits() {
            return Some(!(a.is_float() && a.get_float().is_nan()));
        }
        if a.is_bool() {
            let n = JSValue::new_int(if a.get_bool() { 1 } else { 0 });
            return self.loose_equal_value_slow(ctx, n, b);
        }
        if b.is_bool() {
            let n = JSValue::new_int(if b.get_bool() { 1 } else { 0 });
            return self.loose_equal_value_slow(ctx, a, n);
        }
        let a_primitive = a.is_string()
            || a.is_int()
            || a.is_float()
            || a.is_bigint()
            || a.is_symbol();
        let b_primitive = b.is_string()
            || b.is_int()
            || b.is_float()
            || b.is_bigint()
            || b.is_symbol();
        if (a.is_object() || a.is_function()) && b_primitive {
            let prim = self.ordinary_to_primitive(&a, "default", ctx);
            if self.pending_throw.is_some() {
                return None;
            }
            if prim.is_object() || prim.is_function() {
                return Some(false);
            }
            return self.loose_equal_value_slow(ctx, prim, b);
        }
        if (b.is_object() || b.is_function()) && a_primitive {
            let prim = self.ordinary_to_primitive(&b, "default", ctx);
            if self.pending_throw.is_some() {
                return None;
            }
            if prim.is_object() || prim.is_function() {
                return Some(false);
            }
            return self.loose_equal_value_slow(ctx, a, prim);
        }
        if (a.is_object() || a.is_function()) || (b.is_object() || b.is_function()) {
            return Some(false);
        }
        if a.is_string() && b.is_string() {
            return Some(a.get_atom().0 == b.get_atom().0);
        }
        if JSValue::both_int(&a, &b) {
            return Some(a.get_int() == b.get_int());
        }
        if a.is_bigint() && b.is_bigint() {
            return Some(a.strict_eq(&b));
        }
        if a.is_float() && b.is_float() {
            return Some(a.get_float() == b.get_float());
        }
        if a.is_int() && b.is_float() {
            return Some((a.get_int() as f64) == b.get_float());
        }
        if a.is_float() && b.is_int() {
            return Some(a.get_float() == (b.get_int() as f64));
        }
        if a.is_string() && (b.is_int() || b.is_float()) {
            let n = Self::string_to_number(&ctx.get_atom_str(a.get_atom()));
            return self.loose_equal_value_slow(ctx, JSValue::new_float(n), b);
        }
        if (a.is_int() || a.is_float()) && b.is_string() {
            let n = Self::string_to_number(&ctx.get_atom_str(b.get_atom()));
            return self.loose_equal_value_slow(ctx, a, JSValue::new_float(n));
        }
        if a.is_string() && b.is_bigint() {
            let b_int = Self::get_bigint_int(&b).unwrap_or(0);
            return Some(
                Self::parse_bigint_string(&ctx.get_atom_str(a.get_atom()))
                    .map_or(false, |a_int| a_int == b_int),
            );
        }
        if a.is_bigint() && b.is_string() {
            let a_int = Self::get_bigint_int(&a).unwrap_or(0);
            return Some(
                Self::parse_bigint_string(&ctx.get_atom_str(b.get_atom()))
                    .map_or(false, |b_int| a_int == b_int),
            );
        }
        if a.is_bigint() && (b.is_int() || b.is_float()) {
            let a_int = Self::get_bigint_int(&a).unwrap_or(0);
            let n = if b.is_int() {
                b.get_int() as f64
            } else {
                b.get_float()
            };
            return Some(Self::bigint_number_equal(a_int, n));
        }
        if (a.is_int() || a.is_float()) && b.is_bigint() {
            let b_int = Self::get_bigint_int(&b).unwrap_or(0);
            let n = if a.is_int() {
                a.get_int() as f64
            } else {
                a.get_float()
            };
            return Some(Self::bigint_number_equal(b_int, n));
        }
        Some(false)
    }

    fn bigint_number_equal(b: i128, n: f64) -> bool {
        if !n.is_finite() || n.fract() != 0.0 {
            return false;
        }
        let two_127 = 170141183460469231731687303715884105728.0f64;
        if n >= two_127 || n < -two_127 {
            return false;
        }
        (n as i128) == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bytecode(code: Vec<u8>, constants: Vec<JSValue>, locals_count: u32) -> Bytecode {
        Bytecode {
            code,
            constants,
            locals_count,
            param_count: 0,
            line_number_table: None,
            ic_table: crate::compiler::InlineCacheTable::new(),
            shared_ic_table_ptr: std::ptr::null_mut(),
            uses_arguments: false,
            has_simple_parameters: false,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            nested_bytecodes: Vec::new(),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
            shared_code_ptr: std::ptr::null(),
            shared_code_len: 0,
            shared_const_ptr: std::ptr::null(),
            shared_const_len: 0,
            shared_nested_bytecodes_ptr: std::ptr::null_mut(),
        }
    }

    fn emit_u16(code: &mut Vec<u8>, v: u16) {
        code.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_u32(code: &mut Vec<u8>, v: u32) {
        code.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_i32(code: &mut Vec<u8>, v: i32) {
        code.extend_from_slice(&v.to_le_bytes());
    }

    fn run_bytecode(code: Vec<u8>, constants: Vec<JSValue>, locals_count: u32) -> JSValue {
        let bc = make_bytecode(code, constants, locals_count);
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let mut vm = VM::new();
        match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        }
    }

    #[test]
    fn test_vm_new_encoding_add() {
        let mut code = Vec::new();

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 1);
        emit_i32(&mut code, 40);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 2);
        emit_i32(&mut code, 2);

        code.push(Opcode::Add as u8);
        emit_u16(&mut code, 3);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 3);

        code.push(Opcode::End as u8);

        let result = run_bytecode(code, vec![], 4);
        assert_eq!(result.get_int(), 42);
    }

    #[test]
    fn test_vm_new_encoding_set_get_prop() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let atom_x = ctx.atom_table_mut().intern("x");

        let mut code = Vec::new();

        code.push(Opcode::NewObject as u8);
        emit_u16(&mut code, 1);

        code.push(Opcode::LoadConst as u8);
        emit_u16(&mut code, 2);
        emit_u32(&mut code, 0);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 3);
        emit_i32(&mut code, 42);

        code.push(Opcode::SetProp as u8);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);
        emit_u16(&mut code, 3);

        code.push(Opcode::GetProp as u8);
        emit_u16(&mut code, 4);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 4);

        code.push(Opcode::End as u8);

        let bc = make_bytecode(code, vec![JSValue::new_string(atom_x)], 5);
        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 42);
    }

    fn builtin_const_99(_ctx: &mut JSContext, _args: &[JSValue]) -> JSValue {
        JSValue::new_int(99)
    }

    fn builtin_inc(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
        let v = args.first().copied().unwrap_or_else(JSValue::undefined);
        if v.is_int() {
            JSValue::new_int(v.get_int() + 1)
        } else {
            JSValue::new_int(1)
        }
    }

    #[test]
    fn test_vm_call0_new_encoding() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern("f0"), 0);
        func.builtin_func = Some(builtin_const_99);
        let func_ptr = Box::into_raw(Box::new(func)) as usize;
        ctx.runtime_mut().gc_heap_mut().track_function(func_ptr);

        let mut code = Vec::new();
        code.push(Opcode::LoadConst as u8);
        emit_u16(&mut code, 1);
        emit_u32(&mut code, 0);

        code.push(Opcode::Call0 as u8);
        emit_u16(&mut code, 2);
        emit_u16(&mut code, 1);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 2);

        code.push(Opcode::End as u8);

        let bc = make_bytecode(code, vec![JSValue::new_function(func_ptr)], 3);
        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 99);
    }

    #[test]
    fn test_vm_call1_new_encoding() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern("f1"), 1);
        func.builtin_func = Some(builtin_inc);
        let func_ptr = Box::into_raw(Box::new(func)) as usize;

        let mut code = Vec::new();
        code.push(Opcode::LoadConst as u8);
        emit_u16(&mut code, 1);
        emit_u32(&mut code, 0);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 2);
        emit_i32(&mut code, 41);

        code.push(Opcode::Call1 as u8);
        emit_u16(&mut code, 3);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 3);

        code.push(Opcode::End as u8);

        let bc = make_bytecode(code, vec![JSValue::new_function(func_ptr)], 4);
        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 42);
    }

    #[test]
    fn test_vm_call2_new_encoding() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        fn builtin_add2(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
            let a = args.first().copied().unwrap_or_else(JSValue::undefined);
            let b = args.get(1).copied().unwrap_or_else(JSValue::undefined);
            JSValue::new_int(a.get_int() + b.get_int())
        }

        let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern("f2"), 2);
        func.builtin_func = Some(builtin_add2);
        let func_ptr = Box::into_raw(Box::new(func)) as usize;

        let mut code = Vec::new();
        code.push(Opcode::LoadConst as u8);
        emit_u16(&mut code, 1);
        emit_u32(&mut code, 0);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 2);
        emit_i32(&mut code, 40);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 3);
        emit_i32(&mut code, 2);

        code.push(Opcode::Call2 as u8);
        emit_u16(&mut code, 4);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);
        emit_u16(&mut code, 3);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 4);

        code.push(Opcode::End as u8);

        let bc = make_bytecode(code, vec![JSValue::new_function(func_ptr)], 5);
        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 42);
    }

    #[test]
    fn test_vm_call3_new_encoding() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        fn builtin_add3(_ctx: &mut JSContext, args: &[JSValue]) -> JSValue {
            let a = args.first().copied().unwrap_or_else(JSValue::undefined);
            let b = args.get(1).copied().unwrap_or_else(JSValue::undefined);
            let c = args.get(2).copied().unwrap_or_else(JSValue::undefined);
            JSValue::new_int(a.get_int() + b.get_int() + c.get_int())
        }

        let mut func = crate::object::function::JSFunction::new_builtin(ctx.intern("f3"), 3);
        func.builtin_func = Some(builtin_add3);
        let func_ptr = Box::into_raw(Box::new(func)) as usize;

        let mut code = Vec::new();
        code.push(Opcode::LoadConst as u8);
        emit_u16(&mut code, 1);
        emit_u32(&mut code, 0);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 2);
        emit_i32(&mut code, 39);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 3);
        emit_i32(&mut code, 2);

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 4);
        emit_i32(&mut code, 1);

        code.push(Opcode::Call3 as u8);
        emit_u16(&mut code, 5);
        emit_u16(&mut code, 1);
        emit_u16(&mut code, 2);
        emit_u16(&mut code, 3);
        emit_u16(&mut code, 4);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 5);

        code.push(Opcode::End as u8);

        let bc = make_bytecode(code, vec![JSValue::new_function(func_ptr)], 6);
        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bc).unwrap() {
            ExecutionOutcome::Complete(v) => v,
            ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 42);
    }

    #[test]
    fn test_vm_inc_local_new_encoding() {
        let mut code = Vec::new();

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 1);
        emit_i32(&mut code, 41);

        code.push(Opcode::IncLocal as u8);
        emit_u16(&mut code, 1);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 1);

        code.push(Opcode::End as u8);

        let result = run_bytecode(code, vec![], 2);
        assert_eq!(result.get_int(), 42);
    }

    #[test]
    fn test_vm_dec_local_new_encoding() {
        let mut code = Vec::new();

        code.push(Opcode::LoadInt as u8);
        emit_u16(&mut code, 1);
        emit_i32(&mut code, 41);

        code.push(Opcode::DecLocal as u8);
        emit_u16(&mut code, 1);

        code.push(Opcode::Move as u8);
        emit_u16(&mut code, 0);
        emit_u16(&mut code, 1);

        code.push(Opcode::End as u8);

        let result = run_bytecode(code, vec![], 2);
        assert_eq!(result.get_int(), 40);
    }
}

impl Drop for VM {
    fn drop(&mut self) {
        for frame in self.frames.iter_mut() {
            frame.saved_args = Vec::new();
            frame.upvalue_sync_map = None;
            frame.eval_bindings = None;
            frame.cached_arguments = None;
        }
    }
}
