pub mod ast;
pub mod codegen;
pub mod early_errors;
pub mod ic;
pub mod lexer;
pub mod location;
pub mod mir;
pub mod opcode;
pub mod parser;
pub mod peephole;

pub use codegen::{CodeGenerator, OptLevel};
pub use ic::{InlineCache, InlineCacheTable};
pub use lexer::{Lexer, Token, TokenType};
pub use location::{FrameInfo, LineNumberTable, SourceLocation, TracebackFormatter};
pub use opcode::{Bytecode, Opcode};

use crate::builtins::promise::run_microtasks_with_vm;
use crate::runtime::JSContext;
use crate::runtime::event_loop::EventLoopResult;
use crate::value::JSValue;

pub fn eval_code(ctx: &mut JSContext, source: &str) -> Result<JSValue, String> {
    eval_code_via_ast_with_opt_level(ctx, source, ctx.get_compiler_opt_level())
}

pub fn eval_code_via_ast(ctx: &mut JSContext, source: &str) -> Result<JSValue, String> {
    eval_code_via_ast_with_opt_level(ctx, source, ctx.get_compiler_opt_level())
}

fn program_has_use_strict_directive(program: &ast::Program) -> bool {
    for stmt in &program.body {
        match stmt {
            ast::ASTNode::ExpressionStatement(es) => {
                if let ast::Expression::Literal(ast::Literal::String(s, has_escape, _)) =
                    &es.expression
                {
                    if !has_escape && s == "use strict" {
                        return true;
                    }
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    false
}

fn collect_var_declared_names(stmt: &ast::ASTNode, out: &mut Vec<String>) {
    match stmt {
        ast::ASTNode::VariableDeclaration(decl) => {
            if decl.kind == ast::VariableKind::Var {
                for d in &decl.declarations {
                    collect_binding_pattern_names(&d.id, out);
                }
            }
        }
        ast::ASTNode::FunctionDeclaration(func) => out.push(func.name.clone()),
        ast::ASTNode::BlockStatement(block) => {
            for stmt in &block.body {
                collect_var_declared_names(stmt, out);
            }
        }
        ast::ASTNode::IfStatement(if_stmt) => {
            collect_var_declared_names(&if_stmt.consequent, out);
            if let Some(alternate) = &if_stmt.alternate {
                collect_var_declared_names(alternate, out);
            }
        }
        ast::ASTNode::ForStatement(for_stmt) => {
            if let Some(ast::ForInit::VariableDeclaration(decl)) = &for_stmt.init {
                if decl.kind == ast::VariableKind::Var {
                    for d in &decl.declarations {
                        collect_binding_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_declared_names(&for_stmt.body, out);
        }
        ast::ASTNode::ForInStatement(for_stmt) => {
            if let ast::ForInOfLeft::VariableDeclaration(decl) = &for_stmt.left {
                if decl.kind == ast::VariableKind::Var {
                    for d in &decl.declarations {
                        collect_binding_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_declared_names(&for_stmt.body, out);
        }
        ast::ASTNode::ForOfStatement(for_stmt) => {
            if let ast::ForInOfLeft::VariableDeclaration(decl) = &for_stmt.left {
                if decl.kind == ast::VariableKind::Var {
                    for d in &decl.declarations {
                        collect_binding_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_declared_names(&for_stmt.body, out);
        }
        ast::ASTNode::WhileStatement(while_stmt) => {
            collect_var_declared_names(&while_stmt.body, out);
        }
        ast::ASTNode::DoWhileStatement(do_stmt) => {
            collect_var_declared_names(&do_stmt.body, out);
        }
        ast::ASTNode::SwitchStatement(switch_stmt) => {
            for case in &switch_stmt.cases {
                for stmt in &case.consequent {
                    collect_var_declared_names(stmt, out);
                }
            }
        }
        ast::ASTNode::TryStatement(try_stmt) => {
            for stmt in &try_stmt.block.body {
                collect_var_declared_names(stmt, out);
            }
            if let Some(handler) = &try_stmt.handler {
                for stmt in &handler.body.body {
                    collect_var_declared_names(stmt, out);
                }
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                for stmt in &finalizer.body {
                    collect_var_declared_names(stmt, out);
                }
            }
        }
        ast::ASTNode::WithStatement(with_stmt) => {
            collect_var_declared_names(&with_stmt.body, out);
        }
        ast::ASTNode::LabelledStatement(labelled) => {
            collect_var_declared_names(&labelled.body, out);
        }
        ast::ASTNode::ExportNamedDeclaration(export) => {
            if let Some(ast::ExportDeclaration::Variable(decl)) = &export.declaration {
                if decl.kind == ast::VariableKind::Var {
                    for d in &decl.declarations {
                        collect_binding_pattern_names(&d.id, out);
                    }
                }
            } else if let Some(ast::ExportDeclaration::Function(func)) = &export.declaration {
                out.push(func.name.clone());
            }
        }
        _ => {}
    }
}

fn collect_binding_pattern_names(pattern: &ast::BindingPattern, out: &mut Vec<String>) {
    match pattern {
        ast::BindingPattern::Identifier(name) => out.push(name.clone()),
        ast::BindingPattern::ArrayPattern(array) => {
            for element in &array.elements {
                if let Some(element) = element {
                    match element {
                        ast::PatternElement::Pattern(target) => {
                            collect_assignment_target_names(target, out)
                        }
                        ast::PatternElement::RestElement(rest) => {
                            collect_assignment_target_names(&rest.argument, out)
                        }
                        ast::PatternElement::AssignmentPattern(assignment) => {
                            collect_assignment_target_names(&assignment.left, out)
                        }
                    }
                }
            }
        }
        ast::BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                match property {
                    ast::ObjectPatternProperty::Property { value, .. } => {
                        collect_assignment_target_names(value, out)
                    }
                    ast::ObjectPatternProperty::RestElement(rest) => {
                        collect_assignment_target_names(&rest.argument, out)
                    }
                }
            }
        }
        ast::BindingPattern::AssignmentPattern(assignment) => {
            collect_assignment_target_names(&assignment.left, out)
        }
    }
}

fn collect_assignment_target_names(target: &ast::AssignmentTarget, out: &mut Vec<String>) {
    match target {
        ast::AssignmentTarget::Identifier(name) => out.push(name.clone()),
        ast::AssignmentTarget::ArrayPattern(array) => {
            for element in &array.elements {
                if let Some(element) = element {
                    match element {
                        ast::PatternElement::Pattern(target) => {
                            collect_assignment_target_names(target, out)
                        }
                        ast::PatternElement::RestElement(rest) => {
                            collect_assignment_target_names(&rest.argument, out)
                        }
                        ast::PatternElement::AssignmentPattern(assignment) => {
                            collect_assignment_target_names(&assignment.left, out)
                        }
                    }
                }
            }
        }
        ast::AssignmentTarget::ObjectPattern(object) => {
            for property in &object.properties {
                match property {
                    ast::ObjectPatternProperty::Property { value, .. } => {
                        collect_assignment_target_names(value, out)
                    }
                    ast::ObjectPatternProperty::RestElement(rest) => {
                        collect_assignment_target_names(&rest.argument, out)
                    }
                }
            }
        }
        _ => {}
    }
}

pub fn eval_source_var_names(source: &str) -> Option<Vec<String>> {
    let mut parser = parser::Parser::new(source);
    let Ok(program) = parser.parse() else {
        return None;
    };
    if program_has_use_strict_directive(&program) {
        return None;
    }
    let mut names = Vec::new();
    for stmt in &program.body {
        collect_var_declared_names(stmt, &mut names);
    }
    Some(names)
}

pub fn eval_source_declares_var(source: &str, name: &str) -> bool {
    eval_source_var_names(source)
        .map_or(false, |names| names.iter().any(|declared| declared == name))
}

pub fn compile_script_to_bytecode_with_opt_level(
    ctx: &mut JSContext,
    source: &str,
    opt_level: OptLevel,
    is_strict: bool,
    is_eval: bool,
) -> Result<Bytecode, String> {
    compile_script_to_bytecode_with_opt_level_and_super(
        ctx, source, opt_level, is_strict, is_eval, false, false, false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compile_script_to_bytecode_with_opt_level_and_super(
    ctx: &mut JSContext,
    source: &str,
    opt_level: OptLevel,
    is_strict: bool,
    is_eval: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    in_field_initializer: bool,
) -> Result<Bytecode, String> {
    let ast = parser::Parser::new(source).parse()?;
    early_errors::validate_program_with_field_initializer(
        &ast,
        is_strict,
        allow_super_property,
        allow_super_call,
        in_field_initializer,
    )?;

    let mut func_codegen = CodeGenerator::with_opt_level(opt_level);
    func_codegen.is_strict = is_strict;
    func_codegen.is_eval = is_eval;
    func_codegen.in_field_initializer = in_field_initializer;
    let block = ast::BlockStatement {
        body: ast.body,
        lines: ast.lines,
    };
    let (mut rb, _) = func_codegen.compile_script(&block, ctx)?;

    peephole::optimize_with_level(&mut rb, opt_level);

    Ok(rb)
}

fn eval_bytecode_with_caller(
    ctx: &mut JSContext,
    rb: &Bytecode,
    caller_vm: Option<*mut crate::runtime::vm::VM>,
) -> Result<JSValue, String> {
    use crate::runtime::vm::{ExecutionOutcome, VM};

    let saved_register_vm_ptr = ctx.get_register_vm_ptr();

    let mut vm = VM::new();
    let vm_ptr = &mut vm as *mut _ as usize;
    ctx.set_register_vm_ptr(Some(vm_ptr));

    let caller_ptr = caller_vm.map(|p| p as usize);

    let caller_this = if let Some(ptr) = caller_vm {
        let caller = unsafe { &*ptr };
        let caller_frame = &caller.frames[caller.frame_index];
        let mut this_val = caller_frame.this_value;
        if !caller_frame.is_strict_frame && (this_val.is_undefined() || this_val.is_null()) {
            this_val = ctx.global();
        }
        this_val
    } else {
        ctx.global()
    };

    let result = match vm.execute_eval(ctx, rb, caller_this, caller_ptr) {
        Ok(ExecutionOutcome::Complete(v)) => Ok(v),
        Ok(ExecutionOutcome::Yield(v)) => Ok(v),
        Err(e) => Err(e),
    };

    run_microtasks_with_vm(ctx, &mut vm);

    ctx.set_register_vm_ptr(saved_register_vm_ptr);

    result
}

pub fn eval_code_via_ast_with_opt_level_as_eval_with_caller(
    ctx: &mut JSContext,
    source: &str,
    opt_level: OptLevel,
    is_strict: bool,
    caller_vm: *mut crate::runtime::vm::VM,
    allow_super_property: bool,
    allow_super_call: bool,
    in_field_initializer: bool,
) -> Result<JSValue, String> {
    let rb = compile_script_to_bytecode_with_opt_level_and_super(
        ctx,
        source,
        opt_level,
        is_strict,
        true,
        allow_super_property,
        allow_super_call,
        in_field_initializer,
    )?;
    eval_bytecode_with_caller(ctx, &rb, Some(caller_vm))
}

pub fn eval_code_via_ast_with_opt_level(
    ctx: &mut JSContext,
    source: &str,
    opt_level: OptLevel,
) -> Result<JSValue, String> {
    let rb = compile_script_to_bytecode_with_opt_level(ctx, source, opt_level, false, false)?;

    let saved_register_vm_ptr = ctx.get_register_vm_ptr();

    let mut vm = crate::runtime::vm::VM::new();
    let vm_ptr = &mut vm as *mut _ as usize;
    ctx.set_register_vm_ptr(Some(vm_ptr));

    let result = match vm.execute_preserving_registers(ctx, &rb) {
        Ok(crate::runtime::vm::ExecutionOutcome::Complete(v)) => Ok(v),
        Ok(crate::runtime::vm::ExecutionOutcome::Yield(v)) => Ok(v),
        Err(e) => Err(e),
    };

    run_microtasks_with_vm(ctx, &mut vm);

    ctx.set_register_vm_ptr(saved_register_vm_ptr);

    result
}

fn eval_exception_from_message(ctx: &mut JSContext, message: &str) -> JSValue {
    let trimmed = message.strip_prefix("Uncaught: ").unwrap_or(message);
    let label = trimmed.split(':').next().unwrap_or("");
    let name = match label {
        "TypeError" | "SyntaxError" | "ReferenceError" | "RangeError" | "URIError"
        | "EvalError" => label,
        _ => "Error",
    };
    let mut err = crate::object::object::JSObject::new();
    err.set(ctx.intern("name"), JSValue::new_string(ctx.intern(name)));
    err.set(
        ctx.intern("message"),
        JSValue::new_string(ctx.intern(trimmed)),
    );
    let proto = match name {
        "TypeError" => ctx.get_type_error_prototype(),
        "SyntaxError" => ctx.get_syntax_error_prototype(),
        "ReferenceError" => ctx.get_reference_error_prototype(),
        "RangeError" => ctx.get_range_error_prototype(),
        "URIError" => ctx.get_uri_error_prototype(),
        "EvalError" => ctx.get_eval_error_prototype(),
        _ => ctx.get_error_prototype(),
    };
    if let Some(proto) = proto {
        err.prototype = Some(proto);
    }
    let ptr = Box::into_raw(Box::new(err)) as usize;
    ctx.runtime_mut().gc_heap_mut().track(ptr);
    JSValue::new_object(ptr)
}

pub fn eval_code_with_exception(
    ctx: &mut JSContext,
    source: &str,
) -> Result<JSValue, JSValue> {
    let opt_level = ctx.get_compiler_opt_level();
    let rb = match compile_script_to_bytecode_with_opt_level(ctx, source, opt_level, false, false) {
        Ok(rb) => rb,
        Err(e) => return Err(eval_exception_from_message(ctx, &e)),
    };

    let saved_register_vm_ptr = ctx.get_register_vm_ptr();

    let mut vm = crate::runtime::vm::VM::new();
    let vm_ptr = &mut vm as *mut _ as usize;
    ctx.set_register_vm_ptr(Some(vm_ptr));

    let result = match vm.execute_preserving_registers(ctx, &rb) {
        Ok(crate::runtime::vm::ExecutionOutcome::Complete(v)) => Ok(v),
        Ok(crate::runtime::vm::ExecutionOutcome::Yield(v)) => Ok(v),
        Err(e) => Err(vm
            .take_pending_exception(ctx)
            .unwrap_or_else(|| eval_exception_from_message(ctx, &e))),
    };

    run_microtasks_with_vm(ctx, &mut vm);

    ctx.set_register_vm_ptr(saved_register_vm_ptr);

    result
}

pub fn run_event_loop(ctx: &mut JSContext) -> Result<EventLoopResult, String> {
    ctx.run_event_loop()
}

pub fn run_event_loop_with_timeout(
    ctx: &mut JSContext,
    timeout_ms: u64,
) -> Result<EventLoopResult, String> {
    ctx.run_event_loop_with_timeout(timeout_ms)
}
