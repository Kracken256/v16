use crate::compiler::codegen::OptLevel;
use crate::compiler::opcode::{Bytecode, Opcode};

pub fn optimize(bytecode: &mut Bytecode) {
    optimize_with_level(bytecode, OptLevel::O2)
}

pub fn optimize_with_level(bytecode: &mut Bytecode, opt_level: OptLevel) {
    if matches!(opt_level, OptLevel::O0) {
        return;
    }
    let mut _total_changed = 0usize;
    for _ in 0..3 {
        let changed = run_pass(bytecode);
        if changed {
            _total_changed += 1;
        } else {
            break;
        }
    }
}

fn run_pass(bytecode: &mut Bytecode) -> bool {
    let mut changed = false;
    let mut pc = 0usize;
    let code = &mut bytecode.code;

    while pc < code.len() {
        let op = Opcode::from_u8_unchecked(code[pc]);
        let size = Opcode::instruction_size_at(code, pc);
        if size == 0 || pc + size > code.len() {
            break;
        }

        if op == Opcode::Move {
            let dst = read_u16(code, pc + 1);
            let src = read_u16(code, pc + 3);
            if dst == src {
                code[pc] = Opcode::Nop as u8;
                code[pc + 1] = 0;
                code[pc + 2] = 0;
                code[pc + 3] = 0;
                code[pc + 4] = 0;
                changed = true;
            }
        }

        if op == Opcode::Jump {
            let offset = read_i32(code, pc + 1);
            let target = (pc as i32 + size as i32 + offset) as usize;
            if target < code.len() {
                let target_op = Opcode::from_u8_unchecked(code[target]);
                if target_op == Opcode::Jump {
                    let target_offset = read_i32(code, target + 1);
                    let final_target = (target as i32
                        + Opcode::instruction_size(Opcode::Jump) as i32
                        + target_offset) as i32;
                    let new_offset = final_target - (pc as i32 + size as i32);
                    write_i32(code, pc + 1, new_offset);
                    changed = true;
                }
            }
        }

        if op == Opcode::LoadInt {
            let dst1 = read_u16(code, pc + 1);
            let a = read_i32(code, pc + 3);
            let next1 = pc + size;
            if next1 + size <= code.len()
                && Opcode::from_u8_unchecked(code[next1]) == Opcode::LoadInt
            {
                let dst2 = read_u16(code, next1 + 1);
                let b = read_i32(code, next1 + 3);
                let next2 = next1 + size;
                if next2 + size <= code.len() && dst1 != dst2 {
                    let op3 = Opcode::from_u8_unchecked(code[next2]);
                    let src1 = read_u16(code, next2 + 3);
                    let src2 = read_u16(code, next2 + 5);
                    if src1 == dst1 && src2 == dst2 {
                        let result = match op3 {
                            Opcode::Add => a.checked_add(b),
                            Opcode::Sub => a.checked_sub(b),
                            Opcode::Mul => a.checked_mul(b),
                            Opcode::BitAnd => Some(a & b),
                            Opcode::BitOr => Some(a | b),
                            Opcode::BitXor => Some(a ^ b),
                            _ => None,
                        };
                        if let Some(r) = result {
                            code[pc] = Opcode::Nop as u8;
                            for i in 1..size {
                                code[pc + i] = 0;
                            }
                            code[next1] = Opcode::LoadInt as u8;
                            code[next1 + 1] = code[next2 + 1];
                            code[next1 + 2] = code[next2 + 2];
                            write_i32(code, next1 + 3, r);
                            code[next2] = Opcode::Nop as u8;
                            for i in 1..size {
                                code[next2 + i] = 0;
                            }
                            changed = true;
                            pc = next2 + size;
                            continue;
                        }
                    }
                }
            }
        }

        if op == Opcode::LoadTrue || op == Opcode::LoadFalse {
            let is_true = op == Opcode::LoadTrue;
            let dst = read_u16(code, pc + 1);
            let next_pc = pc + size;
            if next_pc < code.len() {
                let next_op = Opcode::from_u8_unchecked(code[next_pc]);
                if next_op == Opcode::JumpIf || next_op == Opcode::JumpIfNot {
                    let src = read_u16(code, next_pc + 1);
                    let offset = read_i32(code, next_pc + 3);
                    if src == dst {
                        let jump_taken = if next_op == Opcode::JumpIf {
                            is_true
                        } else {
                            !is_true
                        };
                        if jump_taken {
                            let new_offset = offset + 2;
                            code[next_pc] = Opcode::Jump as u8;
                            let bytes = new_offset.to_le_bytes();
                            code[next_pc + 1] = bytes[0];
                            code[next_pc + 2] = bytes[1];
                            code[next_pc + 3] = bytes[2];
                            code[next_pc + 4] = bytes[3];

                            code[next_pc + 5] = 0;
                            code[next_pc + 6] = 0;
                        } else {
                            code[next_pc] = Opcode::Nop as u8;
                            for i in 1..7 {
                                code[next_pc + i] = 0;
                            }
                        }

                        code[pc] = Opcode::Nop as u8;
                        for i in 1..size {
                            code[pc + i] = 0;
                        }
                        changed = true;
                    }
                } else if next_op == Opcode::JumpIf8 || next_op == Opcode::JumpIfNot8 {
                    let src = read_u16(code, next_pc + 1);
                    let jif8_offset = code[next_pc + 3] as i8;
                    if src == dst {
                        let jump_taken = if next_op == Opcode::JumpIf8 {
                            is_true
                        } else {
                            !is_true
                        };
                        if jump_taken {
                            let adjusted = jif8_offset as i32 + 2;
                            if adjusted >= i8::MIN as i32 && adjusted <= i8::MAX as i32 {
                                code[next_pc] = Opcode::Jump8 as u8;
                                code[next_pc + 1] = adjusted as i8 as u8;
                                code[next_pc + 2] = Opcode::Nop as u8;
                                code[next_pc + 3] = 0;
                                code[pc] = Opcode::Nop as u8;
                                for i in 1..size {
                                    code[pc + i] = 0;
                                }
                                changed = true;
                            }
                        } else {
                            code[pc] = Opcode::Nop as u8;
                            for i in 1..size {
                                code[pc + i] = 0;
                            }
                            code[next_pc] = Opcode::Nop as u8;
                            for i in 1..4 {
                                code[next_pc + i] = 0;
                            }
                            changed = true;
                        }
                    }
                }
            }
        }

        pc += size;
    }

    changed
}

#[inline(always)]
fn write_i32(code: &mut [u8], offset: usize, val: i32) {
    let bytes = val.to_le_bytes();
    if offset + 3 >= code.len() {
        return;
    }
    code[offset] = bytes[0];
    code[offset + 1] = bytes[1];
    code[offset + 2] = bytes[2];
    code[offset + 3] = bytes[3];
}

#[inline(always)]
fn read_u16(code: &[u8], offset: usize) -> u16 {
    if offset + 1 >= code.len() {
        return 0;
    }
    u16::from_le_bytes([code[offset], code[offset + 1]])
}

#[inline(always)]
fn read_i32(code: &[u8], offset: usize) -> i32 {
    if offset + 3 >= code.len() {
        return 0;
    }
    i32::from_le_bytes([
        code[offset],
        code[offset + 1],
        code[offset + 2],
        code[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_int(dst: u16, val: i32) -> Vec<u8> {
        let mut code = vec![Opcode::LoadInt as u8];
        code.extend_from_slice(&dst.to_le_bytes());
        code.extend_from_slice(&val.to_le_bytes());
        code
    }

    fn unary(op: Opcode, dst: u16, src: u16) -> Vec<u8> {
        let mut code = vec![op as u8];
        code.extend_from_slice(&dst.to_le_bytes());
        code.extend_from_slice(&src.to_le_bytes());
        code
    }

    fn binary(op: Opcode, dst: u16, a: u16, b: u16) -> Vec<u8> {
        let mut code = vec![op as u8];
        code.extend_from_slice(&dst.to_le_bytes());
        code.extend_from_slice(&a.to_le_bytes());
        code.extend_from_slice(&b.to_le_bytes());
        code
    }

    fn optimize_code(code: Vec<u8>) -> Vec<u8> {
        let mut bytecode = Bytecode::new();
        bytecode.code = code;
        optimize_with_level(&mut bytecode, OptLevel::O2);
        bytecode.code
    }

    #[test]
    fn self_move_becomes_nop() {
        let out = optimize_code(unary(Opcode::Move, 3, 3));
        assert_eq!(out[0], Opcode::Nop as u8);
    }

    #[test]
    fn jump_offset_equal_to_own_size_is_preserved() {
        let mut code = vec![Opcode::Jump as u8];
        code.extend_from_slice(&5i32.to_le_bytes());
        code.extend_from_slice(&[Opcode::Nop as u8; 7]);
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::Jump as u8);
        assert_eq!(read_i32(&out, 1), 5);
    }

    #[test]
    fn double_not_pair_is_preserved() {
        let mut code = unary(Opcode::Not, 1, 0);
        code.extend_from_slice(&unary(Opcode::Not, 1, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::Not as u8);
        assert_eq!(out[5], Opcode::Not as u8);
    }

    #[test]
    fn load_int_add_zero_is_preserved() {
        let mut code = load_int(1, 0);
        code.extend_from_slice(&binary(Opcode::Add, 2, 3, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 3), 0);
    }

    #[test]
    fn load_int_sub_zero_is_preserved() {
        let mut code = load_int(1, 0);
        code.extend_from_slice(&binary(Opcode::Sub, 2, 2, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 3), 0);
    }

    #[test]
    fn load_int_mul_one_is_preserved() {
        let mut code = load_int(1, 1);
        code.extend_from_slice(&binary(Opcode::Mul, 2, 3, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 3), 1);
    }

    #[test]
    fn load_int_move_pair_is_preserved() {
        let mut code = load_int(1, 5);
        code.extend_from_slice(&unary(Opcode::Move, 2, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(out[7], Opcode::Move as u8);
    }

    #[test]
    fn load_int_pair_with_binop_folds_to_constant() {
        let mut code = load_int(1, 300);
        code.extend_from_slice(&load_int(2, 500));
        code.extend_from_slice(&binary(Opcode::Add, 3, 1, 2));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::Nop as u8);
        assert_eq!(out[7], Opcode::LoadInt as u8);
        assert_eq!(read_u16(&out, 8), 3);
        assert_eq!(read_i32(&out, 10), 800);
        assert_eq!(out[14], Opcode::Nop as u8);
    }

    #[test]
    fn load_int_pair_with_aliased_dst_is_preserved() {
        let mut code = load_int(1, 300);
        code.extend_from_slice(&load_int(1, 500));
        code.extend_from_slice(&binary(Opcode::Add, 2, 1, 1));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 3), 300);
        assert_eq!(out[7], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 10), 500);
    }

    #[test]
    fn load_int_fold_overflow_is_preserved() {
        let mut code = load_int(1, i32::MAX);
        code.extend_from_slice(&load_int(2, 1));
        code.extend_from_slice(&binary(Opcode::Add, 3, 1, 2));
        let out = optimize_code(code);
        assert_eq!(out[0], Opcode::LoadInt as u8);
        assert_eq!(read_i32(&out, 3), i32::MAX);
        assert_eq!(out[7], Opcode::LoadInt as u8);
        assert_eq!(out[14], Opcode::Add as u8);
    }

    fn embedded_function(dst: u16) -> Vec<u8> {
        let mut code = vec![Opcode::NewFunction as u8];
        code.extend_from_slice(&0u16.to_le_bytes());
        code.push(0);
        code.push(0);
        code.push(1);
        code.push(0);
        code.extend_from_slice(&2u16.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend(std::iter::repeat(Opcode::Nop as u8).take(3));
        code.extend_from_slice(&1u32.to_le_bytes());
        code.push(0);
        code.extend_from_slice(&1u32.to_le_bytes());
        code.extend_from_slice(&[0u8; 8]);
        code.extend_from_slice(&2u32.to_le_bytes());
        code.extend_from_slice(&[0u8; 8]);
        code.extend_from_slice(&[0u8; 8]);
        code.extend_from_slice(&123u32.to_le_bytes());
        code.extend_from_slice(&1u32.to_le_bytes());
        code.extend_from_slice(&[0u8; 7]);
        code.extend_from_slice(&dst.to_le_bytes());
        code
    }

    #[test]
    fn load_int_pair_fold_applies_after_embedded_function() {
        let mut code = embedded_function(0);
        code.extend_from_slice(&load_int(1, 300));
        code.extend_from_slice(&load_int(2, 500));
        code.extend_from_slice(&binary(Opcode::Add, 3, 1, 2));
        let out = optimize_code(code);
        let fold_at = out.len() - 14;
        assert_eq!(out[fold_at], Opcode::LoadInt as u8);
        assert_eq!(read_u16(&out, fold_at + 1), 3);
        assert_eq!(read_i32(&out, fold_at + 3), 800);
        assert_eq!(out[fold_at - 7], Opcode::Nop as u8);
        assert_eq!(out[fold_at + 7], Opcode::Nop as u8);
    }
}
