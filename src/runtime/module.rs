use crate::object::JSObject;
use crate::runtime::atom::AtomTable;
use crate::runtime::context::JSContext;
use crate::util::FxHashMap;
use crate::value::JSValue;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Unlinked,
    Linking,
    Linked,
    Evaluating,
    Evaluated,
    Errored,
}

#[derive(Clone)]
pub struct ModuleExport {
    pub name: String,
    pub value: JSValue,
    pub mutable: bool,
    pub getter: Option<JSValue>,
}

#[derive(Clone)]
pub struct ModuleImport {
    pub module_specifier: String,
    pub import_name: String,
    pub local_name: String,
}

pub struct Module {
    pub specifier: String,
    pub state: ModuleState,
    pub exports: FxHashMap<String, ModuleExport>,
    pub imports: Vec<ModuleImport>,
    pub namespace_object: Option<usize>,
    pub source: String,
    pub import_meta_object: Option<usize>,
    pub error: Option<String>,
}

impl Module {
    pub fn new(specifier: String, source: String) -> Self {
        Module {
            specifier,
            state: ModuleState::Unlinked,
            exports: FxHashMap::default(),
            imports: Vec::new(),
            namespace_object: None,
            source,
            import_meta_object: None,
            error: None,
        }
    }

    pub fn add_export(&mut self, name: String, value: JSValue, mutable: bool) {
        self.exports.insert(
            name.clone(),
            ModuleExport {
                name,
                value,
                mutable,
                getter: None,
            },
        );
    }

    pub fn add_live_export(&mut self, name: String, getter: JSValue) {
        self.exports.insert(
            name.clone(),
            ModuleExport {
                name,
                value: JSValue::undefined(),
                mutable: true,
                getter: Some(getter),
            },
        );
    }

    pub fn add_import(
        &mut self,
        module_specifier: String,
        import_name: String,
        local_name: String,
    ) {
        self.imports.push(ModuleImport {
            module_specifier,
            import_name,
            local_name,
        });
    }

    pub fn get_export(&self, name: &str) -> Option<&ModuleExport> {
        self.exports.get(name)
    }

    pub fn get_export_mut(&mut self, name: &str) -> Option<&mut ModuleExport> {
        self.exports.get_mut(name)
    }

    pub fn get_export_value(&self, name: &str) -> JSValue {
        self.exports
            .get(name)
            .map(|e| e.value.clone())
            .unwrap_or(JSValue::undefined())
    }

    pub fn set_export_value(&mut self, name: &str, value: JSValue) {
        if let Some(export) = self.exports.get_mut(name) {
            export.value = value;
        }
    }

    pub fn create_namespace_object(
        &mut self,
        atom_table: &mut AtomTable,
        to_string_tag_key: crate::runtime::atom::Atom,
        module_tag_atom: crate::runtime::atom::Atom,
    ) -> usize {
        let mut names: Vec<String> = self.exports.keys().cloned().collect();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let mut ns_obj = JSObject::new();
        for name in names {
            let export = self.exports.get(&name).cloned();
            let atom = atom_table.intern(&name);
            if let Some(getter) = export.as_ref().and_then(|e| e.getter) {
                ns_obj.define_property_ext(
                    atom,
                    crate::object::object::PropertyDescriptor {
                        value: None,
                        writable: false,
                        enumerable: true,
                        configurable: false,
                        get: Some(getter),
                        set: None,
                    },
                    false,
                    true,
                    true,
                );
            } else {
                let value = export
                    .as_ref()
                    .map(|e| e.value.clone())
                    .unwrap_or_else(JSValue::undefined);
                ns_obj.define_property_ext(
                    atom,
                    crate::object::object::PropertyDescriptor {
                        value: Some(value),
                        writable: false,
                        enumerable: true,
                        configurable: false,
                        get: None,
                        set: None,
                    },
                    true,
                    true,
                    true,
                );
            }
        }
        ns_obj.define_property_ext(
            to_string_tag_key,
            crate::object::object::PropertyDescriptor {
                value: Some(JSValue::new_string(module_tag_atom)),
                writable: false,
                enumerable: false,
                configurable: false,
                get: None,
                set: None,
            },
            true,
            true,
            true,
        );
        ns_obj.set_extensible(false);
        ns_obj.set_module_namespace();
        let ptr = Box::into_raw(Box::new(ns_obj)) as usize;
        self.namespace_object = Some(ptr);
        ptr
    }

    pub fn get_or_create_namespace_object(
        &mut self,
        atom_table: &mut AtomTable,
        to_string_tag_key: crate::runtime::atom::Atom,
        module_tag_atom: crate::runtime::atom::Atom,
    ) -> usize {
        if let Some(ptr) = self.namespace_object {
            ptr
        } else {
            self.create_namespace_object(atom_table, to_string_tag_key, module_tag_atom)
        }
    }

    pub fn has_dependencies(&self) -> bool {
        !self.imports.is_empty()
    }

    pub fn get_dependencies(&self) -> Vec<String> {
        self.imports
            .iter()
            .map(|imp| imp.module_specifier.clone())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

pub struct ModuleRegistry {
    modules: FxHashMap<String, RefCell<Module>>,
    resolving: Vec<String>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry {
            modules: FxHashMap::default(),
            resolving: Vec::new(),
        }
    }

    pub fn register(&mut self, module: Module) {
        self.modules
            .insert(module.specifier.clone(), RefCell::new(module));
    }

    pub fn get(&self, specifier: &str) -> Option<&RefCell<Module>> {
        self.modules.get(specifier)
    }

    pub fn get_mut(&mut self, specifier: &str) -> Option<&mut RefCell<Module>> {
        self.modules.get_mut(specifier)
    }

    pub fn has(&self, specifier: &str) -> bool {
        self.modules.contains_key(specifier)
    }

    pub fn begin_resolve(&mut self, specifier: &str) -> bool {
        if self.resolving.contains(&specifier.to_string()) {
            return false;
        }
        self.resolving.push(specifier.to_string());
        true
    }

    pub fn end_resolve(&mut self, specifier: &str) {
        self.resolving.retain(|s| s != specifier);
    }

    pub fn is_resolving(&self, specifier: &str) -> bool {
        self.resolving.contains(&specifier.to_string())
    }

    pub fn is_state(&self, specifier: &str, state: ModuleState) -> bool {
        self.modules
            .get(specifier)
            .map(|m| m.borrow().state == state)
            .unwrap_or(false)
    }

    pub fn set_state(&mut self, specifier: &str, state: ModuleState) {
        if let Some(m) = self.modules.get(specifier) {
            m.borrow_mut().state = state;
        }
    }

    pub fn get_all_specifiers(&self) -> Vec<String> {
        self.modules.keys().cloned().collect()
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn resolve_specifier(specifier: &str, base_path: &str) -> String {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let base = std::path::Path::new(base_path);
        let base_dir = if base.is_dir() {
            base
        } else {
            base.parent().unwrap_or(std::path::Path::new("."))
        };
        let resolved = base_dir.join(specifier);
        let canonical = std::fs::canonicalize(&resolved).unwrap_or(resolved);
        canonical.to_string_lossy().to_string()
    } else if specifier.starts_with('/') {
        specifier.to_string()
    } else {
        let path = std::path::Path::new(".").join(specifier);
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        canonical.to_string_lossy().to_string()
    }
}

fn collect_static_dependencies(body: &[crate::compiler::ast::ASTNode]) -> Vec<String> {
    let mut dependencies = Vec::new();
    for node in body {
        match node {
            crate::compiler::ast::ASTNode::ImportDeclaration(decl) => {
                dependencies.push(decl.source.clone());
            }
            crate::compiler::ast::ASTNode::ExportNamedDeclaration(decl) => {
                if let Some(source) = &decl.source {
                    dependencies.push(source.clone());
                }
            }
            crate::compiler::ast::ASTNode::ExportAllDeclaration(decl) => {
                dependencies.push(decl.source.clone());
            }
            _ => {}
        }
    }
    dependencies
}

fn namespace_object_for(ctx: &mut JSContext, resolved: &str) -> Result<usize, String> {
    let to_string_tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
    let module_tag_atom = ctx.intern("Module");
    let atom_table_ptr = ctx.atom_table_mut() as *mut _;
    unsafe {
        let registry = ctx.runtime_mut().module_registry_mut();
        if let Some(module_cell) = registry.get_mut(resolved) {
            Ok(module_cell.borrow_mut().get_or_create_namespace_object(
                &mut *atom_table_ptr,
                to_string_tag_key,
                module_tag_atom,
            ))
        } else {
            Err(format!("Module {} not found", resolved))
        }
    }
}

pub fn load_module_source(specifier: &str) -> Result<String, String> {
    let path = if specifier.ends_with(".js") || specifier.ends_with(".mjs") {
        specifier.to_string()
    } else {
        format!("{}.js", specifier)
    };

    std::fs::read_to_string(&path).map_err(|e| format!("Failed to load module '{}': {}", path, e))
}

pub fn load_and_evaluate_module(ctx: &mut JSContext, specifier: &str) -> Result<usize, String> {
    use crate::compiler::ast::BlockStatement;
    use crate::compiler::codegen::CodeGenerator;
    use crate::compiler::parser::Parser;
    use crate::compiler::peephole;
    use crate::runtime::vm::VM;

    let resolved = if ctx.runtime().module_registry().has(specifier) {
        specifier.to_string()
    } else if specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
    {
        if let Some(current) = ctx.get_current_module() {
            resolve_specifier(specifier, current)
        } else if let Some(base) = ctx.get_script_base_path() {
            resolve_specifier(specifier, base)
        } else {
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string());
            resolve_specifier(specifier, &cwd)
        }
    } else {
        specifier.to_string()
    };

    let already_evaluated = {
        let registry = ctx.runtime().module_registry();
        registry
            .get(&resolved)
            .map(|m| m.borrow().state == ModuleState::Evaluated)
            .unwrap_or(false)
    };

    if already_evaluated {
        return namespace_object_for(ctx, &resolved);
    }

    let (evaluating, errored) = {
        let registry = ctx.runtime().module_registry();
        registry
            .get(&resolved)
            .map(|m| {
                let module = m.borrow();
                (
                    module.state == ModuleState::Evaluating,
                    module.state == ModuleState::Errored,
                )
            })
            .unwrap_or((false, false))
    };

    if errored {
        let registry = ctx.runtime().module_registry();
        let message = registry
            .get(&resolved)
            .and_then(|m| m.borrow().error.clone())
            .unwrap_or_else(|| format!("Module {} failed to evaluate", resolved));
        return Err(message);
    }

    if evaluating {
        return namespace_object_for(ctx, &resolved);
    }

    let is_manually_registered = {
        let registry = ctx.runtime().module_registry();
        registry
            .get(&resolved)
            .map(|m| {
                let module = m.borrow();
                !module.exports.is_empty()
            })
            .unwrap_or(false)
    };

    if is_manually_registered {
        {
            let registry = ctx.runtime_mut().module_registry_mut();
            if let Some(module_cell) = registry.get_mut(&resolved) {
                module_cell.borrow_mut().state = ModuleState::Evaluated;
            }
        }
        return namespace_object_for(ctx, &resolved);
    }

    {
        let registry = ctx.runtime().module_registry();
        if !registry.has(&resolved) {
            let source = load_module_source(&resolved)?;
            let module = Module::new(resolved.clone(), source);
            ctx.runtime_mut().module_registry_mut().register(module);
        }
    }

    let source = {
        let registry = ctx.runtime().module_registry();
        if let Some(module_cell) = registry.get(&resolved) {
            module_cell.borrow().source.clone()
        } else {
            return Err(format!("Module {} not found after registration", resolved));
        }
    };

    let ast = Parser::new(&source).parse()?;
    crate::compiler::early_errors::validate_program(&ast, true)?;

    let dependencies = collect_static_dependencies(&ast.body);
    {
        let registry = ctx.runtime_mut().module_registry_mut();
        if let Some(module_cell) = registry.get_mut(&resolved) {
            let mut module = module_cell.borrow_mut();
            module.imports.clear();
            for dependency in &dependencies {
                module.add_import(dependency.clone(), String::new(), String::new());
            }
        }
    }

    {
        let registry = ctx.runtime_mut().module_registry_mut();
        if let Some(module_cell) = registry.get_mut(&resolved) {
            module_cell.borrow_mut().state = ModuleState::Evaluating;
        }
    }

    for dep_specifier in &dependencies {
        let resolved_dep = if dep_specifier.starts_with("./") || dep_specifier.starts_with("../") {
            resolve_specifier(dep_specifier, &resolved)
        } else {
            dep_specifier.clone()
        };
        if resolved_dep == resolved {
            continue;
        }
        let dep_evaluating = {
            let registry = ctx.runtime().module_registry();
            registry
                .get(&resolved_dep)
                .map(|m| m.borrow().state == ModuleState::Evaluating)
                .unwrap_or(false)
        };
        if dep_evaluating {
            continue;
        }
        load_and_evaluate_module(ctx, &resolved_dep)?;
    }

    let old_module = ctx.get_current_module().map(|s| s.to_string());
    ctx.set_current_module(Some(resolved.clone()));

    let rb = {
        let opt_level = ctx.get_compiler_opt_level();
        let mut codegen = CodeGenerator::with_opt_level(opt_level);
        let block = BlockStatement {
            body: ast.body,
            lines: ast.lines,
        };
        let (mut rb, _) = codegen.compile_script(&block, ctx)?;
        peephole::optimize_with_level(&mut rb, opt_level);
        rb
    };

    let mut module_vm = VM::new();
    let result = module_vm.execute_preserving_registers(ctx, &rb);
    let thrown = module_vm.pending_throw.take();

    ctx.set_current_module(old_module);

    match result {
        Ok(_) => {
            {
                let registry = ctx.runtime_mut().module_registry_mut();
                if let Some(module_cell) = registry.get_mut(&resolved) {
                    module_cell.borrow_mut().state = ModuleState::Evaluated;
                }
            }
            {
                let registry = ctx.runtime_mut().module_registry_mut();
                if let Some(module_cell) = registry.get_mut(&resolved) {
                    module_cell.borrow_mut().namespace_object = None;
                }
            }
            namespace_object_for(ctx, &resolved)
        }
        Err(e) => {
            if let Some(value) = thrown {
                ctx.pending_exception = Some(value);
            }
            {
                let registry = ctx.runtime_mut().module_registry_mut();
                if let Some(module_cell) = registry.get_mut(&resolved) {
                    let mut module = module_cell.borrow_mut();
                    module.state = ModuleState::Errored;
                    module.error = Some(e.clone());
                }
            }
            Err(e)
        }
    }
}
