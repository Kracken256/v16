use super::ast::{Ast, CharClass, Quantifier};

use super::opcode::*;

pub struct Compiler {
    bytecode: Vec<u8>,

    capture_count: usize,

    ignore_case: bool,

    unicode: bool,

    char_ranges: Vec<super::charclass::CharRange>,

    named_groups: Vec<(String, usize)>,

    repeat_depth: usize,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub bytecode: Vec<u8>,

    pub capture_count: usize,

    pub flags: u16,

    pub char_ranges: Vec<super::charclass::CharRange>,
}

impl Program {
    pub fn flags(&self) -> u16 {
        u16::from_le_bytes([self.bytecode[HEADER_FLAGS], self.bytecode[HEADER_FLAGS + 1]])
    }

    pub fn capture_count(&self) -> usize {
        self.bytecode[HEADER_CAPTURE_COUNT] as usize
    }

    pub fn code(&self) -> &[u8] {
        &self.bytecode[HEADER_LEN..]
    }
}

pub fn compile(ast: &Ast, flags: u16) -> Result<Program, String> {
    let mut named_groups = Vec::new();
    let mut counter = 1usize;
    collect_named_groups(ast, &mut counter, &mut named_groups);

    let mut compiler = Compiler::new(flags);
    compiler.named_groups = named_groups;

    compiler.write_header_placeholder();

    compiler.emit_op_u8(OpCode::SaveStart, 0);

    compiler.compile_node(ast)?;

    compiler.emit_op_u8(OpCode::SaveEnd, 0);

    compiler.emit_op(OpCode::Success);

    compiler.update_header(flags)?;

    Ok(Program {
        bytecode: compiler.bytecode,
        capture_count: compiler.capture_count,
        flags,
        char_ranges: compiler.char_ranges,
    })
}

impl Compiler {
    fn new(flags: u16) -> Self {
        Self {
            bytecode: Vec::new(),
            capture_count: 1,
            ignore_case: (flags & FLAG_IGNORE_CASE) != 0,
            unicode: (flags & FLAG_UNICODE) != 0,
            char_ranges: Vec::new(),
            named_groups: Vec::new(),
            repeat_depth: 0,
        }
    }

    fn write_header_placeholder(&mut self) {
        self.bytecode.extend_from_slice(&[0, 0]);

        self.bytecode.push(0);

        self.bytecode.push(REG_COUNT as u8);

        self.bytecode.extend_from_slice(&[0, 0, 0, 0]);
    }

    fn update_header(&mut self, flags: u16) -> Result<(), String> {
        let flag_bytes = flags.to_le_bytes();
        self.bytecode[HEADER_FLAGS] = flag_bytes[0];
        self.bytecode[HEADER_FLAGS + 1] = flag_bytes[1];

        if self.capture_count > MAX_CAPTURES {
            return Err(format!("Too many capture groups: {}", self.capture_count));
        }
        self.bytecode[HEADER_CAPTURE_COUNT] = self.capture_count as u8;

        let len = self.bytecode.len() - HEADER_LEN;
        let len_bytes = (len as u32).to_le_bytes();
        self.bytecode[HEADER_CODE_LEN..HEADER_CODE_LEN + 4].copy_from_slice(&len_bytes);

        Ok(())
    }

    fn compile_node(&mut self, node: &Ast) -> Result<(), String> {
        match node {
            Ast::Empty => Ok(()),
            Ast::Char(c) => self.compile_char(*c),
            Ast::Class(class) => self.compile_class(class),
            Ast::Any => {
                self.emit_op(OpCode::MatchDot);
                Ok(())
            }
            Ast::AnyAll => {
                self.emit_op(OpCode::MatchAny);
                Ok(())
            }
            Ast::StartOfLine => {
                self.emit_op(OpCode::CheckLineStart);
                Ok(())
            }
            Ast::EndOfLine => {
                self.emit_op(OpCode::CheckLineEnd);
                Ok(())
            }
            Ast::WordBoundary => {
                if self.ignore_case {
                    self.emit_op(OpCode::CheckWordBoundaryI);
                } else {
                    self.emit_op(OpCode::CheckWordBoundary);
                }
                Ok(())
            }
            Ast::NotWordBoundary => {
                if self.ignore_case {
                    self.emit_op(OpCode::CheckNotWordBoundaryI);
                } else {
                    self.emit_op(OpCode::CheckNotWordBoundary);
                }
                Ok(())
            }
            Ast::Concat(nodes) => {
                for node in nodes {
                    self.compile_node(node)?;
                }
                Ok(())
            }
            Ast::Alt(nodes) => self.compile_alt(nodes),
            Ast::Quant(inner, q) => self.compile_quant(inner, q),
            Ast::Capture(inner, _name) => self.compile_capture(inner),
            Ast::BackRef(idx) => self.compile_backref(*idx),
            Ast::NamedBackRef(name) => {
                let idx = self
                    .named_groups
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, i)| *i)
                    .ok_or_else(|| format!("Unknown named backref: {}", name))?;
                self.compile_backref(idx)
            }
            Ast::Lookahead(inner) => self.compile_lookahead(inner, false),
            Ast::NegativeLookahead(inner) => self.compile_lookahead(inner, true),
        }
    }

    fn compile_char(&mut self, c: char) -> Result<(), String> {
        let cp = c as u32;

        if self.ignore_case {
            let folded = if self.unicode {
                super::charclass::simple_case_fold(cp)
            } else {
                cp
            };
            if folded > 0xFFFF {
                self.emit_match_char32_i(REG_POS, folded);
            } else {
                self.emit_match_char_i(REG_POS, folded as u16);
            }
        } else {
            if cp > 0xFFFF {
                self.emit_match_char32(REG_POS, cp);
            } else {
                self.emit_match_char(REG_POS, cp as u16);
            }
        }
        Ok(())
    }

    fn compile_class(&mut self, class: &CharClass) -> Result<(), String> {
        let range_idx = self.char_ranges.len();

        let mut ranges = class.ranges.clone();
        if class.negated {
            ranges.invert();
        }
        if self.ignore_case {
            ranges.canonicalize(self.unicode);
        }
        self.char_ranges.push(ranges);

        if range_idx > u16::MAX as usize {
            return Err("Too many character classes".to_string());
        }

        let opcode = if self.ignore_case {
            OpCode::MatchClassI
        } else {
            OpCode::MatchClass
        };

        self.bytecode.push(opcode as u8);
        self.bytecode.push(REG_POS as u8);
        self.bytecode
            .extend_from_slice(&(range_idx as u16).to_le_bytes());

        Ok(())
    }

    fn compile_alt(&mut self, nodes: &[Ast]) -> Result<(), String> {
        if nodes.is_empty() {
            return Ok(());
        }
        if nodes.len() == 1 {
            return self.compile_node(&nodes[0]);
        }

        let mut jump_offsets = Vec::new();

        for (i, node) in nodes.iter().enumerate() {
            if i < nodes.len() - 1 {
                let push_pos = self.bytecode.len();
                self.bytecode.push(OpCode::PushBacktrack as u8);
                self.bytecode.extend_from_slice(&0i32.to_le_bytes());

                self.compile_node(node)?;

                let jmp_pos = self.bytecode.len();
                self.bytecode.push(OpCode::Jmp as u8);
                self.bytecode.extend_from_slice(&0i32.to_le_bytes());
                jump_offsets.push(jmp_pos);

                let fail_target = self.bytecode.len();
                let push_offset = (fail_target as i32 - push_pos as i32 - 5) as i32;
                self.bytecode[push_pos + 1..push_pos + 5]
                    .copy_from_slice(&push_offset.to_le_bytes());
            } else {
                self.compile_node(node)?;
            }
        }

        let end_pos = self.bytecode.len();
        for jmp_pos in jump_offsets {
            let offset = (end_pos as i32 - jmp_pos as i32 - 5) as i32;
            self.bytecode[jmp_pos + 1..jmp_pos + 5].copy_from_slice(&offset.to_le_bytes());
        }

        Ok(())
    }

    fn compile_quant(&mut self, inner: &Ast, q: &Quantifier) -> Result<(), String> {
        let min = q.min;
        let max = q.max.unwrap_or(usize::MAX as u32) as usize;
        let greedy = q.greedy;

        if min == 0 && max == 0 {
            return Ok(());
        }

        if min == 1 && max == 1 {
            return self.compile_node(inner);
        }

        if min == 0 && max == 1 {
            return self.compile_optional(inner, greedy);
        }

        if min == 0 && max == usize::MAX {
            return self.compile_star(inner, greedy);
        }

        if min == 1 && max == usize::MAX {
            return self.compile_plus(inner, greedy);
        }

        self.compile_repeat(inner, min as usize, max, greedy)
    }

    fn compile_optional(&mut self, inner: &Ast, greedy: bool) -> Result<(), String> {
        if greedy {
            let push_pos = self.bytecode.len();
            self.bytecode.push(OpCode::PushBacktrack as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());

            self.compile_node(inner)?;

            let jmp_pos = self.bytecode.len();
            self.bytecode.push(OpCode::Jmp as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());

            let skip_target = self.bytecode.len();
            let push_offset = (skip_target as i32 - push_pos as i32 - 5) as i32;
            self.bytecode[push_pos + 1..push_pos + 5].copy_from_slice(&push_offset.to_le_bytes());

            let done_target = self.bytecode.len();
            let jmp_offset = (done_target as i32 - jmp_pos as i32 - 5) as i32;
            self.bytecode[jmp_pos + 1..jmp_pos + 5].copy_from_slice(&jmp_offset.to_le_bytes());
        } else {
            let push_pos = self.bytecode.len();
            self.bytecode.push(OpCode::PushBacktrack as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());

            let jmp_pos = self.bytecode.len();
            self.bytecode.push(OpCode::Jmp as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());

            let match_target = self.bytecode.len();
            let push_offset = (match_target as i32 - push_pos as i32 - 5) as i32;
            self.bytecode[push_pos + 1..push_pos + 5].copy_from_slice(&push_offset.to_le_bytes());

            self.compile_node(inner)?;

            let done_target = self.bytecode.len();
            let jmp_offset = (done_target as i32 - jmp_pos as i32 - 5) as i32;
            self.bytecode[jmp_pos + 1..jmp_pos + 5].copy_from_slice(&jmp_offset.to_le_bytes());
        }

        Ok(())
    }

    fn compile_star(&mut self, inner: &Ast, _greedy: bool) -> Result<(), String> {
        let start_pos = self.bytecode.len();

        let push_pos = self.bytecode.len();
        self.bytecode.push(OpCode::PushBacktrack as u8);
        self.bytecode.extend_from_slice(&0i32.to_le_bytes());

        self.compile_node(inner)?;

        self.bytecode.push(OpCode::Jmp as u8);
        let loop_offset = (start_pos as i32 - self.bytecode.len() as i32 - 5) as i32;
        self.bytecode.extend_from_slice(&loop_offset.to_le_bytes());

        let done_pos = self.bytecode.len();
        let push_offset = (done_pos as i32 - push_pos as i32 - 5) as i32;
        self.bytecode[push_pos + 1..push_pos + 5].copy_from_slice(&push_offset.to_le_bytes());

        Ok(())
    }

    fn compile_plus(&mut self, inner: &Ast, greedy: bool) -> Result<(), String> {
        self.compile_node(inner)?;

        self.compile_star(inner, greedy)
    }

    fn compile_repeat(
        &mut self,
        inner: &Ast,
        min: usize,
        max: usize,
        greedy: bool,
    ) -> Result<(), String> {
        let counter_reg = match self.repeat_depth {
            0 => REG_COUNTER,
            1 => REG_SAVE0,
            2 => REG_SAVE1,
            3 => REG_SAVE2,
            _ => REG_SAVE3,
        };

        self.repeat_depth += 1;

        self.emit_mov_imm(counter_reg, 0);

        let loop_start = self.bytecode.len();

        let max_check = if max < usize::MAX {
            let eq_pos = self.bytecode.len();
            self.bytecode.push(OpCode::JmpEq as u8);
            self.bytecode.push(counter_reg as u8);
            self.bytecode.extend_from_slice(&(max as u32).to_le_bytes());
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());
            Some(eq_pos)
        } else {
            None
        };

        let min_check = if min > 0 {
            let lt_pos = self.bytecode.len();
            self.bytecode.push(OpCode::JmpLt as u8);
            self.bytecode.push(counter_reg as u8);
            self.bytecode.extend_from_slice(&(min as u32).to_le_bytes());
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());
            Some(lt_pos)
        } else {
            None
        };

        let push_pos = self.bytecode.len();
        self.bytecode.push(OpCode::PushBacktrack as u8);
        self.bytecode.extend_from_slice(&0i32.to_le_bytes());

        let body_pos;
        let skip_jmp_pos;
        if greedy {
            body_pos = push_pos + 5;
            skip_jmp_pos = None;
        } else {
            let jmp_pos = self.bytecode.len();
            self.bytecode.push(OpCode::Jmp as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());
            body_pos = jmp_pos + 5;
            skip_jmp_pos = Some(jmp_pos);
        }

        self.bytecode.push(OpCode::MovReg as u8);
        self.bytecode.push(REG_TEMP as u8);
        self.bytecode.push(REG_POS as u8);

        self.compile_node(inner)?;

        self.bytecode.push(OpCode::Inc as u8);
        self.bytecode.push(counter_reg as u8);

        let progress_jmp_pos = if max == u32::MAX as usize {
            let pos = self.bytecode.len();
            self.bytecode.push(OpCode::JmpNoProgress as u8);
            self.bytecode.push(REG_TEMP as u8);
            self.bytecode.extend_from_slice(&0i32.to_le_bytes());
            Some(pos)
        } else {
            None
        };

        let back_jmp_pos = self.bytecode.len();
        self.bytecode.push(OpCode::Jmp as u8);
        let back_offset = (loop_start as i32 - back_jmp_pos as i32 - 5) as i32;
        self.bytecode.extend_from_slice(&back_offset.to_le_bytes());

        let done_pos = self.bytecode.len();

        if let Some(progress_pos) = progress_jmp_pos {
            let progress_offset = (done_pos as i32 - progress_pos as i32 - 6) as i32;
            self.bytecode[progress_pos + 2..progress_pos + 6]
                .copy_from_slice(&progress_offset.to_le_bytes());
        }

        let push_target = if greedy { done_pos } else { body_pos };
        let push_offset = (push_target as i32 - push_pos as i32 - 5) as i32;
        self.bytecode[push_pos + 1..push_pos + 5].copy_from_slice(&push_offset.to_le_bytes());

        if let Some(jmp_pos) = skip_jmp_pos {
            let skip_offset = (done_pos as i32 - jmp_pos as i32 - 5) as i32;
            self.bytecode[jmp_pos + 1..jmp_pos + 5].copy_from_slice(&skip_offset.to_le_bytes());
        }

        if let Some(eq_pos) = max_check {
            let eq_offset = (done_pos as i32 - eq_pos as i32 - 10) as i32;
            self.bytecode[eq_pos + 6..eq_pos + 10].copy_from_slice(&eq_offset.to_le_bytes());
        }

        if let Some(lt_pos) = min_check {
            let lt_offset = (body_pos as i32 - lt_pos as i32 - 10) as i32;
            self.bytecode[lt_pos + 6..lt_pos + 10].copy_from_slice(&lt_offset.to_le_bytes());
        }

        self.repeat_depth -= 1;

        Ok(())
    }

    fn compile_capture(&mut self, inner: &Ast) -> Result<(), String> {
        let capture_idx = self.capture_count;
        self.capture_count += 1;

        self.emit_op_u8(OpCode::SaveStart, capture_idx as u8);
        self.compile_node(inner)?;
        self.emit_op_u8(OpCode::SaveEnd, capture_idx as u8);

        Ok(())
    }

    fn compile_backref(&mut self, idx: usize) -> Result<(), String> {
        if idx >= MAX_CAPTURES {
            return Err(format!("Backreference index too large: {}", idx));
        }

        let opcode = if self.ignore_case {
            OpCode::CheckBackrefI
        } else {
            OpCode::CheckBackref
        };

        self.emit_op_u8(opcode, idx as u8);
        Ok(())
    }

    fn compile_lookahead(&mut self, inner: &Ast, negative: bool) -> Result<(), String> {
        self.bytecode.push(OpCode::Mark as u8);
        self.bytecode.push(REG_MARK as u8);

        let push_pos = self.bytecode.len();
        self.bytecode.push(OpCode::PushBacktrack as u8);
        self.bytecode.extend_from_slice(&0i32.to_le_bytes());

        self.compile_node(inner)?;

        self.bytecode.push(OpCode::PopBacktrack as u8);

        if negative {
            self.bytecode.push(OpCode::Restore as u8);
            self.bytecode.push(REG_MARK as u8);
            self.bytecode.push(OpCode::Fail as u8);
        }

        self.bytecode.push(OpCode::Restore as u8);
        self.bytecode.push(REG_MARK as u8);

        let end_pos = self.bytecode.len();
        let offset = (end_pos as i32 - push_pos as i32 - 5) as i32;
        self.bytecode[push_pos + 1..push_pos + 5].copy_from_slice(&offset.to_le_bytes());

        if !negative {
            self.bytecode.push(OpCode::Fail as u8);
        }

        Ok(())
    }

    fn emit_op(&mut self, op: OpCode) {
        self.bytecode.push(op as u8);
    }

    fn emit_op_u8(&mut self, op: OpCode, val: u8) {
        self.bytecode.push(op as u8);
        self.bytecode.push(val);
    }

    fn emit_match_char(&mut self, reg: usize, ch: u16) {
        self.bytecode.push(OpCode::MatchChar as u8);
        self.bytecode.push(reg as u8);
        self.bytecode.extend_from_slice(&ch.to_le_bytes());
    }

    fn emit_match_char_i(&mut self, reg: usize, ch: u16) {
        self.bytecode.push(OpCode::MatchCharI as u8);
        self.bytecode.push(reg as u8);
        self.bytecode.extend_from_slice(&ch.to_le_bytes());
    }

    fn emit_match_char32(&mut self, reg: usize, ch: u32) {
        self.bytecode.push(OpCode::MatchChar32 as u8);
        self.bytecode.push(reg as u8);
        self.bytecode.extend_from_slice(&ch.to_le_bytes());
    }

    fn emit_match_char32_i(&mut self, reg: usize, ch: u32) {
        self.bytecode.push(OpCode::MatchChar32I as u8);
        self.bytecode.push(reg as u8);
        self.bytecode.extend_from_slice(&ch.to_le_bytes());
    }

    fn emit_mov_imm(&mut self, reg: usize, imm: usize) {
        self.bytecode.push(OpCode::MovImm as u8);
        self.bytecode.push(reg as u8);
        self.bytecode.extend_from_slice(&(imm as u32).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse;
    use super::*;

    #[test]
    fn test_compile_simple() {
        let ast = parse("abc", 0).unwrap();
        let prog = compile(&ast, 0).unwrap();
        assert!(prog.bytecode.len() > HEADER_LEN);
    }

    #[test]
    fn test_compile_capture() {
        let ast = parse("(a)", 0).unwrap();
        let prog = compile(&ast, 0).unwrap();
        assert_eq!(prog.capture_count, 2);
    }

    #[test]
    fn test_compile_alt() {
        let ast = parse("a|b", 0).unwrap();
        let prog = compile(&ast, 0).unwrap();
        assert!(prog.bytecode.len() > HEADER_LEN);
    }

    #[test]
    fn test_compile_quant() {
        let ast = parse("a*", 0).unwrap();
        let prog = compile(&ast, 0).unwrap();
        assert!(prog.bytecode.len() > HEADER_LEN);
    }
}

fn collect_named_groups(ast: &Ast, counter: &mut usize, out: &mut Vec<(String, usize)>) {
    match ast {
        Ast::Capture(inner, name) => {
            let idx = *counter;
            *counter += 1;
            if let Some(n) = name {
                out.push((n.clone(), idx));
            }
            collect_named_groups(inner, counter, out);
        }
        Ast::Concat(nodes) | Ast::Alt(nodes) => {
            for node in nodes {
                collect_named_groups(node, counter, out);
            }
        }
        Ast::Quant(inner, _) => collect_named_groups(inner, counter, out),
        Ast::Lookahead(inner) | Ast::NegativeLookahead(inner) => {
            collect_named_groups(inner, counter, out)
        }
        _ => {}
    }
}
