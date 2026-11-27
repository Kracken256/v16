use super::atom::{Atom, AtomTable};
use super::context::JSContext;
use super::gc::GcHeap;
use super::module::ModuleRegistry;
use crate::value::JSValue;
use std::sync::atomic::{AtomicBool, Ordering};

pub static EXTERNAL_INTERRUPT: AtomicBool = AtomicBool::new(false);

pub struct JSRuntime {
    atom_table: AtomTable,
    gc_heap: GcHeap,
    ctx_count: u32,
    interrupt: AtomicBool,
    module_registry: ModuleRegistry,
    next_symbol_id: u32,
    symbol_registry: Option<*mut crate::object::object::JSObject>,
    symbol_descriptions: crate::util::FxHashMap<u32, Atom>,

    pub argv: Vec<String>,
}

impl JSRuntime {
    pub fn new() -> Self {
        JSRuntime {
            atom_table: AtomTable::new(),
            gc_heap: GcHeap::new(),
            ctx_count: 0,
            interrupt: AtomicBool::new(false),
            module_registry: ModuleRegistry::new(),
            next_symbol_id: 1,
            symbol_registry: None,
            symbol_descriptions: crate::util::FxHashMap::default(),
            argv: Vec::new(),
        }
    }

    pub fn set_interrupt(&self, flag: bool) {
        self.interrupt.store(flag, Ordering::SeqCst);
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupt.load(Ordering::SeqCst)
    }

    pub fn atom_table(&self) -> &AtomTable {
        &self.atom_table
    }

    pub fn atom_table_mut(&mut self) -> &mut AtomTable {
        &mut self.atom_table
    }

    pub fn gc_heap(&self) -> &GcHeap {
        &self.gc_heap
    }

    pub fn gc_heap_mut(&mut self) -> &mut GcHeap {
        &mut self.gc_heap
    }

    pub fn module_registry(&self) -> &ModuleRegistry {
        &self.module_registry
    }

    pub fn module_registry_mut(&mut self) -> &mut ModuleRegistry {
        &mut self.module_registry
    }

    pub fn module_registry_and_atoms_mut(&mut self) -> (&mut ModuleRegistry, &mut AtomTable) {
        (&mut self.module_registry, &mut self.atom_table)
    }

    pub fn new_context(&mut self) -> Box<JSContext<'_>> {
        self.ctx_count += 1;
        JSContext::new(self)
    }

    pub fn run_gc(&mut self, roots: &[JSValue]) -> usize {
        self.gc_heap.run_gc(roots)
    }

    pub fn minor_gc(&mut self, roots: &[JSValue]) -> usize {
        self.gc_heap.minor_gc(roots)
    }

    pub fn atom_count(&self) -> usize {
        self.atom_table.len()
    }

    pub fn context_count(&self) -> u32 {
        self.ctx_count
    }

    pub fn next_symbol_id(&mut self) -> u32 {
        let id = self.next_symbol_id;
        self.next_symbol_id += 1;
        id
    }

    pub fn register_symbol_description(&mut self, id: u32, description: Atom) {
        self.symbol_descriptions.insert(id, description);
    }

    pub fn symbol_description(&self, id: u32) -> Option<Atom> {
        self.symbol_descriptions.get(&id).copied()
    }

    pub fn symbol_registry(&self) -> Option<*mut crate::object::object::JSObject> {
        self.symbol_registry
    }

    pub fn set_symbol_registry(&mut self, ptr: Option<*mut crate::object::object::JSObject>) {
        self.symbol_registry = ptr;
    }

    pub fn set_argv(&mut self, args: Vec<String>) {
        self.argv = args;
    }
}

impl Default for JSRuntime {
    fn default() -> Self {
        Self::new()
    }
}
