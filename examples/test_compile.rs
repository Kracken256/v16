use v16::compiler::ast::BlockStatement;
use std::fs;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let source = fs::read_to_string(path).unwrap();
    println!("Parsing...");
    let ast = v16::compiler::parser::Parser::new(&source)
        .parse()
        .unwrap();
    println!("Parsed OK, body len = {}", ast.body.len());
    println!("Compiling...");
    let mut codegen = v16::compiler::codegen::CodeGenerator::new();
    let block = BlockStatement {
        body: ast.body,
        lines: ast.lines,
    };
    let mut rt = v16::JSRuntime::new();
    let mut ctx = v16::runtime::context::JSContext::new(&mut rt);
    let (rb, _) = codegen.compile_function(&[], &block, &mut ctx).unwrap();
    println!(
        "Compiled OK, code len = {}, locals = {}",
        rb.code.len(),
        rb.locals_count
    );
}
