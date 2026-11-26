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
    pub namespace_object: Option<*mut crate::object::object::JSObject>,
    pub source: String,
    pub import_meta_object: Option<*mut crate::object::object::JSObject>,
    pub error: Option<String>,
    pub error_value: Option<JSValue>,
    pub local_exports: Vec<(String, String)>,
    pub indirect_exports: Vec<(String, String, String)>,
    pub namespace_exports: Vec<(String, String)>,
    pub star_exports: Vec<String>,
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
            error_value: None,
            local_exports: Vec::new(),
            indirect_exports: Vec::new(),
            namespace_exports: Vec::new(),
            star_exports: Vec::new(),
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
    ) -> *mut crate::object::object::JSObject {
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
        let ptr = Box::into_raw(Box::new(ns_obj));
        self.namespace_object = Some(ptr);
        ptr
    }

    pub fn get_or_create_namespace_object(
        &mut self,
        atom_table: &mut AtomTable,
        to_string_tag_key: crate::runtime::atom::Atom,
        module_tag_atom: crate::runtime::atom::Atom,
    ) -> *mut crate::object::object::JSObject {
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

fn namespace_object_for(ctx: &mut JSContext, resolved: &str) -> Result<*mut crate::object::object::JSObject, String> {
    let to_string_tag_key = crate::builtins::symbol::get_symbol_to_string_tag_prop_key(ctx);
    let module_tag_atom = ctx.intern("Module");
    let (registry, atom_table) = ctx.runtime_mut().module_registry_and_atoms_mut();
    if let Some(module_cell) = registry.get_mut(resolved) {
        Ok(module_cell.borrow_mut().get_or_create_namespace_object(
            atom_table,
            to_string_tag_key,
            module_tag_atom,
        ))
    } else {
        Err(format!("Module {} not found", resolved))
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

pub fn create_syntax_error(ctx: &mut JSContext, message: &str) -> JSValue {
    let mut err = JSObject::new_typed(crate::object::object::ObjectType::Error);
    err.set(
        ctx.common_atoms.name,
        JSValue::new_string(ctx.intern("SyntaxError")),
    );
    err.set(
        ctx.common_atoms.message,
        JSValue::new_string(ctx.intern(message)),
    );
    if let Some(proto) = ctx.get_syntax_error_prototype() {
        err.prototype = Some(proto);
    }
    let ptr = Box::into_raw(Box::new(err));
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

fn resolve_dependency_path(module: &str, source: &str) -> String {
    if source.starts_with("./") || source.starts_with("../") {
        resolve_specifier(source, module)
    } else {
        source.to_string()
    }
}

fn mark_module_errored(ctx: &mut JSContext, resolved: &str, message: &str, value: Option<JSValue>) {
    let registry = ctx.runtime_mut().module_registry_mut();
    if let Some(module_cell) = registry.get_mut(resolved) {
        let mut module = module_cell.borrow_mut();
        module.state = ModuleState::Errored;
        module.error = Some(message.to_string());
        module.error_value = value;
    }
    if let Some(value) = value {
        ctx.runtime_mut().gc_heap_mut().extra_roots.push(value);
    }
}

fn module_export_records(ctx: &JSContext, module: &str) -> Option<ModuleExportRecords> {
    ctx.runtime().module_registry().get(module).map(|cell| {
        let module = cell.borrow();
        ModuleExportRecords {
            local_exports: module.local_exports.clone(),
            indirect_exports: module.indirect_exports.clone(),
            namespace_exports: module.namespace_exports.clone(),
            star_exports: module.star_exports.clone(),
        }
    })
}

struct ModuleExportRecords {
    local_exports: Vec<(String, String)>,
    indirect_exports: Vec<(String, String, String)>,
    namespace_exports: Vec<(String, String)>,
    star_exports: Vec<String>,
}

enum ExportResolution {
    Resolved(String, String),
    Ambiguous,
    NotFound,
}

fn collect_export_records(
    body: &[crate::compiler::ast::ASTNode],
) -> (Vec<(String, String)>, Vec<(String, String, String)>, Vec<(String, String)>, Vec<String>) {
    use crate::compiler::ast::{
        ASTNode, AssignmentTarget, BindingPattern, ExportAllDeclaration, ExportDeclaration,
        ExportNamedDeclaration, ObjectPatternProperty, PatternElement,
    };

    fn collect_target_names(target: &AssignmentTarget, out: &mut Vec<String>) {
        match target {
            AssignmentTarget::Identifier(name) => out.push(name.clone()),
            AssignmentTarget::ArrayPattern(arr) => {
                for elem in arr.elements.iter().flatten() {
                    match elem {
                        PatternElement::Pattern(t) => collect_target_names(t, out),
                        PatternElement::RestElement(rest) => {
                            collect_target_names(&rest.argument, out)
                        }
                        PatternElement::AssignmentPattern(ap) => collect_target_names(&ap.left, out),
                    }
                }
            }
            AssignmentTarget::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            collect_target_names(value, out)
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            collect_target_names(&rest.argument, out)
                        }
                    }
                }
            }
            AssignmentTarget::AssignmentPattern(ap) => collect_target_names(&ap.left, out),
            AssignmentTarget::RestElement(rest) => collect_target_names(&rest.argument, out),
            _ => {}
        }
    }

    fn collect_pattern_names(pattern: &BindingPattern, out: &mut Vec<String>) {
        match pattern {
            BindingPattern::Identifier(name) => out.push(name.clone()),
            BindingPattern::ArrayPattern(arr) => {
                for elem in arr.elements.iter().flatten() {
                    match elem {
                        PatternElement::Pattern(t) => collect_target_names(t, out),
                        PatternElement::RestElement(rest) => {
                            collect_target_names(&rest.argument, out)
                        }
                        PatternElement::AssignmentPattern(ap) => collect_target_names(&ap.left, out),
                    }
                }
            }
            BindingPattern::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            collect_target_names(value, out)
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            collect_target_names(&rest.argument, out)
                        }
                    }
                }
            }
            BindingPattern::AssignmentPattern(ap) => collect_target_names(&ap.left, out),
        }
    }

    let mut local_exports: Vec<(String, String)> = Vec::new();
    let mut indirect_exports: Vec<(String, String, String)> = Vec::new();
    let mut namespace_exports: Vec<(String, String)> = Vec::new();
    let mut star_exports: Vec<String> = Vec::new();

    for node in body {
        match node {
            ASTNode::ExportNamedDeclaration(ExportNamedDeclaration {
                declaration,
                specifiers,
                source,
            }) => {
                if let Some(declaration) = declaration {
                    match declaration {
                        ExportDeclaration::Variable(variable) => {
                            for declarator in &variable.declarations {
                                let mut names = Vec::new();
                                collect_pattern_names(&declarator.id, &mut names);
                                for name in names {
                                    local_exports.push((name.clone(), name));
                                }
                            }
                        }
                        ExportDeclaration::Function(function) => {
                            local_exports
                                .push((function.name.clone(), function.name.clone()));
                        }
                        ExportDeclaration::Class(class) => {
                            local_exports.push((class.name.clone(), class.name.clone()));
                        }
                    }
                } else if let Some(source) = source {
                    for specifier in specifiers {
                        indirect_exports.push((
                            specifier.exported.clone(),
                            source.clone(),
                            specifier.local.clone(),
                        ));
                    }
                } else {
                    for specifier in specifiers {
                        local_exports
                            .push((specifier.exported.clone(), specifier.local.clone()));
                    }
                }
            }
            ASTNode::ExportDefaultDeclaration(_) => {
                local_exports.push(("default".to_string(), "default".to_string()));
            }
            ASTNode::ExportAllDeclaration(ExportAllDeclaration { source, exported }) => match exported
            {
                Some(name) => namespace_exports.push((name.clone(), source.clone())),
                None => star_exports.push(source.clone()),
            },
            _ => {}
        }
    }

    (
        local_exports,
        indirect_exports,
        namespace_exports,
        star_exports,
    )
}

fn get_exported_names(ctx: &JSContext, module: &str, export_star_set: &mut Vec<String>) -> Vec<String> {
    if export_star_set.iter().any(|m| m == module) {
        return Vec::new();
    }
    export_star_set.push(module.to_string());
    let Some(records) = module_export_records(ctx, module) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for (export_name, _) in &records.local_exports {
        if !names.contains(export_name) {
            names.push(export_name.clone());
        }
    }
    for (export_name, _, _) in &records.indirect_exports {
        if !names.contains(export_name) {
            names.push(export_name.clone());
        }
    }
    for (export_name, _) in &records.namespace_exports {
        if !names.contains(export_name) {
            names.push(export_name.clone());
        }
    }
    for source in &records.star_exports {
        let dependency = resolve_dependency_path(module, source);
        for name in get_exported_names(ctx, &dependency, export_star_set) {
            if name == "default" {
                continue;
            }
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn resolve_export(
    ctx: &JSContext,
    module: &str,
    export_name: &str,
    resolve_set: &mut Vec<(String, String)>,
) -> ExportResolution {
    if resolve_set
        .iter()
        .any(|(m, n)| m == module && n == export_name)
    {
        return ExportResolution::NotFound;
    }
    resolve_set.push((module.to_string(), export_name.to_string()));
    let Some(records) = module_export_records(ctx, module) else {
        return ExportResolution::NotFound;
    };
    for (name, local_name) in &records.local_exports {
        if name == export_name {
            return ExportResolution::Resolved(module.to_string(), local_name.clone());
        }
    }
    for (name, source, import_name) in &records.indirect_exports {
        if name == export_name {
            let dependency = resolve_dependency_path(module, source);
            return resolve_export(ctx, &dependency, import_name, resolve_set);
        }
    }
    for (name, source) in &records.namespace_exports {
        if name == export_name {
            let dependency = resolve_dependency_path(module, source);
            return ExportResolution::Resolved(dependency, "namespace".to_string());
        }
    }
    if export_name == "default" {
        return ExportResolution::NotFound;
    }
    let mut star_resolution: Option<(String, String)> = None;
    for source in &records.star_exports {
        let dependency = resolve_dependency_path(module, source);
        match resolve_export(ctx, &dependency, export_name, resolve_set) {
            ExportResolution::Ambiguous => return ExportResolution::Ambiguous,
            ExportResolution::Resolved(found_module, binding_name) => {
                match &star_resolution {
                    None => star_resolution = Some((found_module, binding_name)),
                    Some((module, name)) if *module == found_module && *name == binding_name => {}
                    Some(_) => return ExportResolution::Ambiguous,
                }
            }
            ExportResolution::NotFound => {}
        }
    }
    match star_resolution {
        Some((module, binding_name)) => ExportResolution::Resolved(module, binding_name),
        None => ExportResolution::NotFound,
    }
}

fn link_module(ctx: &mut JSContext, resolved: &str) -> Result<(), String> {
    use crate::compiler::ast::ASTNode;

    let state = ctx
        .runtime()
        .module_registry()
        .get(resolved)
        .map(|cell| cell.borrow().state);
    match state {
        Some(ModuleState::Linking)
        | Some(ModuleState::Linked)
        | Some(ModuleState::Evaluating)
        | Some(ModuleState::Evaluated) => return Ok(()),
        Some(ModuleState::Errored) => {
            let registry = ctx.runtime().module_registry();
            let message = registry
                .get(resolved)
                .and_then(|cell| cell.borrow().error.clone())
                .unwrap_or_else(|| format!("Module {} failed to link", resolved));
            return Err(message);
        }
        _ => {}
    }

    let registered = {
        let registry = ctx.runtime().module_registry();
        registry.has(resolved)
    };
    if !registered {
        let source = load_module_source(resolved)?;
        let module = Module::new(resolved.to_string(), source);
        ctx.runtime_mut().module_registry_mut().register(module);
    }

    ctx.runtime_mut()
        .module_registry_mut()
        .set_state(resolved, ModuleState::Linking);

    let source = {
        let registry = ctx.runtime().module_registry();
        registry
            .get(resolved)
            .map(|cell| cell.borrow().source.clone())
            .unwrap_or_default()
    };

    let parsed = {
        use crate::compiler::parser::Parser;
        let mut parser = Parser::new(&source);
        parser.in_async = true;
        parser.parse()
    };
    let ast = match parsed {
        Ok(ast) => ast,
        Err(message) => {
            mark_module_errored(ctx, resolved, &message, None);
            return Err(message);
        }
    };

    let (local_exports, indirect_exports, namespace_exports, star_exports) =
        collect_export_records(&ast.body);
    {
        let registry = ctx.runtime_mut().module_registry_mut();
        if let Some(module_cell) = registry.get_mut(resolved) {
            let mut module = module_cell.borrow_mut();
            module.local_exports = local_exports;
            module.indirect_exports = indirect_exports;
            module.namespace_exports = namespace_exports;
            module.star_exports = star_exports;
        }
    }

    let mut dependencies = Vec::new();
    for node in &ast.body {
        if let ASTNode::ImportDeclaration(declaration) = node {
            dependencies.push(declaration.source.clone());
        }
    }
    if let Some(records) = module_export_records(ctx, resolved) {
        for (_, source, _) in &records.indirect_exports {
            dependencies.push(source.clone());
        }
        for (_, source) in &records.namespace_exports {
            dependencies.push(source.clone());
        }
        for source in &records.star_exports {
            dependencies.push(source.clone());
        }
    }

    for dependency_source in &dependencies {
        let dependency = resolve_dependency_path(resolved, dependency_source);
        if dependency == resolved {
            continue;
        }
        if let Err(message) = link_module(ctx, &dependency) {
            let value = ctx
                .runtime()
                .module_registry()
                .get(&dependency)
                .and_then(|cell| cell.borrow().error_value);
            mark_module_errored(ctx, resolved, &message, value);
            return Err(message);
        }
    }

    let exported_names = get_exported_names(ctx, resolved, &mut Vec::new());
    for export_name in &exported_names {
        if export_name == "default" {
            continue;
        }
        match resolve_export(ctx, resolved, export_name, &mut Vec::new()) {
            ExportResolution::NotFound => {
                let message = format!(
                    "The requested module '{}' does not provide an export named '{}'",
                    resolved, export_name
                );
                let value = create_syntax_error(ctx, &message);
                mark_module_errored(ctx, resolved, &message, Some(value));
                return Err(message);
            }
            ExportResolution::Ambiguous => {
                let message = format!(
                    "The requested module '{}' contains conflicting star exports for name '{}'",
                    resolved, export_name
                );
                let value = create_syntax_error(ctx, &message);
                mark_module_errored(ctx, resolved, &message, Some(value));
                return Err(message);
            }
            ExportResolution::Resolved(..) => {}
        }
    }

    ctx.runtime_mut()
        .module_registry_mut()
        .set_state(resolved, ModuleState::Linked);
    Ok(())
}

pub fn load_and_evaluate_module(ctx: &mut JSContext, specifier: &str) -> Result<*mut crate::object::object::JSObject, String> {
    load_and_evaluate_module_with_attributes(ctx, specifier, None)
}

fn find_errored_transitive_dependency(
    ctx: &mut JSContext,
    specifier: &str,
) -> Option<(String, Option<JSValue>)> {
    let mut queue: Vec<(String, String)> = Vec::new();
    let mut visited: Vec<String> = vec![specifier.to_string()];
    {
        let registry = ctx.runtime().module_registry();
        if let Some(module_cell) = registry.get(specifier) {
            for import in &module_cell.borrow().imports {
                queue.push((import.module_specifier.clone(), specifier.to_string()));
            }
        }
    }
    let mut head = 0;
    while head < queue.len() {
        let (raw, base) = queue[head].clone();
        head += 1;
        let resolved = if raw.starts_with("./") || raw.starts_with("../") {
            resolve_specifier(&raw, &base)
        } else {
            raw
        };
        if visited.contains(&resolved) {
            continue;
        }
        visited.push(resolved.clone());
        let registry = ctx.runtime().module_registry();
        let entry = match registry.get(&resolved) {
            Some(module_cell) => {
                let module = module_cell.borrow();
                Some((
                    module.state,
                    module
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("Module {} failed to evaluate", resolved)),
                    module.error_value,
                    module.imports.clone(),
                ))
            }
            None => None,
        };
        match entry {
            Some((state, message, value, imports)) => {
                if state == ModuleState::Errored {
                    return Some((message, value));
                }
                for import in imports {
                    queue.push((import.module_specifier, resolved.clone()));
                }
            }
            None => {}
        }
    }
    None
}

pub fn load_and_evaluate_module_with_attributes(
    ctx: &mut JSContext,
    specifier: &str,
    attribute_type: Option<&str>,
) -> Result<*mut crate::object::object::JSObject, String> {
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
        if let Some((message, value)) = find_errored_transitive_dependency(ctx, &resolved) {
            if let Some(value) = value {
                ctx.pending_exception = Some(value);
            }
            return Err(message);
        }
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
        let (message, value) = {
            let registry = ctx.runtime().module_registry();
            registry
                .get(&resolved)
                .map(|m| {
                    let module = m.borrow();
                    (
                        module
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("Module {} failed to evaluate", resolved)),
                        module.error_value,
                    )
                })
                .unwrap_or_else(|| (format!("Module {} failed to evaluate", resolved), None))
        };
        if let Some(value) = value {
            ctx.pending_exception = Some(value);
        }
        return Err(message);
    }

    if evaluating {
        return namespace_object_for(ctx, &resolved);
    }

    if attribute_type == Some("text") {
        let registered = ctx.runtime().module_registry().has(&resolved);
        if !registered {
            let text = std::fs::read_to_string(&resolved)
                .map_err(|e| format!("Failed to load module '{}': {}", resolved, e))?;
            let module = Module::new(resolved.clone(), text);
            ctx.runtime_mut().module_registry_mut().register(module);
        }
        let source = {
            let registry = ctx.runtime().module_registry();
            registry
                .get(&resolved)
                .map(|cell| cell.borrow().source.clone())
                .unwrap_or_default()
        };
        let default_value = JSValue::new_string(ctx.intern(&source));
        {
            let registry = ctx.runtime_mut().module_registry_mut();
            if let Some(module_cell) = registry.get_mut(&resolved) {
                let mut module = module_cell.borrow_mut();
                if module.exports.is_empty() {
                    module.add_export("default".to_string(), default_value, false);
                }
                module.state = ModuleState::Evaluated;
            }
        }
        return namespace_object_for(ctx, &resolved);
    }

    if attribute_type == Some("json") {
        let registered = ctx.runtime().module_registry().has(&resolved);
        if !registered {
            let text = std::fs::read_to_string(&resolved)
                .map_err(|e| format!("Failed to load module '{}': {}", resolved, e))?;
            let value =
                match crate::builtins::json_parser::JsonParser::new(&text).parse_root(ctx) {
                    Ok(value) => value,
                    Err(message) => {
                        let error = create_syntax_error(ctx, &message);
                        ctx.pending_exception = Some(error);
                        return Err(message);
                    }
                };
            ctx.runtime_mut().gc_heap_mut().extra_roots.push(value);
            let module = Module::new(resolved.clone(), text);
            ctx.runtime_mut().module_registry_mut().register(module);
            {
                let registry = ctx.runtime_mut().module_registry_mut();
                if let Some(module_cell) = registry.get_mut(&resolved) {
                    let mut module = module_cell.borrow_mut();
                    module.add_export("default".to_string(), value, false);
                }
            }
        }
        {
            let registry = ctx.runtime_mut().module_registry_mut();
            if let Some(module_cell) = registry.get_mut(&resolved) {
                module_cell.borrow_mut().state = ModuleState::Evaluated;
            }
        }
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

    if let Err(message) = link_module(ctx, &resolved) {
        let value = {
            let registry = ctx.runtime().module_registry();
            registry
                .get(&resolved)
                .and_then(|m| m.borrow().error_value)
        };
        if let Some(value) = value {
            ctx.pending_exception = Some(value);
        }
        return Err(message);
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

    let mut module_parser = Parser::new(&source);
    module_parser.in_async = true;
    let ast = module_parser.parse()?;
    crate::compiler::early_errors::validate_program_with_source_type(&ast, true, false, false, true)?;

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
        let (mut rb, _) = codegen.compile_module(&block, ctx)?;
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
                ctx.runtime_mut().gc_heap_mut().extra_roots.push(value);
            }
            {
                let registry = ctx.runtime_mut().module_registry_mut();
                if let Some(module_cell) = registry.get_mut(&resolved) {
                    let mut module = module_cell.borrow_mut();
                    module.state = ModuleState::Errored;
                    module.error = Some(e.clone());
                    module.error_value = thrown;
                }
            }
            Err(e)
        }
    }
}
