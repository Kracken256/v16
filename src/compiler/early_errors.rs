use crate::compiler::ast::*;
use std::collections::HashSet;

/// Parsing context used for early-error validation. `strict` tracks strict mode,
/// while `in_async` / `in_generator` mark the enclosing function kind so that
/// `await` and `yield` can be rejected where they are reserved.
#[derive(Clone, Copy)]
struct Ctx {
    strict: bool,
    in_async: bool,
    in_generator: bool,
    in_static_block: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    in_function: bool,
    in_field_initializer: bool,
}

impl Ctx {
    fn script(strict: bool) -> Self {
        Ctx {
            strict,
            in_async: false,
            in_generator: false,
            in_static_block: false,
            allow_super_property: false,
            allow_super_call: false,
            in_function: false,
            in_field_initializer: false,
        }
    }

    fn with_strict(self, strict: bool) -> Self {
        Ctx {
            strict: self.strict || strict,
            ..self
        }
    }
}

/// Build the context for a function body. `is_class_method` forces strict mode.
fn function_ctx(
    outer: Ctx,
    is_async: bool,
    generator: bool,
    body: &[ASTNode],
    is_class_method: bool,
    allow_super_property: bool,
    allow_super_call: bool,
) -> Ctx {
    let strict = outer.strict || is_class_method || has_use_strict_directive(body);
    Ctx {
        strict,
        in_async: is_async,
        in_generator: generator,
        in_static_block: false,
        allow_super_property,
        allow_super_call,
        in_function: true,
        in_field_initializer: false,
    }
}

fn is_strict_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
    )
}

fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

fn check_binding_identifier(name: &str, ctx: Ctx) -> Result<(), String> {
    if is_reserved_word(name) {
        return Err(format!("SyntaxError: '{}' is a reserved word", name));
    }
    if ctx.in_async && name == "await" {
        return Err(
            "SyntaxError: 'await' is not allowed as a binding identifier in async functions"
                .to_string(),
        );
    }
    if ctx.in_generator && name == "yield" {
        return Err(
            "SyntaxError: 'yield' is not allowed as a binding identifier in generator functions"
                .to_string(),
        );
    }
    if ctx.strict {
        if is_strict_reserved_word(name) {
            return Err(format!(
                "SyntaxError: '{}' is a reserved word in strict mode",
                name
            ));
        }
        if name == "eval" || name == "arguments" {
            return Err(format!(
                "SyntaxError: '{}' cannot be used as a binding identifier in strict mode",
                name
            ));
        }
    }
    Ok(())
}

fn check_identifier_reference(name: &str, ctx: Ctx) -> Result<(), String> {
    if is_reserved_word(name) {
        return Err(format!("SyntaxError: '{}' is a reserved word", name));
    }
    if ctx.in_async && name == "await" {
        return Err(
            "SyntaxError: 'await' is not allowed as an identifier reference in async functions"
                .to_string(),
        );
    }
    if ctx.in_generator && name == "yield" {
        return Err(
            "SyntaxError: 'yield' is not allowed as an identifier reference in generator functions"
                .to_string(),
        );
    }
    if ctx.strict && is_strict_reserved_word(name) {
        return Err(format!(
            "SyntaxError: '{}' is a reserved word in strict mode",
            name
        ));
    }
    if ctx.in_field_initializer && name == "arguments" {
        return Err(
            "SyntaxError: 'arguments' is not allowed in class field initializers".to_string(),
        );
    }
    Ok(())
}

fn check_label_identifier(name: &str, ctx: Ctx) -> Result<(), String> {
    if is_reserved_word(name) {
        return Err(format!("SyntaxError: '{}' is a reserved word", name));
    }
    if ctx.in_async && name == "await" {
        return Err(
            "SyntaxError: 'await' is not allowed as a label in async functions".to_string(),
        );
    }
    if ctx.in_generator && name == "yield" {
        return Err(
            "SyntaxError: 'yield' is not allowed as a label in generator functions".to_string(),
        );
    }
    if ctx.strict && is_strict_reserved_word(name) {
        return Err(format!(
            "SyntaxError: '{}' is a reserved word in strict mode",
            name
        ));
    }
    Ok(())
}

pub fn validate_program(
    program: &Program,
    is_strict: bool,
    allow_super_property: bool,
    allow_super_call: bool,
) -> Result<(), String> {
    validate_program_with_source_type(
        program,
        is_strict,
        allow_super_property,
        allow_super_call,
        program.source_type == SourceType::Module,
    )
}

pub fn validate_program_with_source_type(
    program: &Program,
    is_strict: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    is_module: bool,
) -> Result<(), String> {
    validate_program_with_context(
        program,
        is_strict,
        allow_super_property,
        allow_super_call,
        is_module,
        false,
    )
}

pub fn validate_program_with_field_initializer(
    program: &Program,
    is_strict: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    in_field_initializer: bool,
) -> Result<(), String> {
    validate_program_with_context(
        program,
        is_strict,
        allow_super_property,
        allow_super_call,
        program.source_type == SourceType::Module,
        in_field_initializer,
    )
}

fn validate_program_with_context(
    program: &Program,
    is_strict: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    is_module: bool,
    in_field_initializer: bool,
) -> Result<(), String> {
    if !is_module {
        for stmt in &program.body {
            if matches!(
                stmt,
                ASTNode::ImportDeclaration(_)
                    | ASTNode::ExportNamedDeclaration(_)
                    | ASTNode::ExportDefaultDeclaration(_)
                    | ASTNode::ExportAllDeclaration(_)
            ) {
                return Err(
                    "SyntaxError: 'import' and 'export' may only appear in a module".to_string(),
                );
            }
            if let ASTNode::VariableDeclaration(vd) = stmt {
                if vd.kind.is_using() {
                    return Err(
                        "SyntaxError: 'using' declarations are not allowed at the top level of a script"
                            .to_string(),
                    );
                }
            }
        }
    }
    let strict = is_strict || program.source_type == SourceType::Module;
    let ctx = Ctx {
        strict,
        in_async: program.source_type == SourceType::Module,
        in_generator: false,
        in_static_block: false,
        allow_super_property,
        allow_super_call,
        in_function: allow_super_property || allow_super_call || in_field_initializer,
        in_field_initializer,
    };
    validate_body(&program.body, ctx)
}

fn validate_body(body: &[ASTNode], outer: Ctx) -> Result<(), String> {
    let ctx = outer.with_strict(has_use_strict_directive(body));
    check_top_level_declarations(body, ctx.strict)?;
    for stmt in body {
        validate_statement(stmt, ctx)?;
    }
    Ok(())
}

fn validate_block(block: &BlockStatement, ctx: Ctx) -> Result<(), String> {
    check_block_declarations(&block.body, ctx.strict)?;
    for stmt in &block.body {
        validate_statement(stmt, ctx)?;
    }
    Ok(())
}

fn has_use_strict_directive(body: &[ASTNode]) -> bool {
    for stmt in body {
        match stmt {
            ASTNode::ExpressionStatement(es) => {
                if let Expression::Literal(Literal::String(s, has_escape, _)) = &es.expression {
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

fn redeclaration_error(name: &str) -> String {
    format!("SyntaxError: Identifier '{}' has already been declared", name)
}

fn check_lexical_against_lexical(lexical: &[(String, bool)], strict: bool) -> Result<(), String> {
    for i in 0..lexical.len() {
        for j in (i + 1)..lexical.len() {
            let (n1, f1) = &lexical[i];
            let (n2, f2) = &lexical[j];
            if n1 == n2 {
                if !strict && *f1 && *f2 {
                    continue;
                }
                return Err(redeclaration_error(n1));
            }
        }
    }
    Ok(())
}

fn check_block_declarations(stmts: &[ASTNode], strict: bool) -> Result<(), String> {
    let mut lexical: Vec<(String, bool)> = Vec::new();
    for stmt in stmts {
        collect_lexical_names(stmt, false, &mut lexical);
    }
    check_lexical_against_lexical(&lexical, strict)?;

    let mut vars: HashSet<String> = HashSet::new();
    for stmt in stmts {
        collect_var_names(stmt, false, &mut vars);
    }
    for (name, _) in &lexical {
        if vars.contains(name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn check_top_level_declarations(stmts: &[ASTNode], strict: bool) -> Result<(), String> {
    let mut lexical: Vec<(String, bool)> = Vec::new();
    for stmt in stmts {
        collect_lexical_names(stmt, true, &mut lexical);
    }
    check_lexical_against_lexical(&lexical, strict)?;

    let mut vars: HashSet<String> = HashSet::new();
    for stmt in stmts {
        collect_var_names(stmt, true, &mut vars);
    }
    for (name, _) in &lexical {
        if vars.contains(name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn check_case_block(cases: &[SwitchCase], strict: bool) -> Result<(), String> {
    let mut lexical: Vec<(String, bool)> = Vec::new();
    let mut vars: HashSet<String> = HashSet::new();
    for case in cases {
        for stmt in &case.consequent {
            collect_lexical_names(stmt, false, &mut lexical);
            collect_var_names(stmt, false, &mut vars);
        }
    }
    check_lexical_against_lexical(&lexical, strict)?;
    for (name, _) in &lexical {
        if vars.contains(name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn collect_lexical_names(stmt: &ASTNode, top_level: bool, out: &mut Vec<(String, bool)>) {
    match stmt {
        ASTNode::VariableDeclaration(vd) => {
            if vd.kind != VariableKind::Var {
                for d in &vd.declarations {
                    collect_binding_pattern_names(&d.id, out);
                }
            }
        }
        ASTNode::ClassDeclaration(cd) => out.push((cd.name.clone(), false)),
        ASTNode::FunctionDeclaration(fd) => {
            if !top_level {
                out.push((fd.name.clone(), !fd.generator && !fd.is_async))
            }
        }
        ASTNode::ExportNamedDeclaration(ed) => {
            if let Some(decl) = &ed.declaration {
                match decl {
                    ExportDeclaration::Variable(vd) => {
                        if vd.kind != VariableKind::Var {
                            for d in &vd.declarations {
                                collect_binding_pattern_names(&d.id, out);
                            }
                        }
                    }
                    ExportDeclaration::Function(fd) => {
                        if !top_level {
                            out.push((fd.name.clone(), !fd.generator && !fd.is_async))
                        }
                    }
                    ExportDeclaration::Class(cd) => out.push((cd.name.clone(), false)),
                }
            }
        }
        ASTNode::ExportDefaultDeclaration(ed) => match &ed.declaration {
            ExportDefaultDeclarationKind::Function(fd) => {
                if !top_level && !fd.name.is_empty() {
                    out.push((fd.name.clone(), !fd.generator && !fd.is_async))
                }
            }
            ExportDefaultDeclarationKind::Class(cd) => {
                if !cd.name.is_empty() {
                    out.push((cd.name.clone(), false))
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn collect_binding_pattern_names(p: &BindingPattern, out: &mut Vec<(String, bool)>) {
    match p {
        BindingPattern::Identifier(n) => out.push((n.clone(), false)),
        BindingPattern::ArrayPattern(ap) => {
            for el in &ap.elements {
                if let Some(e) = el {
                    collect_pattern_element_names(e, out);
                }
            }
        }
        BindingPattern::ObjectPattern(op) => {
            for prop in &op.properties {
                match prop {
                    ObjectPatternProperty::Property { value, .. } => {
                        collect_assignment_target_names(value, out)
                    }
                    ObjectPatternProperty::RestElement(rest) => {
                        collect_assignment_target_names(&rest.argument, out)
                    }
                }
            }
        }
        BindingPattern::AssignmentPattern(ap) => collect_assignment_target_names(&ap.left, out),
    }
}

fn collect_pattern_element_names(e: &PatternElement, out: &mut Vec<(String, bool)>) {
    match e {
        PatternElement::Pattern(t) => collect_assignment_target_names(t, out),
        PatternElement::RestElement(rest) => collect_assignment_target_names(&rest.argument, out),
        PatternElement::AssignmentPattern(ap) => collect_assignment_target_names(&ap.left, out),
    }
}

fn collect_assignment_target_names(t: &AssignmentTarget, out: &mut Vec<(String, bool)>) {
    match t {
        AssignmentTarget::Identifier(n) => out.push((n.clone(), false)),
        AssignmentTarget::ObjectPattern(op) => {
            for prop in &op.properties {
                match prop {
                    ObjectPatternProperty::Property { value, .. } => {
                        collect_assignment_target_names(value, out)
                    }
                    ObjectPatternProperty::RestElement(rest) => {
                        collect_assignment_target_names(&rest.argument, out)
                    }
                }
            }
        }
        AssignmentTarget::ArrayPattern(ap) => {
            for el in &ap.elements {
                if let Some(e) = el {
                    collect_pattern_element_names(e, out);
                }
            }
        }
        AssignmentTarget::RestElement(rest) => collect_assignment_target_names(&rest.argument, out),
        AssignmentTarget::AssignmentPattern(ap) => collect_assignment_target_names(&ap.left, out),
        _ => {}
    }
}

fn collect_binding_pattern_names_set(p: &BindingPattern, out: &mut HashSet<String>) {
    let mut names = Vec::new();
    collect_binding_pattern_names(p, &mut names);
    for (n, _) in names {
        out.insert(n);
    }
}


fn collect_var_names(stmt: &ASTNode, top_level: bool, out: &mut HashSet<String>) {
    match stmt {
        ASTNode::FunctionDeclaration(fd) => {
            if top_level {
                out.insert(fd.name.clone());
            }
        }
        ASTNode::VariableDeclaration(vd) => {
            if vd.kind == VariableKind::Var {
                for d in &vd.declarations {
                    collect_binding_pattern_names_set(&d.id, out);
                }
            }
        }
        ASTNode::BlockStatement(b) => {
            for s in &b.body {
                collect_var_names(s, false, out);
            }
        }
        ASTNode::IfStatement(i) => {
            collect_var_names(&i.consequent, false, out);
            if let Some(alt) = &i.alternate {
                collect_var_names(alt, false, out);
            }
        }
        ASTNode::ForStatement(f) => {
            if let Some(ForInit::VariableDeclaration(vd)) = &f.init {
                if vd.kind == VariableKind::Var {
                    for d in &vd.declarations {
                        collect_binding_pattern_names_set(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, false, out);
        }
        ASTNode::ForInStatement(f) => {
            if let ForInOfLeft::VariableDeclaration(vd) = &f.left {
                if vd.kind == VariableKind::Var {
                    for d in &vd.declarations {
                        collect_binding_pattern_names_set(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, false, out);
        }
        ASTNode::ForOfStatement(f) => {
            if let ForInOfLeft::VariableDeclaration(vd) = &f.left {
                if vd.kind == VariableKind::Var {
                    for d in &vd.declarations {
                        collect_binding_pattern_names_set(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, false, out);
        }
        ASTNode::WhileStatement(w) => collect_var_names(&w.body, false, out),
        ASTNode::DoWhileStatement(d) => collect_var_names(&d.body, false, out),
        ASTNode::SwitchStatement(s) => {
            for c in &s.cases {
                for st in &c.consequent {
                    collect_var_names(st, false, out);
                }
            }
        }
        ASTNode::TryStatement(t) => {
            for s in &t.block.body {
                collect_var_names(s, false, out);
            }
            if let Some(h) = &t.handler {
                for s in &h.body.body {
                    collect_var_names(s, false, out);
                }
            }
            if let Some(fin) = &t.finalizer {
                for s in &fin.body {
                    collect_var_names(s, false, out);
                }
            }
        }
        ASTNode::WithStatement(w) => collect_var_names(&w.body, false, out),
        ASTNode::LabelledStatement(l) => collect_var_names(&l.body, false, out),
        ASTNode::ExportNamedDeclaration(ed) => match &ed.declaration {
            Some(ExportDeclaration::Variable(vd)) => {
                if vd.kind == VariableKind::Var {
                    for d in &vd.declarations {
                        collect_binding_pattern_names_set(&d.id, out);
                    }
                }
            }
            Some(ExportDeclaration::Function(fd)) => {
                if top_level {
                    out.insert(fd.name.clone());
                }
            }
            _ => {}
        },
        ASTNode::ExportDefaultDeclaration(ed) => {
            if top_level {
                if let ExportDefaultDeclarationKind::Function(fd) = &ed.declaration {
                    if !fd.name.is_empty() {
                        out.insert(fd.name.clone());
                    }
                }
            }
        }
        _ => {}
    }
}

fn validate_statement(stmt: &ASTNode, ctx: Ctx) -> Result<(), String> {
    match stmt {
        ASTNode::BlockStatement(b) => validate_block(b, ctx),
        ASTNode::IfStatement(i) => {
            validate_expression(&i.test, ctx)?;
            validate_single_statement(&i.consequent, ctx, true)?;
            if let Some(alt) = &i.alternate {
                validate_single_statement(alt, ctx, true)?;
            }
            Ok(())
        }
        ASTNode::ForStatement(f) => {
            match &f.init {
                Some(ForInit::VariableDeclaration(vd)) => {
                    validate_variable_declaration(vd, ctx, true)?;
                    if vd.kind != VariableKind::Var {
                        check_lexical_head_against_body(vd, &f.body)?;
                    }
                }
                Some(ForInit::Expression(e)) => validate_expression(e, ctx)?,
                None => {}
            }
            if let Some(t) = &f.test {
                validate_expression(t, ctx)?;
            }
            if let Some(u) = &f.update {
                validate_expression(u, ctx)?;
            }
            validate_single_statement(&f.body, ctx, false)
        }
        ASTNode::ForInStatement(f) => {
            validate_for_in_of(&f.left, &f.body, ctx)?;
            validate_expression(&f.right, ctx)?;
            validate_single_statement(&f.body, ctx, false)
        }
        ASTNode::ForOfStatement(f) => {
            validate_for_in_of(&f.left, &f.body, ctx)?;
            validate_expression(&f.right, ctx)?;
            validate_single_statement(&f.body, ctx, false)
        }
        ASTNode::WhileStatement(w) => {
            validate_expression(&w.test, ctx)?;
            validate_single_statement(&w.body, ctx, false)
        }
        ASTNode::DoWhileStatement(d) => {
            validate_expression(&d.test, ctx)?;
            validate_single_statement(&d.body, ctx, false)
        }
        ASTNode::WithStatement(w) => {
            if ctx.strict {
                return Err(
                    "SyntaxError: Strict mode code may not include a with statement".to_string()
                );
            }
            validate_expression(&w.object, ctx)?;
            validate_single_statement(&w.body, ctx, false)
        }
        ASTNode::LabelledStatement(l) => {
            let mut labels = std::collections::HashSet::new();
            let mut current = l;
            loop {
                if !labels.insert(current.label.clone()) {
                    return Err("SyntaxError: duplicate label in statement".to_string());
                }
                check_label_identifier(&current.label, ctx)?;
                match &*current.body {
                    ASTNode::LabelledStatement(next) => current = next,
                    _ => break,
                }
            }
            match &*current.body {
                ASTNode::FunctionDeclaration(fd) => {
                    if fd.is_async || fd.generator || ctx.strict {
                        return Err(
                            "SyntaxError: Labelled function declarations are not allowed in this context"
                                .to_string(),
                        );
                    }
                    validate_function_declaration(fd, ctx)
                }
                _ => validate_single_statement(&l.body, ctx, true),
            }
        }
        ASTNode::SwitchStatement(s) => {
            validate_expression(&s.discriminant, ctx)?;
            check_case_block(&s.cases, ctx.strict)?;
            for c in &s.cases {
                if let Some(t) = &c.test {
                    validate_expression(t, ctx)?;
                }
                for st in &c.consequent {
                    if matches!(st, ASTNode::VariableDeclaration(vd) if vd.kind.is_using()) {
                        return Err(
                            "SyntaxError: 'using' declarations are not allowed directly within switch case clauses"
                                .to_string(),
                        );
                    }
                    validate_statement(st, ctx)?;
                }
            }
            Ok(())
        }
        ASTNode::TryStatement(t) => {
            validate_block(&t.block, ctx)?;
            if let Some(h) = &t.handler {
                let mut bound: Vec<(String, bool)> = Vec::new();
                if let Some(p) = &h.param {
                    validate_binding_pattern(p, ctx)?;
                    collect_binding_pattern_names(p, &mut bound);
                }
                let mut seen: HashSet<String> = HashSet::new();
                for (name, _) in &bound {
                    if !seen.insert(name.clone()) {
                        return Err(redeclaration_error(name));
                    }
                }
                let mut lexical: Vec<(String, bool)> = Vec::new();
                for stmt in &h.body.body {
                    collect_lexical_names(stmt, false, &mut lexical);
                }
                for (name, _) in &lexical {
                    if seen.contains(name) {
                        return Err(redeclaration_error(name));
                    }
                }
                validate_block(&h.body, ctx)?;
            }
            if let Some(fin) = &t.finalizer {
                validate_block(fin, ctx)?;
            }
            Ok(())
        }
        ASTNode::ReturnStatement(r) => {
            if ctx.in_static_block {
                return Err(
                    "SyntaxError: Illegal return statement in class static block".to_string()
                );
            }
            if let Some(a) = &r.argument {
                validate_expression(a, ctx)?;
            }
            Ok(())
        }
        ASTNode::ThrowStatement(t) => validate_expression(&t.argument, ctx),
        ASTNode::VariableDeclaration(vd) => validate_variable_declaration(vd, ctx, true),
        ASTNode::FunctionDeclaration(fd) => validate_function_declaration(fd, ctx),
        ASTNode::ClassDeclaration(cd) => validate_class_declaration(cd, ctx),
        ASTNode::ExportNamedDeclaration(ed) => match &ed.declaration {
            Some(ExportDeclaration::Variable(vd)) => validate_variable_declaration(vd, ctx, true),
            Some(ExportDeclaration::Function(fd)) => validate_function_declaration(fd, ctx),
            Some(ExportDeclaration::Class(cd)) => validate_class_declaration(cd, ctx),
            None => Ok(()),
        },
        ASTNode::ExportDefaultDeclaration(ed) => match &ed.declaration {
            ExportDefaultDeclarationKind::Function(fd) => validate_function_declaration(fd, ctx),
            ExportDefaultDeclarationKind::Class(cd) => validate_class_declaration(cd, ctx),
            ExportDefaultDeclarationKind::Expression(expr) => validate_expression(expr, ctx),
        },
        ASTNode::ExpressionStatement(es) => validate_expression(&es.expression, ctx),
        _ => Ok(()),
    }
}

fn statement_is_labelled_function(stmt: &ASTNode) -> bool {
    match stmt {
        ASTNode::LabelledStatement(l) => match &*l.body {
            ASTNode::FunctionDeclaration(_) => true,
            ASTNode::LabelledStatement(_) => statement_is_labelled_function(&l.body),
            _ => false,
        },
        _ => false,
    }
}

fn validate_single_statement(
    stmt: &ASTNode,
    ctx: Ctx,
    allow_sloppy_function: bool,
) -> Result<(), String> {
    if statement_is_labelled_function(stmt) {
        return Err(
            "SyntaxError: Labelled function declarations are not allowed in this context"
                .to_string(),
        );
    }
    if matches!(stmt, ASTNode::FunctionDeclaration(_)) && (ctx.strict || !allow_sloppy_function) {
        return Err(
            "SyntaxError: Function declarations are not allowed in single-statement context"
                .to_string(),
        );
    }
    validate_statement(stmt, ctx)
}

fn validate_variable_declaration(
    vd: &VariableDeclaration,
    ctx: Ctx,
    require_initializer: bool,
) -> Result<(), String> {
    for d in &vd.declarations {
        validate_binding_pattern(&d.id, ctx)?;
        if vd.kind != VariableKind::Var {
            let mut names: Vec<(String, bool)> = Vec::new();
            collect_binding_pattern_names(&d.id, &mut names);
            for (name, _) in &names {
                if name == "let" {
                    return Err(
                        "SyntaxError: 'let' is disallowed as a lexically bound name".to_string(),
                    );
                }
            }
        }
        if vd.kind.is_using() {
            if !matches!(d.id, BindingPattern::Identifier(_)) {
                return Err(
                    "SyntaxError: 'using' declarations only allow identifier bindings".to_string(),
                );
            }
            if require_initializer && d.init.is_none() {
                return Err("SyntaxError: Missing initializer in using declaration".to_string());
            }
        }
        if let Some(init) = &d.init {
            validate_expression(init, ctx)?;
        }
    }
    Ok(())
}

fn validate_for_in_of_left(left: &ForInOfLeft, ctx: Ctx) -> Result<(), String> {
    match left {
        ForInOfLeft::VariableDeclaration(vd) => validate_variable_declaration(vd, ctx, false),
        ForInOfLeft::AssignmentTarget(t) => validate_assignment_target(t, ctx),
    }
}

fn validate_for_in_of(
    left: &ForInOfLeft,
    body: &ASTNode,
    ctx: Ctx,
) -> Result<(), String> {
    validate_for_in_of_left(left, ctx)?;
    let ForInOfLeft::VariableDeclaration(vd) = left else {
        return Ok(());
    };
    if vd.kind == VariableKind::Var {
        return Ok(());
    }
    let mut names: Vec<(String, bool)> = Vec::new();
    for d in &vd.declarations {
        collect_binding_pattern_names(&d.id, &mut names);
    }
    let mut seen: HashSet<String> = HashSet::new();
    for (name, _) in &names {
        if !seen.insert(name.clone()) {
            return Err(redeclaration_error(name));
        }
    }
    let mut vars: HashSet<String> = HashSet::new();
    collect_var_names(body, false, &mut vars);
    for (name, _) in &names {
        if vars.contains(name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn validate_function_declaration(fd: &FunctionDeclaration, ctx: Ctx) -> Result<(), String> {
    let name_ctx = ctx.with_strict(has_use_strict_directive(&fd.body.body));
    check_binding_identifier(&fd.name, name_ctx)?;
    validate_function_body(&fd.params, &fd.body, fd.is_async, fd.generator, ctx, false)
}

fn validate_class_declaration(cd: &ClassDeclaration, ctx: Ctx) -> Result<(), String> {
    let strict_ctx = Ctx {
        strict: true,
        in_async: ctx.in_async,
        in_generator: ctx.in_generator,
        in_static_block: false,
        allow_super_property: false,
        allow_super_call: false,
        in_function: false,
        in_field_initializer: false,
    };
    check_binding_identifier(&cd.name, strict_ctx)?;
    if let Some(sc) = &cd.super_class {
        validate_expression(sc, strict_ctx)?;
    }
    validate_class_body(&cd.body, strict_ctx, cd.super_class.is_some())
}

fn collect_parameter_names(params: &[Parameter], out: &mut Vec<(String, bool)>) {
    for p in params {
        match p {
            Parameter::Identifier(name) => out.push((name.clone(), false)),
            Parameter::Pattern(bp) => collect_binding_pattern_names(bp, out),
            Parameter::AssignmentPattern(ap) => collect_assignment_target_names(&ap.left, out),
            Parameter::RestElement(rest) => collect_assignment_target_names(&rest.argument, out),
        }
    }
}

fn validate_parameters_unique(params: &[Parameter], require_unique: bool) -> Result<(), String> {
    let mut names = Vec::new();
    collect_parameter_names(params, &mut names);
    let mut seen = HashSet::new();
    let mut duplicate = false;
    for (name, _) in &names {
        if !seen.insert(name.clone()) {
            duplicate = true;
        }
    }
    if !duplicate {
        return Ok(());
    }
    let simple = params
        .iter()
        .all(|p| matches!(p, Parameter::Identifier(_)));
    if require_unique || !simple {
        return Err(redeclaration_error("duplicate parameter name"));
    }
    Ok(())
}

fn validate_function_body(
    params: &[Parameter],
    body: &BlockStatement,
    is_async: bool,
    generator: bool,
    outer: Ctx,
    is_class_method: bool,
) -> Result<(), String> {
    validate_function_body_with_super(
        params,
        body,
        is_async,
        generator,
        outer,
        is_class_method,
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_function_body_with_super(
    params: &[Parameter],
    body: &BlockStatement,
    is_async: bool,
    generator: bool,
    outer: Ctx,
    is_class_method: bool,
    allow_super_property: bool,
    allow_super_call: bool,
) -> Result<(), String> {
    let ctx = function_ctx(
        outer,
        is_async,
        generator,
        &body.body,
        is_class_method,
        allow_super_property,
        allow_super_call,
    );
    let require_unique = ctx.strict || has_use_strict_directive(&body.body);
    validate_parameters_unique(params, require_unique)?;
    for p in params {
        validate_parameter(p, ctx)?;
    }
    check_parameters_against_body_lexical(params, &body.body)?;
    validate_body(&body.body, ctx)
}

fn check_lexical_head_against_body(
    vd: &VariableDeclaration,
    body: &ASTNode,
) -> Result<(), String> {
    let mut lexical: Vec<(String, bool)> = Vec::new();
    for d in &vd.declarations {
        collect_binding_pattern_names(&d.id, &mut lexical);
    }
    if lexical.is_empty() {
        return Ok(());
    }
    let mut vars: HashSet<String> = HashSet::new();
    collect_var_names(body, false, &mut vars);
    for (name, _) in &lexical {
        if vars.contains(name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn check_parameters_against_body_lexical(
    params: &[Parameter],
    body: &[ASTNode],
) -> Result<(), String> {
    let mut param_names = Vec::new();
    collect_parameter_names(params, &mut param_names);
    if param_names.is_empty() {
        return Ok(());
    }
    let mut lexical: Vec<(String, bool)> = Vec::new();
    for stmt in body {
        collect_lexical_names(stmt, true, &mut lexical);
    }
    for (name, _) in &param_names {
        if lexical.iter().any(|(n, _)| n == name) {
            return Err(redeclaration_error(name));
        }
    }
    Ok(())
}

fn validate_parameter(p: &Parameter, ctx: Ctx) -> Result<(), String> {
    match p {
        Parameter::Identifier(name) => check_binding_identifier(name, ctx),
        Parameter::Pattern(bp) => validate_binding_pattern(bp, ctx),
        Parameter::AssignmentPattern(ap) => {
            validate_assignment_target(&ap.left, ctx)?;
            validate_parameter_default(&ap.right, ctx)
        }
        Parameter::RestElement(rest) => validate_assignment_target(&rest.argument, ctx),
    }
}

/// Formal parameters may not contain YieldExpression (generators) or
/// AwaitExpression (async functions).
fn validate_parameter_default(expr: &Expression, ctx: Ctx) -> Result<(), String> {
    if contains_super_call(expr) {
        return Err(
            "SyntaxError: 'super' call is not allowed in formal parameter defaults".to_string(),
        );
    }
    if ctx.in_generator && contains_yield(expr) {
        return Err(
            "SyntaxError: Yield expression is not allowed in generator parameters".to_string(),
        );
    }
    if ctx.in_async && contains_await(expr) {
        return Err(
            "SyntaxError: Await expression is not allowed in async function parameters".to_string(),
        );
    }
    validate_expression(expr, ctx)
}

fn validate_binding_pattern(p: &BindingPattern, ctx: Ctx) -> Result<(), String> {
    match p {
        BindingPattern::Identifier(name) => check_binding_identifier(name, ctx),
        BindingPattern::ArrayPattern(ap) => {
            for el in &ap.elements {
                if let Some(e) = el {
                    validate_pattern_element(e, ctx)?;
                }
            }
            Ok(())
        }
        BindingPattern::ObjectPattern(op) => validate_object_pattern(op, ctx),
        BindingPattern::AssignmentPattern(ap) => {
            validate_assignment_target(&ap.left, ctx)?;
            validate_expression(&ap.right, ctx)
        }
    }
}

fn validate_pattern_element(e: &PatternElement, ctx: Ctx) -> Result<(), String> {
    match e {
        PatternElement::Pattern(t) => validate_assignment_target(t, ctx),
        PatternElement::RestElement(rest) => validate_assignment_target(&rest.argument, ctx),
        PatternElement::AssignmentPattern(ap) => {
            validate_assignment_target(&ap.left, ctx)?;
            validate_expression(&ap.right, ctx)
        }
    }
}

fn validate_object_pattern(op: &ObjectPattern, ctx: Ctx) -> Result<(), String> {
    for prop in &op.properties {
        match prop {
            ObjectPatternProperty::Property { key, value, .. } => {
                if let PropertyKey::Computed(k) = key {
                    validate_expression(k, ctx)?;
                }
                validate_assignment_target(value, ctx)?;
            }
            ObjectPatternProperty::RestElement(rest) => {
                validate_assignment_target(&rest.argument, ctx)?;
            }
        }
    }
    Ok(())
}


fn class_element_name(key: &PropertyKey) -> Option<String> {
    match key {
        PropertyKey::Identifier(name) => Some(name.clone()),
        PropertyKey::Literal(Literal::String(name, _, _)) => Some(name.clone()),
        PropertyKey::PrivateIdentifier(name) => {
            Some(name.trim_start_matches('#').to_string())
        }
        _ => None,
    }
}

fn validate_class_body(
    body: &ClassBody,
    outer: Ctx,
    has_super: bool,
) -> Result<(), String> {
    let mut private_names: std::collections::HashMap<String, (bool, bool, bool)> =
        std::collections::HashMap::new();
    for element in &body.elements {
        match element {
            ClassElement::Method(m) => {
                if let PropertyKey::Computed(k) = &m.name {
                    validate_expression(k, outer)?;
                }
                let name = class_element_name(&m.name);
                if m.is_private {
                    let Some(private_name) = name.as_ref() else {
                        return Err("SyntaxError: invalid private name".to_string());
                    };
                    if private_name == "constructor" {
                        return Err(
                            "SyntaxError: private name #constructor is not allowed".to_string()
                        );
                    }
                    let is_get = m.kind == MethodKind::Get;
                    let is_set = m.kind == MethodKind::Set;
                    if let Some((was_static, was_get, was_set)) =
                        private_names.get(private_name).copied()
                    {
                        let accessor_pair = (is_get && was_set) || (is_set && was_get);
                        if was_static != m.is_static || !accessor_pair {
                            return Err(
                                "SyntaxError: duplicate private name in class".to_string()
                            );
                        }
                        private_names.insert(private_name.clone(), (m.is_static, is_get, is_set));
                    } else {
                        private_names.insert(private_name.clone(), (m.is_static, is_get, is_set));
                    }
                }
                let is_constructor_name = name.as_deref() == Some("constructor");
                if is_constructor_name && !m.is_static && m.kind != MethodKind::Constructor {
                    return Err(
                        "SyntaxError: class constructor may not be an accessor or generator"
                            .to_string(),
                    );
                }
                if m.kind == MethodKind::Constructor
                    && (m.is_async || m.generator || m.is_static)
                {
                    return Err("SyntaxError: invalid class constructor".to_string());
                }
                if m.body.body.iter().any(|s| stmt_contains(s, Scan::SuperCall))
                    && !(m.kind == MethodKind::Constructor && has_super && !m.is_static)
                {
                    return Err(
                        "SyntaxError: 'super' call is only allowed in a derived constructor"
                            .to_string(),
                    );
                }
                match m.kind {
                    MethodKind::Get if !m.params.is_empty() => {
                        return Err(
                            "SyntaxError: Getter must not have any formal parameters".to_string()
                        );
                    }
                    MethodKind::Set if m.params.len() != 1 => {
                        return Err(
                            "SyntaxError: Setter must have exactly one formal parameter".to_string()
                        );
                    }
                    _ => {}
                }
                validate_function_body_with_super(
                    &m.params,
                    &m.body,
                    m.is_async,
                    m.generator,
                    outer,
                    true,
                    true,
                    m.kind == MethodKind::Constructor && has_super && !m.is_static,
                )?;
            }
            ClassElement::StaticBlock(sb) => {
                if sb.body.iter().any(|s| stmt_contains(s, Scan::Arguments)) {
                    return Err(
                        "SyntaxError: 'arguments' is not allowed in class static block".to_string(),
                    );
                }
                if sb.body.iter().any(|s| stmt_contains(s, Scan::SuperCall)) {
                    return Err(
                        "SyntaxError: 'super' call is not allowed in class static block"
                            .to_string(),
                    );
                }
                let ctx = Ctx {
                    strict: true,
                    in_async: true,
                    in_generator: true,
                    in_static_block: true,
                    allow_super_property: true,
                    allow_super_call: false,
                    in_function: true,
                    in_field_initializer: true,
                };
                validate_body(&sb.body, ctx)?;
            }
            ClassElement::Property(p) => {
                if let PropertyKey::Computed(k) = &p.name {
                    validate_expression(k, outer)?;
                }
                let name = class_element_name(&p.name);
                if p.is_private {
                    let Some(private_name) = name.as_ref() else {
                        return Err("SyntaxError: invalid private name".to_string());
                    };
                    if private_name == "constructor" {
                        return Err(
                            "SyntaxError: private name #constructor is not allowed".to_string()
                        );
                    }
                    if private_names.contains_key(private_name) {
                        return Err("SyntaxError: duplicate private name in class".to_string());
                    }
                    private_names.insert(private_name.clone(), (p.is_static, false, false));
                } else if name.as_deref() == Some("constructor")
                    || (p.is_static && name.as_deref() == Some("prototype"))
                {
                    return Err(
                        "SyntaxError: invalid class field name in class declaration".to_string(),
                    );
                }
                if let Some(value) = &p.value {
                    let ctx = Ctx {
                        strict: true,
                        in_async: false,
                        in_generator: false,
                        in_static_block: false,
                        allow_super_property: true,
                        allow_super_call: false,
                        in_function: true,
                        in_field_initializer: true,
                    };
                    if contains_arguments(value) {
                        return Err(
                            "SyntaxError: 'arguments' is not allowed in class field initializer"
                                .to_string(),
                        );
                    }
                    if contains_super_call(value) {
                        return Err(
                            "SyntaxError: 'super' call is not allowed in class field initializer"
                                .to_string(),
                        );
                    }
                    validate_expression(value, ctx)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_argument(arg: &Argument, ctx: Ctx) -> Result<(), String> {
    match arg {
        Argument::Expression(e) | Argument::Spread(e) => validate_expression(e, ctx),
    }
}

/// True if `expr` is a dynamic import call, i.e. `import(...)`.
fn is_import_call(expr: &Expression) -> bool {
    matches!(
        expr,
        Expression::CallExpression(c)
            if matches!(&*c.callee, Expression::Identifier(id) if id.name == "import")
    )
}

fn unparenthesized_yield(expr: &Expression) -> bool {
    matches!(expr, Expression::YieldExpression(y) if !y.parenthesized)
}

fn is_private_name_member(expr: &Expression) -> bool {
    match expr {
        Expression::MemberExpression(m) => {
            matches!(m.property, MemberProperty::PrivateIdentifier(_))
        }
        Expression::OptionalMemberExpression(m) => {
            matches!(m.property, MemberProperty::PrivateIdentifier(_))
        }
        Expression::ChainExpression(inner) => is_private_name_member(inner),
        _ => false,
    }
}

fn validate_expression(expr: &Expression, ctx: Ctx) -> Result<(), String> {
    match expr {
        Expression::Identifier(id) => check_identifier_reference(&id.name, ctx),
        Expression::PrivateIdentifier(_)
        | Expression::This
        | Expression::RegExpLiteral(_) => Ok(()),
        Expression::Super => {
            Err("SyntaxError: 'super' keyword unexpected here".to_string())
        }
        Expression::MetaProperty(mp) => {
            if mp.meta == "new" && mp.property == "target" && !ctx.in_function {
                return Err(
                    "SyntaxError: new.target expression is not allowed here".to_string()
                );
            }
            Ok(())
        }
        Expression::Literal(literal) => {
            if ctx.strict {
                match literal {
                    Literal::LegacyOctal(_) => {
                        return Err(
                            "SyntaxError: Legacy octal literal in strict mode code".to_string()
                        );
                    }
                    Literal::String(_, _, true) => {
                        return Err(
                            "SyntaxError: Legacy octal escape in strict mode code".to_string()
                        );
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        Expression::ArrayExpression(arr) => {
            for el in &arr.elements {
                if let Some(e) = el {
                    match e {
                        ArrayElement::Expression(x) | ArrayElement::Spread(x) => {
                            validate_expression(x, ctx)?
                        }
                    }
                }
            }
            Ok(())
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                match prop {
                    Property::Property {
                        key,
                        value,
                        method,
                        getter,
                        setter,
                        ..
                    } => {
                        if let PropertyKey::Computed(k) = key {
                            validate_expression(k, ctx)?;
                        }
                        if let Expression::FunctionExpression(f) = value.as_ref() {
                            if *method || *getter || *setter {
                                validate_parameters_unique(&f.params, true)?;
                                if *getter && !f.params.is_empty() {
                                    return Err(
                                        "SyntaxError: Getter must have no formal parameters"
                                            .to_string(),
                                    );
                                }
                                if *setter && f.params.len() != 1 {
                                    return Err(
                                        "SyntaxError: Setter must have exactly one formal parameter"
                                            .to_string(),
                                    );
                                }
                                validate_function_body_with_super(
                                    &f.params,
                                    &f.body,
                                    f.is_async,
                                    f.generator,
                                    ctx,
                                    false,
                                    true,
                                    false,
                                )?;
                                continue;
                            }
                        }
                        validate_expression(value, ctx)?;
                    }
                    Property::SpreadElement(e) => validate_expression(e, ctx)?,
                }
            }
            Ok(())
        }
        Expression::FunctionExpression(f) => {
            if let Some(name) = &f.name {
                let inner = function_ctx(
                    ctx,
                    f.is_async,
                    f.generator,
                    &f.body.body,
                    false,
                    false,
                    false,
                );
                check_binding_identifier(name, inner)?;
            }
            validate_function_body(&f.params, &f.body, f.is_async, f.generator, ctx, false)
        }
        Expression::ClassExpression(c) => {
            let strict_ctx = Ctx {
                strict: true,
                in_async: ctx.in_async,
                in_generator: ctx.in_generator,
                in_static_block: false,
                allow_super_property: false,
                allow_super_call: false,
                in_function: false,
                in_field_initializer: false,
            };
            if let Some(sc) = &c.super_class {
                validate_expression(sc, strict_ctx)?;
            }
            if let Some(name) = &c.name {
                check_binding_identifier(name, strict_ctx)?;
            }
            validate_class_body(&c.body, strict_ctx, c.super_class.is_some())
        }
        Expression::TemplateLiteral(t) => {
            if t.quasis.iter().any(|q| q.invalid_escape) {
                return Err(
                    "SyntaxError: invalid escape sequence in template literal".to_string()
                );
            }
            for e in &t.expressions {
                validate_expression(e, ctx)?;
            }
            Ok(())
        }
        Expression::UnaryExpression(u) => {
            // A YieldExpression is not a UnaryExpression, so `void yield` etc. is
            // only valid when `yield` is being treated as an identifier (i.e. not
            // inside a generator). Our parser is context-free and always produces a
            // YieldExpression, so reject it here when the enclosing context is a
            // generator.
            if ctx.in_generator
                && matches!(*u.argument, Expression::YieldExpression(ref y) if !y.parenthesized)
            {
                return Err(
                    "SyntaxError: 'yield' is not allowed as a unary operand in generator functions"
                        .to_string(),
                );
            }
            if u.op == UnaryOp::Delete && ctx.strict {
                if let Expression::Identifier(_) = *u.argument {
                    return Err(
                        "SyntaxError: delete of an unqualified identifier in strict mode"
                            .to_string(),
                    );
                }
                if is_private_name_member(&u.argument) {
                    return Err(
                        "SyntaxError: delete of a private member in strict mode".to_string(),
                    );
                }
            }
            validate_expression(&u.argument, ctx)
        }
        Expression::UpdateExpression(u) => {
            if is_import_call(&u.argument) {
                return Err(
                    "SyntaxError: Invalid update expression: import call is not an assignment target"
                        .to_string(),
                );
            }
            validate_update_target(&u.argument, ctx)?;
            validate_expression(&u.argument, ctx)
        }
        Expression::BinaryExpression(b) => {
            if ctx.in_generator
                && (unparenthesized_yield(&b.left)
                    || unparenthesized_yield(&b.right))
            {
                return Err(
                    "SyntaxError: yield expression may not be a binary operand".to_string()
                );
            }
            validate_expression(&b.left, ctx)?;
            validate_expression(&b.right, ctx)
        }
        Expression::LogicalExpression(l) => {
            if ctx.in_generator
                && (unparenthesized_yield(&l.left)
                    || unparenthesized_yield(&l.right))
            {
                return Err(
                    "SyntaxError: yield expression may not be a logical operand".to_string()
                );
            }
            validate_expression(&l.left, ctx)?;
            validate_expression(&l.right, ctx)
        }
        Expression::ConditionalExpression(c) => {
            if ctx.in_generator && unparenthesized_yield(&c.test) {
                return Err(
                    "SyntaxError: yield expression may not be a conditional operand".to_string()
                );
            }
            validate_expression(&c.test, ctx)?;
            validate_expression(&c.consequent, ctx)?;
            validate_expression(&c.alternate, ctx)
        }
        Expression::AssignmentExpression(a) => {
            validate_assignment_target(&a.left, ctx)?;
            validate_expression(&a.right, ctx)
        }
        Expression::SequenceExpression(s) => {
            for e in &s.expressions {
                validate_expression(e, ctx)?;
            }
            Ok(())
        }
        Expression::MemberExpression(m) => {
            if matches!(*m.object, Expression::Super) {
                if matches!(m.property, MemberProperty::PrivateIdentifier(_)) {
                    return Err(
                        "SyntaxError: private names may not be accessed through super".to_string()
                    );
                }
                if !ctx.allow_super_property {
                    return Err(
                        "SyntaxError: 'super' keyword unexpected here".to_string()
                    );
                }
            } else {
                validate_expression(&m.object, ctx)?;
            }
            if let MemberProperty::Computed(p) = &m.property {
                validate_expression(p, ctx)?;
            }
            Ok(())
        }
        Expression::OptionalMemberExpression(m) => {
            if matches!(*m.object, Expression::Super) {
                if !ctx.allow_super_property {
                    return Err(
                        "SyntaxError: 'super' keyword unexpected here".to_string()
                    );
                }
            } else {
                validate_expression(&m.object, ctx)?;
            }
            if let MemberProperty::Computed(p) = &m.property {
                validate_expression(p, ctx)?;
            }
            Ok(())
        }
        Expression::CallExpression(c) => {
            if matches!(&*c.callee, Expression::Super) {
                if !ctx.allow_super_call {
                    return Err(
                        "SyntaxError: 'super' call is not allowed here".to_string()
                    );
                }
            } else if !is_import_call(expr) {
                validate_expression(&c.callee, ctx)?;
            }
            for a in &c.arguments {
                validate_argument(a, ctx)?;
            }
            Ok(())
        }
        Expression::OptionalCallExpression(c) => {
            if matches!(&*c.callee, Expression::Super) {
                if !ctx.allow_super_call {
                    return Err(
                        "SyntaxError: 'super' call is not allowed here".to_string()
                    );
                }
            } else {
                validate_expression(&c.callee, ctx)?;
            }
            for a in &c.arguments {
                validate_argument(a, ctx)?;
            }
            Ok(())
        }
        Expression::ChainExpression(inner) => validate_expression(inner, ctx),
        Expression::NewExpression(n) => {
            if matches!(&*n.callee, Expression::Super) {
                return Err(
                    "SyntaxError: 'super' keyword unexpected here".to_string()
                );
            }
            validate_expression(&n.callee, ctx)?;
            for a in &n.arguments {
                validate_argument(a, ctx)?;
            }
            Ok(())
        }
        Expression::ArrowFunction(arrow) => {
            if ctx.in_generator
                && arrow
                    .params
                    .iter()
                    .any(|p| parameter_contains(p, Scan::Yield))
            {
                return Err(
                    "SyntaxError: yield expression is not allowed in arrow parameters".to_string(),
                );
            }
            if (ctx.in_async || arrow.is_async)
                && arrow
                    .params
                    .iter()
                    .any(|p| parameter_contains(p, Scan::Await))
            {
                return Err(
                    "SyntaxError: await expression is not allowed in arrow parameters".to_string(),
                );
            }
            let param_ctx = Ctx {
                strict: ctx.strict,
                in_async: ctx.in_async || arrow.is_async,
                in_generator: ctx.in_generator,
                in_static_block: ctx.in_static_block,
                allow_super_property: ctx.allow_super_property,
                allow_super_call: ctx.allow_super_call,
                in_function: ctx.in_function,
                in_field_initializer: ctx.in_field_initializer,
            };
            let inner = Ctx {
                strict: ctx.strict,
                in_async: arrow.is_async,
                in_generator: false,
                in_static_block: false,
                allow_super_property: ctx.allow_super_property,
                allow_super_call: ctx.allow_super_call,
                in_function: ctx.in_function,
                in_field_initializer: ctx.in_field_initializer,
            };
            validate_parameters_unique(&arrow.params, true)?;
            for p in &arrow.params {
                validate_parameter(p, param_ctx)?;
            }
            if let ArrowBody::Block(b) = &arrow.body {
                check_parameters_against_body_lexical(&arrow.params, &b.body)?;
            }
            match &arrow.body {
                ArrowBody::Expression(e) => validate_expression(e, inner),
                ArrowBody::Block(b) => validate_body(&b.body, inner),
            }
        }
        Expression::AwaitExpression(a) => {
            if ctx.in_static_block {
                return Err(
                    "SyntaxError: 'await' is not allowed in class static blocks".to_string()
                );
            }
            validate_expression(&a.argument, ctx)
        }
        Expression::YieldExpression(y) => {
            if ctx.in_static_block {
                return Err(
                    "SyntaxError: 'yield' is not allowed in class static blocks".to_string()
                );
            }
            if !ctx.in_generator && ctx.strict {
                return Err("SyntaxError: 'yield' is a reserved word in strict mode".to_string());
            }
            if let Some(arg) = &y.argument {
                validate_expression(arg, ctx)?;
            }
            Ok(())
        }
        Expression::SpreadElement(s) => validate_expression(&s.argument, ctx),
        Expression::TaggedTemplateExpression(t) => {
            validate_expression(&t.tag, ctx)?;
            for e in &t.quasi.expressions {
                validate_expression(e, ctx)?;
            }
            Ok(())
        }
        Expression::ArrayPattern(ap) => {
            for el in &ap.elements {
                if let Some(e) = el {
                    validate_pattern_element(e, ctx)?;
                }
            }
            Ok(())
        }
        Expression::ObjectPattern(op) => validate_object_pattern(op, ctx),
        Expression::RestElement(rest) => validate_assignment_target(&rest.argument, ctx),
        Expression::AssignmentPattern(ap) => {
            validate_assignment_target(&ap.left, ctx)?;
            validate_expression(&ap.right, ctx)
        }
    }
}


fn expression_contains_optional_chain(expr: &Expression) -> bool {
    match expr {
        Expression::ChainExpression(_) | Expression::OptionalMemberExpression(_) => true,
        Expression::MemberExpression(m) => expression_contains_optional_chain(&m.object),
        Expression::CallExpression(c) => expression_contains_optional_chain(&c.callee),
        _ => false,
    }
}

fn validate_update_target(arg: &Expression, ctx: Ctx) -> Result<(), String> {
    match arg {
        Expression::Identifier(name) => {
            if ctx.strict && (name.name == "eval" || name.name == "arguments") {
                return Err(
                    "SyntaxError: Assignment to 'eval' or 'arguments' in strict mode".to_string(),
                );
            }
            Ok(())
        }
        Expression::MemberExpression(m) => {
            if expression_contains_optional_chain(&m.object) {
                return Err(
                    "SyntaxError: Invalid update expression: optional chain is not an assignment target"
                        .to_string(),
                );
            }
            Ok(())
        }
        Expression::CallExpression(_) => {
            if ctx.strict {
                return Err(
                    "SyntaxError: Invalid update expression: call expression is not an assignment target"
                        .to_string(),
                );
            }
            Ok(())
        }
        _ => Err(
            "SyntaxError: Invalid update expression: not a valid assignment target".to_string(),
        ),
    }
}

fn validate_assignment_target(t: &AssignmentTarget, ctx: Ctx) -> Result<(), String> {
        match t {
        AssignmentTarget::Identifier(name) => {
            if ctx.strict && (name == "eval" || name == "arguments") {
                return Err(format!(
                    "SyntaxError: '{}' may not be an assignment target in strict mode",
                    name
                ));
            }
            check_identifier_reference(name, ctx)
        }
        AssignmentTarget::MemberExpression(m) => {
            if expression_contains_optional_chain(&m.object) {
                return Err(
                    "SyntaxError: Invalid assignment target: optional chain".to_string(),
                );
            }
            if matches!(*m.object, Expression::Super) {
                if !ctx.allow_super_property {
                    return Err(
                        "SyntaxError: 'super' keyword unexpected here".to_string(),
                    );
                }
            } else {
                validate_expression(&m.object, ctx)?;
            }
            if let MemberProperty::Computed(p) = &m.property {
                validate_expression(p, ctx)?;
            }
            Ok(())
        }
        AssignmentTarget::OptionalMemberExpression(_) => Err(
            "SyntaxError: Invalid assignment target: optional chain".to_string(),
        ),
        AssignmentTarget::ComputedMember(obj, prop) => {
            validate_expression(obj, ctx)?;
            validate_expression(prop, ctx)
        }
        AssignmentTarget::AssignmentPattern(ap) => {
            validate_assignment_target(&ap.left, ctx)?;
            validate_expression(&ap.right, ctx)
        }
        AssignmentTarget::ObjectPattern(op) => validate_object_pattern(op, ctx),
        AssignmentTarget::ArrayPattern(ap) => {
            for el in &ap.elements {
                if let Some(e) = el {
                    match e {
                        PatternElement::Pattern(t) => validate_assignment_target(t, ctx)?,
                        PatternElement::RestElement(rest) => {
                            validate_assignment_target(&rest.argument, ctx)?
                        }
                        PatternElement::AssignmentPattern(ap) => {
                            validate_assignment_target(&ap.left, ctx)?;
                            validate_expression(&ap.right, ctx)?;
                        }
                    }
                }
            }
            Ok(())
        }
        AssignmentTarget::RestElement(rest) => validate_assignment_target(&rest.argument, ctx),
        _ => Ok(()),
    }
}


/// Which early-error predicate to evaluate while walking an expression.
#[derive(Clone, Copy, PartialEq)]
enum Scan {
    Arguments,
    SuperCall,
    Yield,
    Await,
}

fn contains_arguments(expr: &Expression) -> bool {
    expr_contains(expr, Scan::Arguments)
}

fn contains_super_call(expr: &Expression) -> bool {
    expr_contains(expr, Scan::SuperCall)
}

fn contains_yield(expr: &Expression) -> bool {
    expr_contains(expr, Scan::Yield)
}

fn contains_await(expr: &Expression) -> bool {
    expr_contains(expr, Scan::Await)
}

fn arg_contains(arg: &Argument, mode: Scan) -> bool {
    match arg {
        Argument::Expression(e) | Argument::Spread(e) => expr_contains(e, mode),
    }
}

fn parameter_contains(param: &Parameter, mode: Scan) -> bool {
    match param {
        Parameter::Identifier(_) | Parameter::Pattern(_) | Parameter::RestElement(_) => false,
        Parameter::AssignmentPattern(ap) => expr_contains(&ap.right, mode),
    }
}

fn expr_contains(expr: &Expression, mode: Scan) -> bool {
    match expr {
        Expression::Identifier(id) => mode == Scan::Arguments && id.name == "arguments",
        Expression::FunctionExpression(_) => false,
        Expression::ClassExpression(c) => {
            c.super_class.as_ref().is_some_and(|s| expr_contains(s, mode))
                || class_contains(&c.body, mode)
        }
        Expression::CallExpression(c) => {
            (mode == Scan::SuperCall && matches!(*c.callee, Expression::Super))
                || expr_contains(&c.callee, mode)
                || c.arguments.iter().any(|a| arg_contains(a, mode))
        }
        Expression::OptionalCallExpression(c) => {
            expr_contains(&c.callee, mode) || c.arguments.iter().any(|a| arg_contains(a, mode))
        }
        Expression::ChainExpression(inner) => expr_contains(inner, mode),
        Expression::NewExpression(n) => {
            expr_contains(&n.callee, mode) || n.arguments.iter().any(|a| arg_contains(a, mode))
        }
        Expression::ArrayExpression(arr) => arr.elements.iter().flatten().any(|e| match e {
            ArrayElement::Expression(x) | ArrayElement::Spread(x) => expr_contains(x, mode),
        }),
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|p| match p {
            Property::Property { key, value, .. } => {
                key_contains(key, mode) || expr_contains(value, mode)
            }
            Property::SpreadElement(e) => expr_contains(e, mode),
        }),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_contains(e, mode)),
        Expression::UnaryExpression(u) => expr_contains(&u.argument, mode),
        Expression::UpdateExpression(u) => expr_contains(&u.argument, mode),
        Expression::BinaryExpression(b) => {
            expr_contains(&b.left, mode) || expr_contains(&b.right, mode)
        }
        Expression::LogicalExpression(l) => {
            expr_contains(&l.left, mode) || expr_contains(&l.right, mode)
        }
        Expression::ConditionalExpression(c) => {
            expr_contains(&c.test, mode)
                || expr_contains(&c.consequent, mode)
                || expr_contains(&c.alternate, mode)
        }
        Expression::AssignmentExpression(a) => {
            target_contains(&a.left, mode) || expr_contains(&a.right, mode)
        }
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| expr_contains(e, mode)),
        Expression::MemberExpression(m) => {
            expr_contains(&m.object, mode)
                || matches!(&m.property, MemberProperty::Computed(p) if expr_contains(p, mode))
        }
        Expression::OptionalMemberExpression(m) => {
            expr_contains(&m.object, mode)
                || matches!(&m.property, MemberProperty::Computed(p) if expr_contains(p, mode))
        }
        Expression::AwaitExpression(a) => {
            mode == Scan::Await || expr_contains(&a.argument, mode)
        }
        Expression::YieldExpression(y) => {
            mode == Scan::Yield || y.argument.as_ref().is_some_and(|a| expr_contains(a, mode))
        }
        Expression::SpreadElement(s) => expr_contains(&s.argument, mode),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_contains(e, mode)),
        Expression::LogicalExpression(l) => {
            expr_contains(&l.left, mode) || expr_contains(&l.right, mode)
        }
        Expression::ConditionalExpression(c) => {
            expr_contains(&c.test, mode)
                || expr_contains(&c.consequent, mode)
                || expr_contains(&c.alternate, mode)
        }
        Expression::AssignmentExpression(a) => {
            target_contains(&a.left, mode) || expr_contains(&a.right, mode)
        }
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| expr_contains(e, mode)),
        Expression::OptionalMemberExpression(m) => {
            expr_contains(&m.object, mode)
                || matches!(&m.property, MemberProperty::Computed(p) if expr_contains(p, mode))
        }
        Expression::ArrowFunction(a) => {
            // Arrow bodies are parsed with [~Yield, ~Await], so `yield`/`await`
            // inside them are identifiers rather than Yield/Await expressions.
            if matches!(mode, Scan::Yield | Scan::Await) {
                return false;
            }
            match &a.body {
                ArrowBody::Expression(e) => expr_contains(e, mode),
                ArrowBody::Block(b) => b.body.iter().any(|s| stmt_contains(s, mode)),
            }
        }
        Expression::TaggedTemplateExpression(t) => {
            expr_contains(&t.tag, mode) || t.quasi.expressions.iter().any(|e| expr_contains(e, mode))
        }
        Expression::ArrayPattern(ap) => ap.elements.iter().flatten().any(|e| match e {
            PatternElement::Pattern(t) => target_contains(t, mode),
            PatternElement::RestElement(r) => target_contains(&r.argument, mode),
            PatternElement::AssignmentPattern(ap) => {
                target_contains(&ap.left, mode) || expr_contains(&ap.right, mode)
            }
        }),
        Expression::ObjectPattern(op) => object_pattern_contains(op, mode),
        Expression::RestElement(r) => target_contains(&r.argument, mode),
        Expression::AssignmentPattern(ap) => {
            target_contains(&ap.left, mode) || expr_contains(&ap.right, mode)
        }
        _ => false,
    }
}

fn object_pattern_contains(op: &ObjectPattern, mode: Scan) -> bool {
    op.properties.iter().any(|prop| match prop {
        ObjectPatternProperty::Property { key, value, .. } => {
            key_contains(key, mode) || target_contains(value, mode)
        }
        ObjectPatternProperty::RestElement(r) => target_contains(&r.argument, mode),
    })
}

fn target_contains(t: &AssignmentTarget, mode: Scan) -> bool {
    match t {
        AssignmentTarget::Identifier(_) => false,
        AssignmentTarget::MemberExpression(m) => {
            expr_contains(&m.object, mode)
                || matches!(&m.property, MemberProperty::Computed(p) if expr_contains(p, mode))
        }
        AssignmentTarget::OptionalMemberExpression(m) => {
            expr_contains(&m.object, mode)
                || matches!(&m.property, MemberProperty::Computed(p) if expr_contains(p, mode))
        }
        AssignmentTarget::ComputedMember(obj, prop) => {
            expr_contains(obj, mode) || expr_contains(prop, mode)
        }
        AssignmentTarget::ObjectPattern(op) => object_pattern_contains(op, mode),
        AssignmentTarget::ArrayPattern(ap) => ap.elements.iter().flatten().any(|e| match e {
            PatternElement::Pattern(t) => target_contains(t, mode),
            PatternElement::RestElement(r) => target_contains(&r.argument, mode),
            PatternElement::AssignmentPattern(ap) => {
                target_contains(&ap.left, mode) || expr_contains(&ap.right, mode)
            }
        }),
        AssignmentTarget::RestElement(r) => target_contains(&r.argument, mode),
        AssignmentTarget::AssignmentPattern(ap) => {
            target_contains(&ap.left, mode) || expr_contains(&ap.right, mode)
        }
    }
}

fn key_contains(key: &PropertyKey, mode: Scan) -> bool {
    match key {
        PropertyKey::Computed(e) => expr_contains(e, mode),
        _ => false,
    }
}

fn class_contains(class: &ClassBody, mode: Scan) -> bool {
    class.elements.iter().any(|el| match el {
        ClassElement::Method(m) => key_contains(&m.name, mode),
        ClassElement::StaticBlock(sb) => sb.body.iter().any(|s| stmt_contains(s, mode)),
        ClassElement::Property(p) => {
            key_contains(&p.name, mode) || p.value.as_ref().is_some_and(|v| expr_contains(v, mode))
        }
    })
}

fn for_left_contains(left: &ForInOfLeft, mode: Scan) -> bool {
    match left {
        ForInOfLeft::VariableDeclaration(vd) => vd
            .declarations
            .iter()
            .any(|d| d.init.as_ref().is_some_and(|i| expr_contains(i, mode))),
        ForInOfLeft::AssignmentTarget(t) => target_contains(t, mode),
    }
}

fn stmt_contains(stmt: &ASTNode, mode: Scan) -> bool {
    match stmt {
        ASTNode::FunctionDeclaration(_) | ASTNode::ClassDeclaration(_) => false,
        ASTNode::BlockStatement(b) => b.body.iter().any(|s| stmt_contains(s, mode)),
        ASTNode::IfStatement(i) => {
            expr_contains(&i.test, mode)
                || stmt_contains(&i.consequent, mode)
                || i.alternate.as_ref().is_some_and(|a| stmt_contains(a, mode))
        }
        ASTNode::ForStatement(f) => {
            f.init.as_ref().is_some_and(|init| match init {
                ForInit::VariableDeclaration(vd) => vd
                    .declarations
                    .iter()
                    .any(|d| d.init.as_ref().is_some_and(|i| expr_contains(i, mode))),
                ForInit::Expression(e) => expr_contains(e, mode),
            }) || f.test.as_ref().is_some_and(|t| expr_contains(t, mode))
                || f.update.as_ref().is_some_and(|u| expr_contains(u, mode))
                || stmt_contains(&f.body, mode)
        }
        ASTNode::ForInStatement(f) => {
            for_left_contains(&f.left, mode)
                || expr_contains(&f.right, mode)
                || stmt_contains(&f.body, mode)
        }
        ASTNode::ForOfStatement(f) => {
            for_left_contains(&f.left, mode)
                || expr_contains(&f.right, mode)
                || stmt_contains(&f.body, mode)
        }
        ASTNode::WhileStatement(w) => expr_contains(&w.test, mode) || stmt_contains(&w.body, mode),
        ASTNode::DoWhileStatement(d) => expr_contains(&d.test, mode) || stmt_contains(&d.body, mode),
        ASTNode::SwitchStatement(s) => {
            expr_contains(&s.discriminant, mode)
                || s.cases.iter().any(|c| {
                    c.test.as_ref().is_some_and(|t| expr_contains(t, mode))
                        || c.consequent.iter().any(|st| stmt_contains(st, mode))
                })
        }
        ASTNode::TryStatement(t) => {
            t.block.body.iter().any(|s| stmt_contains(s, mode))
                || t.handler
                    .as_ref()
                    .is_some_and(|h| h.body.body.iter().any(|s| stmt_contains(s, mode)))
                || t.finalizer
                    .as_ref()
                    .is_some_and(|f| f.body.iter().any(|s| stmt_contains(s, mode)))
        }
        ASTNode::WithStatement(w) => expr_contains(&w.object, mode) || stmt_contains(&w.body, mode),
        ASTNode::LabelledStatement(l) => stmt_contains(&l.body, mode),
        ASTNode::ReturnStatement(r) => r.argument.as_ref().is_some_and(|a| expr_contains(a, mode)),
        ASTNode::ThrowStatement(t) => expr_contains(&t.argument, mode),
        ASTNode::VariableDeclaration(vd) => vd
            .declarations
            .iter()
            .any(|d| d.init.as_ref().is_some_and(|i| expr_contains(i, mode))),
        ASTNode::ExpressionStatement(es) => expr_contains(&es.expression, mode),
        _ => false,
    }
}

