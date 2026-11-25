use crate::compiler::ast::*;
use crate::compiler::location::LineNumberTable;
use crate::compiler::opcode::{Bytecode, Opcode};
use crate::runtime::context::JSContext;
use crate::util::FxHashMap;
use crate::value::JSValue;

enum ChainStep {
    Member {
        property: MemberProperty,
        optional: bool,
    },
    Call {
        arguments: Vec<Argument>,
        optional: bool,
    },
}

#[derive(Debug)]
struct BreakableFrame {
    labels: Vec<String>,
    is_label: bool,
    break_patches: Vec<usize>,
    continue_patches: Vec<usize>,
    continue_target: Option<usize>,
    handler_depth: usize,
    continue_depth: usize,
}

struct DisposalRegion {
    stack: u16,
    state: u16,
    index: u16,
    awaited: u16,
    error: u16,
    flags: u16,
    catch_pc_pos: usize,
    finally_pc_pos: usize,
}

impl DisposalRegion {
    fn alloc(cg: &mut CodeGenerator) -> Self {
        let stack = cg.alloc_register();
        let state = cg.alloc_register();
        let index = cg.alloc_register();
        let awaited = cg.alloc_register();
        let error = cg.alloc_register();
        let flags = cg.alloc_register();
        DisposalRegion {
            stack,
            state,
            index,
            awaited,
            error,
            flags,
            catch_pc_pos: 0,
            finally_pc_pos: 0,
        }
    }

    fn free(self, cg: &mut CodeGenerator) {
        cg.free_register(self.stack);
        cg.free_register(self.state);
        cg.free_register(self.index);
        cg.free_register(self.awaited);
        cg.free_register(self.error);
        cg.free_register(self.flags);
    }
}

fn body_has_using_declaration(body: &[ASTNode]) -> bool {
    body.iter().any(
        |stmt| matches!(stmt, ASTNode::VariableDeclaration(decl) if decl.kind.is_using()),
    )
}

#[derive(Clone)]
struct WithScope {
    function_id: u32,
    scope_index: usize,
    inner_bindings: std::collections::HashSet<String>,
}

enum DestructuringReference {
    Identifier {
        name: String,
        location: Option<VarLocation>,
    },
    Named {
        object: u16,
        atom: crate::runtime::atom::Atom,
    },
    Computed {
        object: u16,
        key: u16,
    },
    Private {
        object: u16,
        key: u16,
        owner_reg: u16,
        owner_temp: bool,
    },
}

#[derive(Debug, Clone, Copy)]
enum VarLocation {
    Local(u16),
    Upvalue(u16),
}

#[derive(Debug, Clone, Copy)]
struct ParentVar {
    local_idx: u16,
    kind: VariableKind,
    initialized: bool,
    is_inherited: bool,
    closed: bool,
    is_function_name: bool,
    is_block_decl: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    O0,
    O1,
    O2,
    O3,
}

impl Default for OptLevel {
    fn default() -> Self {
        Self::O2
    }
}

impl OptLevel {
    pub fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "-O0" => Some(Self::O0),
            "-O1" => Some(Self::O1),
            "-O2" => Some(Self::O2),
            "-O3" => Some(Self::O3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P2FoldProfile {
    Raw,
    Ternary,
    Logical,
    Nullish,
    Stable,
    Aggressive,
}

impl P2FoldProfile {
    fn from_env_override() -> Option<Self> {
        let mode = std::env::var("V16_P2_PROFILE").ok()?;
        match mode.as_str() {
            "raw" => Some(Self::Raw),
            "ternary" => Some(Self::Ternary),
            "logical" => Some(Self::Logical),
            "nullish" => Some(Self::Nullish),
            "stable" => Some(Self::Stable),
            "aggressive" => Some(Self::Aggressive),
            _ => None,
        }
    }

    fn allow_ternary_bool(self) -> bool {
        matches!(
            self,
            Self::Ternary | Self::Logical | Self::Nullish | Self::Stable | Self::Aggressive
        )
    }

    fn allow_logical_bool_left(self) -> bool {
        matches!(
            self,
            Self::Logical | Self::Nullish | Self::Stable | Self::Aggressive
        )
    }

    fn allow_nullish_literal_left(self) -> bool {
        matches!(self, Self::Nullish | Self::Stable | Self::Aggressive)
    }

    fn allow_if_bool(self) -> bool {
        matches!(self, Self::Stable | Self::Aggressive)
    }

    fn allow_aggressive_truthy(self) -> bool {
        matches!(self, Self::Aggressive)
    }
}

#[derive(Debug, Clone, Copy)]
struct CodegenSettings {
    p2_fold_profile: P2FoldProfile,
    opt_jump_if_nullish: bool,
    opt_call23: bool,
    opt_fused_cmp_jump: bool,
    opt_fused_getprop_call: bool,
    opt_tiny_inline: bool,
    opt_branch_result_prealloc: bool,
}

impl CodegenSettings {
    fn for_opt_level(opt_level: OptLevel) -> Self {
        match opt_level {
            OptLevel::O0 => Self {
                p2_fold_profile: P2FoldProfile::Raw,
                opt_jump_if_nullish: false,
                opt_call23: false,
                opt_fused_cmp_jump: false,
                opt_fused_getprop_call: false,
                opt_tiny_inline: false,
                opt_branch_result_prealloc: false,
            },
            OptLevel::O1 => Self {
                p2_fold_profile: P2FoldProfile::Ternary,
                opt_jump_if_nullish: true,
                opt_call23: true,
                opt_fused_cmp_jump: false,
                opt_fused_getprop_call: true,
                opt_tiny_inline: false,
                opt_branch_result_prealloc: true,
            },
            OptLevel::O2 => Self {
                p2_fold_profile: P2FoldProfile::Stable,
                opt_jump_if_nullish: true,
                opt_call23: true,
                opt_fused_cmp_jump: true,
                opt_fused_getprop_call: true,
                opt_tiny_inline: true,
                opt_branch_result_prealloc: true,
            },
            OptLevel::O3 => Self {
                p2_fold_profile: P2FoldProfile::Aggressive,
                opt_jump_if_nullish: true,
                opt_call23: true,
                opt_fused_cmp_jump: false,
                opt_fused_getprop_call: true,
                opt_tiny_inline: true,
                opt_branch_result_prealloc: true,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VarEntry {
    pub(crate) slot: u16,
    pub(crate) kind: VariableKind,
    pub(crate) initialized: bool,
    pub(crate) is_function_name: bool,
}

pub struct CodeGenerator {
    code: Vec<u8>,
    constants: Vec<JSValue>,
    next_register: u16,
    max_register: u16,
    free_list: Vec<u16>,
    scope_stack: Vec<FxHashMap<String, VarEntry>>,
    reserved_slots: std::collections::HashSet<u16>,
    breakable_frames: Vec<BreakableFrame>,
    handler_depth: usize,
    scope_dispose_stack: Vec<Option<u16>>,

    parent_vars: FxHashMap<String, ParentVar>,
    upvalues: Vec<(u32, u16)>,
    upvalue_slots: FxHashMap<String, u16>,
    line_table: LineNumberTable,
    depth: u32,
    is_script_level: bool,
    scope_is_global: Vec<bool>,
    global_lexical_names: std::collections::HashSet<String>,
    function_uses_arguments: bool,
    arguments_slot: Option<u16>,
    needs_parent_arguments: bool,
    simple_params: bool,
    current_function_name: Option<String>,
    tiny_inline_functions: FxHashMap<String, FunctionDeclaration>,
    p2_fold_profile: P2FoldProfile,
    opt_jump_if_nullish: bool,
    opt_call23: bool,
    opt_fused_cmp_jump: bool,
    opt_fused_getprop_call: bool,
    opt_tiny_inline: bool,
    opt_branch_result_prealloc: bool,
    suppress_ref_error: bool,
    pub(crate) is_strict: bool,
    pub(crate) is_eval: bool,
    is_arrow: bool,
    in_derived_constructor: bool,
    in_static_method: bool,
    in_async_generator: bool,
    pub(crate) in_field_initializer: bool,
    in_parameter_initializer: bool,
    in_default_derived_constructor: bool,
    pending_field_init_range: Option<(usize, usize)>,
    field_init_name: Option<String>,
    pending_labels: Vec<String>,
    all_var_names: Vec<(String, u16, u8)>,
    with_scopes: Vec<WithScope>,
    function_id: u32,
}

impl CodeGenerator {
    pub fn new() -> Self {
        Self::with_opt_level(OptLevel::default())
    }

    pub fn with_opt_level(opt_level: OptLevel) -> Self {
        let mut settings = CodegenSettings::for_opt_level(opt_level);
        if let Some(profile) = P2FoldProfile::from_env_override() {
            settings.p2_fold_profile = profile;
        }
        settings.opt_jump_if_nullish =
            Self::env_flag("V16_OPT_JUMP_NULLISH", settings.opt_jump_if_nullish);
        settings.opt_call23 = Self::env_flag("V16_OPT_CALL23", settings.opt_call23);
        settings.opt_fused_cmp_jump =
            Self::env_flag("V16_OPT_FUSED_CMP_JUMP", settings.opt_fused_cmp_jump);
        settings.opt_fused_getprop_call = Self::env_flag(
            "V16_OPT_FUSED_GETPROP_CALL",
            settings.opt_fused_getprop_call,
        );
        settings.opt_tiny_inline = Self::env_flag("V16_OPT_TINY_INLINE", settings.opt_tiny_inline);
        settings.opt_branch_result_prealloc = Self::env_flag(
            "V16_OPT_BRANCH_RESULT_PREALLOC",
            settings.opt_branch_result_prealloc,
        );
        Self::with_settings(settings)
    }

    fn with_settings(settings: CodegenSettings) -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            next_register: 0,
            max_register: 0,
            free_list: Vec::new(),
            scope_stack: vec![FxHashMap::default()],
            reserved_slots: std::collections::HashSet::new(),
            breakable_frames: Vec::new(),
            handler_depth: 0,
            scope_dispose_stack: vec![None],
            parent_vars: FxHashMap::default(),
            upvalues: Vec::new(),
            upvalue_slots: FxHashMap::default(),
            line_table: LineNumberTable::new(),
            depth: 0,
            is_script_level: true,
            scope_is_global: vec![false],
            global_lexical_names: std::collections::HashSet::new(),
            function_uses_arguments: false,
            arguments_slot: None,
            needs_parent_arguments: false,
            simple_params: true,
            current_function_name: None,
            tiny_inline_functions: FxHashMap::default(),
            p2_fold_profile: settings.p2_fold_profile,
            opt_jump_if_nullish: settings.opt_jump_if_nullish,
            opt_call23: settings.opt_call23,
            opt_fused_cmp_jump: settings.opt_fused_cmp_jump,
            opt_fused_getprop_call: settings.opt_fused_getprop_call,
            opt_tiny_inline: settings.opt_tiny_inline,
            opt_branch_result_prealloc: settings.opt_branch_result_prealloc,
            suppress_ref_error: false,
            is_strict: false,
            is_eval: false,
            is_arrow: false,
            in_derived_constructor: false,
            in_static_method: false,
            in_async_generator: false,
            in_field_initializer: false,
            in_parameter_initializer: false,
            in_default_derived_constructor: false,
            pending_field_init_range: None,
            field_init_name: None,
            pending_labels: Vec::new(),
            all_var_names: Vec::new(),
            with_scopes: Vec::new(),
            function_id: 0,
        }
    }

    fn settings(&self) -> CodegenSettings {
        CodegenSettings {
            p2_fold_profile: self.p2_fold_profile,
            opt_jump_if_nullish: self.opt_jump_if_nullish,
            opt_call23: self.opt_call23,
            opt_fused_cmp_jump: self.opt_fused_cmp_jump,
            opt_fused_getprop_call: self.opt_fused_getprop_call,
            opt_tiny_inline: self.opt_tiny_inline,
            opt_branch_result_prealloc: self.opt_branch_result_prealloc,
        }
    }

    fn emit_reset_per_iter_bindings(&mut self) {
        let slots: Vec<u16> = self
            .scope_stack
            .last()
            .map(|scope| {
                scope
                    .iter()
                    .filter(|(_, entry)| entry.kind != VariableKind::Var)
                    .map(|(_, entry)| entry.slot)
                    .collect()
            })
            .unwrap_or_default();
        for slot in slots {
            self.emit(Opcode::ResetPerIterVar);
            self.emit_u16(slot);
        }
    }

    fn statement_is_iteration_statement(stmt: &ASTNode) -> bool {
        matches!(
            stmt,
            ASTNode::ForStatement(_)
                | ASTNode::ForInStatement(_)
                | ASTNode::ForOfStatement(_)
                | ASTNode::WhileStatement(_)
                | ASTNode::DoWhileStatement(_)
        )
    }

    fn nested_codegen(&self) -> Self {
        let mut nested = Self::with_settings(self.settings());
        nested.tiny_inline_functions = self.tiny_inline_functions.clone();
        nested.is_strict = self.is_strict;
        nested.is_eval = self.is_eval;
        nested.in_derived_constructor = self.in_derived_constructor;
        nested.in_static_method = self.in_static_method;
        nested.with_scopes = self.with_scopes.clone();
        nested.function_id = self.function_id + 1;
        nested
    }

    fn env_flag(name: &str, default: bool) -> bool {
        match std::env::var(name) {
            Ok(v) => match v.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                _ => default,
            },
            Err(_) => default,
        }
    }

    fn literal_truthiness(lit: &Literal) -> Option<bool> {
        match lit {
            Literal::Boolean(b) => Some(*b),
            Literal::Null | Literal::Undefined => Some(false),
            Literal::Number(n) => Some(*n != 0.0 && !n.is_nan()),
            Literal::LegacyOctal(n) => Some(*n != 0),
            Literal::String(s, _, _) => Some(!s.is_empty()),
            Literal::BigInt(_) => None,
        }
    }

    fn detect_tiny_inline_function(func: &FunctionDeclaration) -> bool {
        if func.generator || func.is_async {
            return false;
        }
        if func.params.len() != 1 {
            return false;
        }
        let param_name = match &func.params[0] {
            Parameter::Identifier(name) => name,
            _ => return false,
        };
        if func.body.body.len() != 1 {
            return false;
        }
        let ret = match &func.body.body[0] {
            ASTNode::ReturnStatement(stmt) => stmt,
            _ => return false,
        };
        let expr = match &ret.argument {
            Some(expr) => expr,
            None => return false,
        };
        match expr {
            Expression::MemberExpression(member) => match member.object.as_ref() {
                Expression::Identifier(id) => {
                    id.name == *param_name
                        && matches!(member.property, MemberProperty::Identifier(_))
                }
                Expression::MemberExpression(inner) => {
                    if inner.computed || member.computed {
                        return false;
                    }
                    match inner.object.as_ref() {
                        Expression::Identifier(id) => {
                            id.name == *param_name
                                && matches!(inner.property, MemberProperty::Identifier(_))
                                && matches!(member.property, MemberProperty::Identifier(_))
                        }
                        _ => false,
                    }
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn try_emit_tiny_inline_call(
        &mut self,
        call: &CallExpression,
        ctx: &mut JSContext,
    ) -> Result<Option<u16>, String> {
        let callee_name = match call.callee.as_ref() {
            Expression::Identifier(id) => &id.name,
            _ => return Ok(None),
        };
        let func = match self.tiny_inline_functions.get(callee_name).cloned() {
            Some(func) => func,
            None => return Ok(None),
        };
        if call.arguments.len() != 1 {
            return Ok(None);
        }
        let arg_reg = match &call.arguments[0] {
            Argument::Expression(expr) => self.gen_expression(expr, ctx)?,
            Argument::Spread(_) => return Ok(None),
        };

        let ret = match &func.body.body[0] {
            ASTNode::ReturnStatement(stmt) => stmt,
            _ => return Ok(None),
        };
        let expr = match &ret.argument {
            Some(expr) => expr,
            None => return Ok(None),
        };

        let dst = self.alloc_register();
        match expr {
            Expression::MemberExpression(member) => match member.object.as_ref() {
                Expression::Identifier(_) => {
                    if let MemberProperty::Identifier(name) = &member.property {
                        let atom = ctx.intern(name);
                        self.emit(Opcode::GetNamedProp);
                        self.emit_u16(dst);
                        self.emit_u16(arg_reg);
                        self.emit_u32(atom.0);
                    } else {
                        self.free_register(arg_reg);
                        self.free_register(dst);
                        return Ok(None);
                    }
                }
                Expression::MemberExpression(inner) => {
                    let mid = self.alloc_register();
                    let atom1 = match &inner.property {
                        MemberProperty::Identifier(name) => ctx.intern(name),
                        _ => {
                            self.free_register(arg_reg);
                            self.free_register(mid);
                            self.free_register(dst);
                            return Ok(None);
                        }
                    };
                    let atom2 = match &member.property {
                        MemberProperty::Identifier(name) => ctx.intern(name),
                        _ => {
                            self.free_register(arg_reg);
                            self.free_register(mid);
                            self.free_register(dst);
                            return Ok(None);
                        }
                    };
                    self.emit(Opcode::GetNamedProp);
                    self.emit_u16(mid);
                    self.emit_u16(arg_reg);
                    self.emit_u32(atom1.0);
                    self.emit(Opcode::GetNamedProp);
                    self.emit_u16(dst);
                    self.emit_u16(mid);
                    self.emit_u32(atom2.0);
                    self.free_register(mid);
                }
                _ => {
                    self.free_register(arg_reg);
                    self.free_register(dst);
                    return Ok(None);
                }
            },
            _ => {
                self.free_register(arg_reg);
                self.free_register(dst);
                return Ok(None);
            }
        }

        self.free_register(arg_reg);
        Ok(Some(dst))
    }

    fn fused_op_from_binary_cmp(bin_op: &BinaryOp, jump_if_true: bool) -> Option<Opcode> {
        use BinaryOp::*;
        let op = match (bin_op, jump_if_true) {
            (Lt, true) => Opcode::LtJumpIf,
            (Lt, false) => Opcode::LtJumpIfNot,
            (Lte, true) => Opcode::LteJumpIf,
            (Lte, false) => Opcode::LteJumpIfNot,
            (Gt, true) => Opcode::GtJumpIf,
            (Gt, false) => Opcode::GtJumpIfNot,
            (Gte, true) => Opcode::GteJumpIf,
            (Gte, false) => Opcode::GteJumpIfNot,
            (Eq, true) => Opcode::EqJumpIf,
            (Eq, false) => Opcode::EqJumpIfNot,
            (Neq, true) => Opcode::NeqJumpIf,
            (Neq, false) => Opcode::NeqJumpIfNot,
            (StrictEq, true) => Opcode::StrictEqJumpIf,
            (StrictEq, false) => Opcode::StrictEqJumpIfNot,
            (StrictNeq, true) => Opcode::StrictNeqJumpIf,
            (StrictNeq, false) => Opcode::StrictNeqJumpIfNot,
            _ => return None,
        };
        Some(op)
    }

    fn emit(&mut self, op: Opcode) {
        self.code.push(op as u8);
    }

    fn emit_u8(&mut self, v: u8) {
        self.code.push(v);
    }

    fn emit_u16(&mut self, v: u16) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_i32(&mut self, v: i32) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_u32(&mut self, v: u32) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_int_const(&mut self, dst: u16, val: i32) {
        if (i8::MIN as i32..=i8::MAX as i32).contains(&val) {
            self.emit(Opcode::LoadInt8);
            self.emit_u16(dst);
            self.emit_u8(val as i8 as u8);
        } else {
            self.emit(Opcode::LoadInt);
            self.emit_u16(dst);
            self.emit_i32(val);
        }
    }

    fn emit_bool_const(&mut self, dst: u16, b: bool) {
        if b {
            self.emit(Opcode::LoadTrue);
        } else {
            self.emit(Opcode::LoadFalse);
        }
        self.emit_u16(dst);
    }

    fn emit_backward_jump(&mut self, target: usize) {
        let compact_offset = (target as i64 - (self.code.len() as i64 + 2)) as i32;
        if compact_offset >= i8::MIN as i32 && compact_offset <= i8::MAX as i32 {
            self.emit(Opcode::Jump8);
            self.emit_u8(compact_offset as i8 as u8);
        } else {
            let full_offset = (target as i64 - (self.code.len() as i64 + 5)) as i32;
            self.emit(Opcode::Jump);
            self.emit_i32(full_offset);
        }
    }

    fn emit_iterator_close(&mut self, iter_reg: u16, done_reg: Option<u16>) {
        let skip_patch = if let Some(d) = done_reg {
            self.emit(Opcode::JumpIf);
            self.emit_u16(d);
            let p = self.code.len();
            self.emit_i32(0);
            Some(p)
        } else {
            None
        };
        self.emit(Opcode::IteratorClose);
        self.emit_u16(iter_reg);
        if let Some(p) = skip_patch {
            let end = self.code.len();
            self.patch_jump(p, end);
        }
    }

    fn begin_iterator_close_region(&mut self) -> (usize, usize) {
        self.emit(Opcode::Try);
        self.handler_depth += 1;
        let catch_pc_pos = self.code.len();
        self.emit_i32(0);
        let finally_pc_pos = self.code.len();
        self.emit_i32(0);
        (catch_pc_pos, finally_pc_pos)
    }

    fn end_iterator_close_region(
        &mut self,
        catch_pc_pos: usize,
        finally_pc_pos: usize,
        iter_reg: u16,
        done_reg: u16,
        always_close: bool,
    ) {
        self.emit(Opcode::Catch);
        self.handler_depth -= 1;
        self.emit(Opcode::Jump);
        let normal_jump = self.code.len();
        self.emit_i32(0);

        let finally_start = self.code.len();
        let finally_bytes = (finally_start as i32).to_le_bytes();
        self.code[catch_pc_pos..catch_pc_pos + 4].copy_from_slice(&finally_bytes);
        self.code[finally_pc_pos..finally_pc_pos + 4].copy_from_slice(&finally_bytes);

        self.emit(Opcode::Finally);
        self.emit(Opcode::JumpIf);
        self.emit_u16(done_reg);
        let close_skip = self.code.len();
        self.emit_i32(0);
        self.emit(Opcode::IteratorCloseFinally);
        self.emit_u16(iter_reg);
        let close_end = self.code.len();
        self.patch_jump(close_skip, close_end);
        self.emit(Opcode::EndFinally);

        let normal_start = self.code.len();
        self.patch_jump(normal_jump, normal_start);
        if always_close {
            self.emit(Opcode::IteratorClose);
            self.emit_u16(iter_reg);
        } else {
            self.emit(Opcode::JumpIf);
            self.emit_u16(done_reg);
            let skip = self.code.len();
            self.emit_i32(0);
            self.emit(Opcode::IteratorClose);
            self.emit_u16(iter_reg);
            let end = self.code.len();
            self.patch_jump(skip, end);
        }
    }

    fn emit_conditional_backward_jump(&mut self, src: u16, target: usize, jump_if_true: bool) {
        let compact_offset = (target as i64 - (self.code.len() as i64 + 4)) as i32;
        if compact_offset >= i8::MIN as i32 && compact_offset <= i8::MAX as i32 {
            if jump_if_true {
                self.emit(Opcode::JumpIf8);
            } else {
                self.emit(Opcode::JumpIfNot8);
            }
            self.emit_u16(src);
            self.emit_u8(compact_offset as i8 as u8);
        } else {
            let full_offset = (target as i64 - (self.code.len() as i64 + 7)) as i32;
            if jump_if_true {
                self.emit(Opcode::JumpIf);
            } else {
                self.emit(Opcode::JumpIfNot);
            }
            self.emit_u16(src);
            self.emit_i32(full_offset);
        }
    }

    fn patch_jump(&mut self, patch_pos: usize, target_pos: usize) {
        if patch_pos >= 3 && self.code[patch_pos - 3] == Opcode::JumpBreak as u8 {
            let offset = (target_pos as i64 - patch_pos as i64 - 4) as i32;
            self.code[patch_pos..patch_pos + 4].copy_from_slice(&offset.to_le_bytes());
            return;
        }
        if patch_pos >= 1 {
            let op_byte = self.code[patch_pos - 1];
            let is_uncond = op_byte == Opcode::Jump as u8;
            let is_cond = op_byte == Opcode::JumpIf as u8 || op_byte == Opcode::JumpIfNot as u8;

            if is_uncond || is_cond {
                let compact_offset = (target_pos as i64 - patch_pos as i64 - 1) as i32;
                if compact_offset >= i8::MIN as i32 && compact_offset <= i8::MAX as i32 {
                    if is_uncond {
                        self.code[patch_pos - 1] = Opcode::Jump8 as u8;
                    } else {
                        self.code[patch_pos - 3] = if op_byte == Opcode::JumpIf as u8 {
                            Opcode::JumpIf8 as u8
                        } else {
                            Opcode::JumpIfNot8 as u8
                        };
                    }
                    self.code[patch_pos] = compact_offset as i8 as u8;

                    let fill_end = patch_pos + 4;
                    for i in (patch_pos + 1)..fill_end {
                        self.code[i] = Opcode::Nop as u8;
                    }
                    return;
                }
            }
        }

        if patch_pos >= 6 {
            let fused_op = self.code[patch_pos - 5];
            let is_fused = fused_op == Opcode::LtJumpIfNot as u8
                || fused_op == Opcode::LtJumpIf as u8
                || fused_op == Opcode::LteJumpIfNot as u8
                || fused_op == Opcode::LteJumpIf as u8
                || fused_op == Opcode::GtJumpIfNot as u8
                || fused_op == Opcode::GtJumpIf as u8
                || fused_op == Opcode::GteJumpIfNot as u8
                || fused_op == Opcode::GteJumpIf as u8
                || fused_op == Opcode::EqJumpIfNot as u8
                || fused_op == Opcode::EqJumpIf as u8
                || fused_op == Opcode::NeqJumpIfNot as u8
                || fused_op == Opcode::NeqJumpIf as u8
                || fused_op == Opcode::StrictEqJumpIfNot as u8
                || fused_op == Opcode::StrictEqJumpIf as u8
                || fused_op == Opcode::StrictNeqJumpIfNot as u8
                || fused_op == Opcode::StrictNeqJumpIf as u8;
            if is_fused {
                let offset = (target_pos as i64 - patch_pos as i64 - 4) as i32;
                self.code[patch_pos..patch_pos + 4].copy_from_slice(&offset.to_le_bytes());
                return;
            }
        }

        let full_offset = (target_pos as i64 - patch_pos as i64 - 4) as i32;
        self.code[patch_pos..patch_pos + 4].copy_from_slice(&full_offset.to_le_bytes());
    }

    fn emit_line(&mut self, line: u32) {
        self.line_table.add_entry(self.code.len() as u32, line);
    }

    fn add_constant(&mut self, val: JSValue) -> u32 {
        let idx = self.constants.len() as u32;
        self.constants.push(val);
        idx
    }

    fn intern_and_add_const(&mut self, ctx: &mut JSContext, name: &str) -> u32 {
        let atom = ctx.atom_table_mut().intern(name);
        self.add_constant(JSValue::new_string(atom))
    }

    fn emit_mark_direct_eval(&mut self, ctx: &mut JSContext) {
        let mut names: Vec<String> = if self.in_parameter_initializer {
            self.scope_stack
                .first()
                .map(|scope| {
                    scope
                        .iter()
                        .filter(|(_, entry)| {
                            entry.kind != VariableKind::Var && !entry.is_function_name
                        })
                        .map(|(name, _)| name.clone())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.scope_stack
                .iter()
                .skip(1)
                .flat_map(|scope| scope.keys().cloned())
                .collect()
        };
        names.sort();
        names.dedup();
        let start = self.constants.len() as u32;
        for name in &names {
            let atom = ctx.intern(name);
            self.add_constant(JSValue::new_string(atom));
        }
        self.emit(Opcode::MarkDirectEval);
        self.emit_u8(if self.in_field_initializer { 1 } else { 0 });
        self.emit_u32(start);
        self.emit_u16(names.len() as u16);
    }

    fn emit_throw_const_assignment(&mut self, name: &str, ctx: &mut JSContext) {
        let idx = self.intern_and_add_const(ctx, name);
        self.emit(Opcode::ThrowConstAssignment);
        self.emit_u32(idx);
    }

    fn binding_is_const(&self, name: &str) -> bool {
        if let Some(entry) = self.lookup_var(name) {
            return entry.kind == VariableKind::Const || entry.kind.is_using();
        }
        self.parent_vars.get(name).map_or(false, |parent| {
            parent.kind == VariableKind::Const || parent.kind.is_using()
        })
    }

    fn binding_is_function_name(&self, name: &str) -> bool {
        if let Some(entry) = self.lookup_var(name) {
            return entry.is_function_name;
        }
        self.parent_vars
            .get(name)
            .map_or(false, |parent| parent.is_function_name)
    }

    fn binding_needs_tdz_check(&self, name: &str) -> bool {
        if let Some(entry) = self.lookup_var(name) {
            return entry.kind != VariableKind::Var && !entry.initialized;
        }
        self.parent_vars.get(name).map_or(false, |parent| {
            !parent.is_inherited && parent.kind != VariableKind::Var && !parent.initialized
        })
    }

    fn emit_embedded_function(
        &mut self,
        bytecode: &Bytecode,
        upvalues: &[(u32, u16)],
        func_name_atom: u32,
        dst: u16,
        is_generator: bool,
        is_async: bool,
        function_kind: u8,
    ) {
        if is_generator && is_async {
            self.emit(Opcode::NewAsyncGeneratorFunction);
        } else if is_generator {
            self.emit(Opcode::NewGeneratorFunction);
        } else if is_async {
            self.emit(Opcode::NewAsyncFunction);
        } else {
            self.emit(Opcode::NewFunction);
        }
        self.emit_u16(bytecode.param_count);
        self.emit_u8(if bytecode.uses_arguments { 1 } else { 0 });
        self.emit_u8(if bytecode.is_strict { 1 } else { 0 });
        self.emit_u8(if bytecode.has_simple_parameters { 1 } else { 0 });
        self.emit_u8(function_kind);
        let locals_bytes = (bytecode.locals_count as u16).to_le_bytes();
        self.code.extend_from_slice(&locals_bytes);
        self.emit_u32(bytecode.code.len() as u32);
        self.code.extend_from_slice(&bytecode.code);
        self.emit_u32(bytecode.constants.len() as u32);
        for c in &bytecode.constants {
            if c.is_undefined() {
                self.emit_u8(0);
            } else if c.is_null() {
                self.emit_u8(1);
            } else if c.is_bool() && c.get_bool() {
                self.emit_u8(2);
            } else if c.is_bool() && !c.get_bool() {
                self.emit_u8(3);
            } else if c.is_int() {
                self.emit_u8(4);
                self.code.extend_from_slice(&c.get_int().to_le_bytes());
            } else if c.is_float() {
                self.emit_u8(5);
                let bytes = c.get_float().to_le_bytes();
                self.code.extend_from_slice(&bytes);
            } else if c.is_string() {
                self.emit_u8(6);
                self.emit_u32(c.get_atom().0);
            } else {
                self.emit_u8(0);
            }
        }
        self.emit_u32(upvalues.len() as u32);
        for (atom_id, local_idx) in upvalues {
            self.emit_u32(*atom_id);
            self.emit_u32(*local_idx as u32);
        }

        if let Some(ref table) = bytecode.line_number_table {
            let entries = table.entries();
            self.emit_u32(entries.len() as u32);
            for (off, line) in entries {
                self.emit_u32(*off);
                self.emit_u32(*line);
            }
        } else {
            self.emit_u32(0);
        }
        self.emit_u32(func_name_atom);
        self.emit_u32(bytecode.var_name_to_slot.len() as u32);
        for &(atom_id, slot, tag) in bytecode.var_name_to_slot.iter() {
            self.emit_u32(atom_id);
            self.emit_u16(slot);
            self.emit_u8(tag);
        }
        self.emit_u16(dst);
    }

    fn alloc_register(&mut self) -> u16 {
        while let Some(free_r) = self.free_list.pop() {
            if !self.reserved_slots.contains(&free_r) {
                if free_r > self.max_register {
                    self.max_register = free_r;
                }
                return free_r;
            }
        }

        let mut attempts = 0u32;
        while self.reserved_slots.contains(&self.next_register) {
            self.next_register = self.next_register.wrapping_add(1);
            attempts += 1;
            if attempts >= 65536 {
                panic!("alloc_register: all u16 slots are reserved");
            }
        }
        let r = self.next_register;
        self.next_register = self.next_register.wrapping_add(1);
        if r > self.max_register {
            self.max_register = r;
        }
        r
    }

    fn free_register(&mut self, r: u16) {
        if !self.reserved_slots.contains(&r) && !self.free_list.contains(&r) {
            self.free_list.push(r);
        }
    }

    fn push_scope(&mut self) {
        self.scope_stack.push(FxHashMap::default());
        self.scope_is_global.push(false);
        self.scope_dispose_stack.push(None);
        self.depth += 1;
    }

    fn pop_scope(&mut self) {
        // Mark variables in the popped scope as closed in parent_vars so
        // that closures created after this scope see them as inaccessible.
        if let Some(scope) = self.scope_stack.last() {
            for (name, entry) in scope.iter() {
                if entry.kind != VariableKind::Var {
                    if let Some(pv) = self.parent_vars.get_mut(name) {
                        if pv.is_block_decl && pv.local_idx == entry.slot {
                            pv.closed = true;
                        }
                    }
                }
            }
        }
        self.scope_stack.pop();
        self.scope_is_global.pop();
        self.scope_dispose_stack.pop();
        self.depth = self.depth.saturating_sub(1);
    }

    fn declare_var(&mut self, name: &str, kind: VariableKind) -> u16 {
        if kind == VariableKind::Var {
            if let Some(entry) = self.scope_stack[0].get(name) {
                if !entry.is_function_name {
                    return entry.slot;
                }
            }
        }
        let slot = self.alloc_register();
        let entry = VarEntry {
            slot,
            kind,
            initialized: kind == VariableKind::Var,
            is_function_name: false,
        };
        self.reserved_slots.insert(slot);
        self.all_var_names.push((name.to_string(), slot, 0));
        if kind != VariableKind::Var {
            let depth = self.depth as usize;
            for scope in self.with_scopes.iter_mut().rev() {
                if scope.function_id == self.function_id && scope.scope_index < depth {
                    scope.inner_bindings.insert(name.to_string());
                }
            }
        }
        match kind {
            VariableKind::Var => {
                self.scope_stack[0].insert(name.to_string(), entry);
            }
            _ => {
                self.scope_stack[self.depth as usize].insert(name.to_string(), entry);
                // Add to parent_vars for nested scopes so closures created
                // after the scope exits can detect them as inaccessible.
                if self.depth > 1 {
                    let overwrite = match self.parent_vars.get(name) {
                        Some(pv) => pv.is_block_decl,
                        None => true,
                    };
                    if overwrite {
                        self.parent_vars.insert(
                            name.to_string(),
                            ParentVar {
                                local_idx: slot,
                                kind,
                                initialized: false,
                                is_inherited: false,
                                closed: false,
                                is_function_name: false,
                                is_block_decl: true,
                            },
                        );
                    }
                }
            }
        }
        slot
    }

    fn declare_function_name(&mut self, name: &str) -> u16 {
        let slot = self.alloc_register();
        let entry = VarEntry {
            slot,
            kind: VariableKind::Var,
            initialized: true,
            is_function_name: true,
        };
        self.reserved_slots.insert(slot);
        self.all_var_names.push((name.to_string(), slot, 1));
        self.scope_stack[0].insert(name.to_string(), entry);
        slot
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
                | "eval"
                | "arguments"
        )
    }

    fn has_use_strict_directive(body: &BlockStatement) -> bool {
        for stmt in &body.body {
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

    fn is_global_scope(&self) -> bool {
        self.scope_is_global.first().copied().unwrap_or(false)
    }

    fn is_global_var(&self, name: &str) -> bool {
        if !self.is_global_scope() || (self.is_strict && self.is_eval) {
            return false;
        }
        let mut position = None;
        for (i, scope) in self.scope_stack.iter().enumerate().rev() {
            if scope.contains_key(name) {
                position = Some(i);
                break;
            }
        }
        position == Some(0)
    }

    fn is_global_lexical(&self, name: &str) -> bool {
        if !self.is_global_scope() || self.is_eval || !self.global_lexical_names.contains(name) {
            return false;
        }
        for (index, scope) in self.scope_stack.iter().enumerate().rev() {
            if scope.contains_key(name) {
                return index == 1;
            }
        }
        false
    }

    fn is_global_binding(&self, name: &str) -> bool {
        self.is_global_var(name) || self.is_global_lexical(name)
    }

    fn emit_global_binding(
        &mut self,
        mode: u8,
        name: &str,
        src: u16,
        ctx: &mut JSContext,
    ) {
        let atom = ctx.intern(name);
        let idx = self.add_constant(JSValue::new_string(atom));
        self.emit(Opcode::GlobalBinding);
        self.emit_u8(mode);
        self.emit_u32(idx);
        self.emit_u16(src);
    }

    fn collect_script_var_and_function_names(
        body: &[ASTNode],
    ) -> (Vec<String>, Vec<String>) {
        let mut var_names: Vec<String> = Vec::new();
        for stmt in body {
            Self::collect_hoisted_var_names(stmt, &mut var_names);
        }
        let mut function_names: Vec<String> = Vec::new();
        for stmt in body {
            if let ASTNode::FunctionDeclaration(func) = stmt {
                if !function_names.contains(&func.name) {
                    function_names.push(func.name.clone());
                }
            }
        }
        (var_names, function_names)
    }

    fn emit_global_declaration_instantiation(
        &mut self,
        body: &[ASTNode],
        ctx: &mut JSContext,
    ) {
        let lexical = Self::collect_global_lexical_declarations(body);
        let (var_names, function_names) = Self::collect_script_var_and_function_names(body);
        let mut variable_names = var_names.clone();
        for name in &function_names {
            if !variable_names.contains(name) {
                variable_names.push(name.clone());
            }
        }
        for (name, _) in &lexical {
            self.emit_global_binding(0, name, 0, ctx);
        }
        for name in &variable_names {
            self.emit_global_binding(1, name, 0, ctx);
        }
        for name in &function_names {
            self.emit_global_binding(2, name, 0, ctx);
        }
        for name in &var_names {
            if !function_names.contains(name) {
                self.emit_global_binding(3, name, 0, ctx);
            }
        }
        for (name, immutable) in &lexical {
            self.emit_global_binding(if *immutable { 5 } else { 4 }, name, 0, ctx);
        }
    }

    fn emit_eval_declaration_instantiation(
        &mut self,
        body: &[ASTNode],
        ctx: &mut JSContext,
    ) {
        let (var_names, function_names) = Self::collect_script_var_and_function_names(body);
        for name in &var_names {
            self.emit_global_binding(11, name, 0, ctx);
        }
        for name in &function_names {
            self.emit_global_binding(11, name, 0, ctx);
        }
        for name in &function_names {
            self.emit_global_binding(12, name, 0, ctx);
        }
        for name in &var_names {
            if !function_names.contains(name) {
                self.emit_global_binding(13, name, 0, ctx);
            }
        }
    }

    fn emit_eval_var_instantiation(&mut self, body: &[ASTNode], ctx: &mut JSContext) {
        let (var_names, function_names) = Self::collect_script_var_and_function_names(body);
        for name in &var_names {
            if !function_names.contains(name) {
                self.emit_global_binding(10, name, 0, ctx);
            }
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarEntry> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(entry) = scope.get(name) {
                return Some(entry);
            }
        }
        None
    }

    fn lookup_var_slot(&self, name: &str) -> Option<u16> {
        self.lookup_var(name).map(|e| e.slot)
    }

    fn lookup_var_scope_index(&self, name: &str) -> Option<usize> {
        let len = self.scope_stack.len();
        for (i, scope) in self.scope_stack.iter().enumerate().rev() {
            if scope.contains_key(name) {
                return Some(len - 1 - i);
            }
        }
        None
    }

    fn gen_private_owner(
        &mut self,
        name: &str,
        ctx: &mut JSContext,
    ) -> Result<(u16, bool), String> {
        match self.resolve_var(name, ctx) {
            Some(VarLocation::Local(slot)) => Ok((slot, false)),
            Some(VarLocation::Upvalue(slot)) => {
                let r = self.alloc_register();
                self.emit(Opcode::GetUpvalue);
                self.emit_u16(r);
                self.emit_u16(slot);
                Ok((r, true))
            }
            None => {
                if self.is_eval {
                    let name_atom = ctx.atom_table_mut().intern(name).0;
                    let known = ctx
                        .eval_private_names
                        .as_ref()
                        .map_or(true, |names| names.iter().any(|a| a.0 == name_atom));
                    if known {
                        let r = self.alloc_register();
                        self.emit(Opcode::GetEvalPrivateOwner);
                        self.emit_u16(r);
                        self.emit_u32(name_atom);
                        Ok((r, true))
                    } else {
                        Err(format!(
                            "SyntaxError: Private name #{} is not defined",
                            name.trim_start_matches('#')
                        ))
                    }
                } else {
                    Err(format!(
                        "SyntaxError: Private name #{} is not defined",
                        name.trim_start_matches('#')
                    ))
                }
            }
        }
    }

    fn with_applies_to_local(&self, name: &str, scope_index: usize) -> bool {
        let _ = scope_index;
        self.with_scopes.iter().rev().any(|scope| {
            scope.function_id == self.function_id && !scope.inner_bindings.contains(name)
        })
    }

    fn with_applies_to_outer(&self, name: &str) -> bool {
        self.with_scopes
            .iter()
            .rev()
            .any(|scope| !scope.inner_bindings.contains(name))
    }

    fn resolve_var(&mut self, name: &str, ctx: &mut JSContext) -> Option<VarLocation> {
        if let Some(entry) = self.lookup_var(name) {
            return Some(VarLocation::Local(entry.slot));
        }
        if let Some(parent_var) = self.parent_vars.get(name) {
            // If the variable's scope has been closed (popped), don't resolve
            // it as an upvalue — closures after the scope should not see it.
            if parent_var.closed {
                return None;
            }
            if name == "arguments" && self.is_arrow {
                self.needs_parent_arguments = true;
            }
            if let Some(&upvalue_slot) = self.upvalue_slots.get(name) {
                return Some(VarLocation::Upvalue(upvalue_slot));
            }
            let upvalue_slot = self.upvalues.len() as u16;
            self.upvalue_slots.insert(name.to_string(), upvalue_slot);
            self.reserved_slots.insert(upvalue_slot);
            let atom = ctx.atom_table_mut().intern(name);
            let local_idx = if parent_var.is_inherited {
                u16::MAX
            } else {
                parent_var.local_idx
            };
            self.upvalues.push((atom.0, local_idx));
            return Some(VarLocation::Upvalue(upvalue_slot));
        }
        None
    }

    fn collect_inherited_upvalues_stmt(
        stmt: &ASTNode,
        locals: &mut std::collections::HashSet<String>,
        parent_vars: &FxHashMap<String, ParentVar>,
        result: &mut std::collections::HashSet<String>,
    ) {
        match stmt {
            ASTNode::ExpressionStatement(es) => {
                Self::collect_inherited_upvalues_expr(&es.expression, locals, parent_vars, result);
            }
            ASTNode::VariableDeclaration(decl) => {
                for d in &decl.declarations {
                    if let BindingPattern::Identifier(name) = &d.id {
                        if let Some(init) = &d.init {
                            Self::collect_inherited_upvalues_expr(
                                init,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                        locals.insert(name.clone());
                    }
                }
            }
            ASTNode::BlockStatement(block) => {
                let mut block_locals = locals.clone();
                for s in &block.body {
                    Self::collect_inherited_upvalues_stmt(
                        s,
                        &mut block_locals,
                        parent_vars,
                        result,
                    );
                }
            }
            ASTNode::IfStatement(stmt) => {
                Self::collect_inherited_upvalues_expr(&stmt.test, locals, parent_vars, result);
                let mut cons_locals = locals.clone();
                Self::collect_inherited_upvalues_stmt(
                    &stmt.consequent,
                    &mut cons_locals,
                    parent_vars,
                    result,
                );
                if let Some(alt) = &stmt.alternate {
                    let mut alt_locals = locals.clone();
                    Self::collect_inherited_upvalues_stmt(
                        alt,
                        &mut alt_locals,
                        parent_vars,
                        result,
                    );
                }
            }
            ASTNode::WhileStatement(stmt) => {
                Self::collect_inherited_upvalues_expr(&stmt.test, locals, parent_vars, result);
                let mut loop_locals = locals.clone();
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut loop_locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::DoWhileStatement(stmt) => {
                let mut loop_locals = locals.clone();
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut loop_locals,
                    parent_vars,
                    result,
                );
                Self::collect_inherited_upvalues_expr(&stmt.test, locals, parent_vars, result);
            }
            ASTNode::ForStatement(stmt) => {
                let mut for_locals = locals.clone();
                if let Some(init) = &stmt.init {
                    match init {
                        ForInit::VariableDeclaration(decl) => {
                            for d in &decl.declarations {
                                if let BindingPattern::Identifier(name) = &d.id {
                                    if let Some(init_expr) = &d.init {
                                        Self::collect_inherited_upvalues_expr(
                                            init_expr,
                                            &mut for_locals,
                                            parent_vars,
                                            result,
                                        );
                                    }
                                    for_locals.insert(name.clone());
                                }
                            }
                        }
                        ForInit::Expression(expr) => {
                            Self::collect_inherited_upvalues_expr(
                                expr,
                                &mut for_locals,
                                parent_vars,
                                result,
                            );
                        }
                    }
                }
                if let Some(test) = &stmt.test {
                    Self::collect_inherited_upvalues_expr(
                        test,
                        &mut for_locals,
                        parent_vars,
                        result,
                    );
                }
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut for_locals,
                    parent_vars,
                    result,
                );
                if let Some(update) = &stmt.update {
                    Self::collect_inherited_upvalues_expr(
                        update,
                        &mut for_locals,
                        parent_vars,
                        result,
                    );
                }
            }
            ASTNode::SwitchStatement(stmt) => {
                Self::collect_inherited_upvalues_expr(
                    &stmt.discriminant,
                    locals,
                    parent_vars,
                    result,
                );
                for case in &stmt.cases {
                    let mut case_locals = locals.clone();
                    if let Some(test) = &case.test {
                        Self::collect_inherited_upvalues_expr(
                            test,
                            &mut case_locals,
                            parent_vars,
                            result,
                        );
                    }
                    for s in &case.consequent {
                        Self::collect_inherited_upvalues_stmt(
                            s,
                            &mut case_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
            }
            ASTNode::ReturnStatement(stmt) => {
                if let Some(expr) = &stmt.argument {
                    Self::collect_inherited_upvalues_expr(expr, locals, parent_vars, result);
                }
            }
            ASTNode::ThrowStatement(stmt) => {
                Self::collect_inherited_upvalues_expr(
                    &stmt.argument,
                    locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::BreakStatement(_) | ASTNode::ContinueStatement(_) => {}
            ASTNode::FunctionDeclaration(func) => {
                let mut func_locals = std::collections::HashSet::new();
                func_locals.insert(func.name.clone());
                for param in &func.params {
                    Self::collect_parameter_bindings(param, &mut func_locals);
                }
                for s in &func.body.body {
                    Self::collect_inherited_upvalues_stmt(s, &mut func_locals, parent_vars, result);
                }
            }
            ASTNode::ForInStatement(stmt) => {
                let mut loop_locals = locals.clone();
                match &stmt.left {
                    ForInOfLeft::VariableDeclaration(decl) => {
                        for d in &decl.declarations {
                            if let Some(init) = &d.init {
                                Self::collect_inherited_upvalues_expr(
                                    init,
                                    &mut loop_locals,
                                    parent_vars,
                                    result,
                                );
                            }
                        }
                        for d in &decl.declarations {
                            Self::collect_binding_pattern_bindings(&d.id, &mut loop_locals);
                        }
                    }
                    ForInOfLeft::AssignmentTarget(target) => {
                        Self::collect_inherited_upvalues_target(
                            target,
                            &mut loop_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
                Self::collect_inherited_upvalues_expr(&stmt.right, locals, parent_vars, result);
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut loop_locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::ForOfStatement(stmt) => {
                let mut loop_locals = locals.clone();
                match &stmt.left {
                    ForInOfLeft::VariableDeclaration(decl) => {
                        for d in &decl.declarations {
                            if let Some(init) = &d.init {
                                Self::collect_inherited_upvalues_expr(
                                    init,
                                    &mut loop_locals,
                                    parent_vars,
                                    result,
                                );
                            }
                        }
                        for d in &decl.declarations {
                            Self::collect_binding_pattern_bindings(&d.id, &mut loop_locals);
                        }
                    }
                    ForInOfLeft::AssignmentTarget(target) => {
                        Self::collect_inherited_upvalues_target(
                            target,
                            &mut loop_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
                Self::collect_inherited_upvalues_expr(&stmt.right, locals, parent_vars, result);
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut loop_locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::ClassDeclaration(class) => {
                if let Some(super_class) = &class.super_class {
                    Self::collect_inherited_upvalues_expr(
                        super_class,
                        locals,
                        parent_vars,
                        result,
                    );
                }
                let mut class_locals = locals.clone();
                class_locals.insert(class.name.clone());
                Self::collect_inherited_upvalues_class_body(
                    &class.body,
                    &class_locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::TryStatement(stmt) => {
                let mut try_locals = locals.clone();
                for s in &stmt.block.body {
                    Self::collect_inherited_upvalues_stmt(
                        s,
                        &mut try_locals,
                        parent_vars,
                        result,
                    );
                }
                if let Some(handler) = &stmt.handler {
                    let mut handler_locals = locals.clone();
                    if let Some(param) = &handler.param {
                        Self::collect_binding_pattern_bindings(param, &mut handler_locals);
                    }
                    for s in &handler.body.body {
                        Self::collect_inherited_upvalues_stmt(
                            s,
                            &mut handler_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
                if let Some(finalizer) = &stmt.finalizer {
                    let mut finalizer_locals = locals.clone();
                    for s in &finalizer.body {
                        Self::collect_inherited_upvalues_stmt(
                            s,
                            &mut finalizer_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
            }
            ASTNode::WithStatement(stmt) => {
                Self::collect_inherited_upvalues_expr(
                    &stmt.object,
                    locals,
                    parent_vars,
                    result,
                );
                let mut with_locals = locals.clone();
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    &mut with_locals,
                    parent_vars,
                    result,
                );
            }
            ASTNode::LabelledStatement(stmt) => {
                Self::collect_inherited_upvalues_stmt(
                    &stmt.body,
                    locals,
                    parent_vars,
                    result,
                );
            }
            _ => {}
        }
    }

    fn collect_inherited_upvalues_class_body(
        body: &ClassBody,
        outer_locals: &std::collections::HashSet<String>,
        parent_vars: &FxHashMap<String, ParentVar>,
        result: &mut std::collections::HashSet<String>,
    ) {
        let mut class_locals = outer_locals.clone();
        for element in &body.elements {
            let name = match element {
                ClassElement::Method(m) => match &m.name {
                    PropertyKey::PrivateIdentifier(n) => Some(n.clone()),
                    PropertyKey::Identifier(n) if m.is_private => Some(n.clone()),
                    _ => None,
                },
                ClassElement::Property(p) => match &p.name {
                    PropertyKey::PrivateIdentifier(n) => Some(n.clone()),
                    PropertyKey::Identifier(n) if p.is_private => Some(n.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(name) = name {
                class_locals.insert(name);
            }
        }
        for element in &body.elements {
            match element {
                ClassElement::Method(m) => {
                    if let PropertyKey::Computed(key) = &m.name {
                        let mut key_locals = outer_locals.clone();
                        Self::collect_inherited_upvalues_expr(
                            key,
                            &mut key_locals,
                            parent_vars,
                            result,
                        );
                    }
                    let mut method_locals = class_locals.clone();
                    for param in &m.params {
                        Self::collect_parameter_bindings(param, &mut method_locals);
                    }
                    for s in &m.body.body {
                        Self::collect_inherited_upvalues_stmt(
                            s,
                            &mut method_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
                ClassElement::Property(p) => {
                    if let PropertyKey::Computed(key) = &p.name {
                        let mut key_locals = outer_locals.clone();
                        Self::collect_inherited_upvalues_expr(
                            key,
                            &mut key_locals,
                            parent_vars,
                            result,
                        );
                    }
                    if let Some(value) = &p.value {
                        let mut value_locals = class_locals.clone();
                        Self::collect_inherited_upvalues_expr(
                            value,
                            &mut value_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
                ClassElement::StaticBlock(block) => {
                    let mut block_locals = class_locals.clone();
                    for s in &block.body {
                        Self::collect_inherited_upvalues_stmt(
                            s,
                            &mut block_locals,
                            parent_vars,
                            result,
                        );
                    }
                }
            }
        }
    }

    fn collect_binding_pattern_bindings(
        pattern: &BindingPattern,
        out: &mut std::collections::HashSet<String>,
    ) {
        match pattern {
            BindingPattern::Identifier(name) => {
                out.insert(name.clone());
            }
            BindingPattern::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(elem) = elem {
                        match elem {
                            PatternElement::Pattern(target) => {
                                Self::collect_assignment_target_bindings(target, out);
                            }
                            PatternElement::RestElement(rest) => {
                                Self::collect_assignment_target_bindings(&rest.argument, out);
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                Self::collect_assignment_target_bindings(&ap.left, out);
                            }
                        }
                    }
                }
            }
            BindingPattern::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            Self::collect_assignment_target_bindings(value, out);
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            Self::collect_assignment_target_bindings(&rest.argument, out);
                        }
                    }
                }
            }
            BindingPattern::AssignmentPattern(ap) => {
                Self::collect_assignment_target_bindings(&ap.left, out);
            }
        }
    }

    fn push_hoisted_var_name(name: String, out: &mut Vec<String>) {
        if !out.contains(&name) {
            out.push(name);
        }
    }

    fn collect_pattern_hoisted_var_names(pattern: &BindingPattern, out: &mut Vec<String>) {
        let mut names = std::collections::HashSet::new();
        Self::collect_binding_pattern_bindings(pattern, &mut names);
        let mut sorted: Vec<String> = names.into_iter().collect();
        sorted.sort();
        for name in sorted {
            Self::push_hoisted_var_name(name, out);
        }
    }

    fn collect_global_lexical_declarations(body: &[ASTNode]) -> Vec<(String, bool)> {
        let mut result: Vec<(String, bool)> = Vec::new();
        for stmt in body {
            match stmt {
                ASTNode::VariableDeclaration(decl) if decl.kind != VariableKind::Var => {
                    let immutable = decl.kind == VariableKind::Const || decl.kind.is_using();
                    for d in &decl.declarations {
                        let mut names = std::collections::HashSet::new();
                        Self::collect_binding_pattern_bindings(&d.id, &mut names);
                        let mut sorted: Vec<String> = names.into_iter().collect();
                        sorted.sort();
                        for name in sorted {
                            result.push((name, immutable));
                        }
                    }
                }
                ASTNode::ClassDeclaration(class) => {
                    result.push((class.name.clone(), false));
                }
                _ => {}
            }
        }
        result
    }

    fn collect_hoisted_var_names(stmt: &ASTNode, out: &mut Vec<String>) {
        match stmt {
            ASTNode::VariableDeclaration(decl) => {
                if decl.kind == VariableKind::Var {
                    for d in &decl.declarations {
                        Self::collect_pattern_hoisted_var_names(&d.id, out);
                    }
                }
            }
            ASTNode::BlockStatement(b) => {
                for stmt in &b.body {
                    Self::collect_hoisted_var_names(stmt, out);
                }
            }
            ASTNode::IfStatement(i) => {
                Self::collect_hoisted_var_names(&i.consequent, out);
                if let Some(alt) = &i.alternate {
                    Self::collect_hoisted_var_names(alt, out);
                }
            }
            ASTNode::ForStatement(f) => {
                if let Some(ForInit::VariableDeclaration(vd)) = &f.init {
                    if vd.kind == VariableKind::Var {
                        for d in &vd.declarations {
                            Self::collect_pattern_hoisted_var_names(&d.id, out);
                        }
                    }
                }
                Self::collect_hoisted_var_names(&f.body, out);
            }
            ASTNode::ForInStatement(f) => {
                if let ForInOfLeft::VariableDeclaration(vd) = &f.left {
                    if vd.kind == VariableKind::Var {
                        for d in &vd.declarations {
                            Self::collect_pattern_hoisted_var_names(&d.id, out);
                        }
                    }
                }
                Self::collect_hoisted_var_names(&f.body, out);
            }
            ASTNode::ForOfStatement(f) => {
                if let ForInOfLeft::VariableDeclaration(vd) = &f.left {
                    if vd.kind == VariableKind::Var {
                        for d in &vd.declarations {
                            Self::collect_pattern_hoisted_var_names(&d.id, out);
                        }
                    }
                }
                Self::collect_hoisted_var_names(&f.body, out);
            }
            ASTNode::WhileStatement(w) => Self::collect_hoisted_var_names(&w.body, out),
            ASTNode::DoWhileStatement(d) => Self::collect_hoisted_var_names(&d.body, out),
            ASTNode::SwitchStatement(sw) => {
                for case in &sw.cases {
                    for stmt in &case.consequent {
                        Self::collect_hoisted_var_names(stmt, out);
                    }
                }
            }
            ASTNode::TryStatement(t) => {
                for stmt in &t.block.body {
                    Self::collect_hoisted_var_names(stmt, out);
                }
                if let Some(h) = &t.handler {
                    for stmt in &h.body.body {
                        Self::collect_hoisted_var_names(stmt, out);
                    }
                }
                if let Some(fin) = &t.finalizer {
                    for stmt in &fin.body {
                        Self::collect_hoisted_var_names(stmt, out);
                    }
                }
            }
            ASTNode::WithStatement(w) => Self::collect_hoisted_var_names(&w.body, out),
            ASTNode::LabelledStatement(l) => Self::collect_hoisted_var_names(&l.body, out),
            ASTNode::ExportNamedDeclaration(ed) => {
                if let Some(ExportDeclaration::Variable(vd)) = &ed.declaration {
                    if vd.kind == VariableKind::Var {
                        for d in &vd.declarations {
                            Self::collect_pattern_hoisted_var_names(&d.id, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_assignment_target_bindings(
        target: &AssignmentTarget,
        out: &mut std::collections::HashSet<String>,
    ) {
        match target {
            AssignmentTarget::Identifier(name) => {
                out.insert(name.clone());
            }
            AssignmentTarget::ArrayPattern(arr) => {
                Self::collect_binding_pattern_bindings(
                    &BindingPattern::ArrayPattern(arr.clone()),
                    out,
                );
            }
            AssignmentTarget::ObjectPattern(obj) => {
                Self::collect_binding_pattern_bindings(
                    &BindingPattern::ObjectPattern(obj.clone()),
                    out,
                );
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                Self::collect_assignment_target_bindings(&ap.left, out);
            }
            AssignmentTarget::RestElement(rest) => {
                Self::collect_assignment_target_bindings(&rest.argument, out);
            }
            _ => {}
        }
    }

    fn collect_parameter_bindings(param: &Parameter, out: &mut std::collections::HashSet<String>) {
        match param {
            Parameter::Identifier(name) => {
                out.insert(name.clone());
            }
            Parameter::Pattern(pattern) => {
                Self::collect_binding_pattern_bindings(pattern, out);
            }
            Parameter::AssignmentPattern(ap) => {
                Self::collect_assignment_target_bindings(&ap.left, out);
            }
            Parameter::RestElement(rest) => {
                Self::collect_assignment_target_bindings(&rest.argument, out);
            }
        }
    }

    fn collect_inherited_upvalues_target(
        target: &AssignmentTarget,
        locals: &mut std::collections::HashSet<String>,
        parent_vars: &FxHashMap<String, ParentVar>,
        result: &mut std::collections::HashSet<String>,
    ) {
        match target {
            AssignmentTarget::Identifier(name) => {
                if !locals.contains(name) && parent_vars.contains_key(name) {
                    result.insert(name.clone());
                }
            }
            AssignmentTarget::MemberExpression(member) => {
                Self::collect_inherited_upvalues_expr(
                    &member.object,
                    locals,
                    parent_vars,
                    result,
                );
                match &member.property {
                    MemberProperty::Computed(e) => {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        if parent_vars.contains_key(name) {
                            result.insert(name.clone());
                        }
                    }
                    _ => {}
                }
            }
            AssignmentTarget::OptionalMemberExpression(member) => {
                Self::collect_inherited_upvalues_expr(
                    &member.object,
                    locals,
                    parent_vars,
                    result,
                );
                match &member.property {
                    MemberProperty::Computed(e) => {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        if parent_vars.contains_key(name) {
                            result.insert(name.clone());
                        }
                    }
                    _ => {}
                }
            }
            AssignmentTarget::ComputedMember(obj, key) => {
                Self::collect_inherited_upvalues_expr(obj, locals, parent_vars, result);
                Self::collect_inherited_upvalues_expr(key, locals, parent_vars, result);
            }
            AssignmentTarget::ObjectPattern(pattern) => {
                for property in &pattern.properties {
                    match property {
                        ObjectPatternProperty::Property {
                            key,
                            value,
                            computed,
                            ..
                        } => {
                            if *computed {
                                if let PropertyKey::Computed(e) = key {
                                    Self::collect_inherited_upvalues_expr(
                                        e,
                                        locals,
                                        parent_vars,
                                        result,
                                    );
                                }
                            }
                            Self::collect_inherited_upvalues_target(
                                value,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            Self::collect_inherited_upvalues_target(
                                &rest.argument,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                    }
                }
            }
            AssignmentTarget::ArrayPattern(pattern) => {
                for element in pattern.elements.iter().flatten() {
                    match element {
                        PatternElement::Pattern(t) => {
                            Self::collect_inherited_upvalues_target(
                                t,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                        PatternElement::RestElement(rest) => {
                            Self::collect_inherited_upvalues_target(
                                &rest.argument,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                        PatternElement::AssignmentPattern(ap) => {
                            Self::collect_inherited_upvalues_target(
                                &ap.left,
                                locals,
                                parent_vars,
                                result,
                            );
                        }
                    }
                }
            }
            AssignmentTarget::RestElement(rest) => {
                Self::collect_inherited_upvalues_target(
                    &rest.argument,
                    locals,
                    parent_vars,
                    result,
                );
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                Self::collect_inherited_upvalues_target(
                    &ap.left,
                    locals,
                    parent_vars,
                    result,
                );
            }
        }
    }

    fn collect_inherited_upvalues_expr(
        expr: &Expression,
        locals: &mut std::collections::HashSet<String>,
        parent_vars: &FxHashMap<String, ParentVar>,
        result: &mut std::collections::HashSet<String>,
    ) {
        match expr {
            Expression::Identifier(id) => {
                if !locals.contains(&id.name) && parent_vars.contains_key(&id.name) {
                    result.insert(id.name.clone());
                }
            }
            Expression::BinaryExpression(bin) => {
                Self::collect_inherited_upvalues_expr(&bin.left, locals, parent_vars, result);
                Self::collect_inherited_upvalues_expr(&bin.right, locals, parent_vars, result);
            }
            Expression::UnaryExpression(unary) => {
                Self::collect_inherited_upvalues_expr(&unary.argument, locals, parent_vars, result);
            }
            Expression::UpdateExpression(update) => {
                if let Expression::Identifier(id) = &*update.argument {
                    if !locals.contains(&id.name) && parent_vars.contains_key(&id.name) {
                        result.insert(id.name.clone());
                    }
                }
                Self::collect_inherited_upvalues_expr(
                    &update.argument,
                    locals,
                    parent_vars,
                    result,
                );
            }
            Expression::PrivateIdentifier(name) => {
                if parent_vars.contains_key(name) {
                    result.insert(name.clone());
                }
            }
            Expression::AssignmentExpression(assign) => {
                Self::collect_inherited_upvalues_target(
                    &assign.left,
                    locals,
                    parent_vars,
                    result,
                );
                Self::collect_inherited_upvalues_expr(&assign.right, locals, parent_vars, result);
            }
            Expression::ConditionalExpression(cond) => {
                Self::collect_inherited_upvalues_expr(&cond.test, locals, parent_vars, result);
                Self::collect_inherited_upvalues_expr(
                    &cond.consequent,
                    locals,
                    parent_vars,
                    result,
                );
                Self::collect_inherited_upvalues_expr(&cond.alternate, locals, parent_vars, result);
            }
            Expression::LogicalExpression(logical) => {
                Self::collect_inherited_upvalues_expr(&logical.left, locals, parent_vars, result);
                Self::collect_inherited_upvalues_expr(&logical.right, locals, parent_vars, result);
            }
            Expression::MemberExpression(member) => {
                Self::collect_inherited_upvalues_expr(&member.object, locals, parent_vars, result);
                match &member.property {
                    MemberProperty::Computed(e) => {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        if parent_vars.contains_key(name) {
                            result.insert(name.clone());
                        }
                    }
                    _ => {}
                }
            }
            Expression::CallExpression(call) => {
                Self::collect_inherited_upvalues_expr(&call.callee, locals, parent_vars, result);
                for arg in &call.arguments {
                    if let Argument::Expression(e) = arg {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                }
            }
            Expression::OptionalMemberExpression(member) => {
                Self::collect_inherited_upvalues_expr(&member.object, locals, parent_vars, result);
                match &member.property {
                    MemberProperty::Computed(e) => {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        if parent_vars.contains_key(name) {
                            result.insert(name.clone());
                        }
                    }
                    _ => {}
                }
            }
            Expression::OptionalCallExpression(call) => {
                Self::collect_inherited_upvalues_expr(&call.callee, locals, parent_vars, result);
                for arg in &call.arguments {
                    if let Argument::Expression(e) = arg {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                }
            }
            Expression::ChainExpression(inner) => {
                Self::collect_inherited_upvalues_expr(inner, locals, parent_vars, result);
            }
            Expression::NewExpression(new) => {
                Self::collect_inherited_upvalues_expr(&new.callee, locals, parent_vars, result);
                for arg in &new.arguments {
                    if let Argument::Expression(e) = arg {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let Some(ArrayElement::Expression(e)) = elem {
                        Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let Property::Property { value, .. } = prop {
                        Self::collect_inherited_upvalues_expr(value, locals, parent_vars, result);
                    }
                }
            }
            Expression::FunctionExpression(func) => {
                let mut func_locals = std::collections::HashSet::new();
                if let Some(ref name) = func.name {
                    func_locals.insert(name.clone());
                }
                for param in &func.params {
                    Self::collect_parameter_bindings(param, &mut func_locals);
                }
                for s in &func.body.body {
                    Self::collect_inherited_upvalues_stmt(s, &mut func_locals, parent_vars, result);
                }
            }
            Expression::ArrowFunction(arrow) => {
                let mut func_locals = std::collections::HashSet::new();
                for param in &arrow.params {
                    Self::collect_parameter_bindings(param, &mut func_locals);
                }
                let body = match &arrow.body {
                    ArrowBody::Expression(expr) => BlockStatement {
                        body: vec![ASTNode::ReturnStatement(ReturnStatement {
                            argument: Some((**expr).clone()),
                        })],
                        lines: vec![],
                    },
                    ArrowBody::Block(block) => block.clone(),
                };
                for s in &body.body {
                    Self::collect_inherited_upvalues_stmt(s, &mut func_locals, parent_vars, result);
                }
            }
            Expression::ClassExpression(class) => {
                if let Some(super_class) = &class.super_class {
                    Self::collect_inherited_upvalues_expr(
                        super_class,
                        locals,
                        parent_vars,
                        result,
                    );
                }
                let mut class_locals = locals.clone();
                if let Some(ref name) = class.name {
                    class_locals.insert(name.clone());
                }
                Self::collect_inherited_upvalues_class_body(
                    &class.body,
                    &class_locals,
                    parent_vars,
                    result,
                );
            }
            Expression::SequenceExpression(seq) => {
                for e in &seq.expressions {
                    Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                }
            }
            Expression::TemplateLiteral(tpl) => {
                for e in &tpl.expressions {
                    Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                }
            }
            Expression::TaggedTemplateExpression(tagged) => {
                Self::collect_inherited_upvalues_expr(&tagged.tag, locals, parent_vars, result);
                for e in &tagged.quasi.expressions {
                    Self::collect_inherited_upvalues_expr(e, locals, parent_vars, result);
                }
            }
            Expression::SpreadElement(spread) => {
                Self::collect_inherited_upvalues_expr(
                    &spread.argument,
                    locals,
                    parent_vars,
                    result,
                );
            }
            Expression::AwaitExpression(await_expr) => {
                Self::collect_inherited_upvalues_expr(
                    &await_expr.argument,
                    locals,
                    parent_vars,
                    result,
                );
            }
            Expression::YieldExpression(yield_expr) => {
                if let Some(argument) = &yield_expr.argument {
                    Self::collect_inherited_upvalues_expr(
                        argument,
                        locals,
                        parent_vars,
                        result,
                    );
                }
            }
            _ => {}
        }
    }

    pub fn compile_expression(
        &mut self,
        expr: &Expression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        self.gen_expression(expr, ctx)
    }

    pub fn compile_function(
        &mut self,
        params: &[Parameter],
        body: &BlockStatement,
        ctx: &mut JSContext,
    ) -> Result<(Bytecode, Vec<(u32, u16)>), String> {
        self.compile_function_with_parent(params, body, ctx, FxHashMap::default(), None, false)
    }

    pub fn compile_script(
        &mut self,
        body: &BlockStatement,
        ctx: &mut JSContext,
    ) -> Result<(Bytecode, Vec<(u32, u16)>), String> {
        let saved_scope_is_global = self.scope_is_global.clone();
        self.scope_is_global = vec![true];
        let result =
            self.compile_function_with_parent(&[], body, ctx, FxHashMap::default(), None, true);
        self.scope_is_global = saved_scope_is_global;
        result
    }

    pub fn compile_module(
        &mut self,
        body: &BlockStatement,
        ctx: &mut JSContext,
    ) -> Result<(Bytecode, Vec<(u32, u16)>), String> {
        let saved_scope_is_global = self.scope_is_global.clone();
        self.scope_is_global = vec![false];
        let result =
            self.compile_function_with_parent(&[], body, ctx, FxHashMap::default(), None, true);
        self.scope_is_global = saved_scope_is_global;
        result
    }

    fn compile_function_with_parent(
        &mut self,
        params: &[Parameter],
        body: &BlockStatement,
        ctx: &mut JSContext,
        parent_vars: FxHashMap<String, ParentVar>,
        function_name: Option<&str>,
        return_last_expr: bool,
    ) -> Result<(Bytecode, Vec<(u32, u16)>), String> {
        self.compile_function_with_parent_gen(
            params,
            body,
            ctx,
            parent_vars,
            function_name,
            return_last_expr,
            false,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_function_with_parent_gen(
        &mut self,
        params: &[Parameter],
        body: &BlockStatement,
        ctx: &mut JSContext,
        parent_vars: FxHashMap<String, ParentVar>,
        function_name: Option<&str>,
        return_last_expr: bool,
        is_generator: bool,
        is_async: bool,
        is_named_expression: bool,
    ) -> Result<(Bytecode, Vec<(u32, u16)>), String> {
        self.in_async_generator = is_generator && is_async;
        let has_use_strict = Self::has_use_strict_directive(body);
        self.is_strict = self.is_strict || has_use_strict;
        let is_simple_params = params.iter().all(|p| matches!(p, Parameter::Identifier(_)));
        if has_use_strict && !is_simple_params {
            return Err(
                "SyntaxError: Illegal 'use strict' directive in function with non-simple parameter list"
                    .to_string(),
            );
        }
        if !is_simple_params {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for p in params {
                let mut names = std::collections::HashSet::new();
                Self::collect_parameter_bindings(p, &mut names);
                for n in names {
                    if !seen.insert(n) {
                        return Err(
                            "SyntaxError: Duplicate parameter name not allowed in this context"
                                .to_string(),
                        );
                    }
                }
            }
        }
        let saved_is_script_level = self.is_script_level;
        let saved_scope_is_global = self.scope_is_global.clone();
        self.next_register = 0;
        self.max_register = 0;
        self.free_list.clear();
        self.scope_stack = vec![FxHashMap::default()];

        if self.scope_is_global != vec![true] {
            self.scope_is_global = vec![false];
        }
        self.reserved_slots.clear();
        self.all_var_names.clear();
        self.code.clear();
        self.constants.clear();
        self.parent_vars = parent_vars.clone();
        self.upvalues.clear();
        self.upvalue_slots.clear();
        self.line_table = LineNumberTable::new();
        self.depth = 0;
        if return_last_expr {
            self.is_script_level = true;
        } else {
            self.is_script_level = false;
        }
        self.function_uses_arguments = false;
        self.arguments_slot = None;
        self.needs_parent_arguments = false;
        self.simple_params = is_simple_params;
        self.current_function_name = function_name.map(|s| s.to_string());

        let this_slot = self.alloc_register();
        assert_eq!(this_slot, 0);
        self.reserved_slots.insert(this_slot);

        // Allocate parameter variable slots AFTER the value register range
        // so that we can load TDZ without corrupting argument values.
        let value_reg_count = params.len() as u16;
        // Leave room for value_regs (1..=value_reg_count) and this_slot (0).
        // Parameter variable slots and save temps start at value_reg_count + 1.
        let param_slot_start = value_reg_count + 1;
        if self.next_register < param_slot_start {
            self.next_register = param_slot_start;
        }

        if is_named_expression {
            if let Some(name) = function_name {
                let slot = self.declare_function_name(name);
                self.emit(Opcode::GetCurrentFunction);
                self.emit_u16(slot);
            }
        }

        if !is_simple_params {
            let mut has_arguments_param = false;
            for param in params {
                let mut names = std::collections::HashSet::new();
                Self::collect_parameter_bindings(param, &mut names);
                if names.contains("arguments") {
                    has_arguments_param = true;
                    break;
                }
            }
            let mark = if !self.is_arrow || has_arguments_param { 2 } else { 1 };
            self.emit(Opcode::ParamInitMark);
            self.emit_u8(mark);
        }

        let mut fixed_param_count = 0u16;
        let mut before_default = true;
        let mut needs_tdz = false;

        for param in params {
            if self.is_strict {
                match param {
                    Parameter::Identifier(name) if Self::is_strict_reserved_word(name) => {
                        return Err(format!(
                            "SyntaxError: '{}' cannot be declared in strict mode code",
                            name
                        ));
                    }
                    _ => {}
                }
            }
            match param {
                Parameter::Identifier(name) => {
                    let slot = self.declare_var(name, VariableKind::Let);
                    if before_default {
                        fixed_param_count += 1;
                    } else {
                        needs_tdz = true;
                    }
                    // Only emit LoadTdz for params that come AFTER the first
                    // default (may be referenced by earlier defaults) or for
                    // params that ARE defaults (self-reference check).
                    if needs_tdz || matches!(param, Parameter::AssignmentPattern(_)) {
                        self.emit(Opcode::LoadTdz);
                        self.emit_u16(slot);
                    }
                }
                Parameter::Pattern(pattern) => {
                    self.predeclare_binding_pattern(pattern, VariableKind::Let);
                    if before_default {
                        fixed_param_count += 1;
                    } else {
                        needs_tdz = true;
                    }
                }
                Parameter::AssignmentPattern(ap) => {
                    self.predeclare_assignment_target(&ap.left, VariableKind::Let);
                    before_default = false;
                    // Put this param's variable slot into TDZ so its own
                    // default value evaluation can detect self-references.
                    if let Some(name) = self.first_binding_name(&ap.left) {
                        if let Some(slot) = self.lookup_var_slot(&name) {
                            self.emit(Opcode::LoadTdz);
                            self.emit_u16(slot);
                        }
                    }
                }
                Parameter::RestElement(rest) => {
                    self.predeclare_assignment_target(&rest.argument, VariableKind::Let);
                    before_default = false;
                }
            }
        }
        self.max_register = self.max_register.max(fixed_param_count);

        // Save argument values into temp registers before loading TDZ.
        // This preserves the actual argument values from being overwritten
        // when we LoadTdz into the same registers (slot == value_reg for
        // simple params).
        let mut arg_saves: Vec<Option<u16>> = Vec::new();
        for (index, param) in params.iter().enumerate() {
            if matches!(param, Parameter::RestElement(_)) {
                arg_saves.push(None);
                continue;
            }
            let value_reg = (index as u16) + 1;
            let tmp = self.alloc_register();
            self.emit(Opcode::Move);
            self.emit_u16(tmp);
            self.emit_u16(value_reg);
            arg_saves.push(Some(tmp));
        }

        // Gather rest parameters BEFORE LoadTdz (which overwrites param
        // slots that may overlap with extra argument registers).
        if params
            .iter()
            .any(|p| matches!(p, Parameter::RestElement(_)))
        {
            let rest_source_reg = fixed_param_count + 1;
            self.emit(Opcode::GatherRest);
            self.emit_u16(rest_source_reg);

            for param in params {
                if let Parameter::RestElement(rest) = param {
                    self.gen_assignment_target_binding(
                        &rest.argument,
                        rest_source_reg,
                        VariableKind::Let,
                        ctx,
                    )?;
                    break;
                }
            }
        }

        self.in_parameter_initializer = true;
        for (index, param) in params.iter().enumerate() {
            let value_reg = (index as u16) + 1;
            let saved_tmp = arg_saves[index];

            if let Some(tmp) = saved_tmp {
                self.emit(Opcode::Move);
                self.emit_u16(value_reg);
                self.emit_u16(tmp);
                self.free_register(tmp);
            }

            match param {
                Parameter::Identifier(name) => {
                    if let Some(slot) = self.lookup_var_slot(name) {
                        if slot != value_reg {
                            self.emit(Opcode::Move);
                            self.emit_u16(slot);
                            self.emit_u16(value_reg);
                        }
                        for scope in self.scope_stack.iter_mut().rev() {
                            if let Some(entry) = scope.get_mut(name) {
                                entry.initialized = true;
                                break;
                            }
                        }
                    }
                }
                Parameter::Pattern(pattern) => {
                    self.gen_binding_pattern(pattern, value_reg, VariableKind::Let, ctx)?;
                }
                Parameter::AssignmentPattern(ap) => {
                    let binding_name = match &*ap.left {
                        AssignmentTarget::Identifier(name) => Some(name.as_str()),
                        _ => None,
                    };
                    self.gen_default_value_binding(&ap.right, value_reg, binding_name, ctx)?;
                    self.gen_assignment_target_binding(
                        &ap.left,
                        value_reg,
                        VariableKind::Let,
                        ctx,
                    )?;
                }
                Parameter::RestElement(_) => break,
            }
        }
        self.in_parameter_initializer = false;

        if !is_simple_params {
            self.emit(Opcode::ParamInitMark);
            self.emit_u8(0);
        }

        let mut locals: std::collections::HashSet<String> =
            self.scope_stack[0].keys().cloned().collect();
        let mut inherited = std::collections::HashSet::new();
        for stmt in &body.body {
            Self::collect_inherited_upvalues_stmt(stmt, &mut locals, &parent_vars, &mut inherited);
        }
        for name in inherited {
            if let Some(parent_var) = parent_vars.get(&name) {
                if self.upvalue_slots.contains_key(&name) {
                    continue;
                }
                let upvalue_slot = self.upvalues.len() as u16;
                self.upvalue_slots.insert(name.clone(), upvalue_slot);
                self.reserved_slots.insert(upvalue_slot);
                let atom = ctx.atom_table_mut().intern(name.as_str());
                let local_idx = if parent_var.is_inherited {
                    u16::MAX
                } else {
                    parent_var.local_idx
                };
                self.upvalues.push((atom.0, local_idx));
            }
        }

        for stmt in &body.body {
            if let ASTNode::FunctionDeclaration(func) = stmt {
                self.declare_var(&func.name, VariableKind::Var);
                if Self::detect_tiny_inline_function(func) {
                    self.tiny_inline_functions
                        .insert(func.name.clone(), func.clone());
                }
            }
        }

        let mut hoisted_vars: Vec<String> = Vec::new();
        for stmt in &body.body {
            Self::collect_hoisted_var_names(stmt, &mut hoisted_vars);
        }
        let mut new_var_slots: Vec<u16> = Vec::new();
        for name in &hoisted_vars {
            let already_declared = self
                .scope_stack[0]
                .get(name)
                .map_or(false, |entry| !entry.is_function_name);
            let slot = self.declare_var(name, VariableKind::Var);
            if !already_declared {
                new_var_slots.push(slot);
            }
        }
        for slot in new_var_slots {
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(slot);
        }

        self.global_lexical_names.clear();
        if self.is_global_scope() && !self.is_eval {
            for (name, _) in Self::collect_global_lexical_declarations(&body.body) {
                self.global_lexical_names.insert(name);
            }
        }

        self.push_scope();
        self.pre_scan_let_const(&body.body)?;

        if self.is_global_scope() {
            if !self.is_eval {
                self.emit_global_declaration_instantiation(&body.body, ctx);
            } else if !self.is_strict {
                self.emit_eval_declaration_instantiation(&body.body, ctx);
            }
        }

        if !self.is_arrow && !self.is_script_level && self.lookup_var("arguments").is_none() {
            let mut arguments_parent: FxHashMap<String, ParentVar> = FxHashMap::default();
            arguments_parent.insert(
                "arguments".to_string(),
                ParentVar {
                    local_idx: 0,
                    kind: VariableKind::Var,
                    initialized: true,
                    is_inherited: false,
                    closed: false,
                    is_function_name: false,
                    is_block_decl: false,
                },
            );
            let mut arguments_locals: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut arguments_mentions: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for stmt in &body.body {
                Self::collect_inherited_upvalues_stmt(
                    stmt,
                    &mut arguments_locals,
                    &arguments_parent,
                    &mut arguments_mentions,
                );
            }
            if arguments_mentions.contains("arguments") {
                let slot = self.alloc_register();
                self.reserved_slots.insert(slot);
                self.all_var_names.push(("arguments".to_string(), slot, 0));
                self.scope_stack.last_mut().unwrap().insert(
                    "arguments".to_string(),
                    VarEntry {
                        slot,
                        kind: VariableKind::Var,
                        initialized: true,
                        is_function_name: false,
                    },
                );
                self.function_uses_arguments = true;
                self.emit(Opcode::GetArguments);
                self.emit_u16(slot);
            }
        }

        self.emit_tdz_for_block(&body.body)?;

        {
            for stmt in &body.body {
                if let ASTNode::FunctionDeclaration(func) = stmt {
                    let slot = self
                        .lookup_var_slot(&func.name)
                        .unwrap_or_else(|| self.declare_var(&func.name, VariableKind::Var));
                    let reg = self.gen_function_expression(
                        &func.params,
                        &func.body,
                        Some(&func.name),
                        ctx,
                        func.generator,
                        func.is_async,
                        false,
                        false,
                        false,
                    )?;
                    if slot != reg {
                        self.emit(Opcode::Move);
                        self.emit_u16(slot);
                        self.emit_u16(reg);
                        self.free_register(reg);
                    }

                    if self.is_global_scope() && !(self.is_strict && self.is_eval) {
                        if self.is_eval {
                            self.emit_global_binding(9, &func.name, slot, ctx);
                        } else {
                            self.emit_global_binding(7, &func.name, slot, ctx);
                        }
                    }
                }
            }
        }

        if self.is_global_scope() {
            if !self.is_eval {
                let (var_names, function_names) =
                    Self::collect_script_var_and_function_names(&body.body);
                for name in &var_names {
                    if !function_names.contains(name) {
                        self.emit_global_binding(8, name, 0, ctx);
                    }
                }
            } else if !self.is_strict {
                self.emit_eval_var_instantiation(&body.body, ctx);
            }
        }

        let stmt_count = body.body.len();
        let mut last_expr_reg: Option<u16> = None;

        let mut last_nonempty_stmt_idx = stmt_count.wrapping_sub(1);
        while last_nonempty_stmt_idx < stmt_count {
            if !matches!(body.body[last_nonempty_stmt_idx], ASTNode::EmptyStatement) {
                break;
            }
            if last_nonempty_stmt_idx == 0 {
                last_nonempty_stmt_idx = stmt_count.wrapping_sub(1);
                break;
            }
            last_nonempty_stmt_idx -= 1;
        }

        let already_returns = body
            .body
            .last()
            .map_or(false, |s| matches!(s, ASTNode::ReturnStatement(_)));

        // For generator functions the parameter prologue must run when the
        // generator is invoked (so parameter errors surface synchronously),
        // but the body must not run until the first `next()`. Emitting an
        // initial `Yield` after the prologue suspends execution there.
        if is_generator {
            let r = self.alloc_register();
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(r);
            self.emit(Opcode::Yield);
            self.emit_u16(r);
            self.free_register(r);
        }

        let disposal = self.begin_scope_disposal(&body.body);

        for (i, stmt) in body.body.iter().enumerate() {
            if let Some((start, end)) = self.pending_field_init_range {
                self.in_field_initializer = i >= start && i < end;
            }
            if let Some(&line) = body.lines.get(i) {
                self.emit_line(line);
            }

            if return_last_expr && i == last_nonempty_stmt_idx {
                if let ASTNode::ExpressionStatement(es) = stmt {
                    if let Some(name) = self.field_init_name.clone() {
                        last_expr_reg = Some(
                            self.gen_expression_with_assign_name(&es.expression, &name, ctx)?,
                        );
                    } else {
                        last_expr_reg = Some(self.gen_expression(&es.expression, ctx)?);
                    }
                } else if matches!(stmt, ASTNode::FunctionDeclaration(_)) {
                } else if let Some(reg) = self.gen_statement_as_expression(stmt, ctx)? {
                    last_expr_reg = Some(reg);
                } else {
                    self.gen_statement(stmt, ctx)?;
                }
            } else if matches!(stmt, ASTNode::FunctionDeclaration(_)) {
            } else if return_last_expr {
                // Track expression statement values for the UpdateEmpty chain.
                // Declaration/control-flow statements produce empty completions
                // and should preserve the previous value.
                if let ASTNode::ExpressionStatement(es) = stmt {
                    let reg = self.gen_expression(&es.expression, ctx)?;
                    if let Some(prev) = last_expr_reg {
                        if prev != reg {
                            self.emit(Opcode::Move);
                            self.emit_u16(prev);
                            self.emit_u16(reg);
                        }
                        self.free_register(reg);
                    } else {
                        last_expr_reg = Some(reg);
                    }
                } else {
                    self.gen_statement(stmt, ctx)?;
                }
            } else {
                self.gen_statement(stmt, ctx)?;
            }
        }

        self.end_scope_disposal(disposal);
        self.pop_scope();

        if !already_returns {
            let return_reg = if let Some(reg) = last_expr_reg {
                reg
            } else {
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(0);
                0
            };
            self.emit(Opcode::Return);
            self.emit_u16(return_reg);
        }

        let line_table = std::mem::take(&mut self.line_table);
        let code = std::mem::take(&mut self.code);
        let mut constants = std::mem::take(&mut self.constants);
        if self.function_uses_arguments {
            let atom = if let Some(atom) = ctx.lookup_atom("__usesArguments__") {
                atom
            } else {
                ctx.atom_table_mut().intern("__usesArguments__")
            };
            if !constants
                .iter()
                .any(|v| v.is_string() && v.get_atom() == atom)
            {
                constants.push(JSValue::new_string(atom));
            }
        }
        self.is_script_level = saved_is_script_level;
        self.scope_is_global = saved_scope_is_global;
        let var_name_to_slot: std::rc::Rc<Vec<(u32, u16, u8)>> = std::rc::Rc::new(
            self.all_var_names
                .iter()
                .map(|(name, slot, tag)| {
                    let atom = ctx.intern(name);
                    (atom.0, *slot, *tag)
                })
                .collect(),
        );
        let mut bc = Bytecode {
            code,
            constants,
            locals_count: (self.max_register as u32) + 1,
            param_count: fixed_param_count,
            line_number_table: if line_table.entries().is_empty() {
                None
            } else {
                Some(std::sync::Arc::new(line_table))
            },
            ic_table: crate::compiler::InlineCacheTable::new(),
            shared_ic_table_ptr: std::ptr::null_mut(),
            shared_nested_bytecodes_ptr: std::ptr::null_mut(),
            shared_code_ptr: std::ptr::null(),
            shared_code_len: 0,
            shared_const_ptr: std::ptr::null(),
            shared_const_len: 0,
            uses_arguments: self.function_uses_arguments,
            has_simple_parameters: is_simple_params,
            is_strict: self.is_strict,
            var_name_to_slot,
            nested_bytecodes: Vec::new(),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        };
        bc.is_simple_constructor = detect_simple_constructor_props(&bc);
        if bc.is_simple_constructor {
            let code = &bc.code;
            let mut pc = 0usize;
            while pc < code.len() {
                let op = Opcode::from_u8_unchecked(code[pc]);
                let size = Opcode::instruction_size(op);
                if size == 0 || pc + size > code.len() {
                    break;
                }
                if op == Opcode::SetNamedProp {
                    let obj = u16::from_le_bytes([code[pc + 1], code[pc + 2]]);
                    let val = u16::from_le_bytes([code[pc + 3], code[pc + 4]]);
                    let atom = u32::from_le_bytes([
                        code[pc + 5],
                        code[pc + 6],
                        code[pc + 7],
                        code[pc + 8],
                    ]);
                    if obj == 0 && val >= 1 && val <= bc.param_count {
                        bc.simple_constructor_props.push((
                            crate::runtime::atom::Atom(atom),
                            val - 1,
                            (pc + size) as u16,
                        ));
                        bc.cached_constructor_atoms
                            .push(crate::runtime::atom::Atom(atom));
                    }
                }
                pc += size;
            }
        }
        Ok((bc, std::mem::take(&mut self.upvalues)))
    }

    pub fn take_bytecode(&mut self) -> (Vec<u8>, Vec<JSValue>, u32, u16) {
        let result = (
            std::mem::take(&mut self.code),
            std::mem::take(&mut self.constants),
            (self.max_register as u32) + 1,
            0,
        );
        self.next_register = 0;
        self.max_register = 0;
        self.free_list.clear();
        self.scope_stack = vec![FxHashMap::default()];
        self.reserved_slots.clear();
        self.upvalues.clear();
        self.upvalue_slots.clear();
        self.depth = 0;
        self.function_uses_arguments = false;

        result
    }

    fn begin_disposal_region(&mut self) -> DisposalRegion {
        let mut region = DisposalRegion::alloc(self);
        for r in [
            region.stack,
            region.state,
            region.index,
            region.awaited,
            region.error,
            region.flags,
        ] {
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(r);
        }
        self.emit(Opcode::Try);
        region.catch_pc_pos = self.code.len();
        self.emit_i32(0);
        region.finally_pc_pos = self.code.len();
        self.emit_i32(0);
        self.handler_depth += 1;
        region
    }

    fn begin_scope_disposal(&mut self, body: &[ASTNode]) -> Option<DisposalRegion> {
        if !body_has_using_declaration(body) {
            return None;
        }
        let region = self.begin_disposal_region();
        *self.scope_dispose_stack.last_mut().unwrap() = Some(region.stack);
        Some(region)
    }

    fn end_scope_disposal(&mut self, region: Option<DisposalRegion>) {
        let Some(region) = region else {
            return;
        };
        self.emit(Opcode::Catch);
        self.handler_depth -= 1;
        let disposal_start = self.code.len();
        let bytes = (disposal_start as i32).to_le_bytes();
        self.code[region.catch_pc_pos..region.catch_pc_pos + 4].copy_from_slice(&bytes);
        self.code[region.finally_pc_pos..region.finally_pc_pos + 4].copy_from_slice(&bytes);
        self.emit(Opcode::Finally);
        self.emit(Opcode::DisposeResourcesFinally);
        self.emit_u16(region.stack);
        self.emit_u16(region.state);
        self.emit_u16(region.index);
        self.emit_u16(region.awaited);
        self.emit_u16(region.error);
        self.emit_u16(region.flags);
        self.emit(Opcode::EndFinally);
        *self.scope_dispose_stack.last_mut().unwrap() = None;
        region.free(self);
    }

    fn pre_scan_let_const(&mut self, body: &[ASTNode]) -> Result<(), String> {
        for node in body {
            match node {
                ASTNode::VariableDeclaration(decl) if decl.kind != VariableKind::Var => {
                    for d in &decl.declarations {
                        self.pre_scan_binding(&d.id, decl.kind);
                    }
                }
                ASTNode::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(crate::compiler::ast::ExportDeclaration::Variable(decl))
                        if decl.kind != VariableKind::Var =>
                    {
                        for d in &decl.declarations {
                            self.pre_scan_binding(&d.id, decl.kind);
                        }
                    }
                    Some(crate::compiler::ast::ExportDeclaration::Class(class)) => {
                        if let Some(last) = self.scope_stack.last() {
                            if !last.contains_key(&class.name) {
                                self.declare_var(&class.name, VariableKind::Let);
                            }
                        }
                    }
                    _ => {}
                },
                ASTNode::ClassDeclaration(class) => {
                    if let Some(last) = self.scope_stack.last() {
                        if !last.contains_key(&class.name) {
                            self.declare_var(&class.name, VariableKind::Let);
                        }
                    }
                }
                ASTNode::BlockStatement(block) => {
                    self.push_scope();
                    self.pre_scan_let_const(&block.body)?;
                    self.pop_scope();
                }
                ASTNode::TryStatement(stmt) => {
                    self.push_scope();
                    self.pre_scan_let_const(&stmt.block.body)?;
                    self.pop_scope();
                    if let Some(handler) = &stmt.handler {
                        self.push_scope();
                        if let Some(param) = &handler.param {
                            self.pre_scan_binding(param, VariableKind::Let);
                        }
                        self.pre_scan_let_const(&handler.body.body)?;
                        self.pop_scope();
                    }
                    if let Some(finalizer) = &stmt.finalizer {
                        self.push_scope();
                        self.pre_scan_let_const(&finalizer.body)?;
                        self.pop_scope();
                    }
                }
                ASTNode::ForStatement(stmt) => {
                    self.push_scope();
                    if let Some(init) = &stmt.init {
                        if let ForInit::VariableDeclaration(vd) = init {
                            if vd.kind != VariableKind::Var {
                                for d in &vd.declarations {
                                    self.pre_scan_binding(&d.id, vd.kind);
                                }
                            }
                        }
                    }
                    if let ASTNode::BlockStatement(block) = stmt.body.as_ref() {
                        self.pre_scan_let_const(&block.body)?;
                    } else {
                        self.pre_scan_let_const(&[stmt.body.as_ref().clone()])?;
                    }
                    self.pop_scope();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn pre_scan_binding(&mut self, pattern: &BindingPattern, kind: VariableKind) {
        match pattern {
            BindingPattern::Identifier(name) => {
                if let Some(last) = self.scope_stack.last() {
                    if !last.contains_key(name) {
                        self.declare_var(name, kind);
                    }
                }
            }
            BindingPattern::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(pat) = elem {
                        match pat {
                            PatternElement::Pattern(p) => {
                                self.pre_scan_assignment_target_binding(p, kind)
                            }
                            PatternElement::RestElement(r) => {
                                self.pre_scan_assignment_target_binding(&r.argument, kind)
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                self.pre_scan_assignment_target_binding(&ap.left, kind)
                            }
                        }
                    }
                }
            }
            BindingPattern::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            self.pre_scan_assignment_target_binding(value, kind);
                        }
                        ObjectPatternProperty::RestElement(r) => {
                            self.pre_scan_assignment_target_binding(&r.argument, kind);
                        }
                    }
                }
            }
            BindingPattern::AssignmentPattern(ap) => {
                self.pre_scan_assignment_target_binding(&ap.left, kind);
            }
        }
    }

    fn pre_scan_assignment_target_binding(
        &mut self,
        target: &AssignmentTarget,
        kind: VariableKind,
    ) {
        match target {
            AssignmentTarget::Identifier(name) => {
                if let Some(last) = self.scope_stack.last() {
                    if !last.contains_key(name) {
                        self.declare_var(name, kind);
                    }
                }
            }
            AssignmentTarget::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(pat) = elem {
                        match pat {
                            PatternElement::Pattern(p) => {
                                self.pre_scan_assignment_target_binding(p, kind)
                            }
                            PatternElement::RestElement(r) => {
                                self.pre_scan_assignment_target_binding(&r.argument, kind)
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                self.pre_scan_assignment_target_binding(&ap.left, kind)
                            }
                        }
                    }
                }
            }
            AssignmentTarget::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            self.pre_scan_assignment_target_binding(value, kind);
                        }
                        ObjectPatternProperty::RestElement(r) => {
                            self.pre_scan_assignment_target_binding(&r.argument, kind);
                        }
                    }
                }
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                self.pre_scan_assignment_target_binding(&ap.left, kind);
            }
            _ => {}
        }
    }

    fn emit_tdz_for_block(&mut self, body: &[ASTNode]) -> Result<(), String> {
        for node in body {
            if let ASTNode::VariableDeclaration(decl) = node {
                if decl.kind != VariableKind::Var {
                    for d in &decl.declarations {
                        self.emit_tdz_for_binding(&d.id)?;
                    }
                }
            } else if let ASTNode::ExportNamedDeclaration(export) = node {
                match &export.declaration {
                    Some(crate::compiler::ast::ExportDeclaration::Variable(decl))
                        if decl.kind != VariableKind::Var =>
                    {
                        for d in &decl.declarations {
                            self.emit_tdz_for_binding(&d.id)?;
                        }
                    }
                    Some(crate::compiler::ast::ExportDeclaration::Class(class)) => {
                        if let Some(slot) = self.lookup_var_slot(&class.name) {
                            self.emit(Opcode::LoadTdz);
                            self.emit_u16(slot);
                        }
                    }
                    _ => {}
                }
            } else if let ASTNode::ClassDeclaration(class) = node {
                if let Some(slot) = self.lookup_var_slot(&class.name) {
                    self.emit(Opcode::LoadTdz);
                    self.emit_u16(slot);
                }
            }
        }
        Ok(())
    }

    fn emit_tdz_for_binding(&mut self, pattern: &BindingPattern) -> Result<(), String> {
        match pattern {
            BindingPattern::Identifier(name) => {
                if let Some(slot) = self.lookup_var_slot(name) {
                    self.emit(Opcode::LoadTdz);
                    self.emit_u16(slot);
                }
            }
            BindingPattern::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(pat) = elem {
                        match pat {
                            PatternElement::Pattern(p) => self.emit_tdz_for_assignment_target(p)?,
                            PatternElement::RestElement(r) => {
                                self.emit_tdz_for_assignment_target(&r.argument)?
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                self.emit_tdz_for_assignment_target(&ap.left)?
                            }
                        }
                    }
                }
            }
            BindingPattern::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            self.emit_tdz_for_assignment_target(value)?;
                        }
                        ObjectPatternProperty::RestElement(r) => {
                            self.emit_tdz_for_assignment_target(&r.argument)?;
                        }
                    }
                }
            }
            BindingPattern::AssignmentPattern(ap) => {
                self.emit_tdz_for_assignment_target(&ap.left)?;
            }
        }
        Ok(())
    }

    fn emit_tdz_for_assignment_target(&mut self, target: &AssignmentTarget) -> Result<(), String> {
        match target {
            AssignmentTarget::Identifier(name) => {
                if let Some(slot) = self.lookup_var_slot(name) {
                    self.emit(Opcode::LoadTdz);
                    self.emit_u16(slot);
                }
            }
            AssignmentTarget::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(pat) = elem {
                        match pat {
                            PatternElement::Pattern(p) => self.emit_tdz_for_assignment_target(p)?,
                            PatternElement::RestElement(r) => {
                                self.emit_tdz_for_assignment_target(&r.argument)?
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                self.emit_tdz_for_assignment_target(&ap.left)?
                            }
                        }
                    }
                }
            }
            AssignmentTarget::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            self.emit_tdz_for_assignment_target(value)?;
                        }
                        ObjectPatternProperty::RestElement(r) => {
                            self.emit_tdz_for_assignment_target(&r.argument)?;
                        }
                    }
                }
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                self.emit_tdz_for_assignment_target(&ap.left)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn gen_with_statement_common(
        &mut self,
        stmt: &WithStatement,
        ctx: &mut JSContext,
        target_reg: Option<u16>,
    ) -> Result<(), String> {
        if self.is_strict {
            return Err(
                "SyntaxError: Strict mode code may not include a with statement".to_string(),
            );
        }
        if let Some(target) = target_reg {
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(target);
        }
        let object_reg = self.gen_expression(&stmt.object, ctx)?;
        self.emit(Opcode::WithEnter);
        self.emit_u16(object_reg);
        self.free_register(object_reg);
        self.with_scopes.push(WithScope {
            function_id: self.function_id,
            scope_index: self.depth as usize,
            inner_bindings: std::collections::HashSet::new(),
        });
        let result = if let Some(target) = target_reg {
            self.gen_statement_with_target(&stmt.body, ctx, target)
        } else {
            self.gen_statement(&stmt.body, ctx)
        };
        self.with_scopes.pop();
        self.emit(Opcode::WithExit);
        result
    }

    fn gen_statement(&mut self, stmt: &ASTNode, ctx: &mut JSContext) -> Result<(), String> {
        match stmt {
            ASTNode::ExpressionStatement(es) => {
                let r = self.gen_expression(&es.expression, ctx)?;

                self.free_register(r);
            }
            ASTNode::VariableDeclaration(decl) => {
                self.gen_variable_declaration(decl, ctx)?;
            }
            ASTNode::FunctionDeclaration(func) => {
                let block_scoped =
                    self.depth > 0 && (self.is_strict || func.generator || func.is_async);
                let slot = if block_scoped {
                    match self.scope_stack.last().and_then(|scope| scope.get(&func.name)) {
                        Some(entry) => entry.slot,
                        None => self.declare_var(&func.name, VariableKind::Let),
                    }
                } else {
                    self.lookup_var_slot(&func.name)
                        .unwrap_or_else(|| self.declare_var(&func.name, VariableKind::Var))
                };
                let reg = self.gen_function_expression(
                    &func.params,
                    &func.body,
                    Some(&func.name),
                    ctx,
                    func.generator,
                    func.is_async,
                    false,
                    false,
                    false,
                )?;
                if slot != reg {
                    self.emit(Opcode::Move);
                    self.emit_u16(slot);
                    self.emit_u16(reg);
                }

                if !block_scoped && self.is_global_scope() && !(self.is_strict && self.is_eval) {
                    let atom = ctx.intern(&func.name);
                    let idx = self.add_constant(JSValue::new_string(atom));
                    if self.is_eval {
                        self.emit(Opcode::SetGlobalVar);
                    } else {
                        self.emit(Opcode::SetGlobal);
                    }
                    self.emit_u32(idx);
                    self.emit_u16(slot);
                }
            }
            ASTNode::ClassDeclaration(class) => {
                self.gen_class_declaration(class, ctx)?;
            }
            ASTNode::BlockStatement(block) => {
                self.push_scope();
                self.pre_scan_let_const(&block.body)?;
                self.emit_tdz_for_block(&block.body)?;
                let disposal = self.begin_scope_disposal(&block.body);
                for (i, s) in block.body.iter().enumerate() {
                    if let Some(&line) = block.lines.get(i) {
                        self.emit_line(line);
                    }
                    self.gen_statement(s, ctx)?;
                }
                self.end_scope_disposal(disposal);
                self.pop_scope();
            }
            ASTNode::IfStatement(stmt) => {
                self.gen_if_statement(stmt, ctx)?;
            }
            ASTNode::WhileStatement(stmt) => {
                self.gen_while_statement(stmt, ctx)?;
            }
            ASTNode::ForStatement(stmt) => {
                self.gen_for_statement(stmt, ctx)?;
            }
            ASTNode::ForInStatement(stmt) => {
                self.gen_for_in_statement(stmt, ctx)?;
            }
            ASTNode::ForOfStatement(stmt) => {
                self.gen_for_of_statement(stmt, ctx)?;
            }
            ASTNode::DoWhileStatement(stmt) => {
                self.gen_do_while_statement(stmt, ctx)?;
            }
            ASTNode::SwitchStatement(stmt) => {
                self.gen_switch_statement(stmt, ctx)?;
            }
            ASTNode::BreakStatement(stmt) => {
                let target = if let Some(label) = &stmt.label {
                    self.breakable_frames
                        .iter()
                        .rposition(|f| f.labels.iter().any(|l| l == label))
                } else {
                    self.breakable_frames.iter().rposition(|f| !f.is_label)
                };
                let Some(index) = target else {
                    return Err(match &stmt.label {
                        Some(label) => format!("SyntaxError: Label '{}' not found", label),
                        None => "SyntaxError: Illegal break statement".to_string(),
                    });
                };
                let depth = self.breakable_frames[index].handler_depth;
                self.emit(Opcode::JumpBreak);
                self.emit_u16(depth as u16);
                let patch = self.code.len();
                self.emit_i32(0);
                self.breakable_frames[index].break_patches.push(patch);
            }
            ASTNode::ContinueStatement(stmt) => {
                let target = if let Some(label) = &stmt.label {
                    self.breakable_frames.iter().rposition(|f| {
                        f.continue_target.is_some() && f.labels.iter().any(|l| l == label)
                    })
                } else {
                    self.breakable_frames
                        .iter()
                        .rposition(|f| f.continue_target.is_some())
                };
                let Some(index) = target else {
                    return Err(match &stmt.label {
                        Some(label) => {
                            format!("SyntaxError: Continue target '{}' not found", label)
                        }
                        None => "SyntaxError: Illegal continue statement".to_string(),
                    });
                };
                let depth = self.breakable_frames[index].continue_depth;
                self.emit(Opcode::JumpBreak);
                self.emit_u16(depth as u16);
                let patch = self.code.len();
                self.emit_i32(0);
                self.breakable_frames[index].continue_patches.push(patch);
            }
            ASTNode::ReturnStatement(stmt) => {
                if self.is_script_level {
                    return Err("SyntaxError: Illegal return statement".to_string());
                }
                let return_reg = if let Some(expr) = &stmt.argument {
                    let reg = self.gen_expression(expr, ctx)?;
                    if self.in_async_generator {
                        self.emit(Opcode::Await);
                        self.emit_u16(reg);
                    }
                    reg
                } else {
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(0);
                    0
                };
                self.emit(Opcode::Return);
                self.emit_u16(return_reg);
            }
            ASTNode::ThrowStatement(stmt) => {
                self.gen_throw_statement(stmt, ctx)?;
            }
            ASTNode::TryStatement(stmt) => {
                self.gen_try_statement(stmt, ctx, None)?;
            }
            ASTNode::WithStatement(stmt) => {
                self.gen_with_statement_common(stmt, ctx, None)?;
            }
            ASTNode::ImportDeclaration(decl) => {
                self.gen_import_declaration(decl, ctx)?;
            }
            ASTNode::ExportNamedDeclaration(decl) => {
                self.gen_export_named_declaration(decl, ctx)?;
            }
            ASTNode::ExportDefaultDeclaration(decl) => {
                self.gen_export_default_declaration(decl, ctx)?;
            }
            ASTNode::ExportAllDeclaration(decl) => {
                self.gen_export_all_declaration(decl, ctx)?;
            }
            ASTNode::EmptyStatement => {}
            ASTNode::LabelledStatement(stmt) => {
                let mut chain: Vec<String> = vec![stmt.label.clone()];
                let mut terminal: &ASTNode = &stmt.body;
                while let ASTNode::LabelledStatement(current) = terminal {
                    chain.push(current.label.clone());
                    terminal = &current.body;
                }
                let saved_pending = std::mem::take(&mut self.pending_labels);
                if Self::statement_is_iteration_statement(terminal) {
                    self.pending_labels = chain;
                    let result = self.gen_statement(terminal, ctx);
                    self.pending_labels = saved_pending;
                    result?;
                } else {
                    self.breakable_frames.push(BreakableFrame {
                        labels: chain,
                        is_label: true,
                        handler_depth: self.handler_depth,
                        continue_depth: self.handler_depth,
                        break_patches: Vec::new(),
                        continue_patches: Vec::new(),
                        continue_target: None,
                    });
                    self.gen_statement(terminal, ctx)?;
                    let frame = self.breakable_frames.pop().unwrap();
                    let end_pos = self.code.len();
                    for patch in frame.break_patches {
                        self.patch_jump(patch, end_pos);
                    }
                    self.pending_labels = saved_pending;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn emit_export_value(&mut self, name: &str, value_reg: u16, ctx: &mut JSContext) {
        let name_idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
        let name_reg = self.alloc_register();
        self.emit(Opcode::LoadConst);
        self.emit_u16(name_reg);
        self.emit_u32(name_idx);
        let fn_idx = self.add_constant(JSValue::new_string(ctx.intern("__v16_export__")));
        let fn_reg = self.alloc_register();
        self.emit(Opcode::GetGlobal);
        self.emit_u16(fn_reg);
        self.emit_u32(fn_idx);
        let dst = self.alloc_register();
        self.emit(Opcode::Call);
        self.emit_u16(dst);
        self.emit_u16(fn_reg);
        self.emit_u16(2);
        self.emit_u16(name_reg);
        self.emit_u16(value_reg);
        self.free_register(dst);
        self.free_register(fn_reg);
        self.free_register(name_reg);
    }

    fn emit_export_binding(
        &mut self,
        binding: &str,
        exported: &str,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let expr = Expression::Identifier(Identifier {
            name: binding.to_string(),
            parenthesized: false,
        });
        let reg = self.gen_expression(&expr, ctx)?;
        let tmp = self.alloc_register();
        self.emit(Opcode::Move);
        self.emit_u16(tmp);
        self.emit_u16(reg);
        self.emit_export_value(exported, tmp, ctx);
        self.free_register(tmp);

        let getter = Expression::FunctionExpression(FunctionExpression {
            name: None,
            params: Vec::new(),
            body: BlockStatement {
                body: vec![ASTNode::ReturnStatement(ReturnStatement {
                    argument: Some(Expression::Identifier(Identifier {
                        name: binding.to_string(),
                        parenthesized: false,
                    })),
                })],
                lines: Vec::new(),
            },
            generator: false,
            is_async: false,
        });
        let getter_reg = self.gen_expression(&getter, ctx)?;
        self.emit_export_live(exported, getter_reg, ctx);
        self.free_register(getter_reg);
        Ok(())
    }

    fn emit_export_live(&mut self, name: &str, value_reg: u16, ctx: &mut JSContext) {
        let name_idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
        let name_reg = self.alloc_register();
        self.emit(Opcode::LoadConst);
        self.emit_u16(name_reg);
        self.emit_u32(name_idx);
        let fn_idx = self.add_constant(JSValue::new_string(ctx.intern("__v16_export_live__")));
        let fn_reg = self.alloc_register();
        self.emit(Opcode::GetGlobal);
        self.emit_u16(fn_reg);
        self.emit_u32(fn_idx);
        let dst = self.alloc_register();
        self.emit(Opcode::Call);
        self.emit_u16(dst);
        self.emit_u16(fn_reg);
        self.emit_u16(2);
        self.emit_u16(name_reg);
        self.emit_u16(value_reg);
        self.free_register(dst);
        self.free_register(fn_reg);
        self.free_register(name_reg);
    }

    fn gen_export_named_declaration(
        &mut self,
        decl: &ExportNamedDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        if let Some(declaration) = &decl.declaration {
            match declaration {
                ExportDeclaration::Variable(variable) => {
                    self.gen_variable_declaration(variable, ctx)?;
                    for declarator in &variable.declarations {
                        let mut names = std::collections::HashSet::new();
                        Self::collect_binding_pattern_bindings(&declarator.id, &mut names);
                        let mut names: Vec<String> = names.into_iter().collect();
                        names.sort();
                        for name in names {
                            self.emit_export_binding(&name, &name, ctx)?;
                        }
                    }
                }
                ExportDeclaration::Function(function) => {
                    self.gen_statement(&ASTNode::FunctionDeclaration(function.clone()), ctx)?;
                    self.emit_export_binding(&function.name, &function.name, ctx)?;
                }
                ExportDeclaration::Class(class) => {
                    self.gen_statement(&ASTNode::ClassDeclaration(class.clone()), ctx)?;
                    self.emit_export_binding(&class.name, &class.name, ctx)?;
                }
            }
            return Ok(());
        }
        if let Some(source) = &decl.source {
            for specifier in &decl.specifiers {
                let expr = Expression::CallExpression(CallExpression {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "__v16_import__".to_string(),
                        parenthesized: false,
                    })),
                    arguments: vec![
                        Argument::Expression(Expression::Literal(Literal::String(
                            source.clone(),
                            false,
                            false,
                        ))),
                        Argument::Expression(Expression::Literal(Literal::String(
                            specifier.local.clone(),
                            false,
                            false,
                        ))),
                    ],
                });
                let reg = self.gen_expression(&expr, ctx)?;
                self.emit_export_value(&specifier.exported, reg, ctx);
                self.free_register(reg);
            }
            return Ok(());
        }
        for specifier in &decl.specifiers {
            self.emit_export_binding(&specifier.local, &specifier.exported, ctx)?;
        }
        Ok(())
    }

    fn gen_export_default_declaration(
        &mut self,
        decl: &ExportDefaultDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match &decl.declaration {
            ExportDefaultDeclarationKind::Function(function) => {
                if function.name.is_empty() {
                    let expression = Expression::FunctionExpression(FunctionExpression {
                        name: None,
                        params: function.params.clone(),
                        body: function.body.clone(),
                        generator: function.generator,
                        is_async: function.is_async,
                    });
                    let reg = self.gen_expression_with_assign_name(
                        &expression,
                        "default",
                        ctx,
                    )?;
                    self.emit_export_value("default", reg, ctx);
                    self.free_register(reg);
                } else {
                    self.gen_statement(&ASTNode::FunctionDeclaration(function.clone()), ctx)?;
                    self.emit_export_binding(&function.name, "default", ctx)?;
                }
            }
            ExportDefaultDeclarationKind::Class(class) => {
                if class.name.is_empty() {
                    let expression = Expression::ClassExpression(ClassExpression {
                        name: None,
                        super_class: class.super_class.clone(),
                        body: class.body.clone(),
                        decorators: class.decorators.clone(),
                    });
                    let reg = self.gen_expression_with_assign_name(
                        &expression,
                        "default",
                        ctx,
                    )?;
                    self.emit_export_value("default", reg, ctx);
                    self.free_register(reg);
                } else {
                    self.gen_statement(&ASTNode::ClassDeclaration(class.clone()), ctx)?;
                    self.emit_export_binding(&class.name, "default", ctx)?;
                }
            }
            ExportDefaultDeclarationKind::Expression(expression) => {
                let reg = self.gen_expression_with_assign_name(expression, "default", ctx)?;
                self.emit_export_value("default", reg, ctx);
                self.free_register(reg);
            }
        }
        Ok(())
    }

    fn gen_export_all_declaration(
        &mut self,
        decl: &ExportAllDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let arguments = vec![Argument::Expression(Expression::Literal(Literal::String(
            decl.source.clone(),
            false,
            false,
        )))];
        let expr = Expression::CallExpression(CallExpression {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "__v16_import__".to_string(),
                parenthesized: false,
            })),
            arguments,
        });
        let reg = self.gen_expression(&expr, ctx)?;
        if decl.exported.is_some() {
            self.emit_export_value(decl.exported.as_ref().unwrap(), reg, ctx);
        } else {
            let all_idx = self.add_constant(JSValue::new_string(ctx.intern("__v16_export_all__")));
            let fn_reg = self.alloc_register();
            self.emit(Opcode::GetGlobal);
            self.emit_u16(fn_reg);
            self.emit_u32(all_idx);
            let dst = self.alloc_register();
            self.emit(Opcode::Call);
            self.emit_u16(dst);
            self.emit_u16(fn_reg);
            self.emit_u16(1);
            self.emit_u16(reg);
            self.free_register(dst);
            self.free_register(fn_reg);
        }
        self.free_register(reg);
        Ok(())
    }

    fn gen_import_declaration(
        &mut self,
        decl: &ImportDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let build_call = |binding: Option<&str>| -> Expression {
            let mut arguments = vec![Argument::Expression(Expression::Literal(Literal::String(
                decl.source.clone(),
                false,
                false,
            )))];
            if let Some(name) = binding {
                arguments.push(Argument::Expression(Expression::Literal(Literal::String(
                    name.to_string(),
                    false,
                    false,
                ))));
            }
            Expression::CallExpression(CallExpression {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "__v16_import__".to_string(),
                    parenthesized: false,
                })),
                arguments,
            })
        };

        if decl.specifiers.is_empty() {
            let reg = self.gen_expression(&build_call(None), ctx)?;
            self.free_register(reg);
            return Ok(());
        }

        for specifier in &decl.specifiers {
            let (local, binding) = match specifier {
                ImportSpecifier::Named { imported, local } => {
                    (local.clone(), Some(imported.clone()))
                }
                ImportSpecifier::Default { local } => (local.clone(), Some("default".to_string())),
                ImportSpecifier::Namespace { local } => (local.clone(), None),
            };
            let declaration = VariableDeclaration {
                kind: VariableKind::Var,
                declarations: vec![VariableDeclarator {
                    id: BindingPattern::Identifier(local),
                    init: Some(build_call(binding.as_deref())),
                }],
            };
            self.gen_variable_declaration(&declaration, ctx)?;
        }

        Ok(())
    }

    fn gen_binding_pattern(
        &mut self,
        pattern: &BindingPattern,
        value_reg: u16,
        kind: VariableKind,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match pattern {
            BindingPattern::Identifier(name) => {
                if self.is_strict && Self::is_strict_reserved_word(name) {
                    return Err(format!(
                        "SyntaxError: '{}' cannot be declared in strict mode code",
                        name
                    ));
                }
                let slot = if kind == VariableKind::Var {
                    self.declare_var(name, kind)
                } else {
                    self.lookup_var_slot(name)
                        .unwrap_or_else(|| self.declare_var(name, kind))
                };

                for scope in self.scope_stack.iter_mut().rev() {
                    if let Some(entry) = scope.get_mut(name) {
                        entry.initialized = true;
                        break;
                    }
                }
                let with_target = if kind == VariableKind::Var && !self.with_scopes.is_empty()
                {
                    match self.lookup_var_scope_index(name) {
                        Some(index) => self.with_applies_to_local(name, index),
                        None => self.with_applies_to_outer(name),
                    }
                } else {
                    false
                };
                if with_target {
                    self.gen_with_store(name, value_reg, ctx)?;
                    return Ok(());
                }
                if slot != value_reg {
                    self.emit(Opcode::Move);
                    self.emit_u16(slot);
                    self.emit_u16(value_reg);
                }

                if kind == VariableKind::Var
                    && self.is_global_scope()
                    && !(self.is_strict && self.is_eval)
                {
                    let atom = ctx.intern(name);
                    let idx = self.add_constant(JSValue::new_string(atom));
                    self.emit(Opcode::SetGlobalVar);
                    self.emit_u32(idx);
                    self.emit_u16(slot);
                }
                Ok(())
            }
            BindingPattern::ArrayPattern(arr) => {
                self.emit(Opcode::CheckObjectCoercible);
                self.emit_u16(value_reg);

                let iter_reg = self.alloc_register();
                self.emit(Opcode::GetIterator);
                self.emit_u16(iter_reg);
                self.emit_u16(value_reg);

                let len = arr.elements.len();
                let mut last_done_reg: Option<u16> = None;
                for (i, elem) in arr.elements.iter().enumerate() {
                    if i == len - 1 {
                        if let Some(PatternElement::RestElement(rest)) = elem {
                            let rest_reg = self.alloc_register();
                            self.emit(Opcode::NewArray);
                            self.emit_u16(rest_reg);
                            self.emit_u16(0);
                            let loop_start = self.code.len();
                            let val_for_rest = self.alloc_register();
                            let done_for_rest = self.alloc_register();
                            self.emit(Opcode::IteratorNext);
                            self.emit_u16(val_for_rest);
                            self.emit_u16(done_for_rest);
                            self.emit_u16(iter_reg);
                            self.emit(Opcode::JumpIf);
                            self.emit_u16(done_for_rest);
                            let exit_patch = self.code.len();
                            self.emit_i32(0);
                            self.free_register(done_for_rest);
                            self.emit(Opcode::ArrayPush);
                            self.emit_u16(rest_reg);
                            self.emit_u16(val_for_rest);
                            self.free_register(val_for_rest);
                            self.emit_backward_jump(loop_start);
                            let end_pos = self.code.len();
                            self.patch_jump(exit_patch, end_pos);
                            self.gen_assignment_target_binding(
                                &rest.argument,
                                rest_reg,
                                kind,
                                ctx,
                            )?;
                            self.free_register(rest_reg);
                            self.free_register(iter_reg);
                            return Ok(());
                        }
                    }

                    let val_reg = self.alloc_register();
                    let done_reg = self.alloc_register();
                    self.emit(Opcode::IteratorNext);
                    self.emit_u16(val_reg);
                    self.emit_u16(done_reg);
                    self.emit_u16(iter_reg);

                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(done_reg);
                    let not_done_skip = self.code.len();
                    self.emit_i32(0);

                    let undef_reg = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(undef_reg);
                    self.emit(Opcode::Move);
                    self.emit_u16(val_reg);
                    self.emit_u16(undef_reg);
                    self.free_register(undef_reg);

                    let after_undef = self.code.len();
                    self.patch_jump(not_done_skip, after_undef);
                    if i == len - 1 {
                        last_done_reg = Some(done_reg);
                    } else {
                        self.free_register(done_reg);
                    }

                    if let Some(elem) = elem {
                        match elem {
                            PatternElement::Pattern(target) => {
                                self.gen_assignment_target_binding(target, val_reg, kind, ctx)?;
                            }
                            PatternElement::RestElement(_) => {
                                return Err("SyntaxError: rest element not at end".to_string());
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                let binding_name = match &*ap.left {
                                    AssignmentTarget::Identifier(name) => Some(name.as_str()),
                                    _ => None,
                                };
                                self.gen_default_value_binding(
                                    &ap.right,
                                    val_reg,
                                    binding_name,
                                    ctx,
                                )?;
                                self.gen_assignment_target_binding(&ap.left, val_reg, kind, ctx)?;
                            }
                        }
                    }
                    self.free_register(val_reg);
                }
                if let Some(d) = last_done_reg {
                    self.emit_iterator_close(iter_reg, Some(d));
                    self.free_register(d);
                }
                self.free_register(iter_reg);
                Ok(())
            }
            BindingPattern::ObjectPattern(obj) => {
                self.emit(Opcode::CheckObjectCoercible);
                self.emit_u16(value_reg);
                let has_rest = obj
                    .properties
                    .iter()
                    .any(|p| matches!(p, ObjectPatternProperty::RestElement(_)));
                let keys_reg = if has_rest {
                    let r = self.alloc_register();
                    self.emit(Opcode::NewArray);
                    self.emit_u16(r);
                    self.emit_u16(0);
                    Some(r)
                } else {
                    None
                };
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property {
                            key,
                            value,
                            computed,
                            ..
                        } => {
                            let key_reg = if *computed {
                                match key {
                                    PropertyKey::Computed(expr) => {
                                        self.gen_expression(expr, ctx)?
                                    }
                                    _ => return Err("Expected computed key".to_string()),
                                }
                            } else {
                                match key {
                                    PropertyKey::Identifier(k)
                                    | PropertyKey::Literal(Literal::String(k, _, _)) => {
                                        let idx = self.intern_and_add_const(ctx, k);
                                        let kreg = self.alloc_register();
                                        self.emit(Opcode::LoadConst);
                                        self.emit_u16(kreg);
                                        self.emit_u32(idx);
                                        kreg
                                    }
                                    PropertyKey::Literal(Literal::Number(n)) => {
                                        let kreg = self.alloc_register();
                                        if *n >= i32::MIN as f64
                                            && *n <= i32::MAX as f64
                                            && n.fract() == 0.0
                                        {
                                            self.emit_int_const(kreg, *n as i32);
                                        } else {
                                            let cidx = self.add_constant(JSValue::new_float(*n));
                                            self.emit(Opcode::LoadConst);
                                            self.emit_u16(kreg);
                                            self.emit_u32(cidx);
                                        }
                                        kreg
                                    }
                                    _ => {
                                        return Err(
                                            "SyntaxError: Invalid property key".to_string()
                                        )
                                    }
                                }
                            };
                            let elem_reg = self.alloc_register();
                            self.emit(Opcode::GetProp);
                            self.emit_u16(elem_reg);
                            self.emit_u16(value_reg);
                            self.emit_u16(key_reg);
                            if let Some(kreg) = keys_reg {
                                self.emit(Opcode::ArrayPush);
                                self.emit_u16(kreg);
                                self.emit_u16(key_reg);
                            }
                            self.free_register(key_reg);
                            self.gen_assignment_target_binding(value, elem_reg, kind, ctx)?;
                            self.free_register(elem_reg);
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            let rest_reg = self.alloc_register();
                            self.emit(Opcode::NewObject);
                            self.emit_u16(rest_reg);
                            if let Some(kreg) = keys_reg {
                                self.emit(Opcode::ObjectRest);
                                self.emit_u16(rest_reg);
                                self.emit_u16(value_reg);
                                self.emit_u16(kreg);
                            } else {
                                self.emit(Opcode::ObjectSpread);
                                self.emit_u16(rest_reg);
                                self.emit_u16(value_reg);
                            }
                            self.gen_assignment_target_binding(
                                &rest.argument,
                                rest_reg,
                                kind,
                                ctx,
                            )?;
                            self.free_register(rest_reg);
                        }
                    }
                }
                if let Some(kreg) = keys_reg {
                    self.free_register(kreg);
                }
                Ok(())
            }
            BindingPattern::AssignmentPattern(ap) => {
                let binding_name = match &*ap.left {
                    AssignmentTarget::Identifier(name) => Some(name.as_str()),
                    _ => None,
                };
                self.gen_default_value_binding(&ap.right, value_reg, binding_name, ctx)?;
                self.gen_assignment_target_binding(&ap.left, value_reg, kind, ctx)
            }
        }
    }

    fn gen_assignment_target_binding(
        &mut self,
        target: &AssignmentTarget,
        value_reg: u16,
        kind: VariableKind,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match target {
            AssignmentTarget::Identifier(name) => {
                if self.is_strict && Self::is_strict_reserved_word(name) {
                    return Err(format!(
                        "SyntaxError: '{}' cannot be declared in strict mode code",
                        name
                    ));
                }
                let slot = if kind == VariableKind::Var {
                    self.declare_var(name, kind)
                } else {
                    self.lookup_var_slot(name)
                        .unwrap_or_else(|| self.declare_var(name, kind))
                };
                if slot != value_reg {
                    self.emit(Opcode::Move);
                    self.emit_u16(slot);
                    self.emit_u16(value_reg);
                }
                // Mark as initialized so CheckTdz is not emitted
                for scope in self.scope_stack.iter_mut().rev() {
                    if let Some(entry) = scope.get_mut(name) {
                        entry.initialized = true;
                        break;
                    }
                }
                if kind == VariableKind::Var
                    && self.is_global_scope()
                    && !(self.is_strict && self.is_eval)
                {
                    let atom = ctx.intern(name);
                    let idx = self.add_constant(JSValue::new_string(atom));
                    self.emit(Opcode::SetGlobal);
                    self.emit_u32(idx);
                    self.emit_u16(slot);
                }
                Ok(())
            }
            AssignmentTarget::ArrayPattern(arr) => self.gen_binding_pattern(
                &BindingPattern::ArrayPattern(arr.clone()),
                value_reg,
                kind,
                ctx,
            ),
            AssignmentTarget::ObjectPattern(obj) => self.gen_binding_pattern(
                &BindingPattern::ObjectPattern(obj.clone()),
                value_reg,
                kind,
                ctx,
            ),
            AssignmentTarget::AssignmentPattern(ap) => {
                let binding_name = match &*ap.left {
                    AssignmentTarget::Identifier(name) => Some(name.as_str()),
                    _ => None,
                };
                self.gen_default_value_binding(&ap.right, value_reg, binding_name, ctx)?;
                self.gen_assignment_target_binding(&ap.left, value_reg, kind, ctx)
            }
            _ => Err("Unsupported target in binding pattern".to_string()),
        }
    }

    fn gen_default_value_binding(
        &mut self,
        default_expr: &Expression,
        value_reg: u16,
        binding_name: Option<&str>,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let cmp_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(cmp_reg);

        self.emit(Opcode::StrictNeq);
        self.emit_u16(cmp_reg);
        self.emit_u16(value_reg);
        self.emit_u16(cmp_reg);

        self.emit(Opcode::JumpIf);
        self.emit_u16(cmp_reg);
        let skip_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(cmp_reg);

        let default_reg = if let Some(name) = binding_name {
            self.gen_expression_with_assign_name(default_expr, name, ctx)?
        } else {
            self.gen_expression(default_expr, ctx)?
        };
        if default_reg != value_reg {
            self.emit(Opcode::Move);
            self.emit_u16(value_reg);
            self.emit_u16(default_reg);
        }
        self.free_register(default_reg);

        let after_pos = self.code.len();
        self.patch_jump(skip_patch, after_pos);

        Ok(())
    }

    fn predeclare_binding_pattern(&mut self, pattern: &BindingPattern, kind: VariableKind) {
        match pattern {
            BindingPattern::Identifier(name) => {
                let _ = self.declare_var(name, kind);
            }
            BindingPattern::ArrayPattern(arr) => {
                for elem in &arr.elements {
                    if let Some(elem) = elem {
                        match elem {
                            PatternElement::Pattern(AssignmentTarget::Identifier(name)) => {
                                let _ = self.declare_var(name, kind);
                            }
                            PatternElement::Pattern(AssignmentTarget::ArrayPattern(arr)) => {
                                self.predeclare_binding_pattern(
                                    &BindingPattern::ArrayPattern(arr.clone()),
                                    kind,
                                );
                            }
                            PatternElement::Pattern(AssignmentTarget::ObjectPattern(obj)) => {
                                self.predeclare_binding_pattern(
                                    &BindingPattern::ObjectPattern(obj.clone()),
                                    kind,
                                );
                            }
                            PatternElement::Pattern(AssignmentTarget::AssignmentPattern(ap)) => {
                                self.predeclare_binding_pattern(
                                    &BindingPattern::AssignmentPattern(ap.clone()),
                                    kind,
                                );
                            }
                            PatternElement::RestElement(rest) => {
                                self.predeclare_assignment_target(&rest.argument, kind);
                            }
                            PatternElement::AssignmentPattern(ap) => {
                                self.predeclare_assignment_target(&ap.left, kind);
                            }
                            _ => {}
                        }
                    }
                }
            }
            BindingPattern::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPatternProperty::Property { value, .. } => {
                            self.predeclare_assignment_target(value, kind);
                        }
                        ObjectPatternProperty::RestElement(rest) => {
                            self.predeclare_assignment_target(&rest.argument, kind);
                        }
                    }
                }
            }
            BindingPattern::AssignmentPattern(ap) => {
                self.predeclare_assignment_target(&ap.left, kind);
            }
        }
    }

    fn first_binding_name(&self, target: &AssignmentTarget) -> Option<String> {
        match target {
            AssignmentTarget::Identifier(name) => Some(name.clone()),
            AssignmentTarget::ArrayPattern(arr) => arr.elements.first().and_then(|e| {
                if let Some(pat) = e {
                    match pat {
                        PatternElement::Pattern(p) => self.first_binding_name(p),
                        PatternElement::RestElement(r) => self.first_binding_name(&r.argument),
                        PatternElement::AssignmentPattern(ap) => self.first_binding_name(&ap.left),
                    }
                } else {
                    None
                }
            }),
            AssignmentTarget::ObjectPattern(obj) => {
                obj.properties.first().and_then(|prop| match prop {
                    ObjectPatternProperty::Property { value, .. } => self.first_binding_name(value),
                    ObjectPatternProperty::RestElement(r) => self.first_binding_name(&r.argument),
                })
            }
            AssignmentTarget::AssignmentPattern(ap) => self.first_binding_name(&ap.left),
            _ => None,
        }
    }

    fn predeclare_assignment_target(&mut self, target: &AssignmentTarget, kind: VariableKind) {
        match target {
            AssignmentTarget::Identifier(name) => {
                let _ = self.declare_var(name, kind);
            }
            AssignmentTarget::ArrayPattern(arr) => {
                self.predeclare_binding_pattern(&BindingPattern::ArrayPattern(arr.clone()), kind);
            }
            AssignmentTarget::ObjectPattern(obj) => {
                self.predeclare_binding_pattern(&BindingPattern::ObjectPattern(obj.clone()), kind);
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                self.predeclare_assignment_target(&ap.left, kind);
            }
            AssignmentTarget::RestElement(rest) => {
                self.predeclare_assignment_target(&rest.argument, kind);
            }
            _ => {}
        }
    }

    fn lookup_var_slot_for_pattern(&self, pattern: &BindingPattern) -> Option<u16> {
        match pattern {
            BindingPattern::Identifier(name) => self.lookup_var_slot(name),
            _ => None,
        }
    }

    fn emit_add_disposable_resource(
        &mut self,
        value_reg: u16,
        async_kind: bool,
    ) -> Result<(), String> {
        let Some(Some(stack_reg)) = self.scope_dispose_stack.last().copied() else {
            return Err("using declaration outside of a dispose scope".to_string());
        };
        self.emit_add_disposable_resource_to(value_reg, async_kind, stack_reg);
        Ok(())
    }

    fn emit_add_disposable_resource_to(&mut self, value_reg: u16, async_kind: bool, stack_reg: u16) {
        self.emit(Opcode::AddDisposableResource);
        self.emit_u16(value_reg);
        self.emit_u16(0xFFFF);
        self.emit_u8(if async_kind { 1 } else { 0 });
        self.emit_u16(stack_reg);
    }

    fn gen_variable_declaration(
        &mut self,
        decl: &VariableDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        for d in &decl.declarations {
            if decl.kind == VariableKind::Const && d.init.is_none() {
                return Err("SyntaxError: Missing initializer in const declaration".to_string());
            }

            if let Some(init) = &d.init {
                let reg = self.gen_expression_with_name(init, &d.id, ctx)?;
                if decl.kind.is_using() {
                    self.emit_add_disposable_resource(reg, decl.kind.is_await_using())?;
                }
                self.gen_binding_pattern(&d.id, reg, decl.kind, ctx)?;
                if self.is_global_scope() && !self.is_eval && decl.kind != VariableKind::Var {
                    let mut names = std::collections::HashSet::new();
                    Self::collect_binding_pattern_bindings(&d.id, &mut names);
                    let mut sorted: Vec<String> = names.into_iter().collect();
                    sorted.sort();
                    for name in sorted {
                        if self.is_global_lexical(&name) {
                            let slot = self.lookup_var_slot(&name).unwrap_or(reg);
                            self.emit_global_binding(6, &name, slot, ctx);
                        }
                    }
                }
                self.free_register(reg);
            } else {
                if decl.kind == VariableKind::Var {
                    self.predeclare_binding_pattern(&d.id, VariableKind::Var);
                    if self.is_global_scope() && !(self.is_strict && self.is_eval) {
                        if let BindingPattern::Identifier(name) = &d.id {
                            let atom = ctx.intern(name);
                            let idx = self.add_constant(JSValue::new_string(atom));
                            let undefined = self.emit_load_undefined();
                            self.emit(Opcode::InitGlobalVar);
                            self.emit_u32(idx);
                            self.emit_u16(undefined);
                            self.free_register(undefined);
                        }
                    }
                } else {
                    if let Some(slot) = self.lookup_var_slot_for_pattern(&d.id) {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(slot);
                    }

                    if let BindingPattern::Identifier(name) = &d.id {
                        for scope in self.scope_stack.iter_mut().rev() {
                            if let Some(entry) = scope.get_mut(name) {
                                entry.initialized = true;
                                break;
                            }
                        }
                    }
                    if self.is_global_scope() && !self.is_eval {
                        let mut names = std::collections::HashSet::new();
                        Self::collect_binding_pattern_bindings(&d.id, &mut names);
                        let mut sorted: Vec<String> = names.into_iter().collect();
                        sorted.sort();
                        for name in sorted {
                            if self.is_global_lexical(&name) {
                                if let Some(slot) = self.lookup_var_slot(&name) {
                                    self.emit_global_binding(6, &name, slot, ctx);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn emit_load_undefined(&mut self) -> u16 {
        let r = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(r);
        r
    }

    fn unwrap_completion(&mut self, val: Option<u16>) -> u16 {
        match val {
            Some(r) => r,
            None => self.emit_load_undefined(),
        }
    }

    fn gen_statement_as_expression(
        &mut self,
        stmt: &ASTNode,
        ctx: &mut JSContext,
    ) -> Result<Option<u16>, String> {
        match stmt {
            ASTNode::ExpressionStatement(es) => Ok(Some(self.gen_expression(&es.expression, ctx)?)),
            ASTNode::IfStatement(if_stmt) => {
                Ok(Some(self.gen_if_statement_as_expression(if_stmt, ctx)?))
            }
            ASTNode::BlockStatement(block) => {
                let reg = self.gen_block_statement_as_expression(block, ctx)?;
                if Self::statement_list_produces_value(&block.body) {
                    Ok(Some(reg))
                } else {
                    self.free_register(reg);
                    Ok(None)
                }
            }
            ASTNode::TryStatement(try_stmt) => {
                Ok(Some(self.gen_try_statement_as_expression(try_stmt, ctx)?))
            }
            ASTNode::SwitchStatement(switch_stmt) => Ok(Some(
                self.gen_switch_statement_as_expression(switch_stmt, ctx)?,
            )),
            ASTNode::VariableDeclaration(_)
            | ASTNode::FunctionDeclaration(_)
            | ASTNode::ClassDeclaration(_) => {
                self.gen_statement(stmt, ctx)?;
                Ok(None)
            }
            ASTNode::ForStatement(f_stmt) => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                self.gen_for_statement_with_result(f_stmt, ctx, Some(reg))?;
                Ok(Some(reg))
            }
            ASTNode::ForInStatement(fi_stmt) => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                self.gen_for_in_statement_with_result(fi_stmt, ctx, Some(reg))?;
                Ok(Some(reg))
            }
            ASTNode::ForOfStatement(fo_stmt) => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                self.gen_for_of_statement_with_result(fo_stmt, ctx, Some(reg))?;
                Ok(Some(reg))
            }
            ASTNode::WhileStatement(w_stmt) => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                self.gen_while_statement_with_result(w_stmt, ctx, Some(reg))?;
                Ok(Some(reg))
            }
            ASTNode::DoWhileStatement(dw_stmt) => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                self.gen_do_while_statement_with_result(dw_stmt, ctx, Some(reg))?;
                Ok(Some(reg))
            }
            ASTNode::ReturnStatement(_)
            | ASTNode::ThrowStatement(_)
            | ASTNode::BreakStatement(_)
            | ASTNode::ContinueStatement(_) => {
                self.gen_statement(stmt, ctx)?;
                Ok(None)
            }
            ASTNode::LabelledStatement(labeled) => {
                let mut chain: Vec<String> = vec![labeled.label.clone()];
                let mut terminal: &ASTNode = &labeled.body;
                while let ASTNode::LabelledStatement(current) = terminal {
                    chain.push(current.label.clone());
                    terminal = &current.body;
                }
                let saved_pending = std::mem::take(&mut self.pending_labels);
                if Self::statement_is_iteration_statement(terminal) {
                    self.pending_labels = chain;
                    let result = self.gen_statement_as_expression(terminal, ctx);
                    self.pending_labels = saved_pending;
                    result
                } else {
                    self.breakable_frames.push(BreakableFrame {
                        labels: chain,
                        is_label: true,
                        handler_depth: self.handler_depth,
                        continue_depth: self.handler_depth,
                        break_patches: Vec::new(),
                        continue_patches: Vec::new(),
                        continue_target: None,
                    });
                    let result = self.gen_statement_as_expression(terminal, ctx);
                    let frame = self.breakable_frames.pop().unwrap();
                    let end_pos = self.code.len();
                    for patch in frame.break_patches {
                        self.patch_jump(patch, end_pos);
                    }
                    self.pending_labels = saved_pending;
                    result
                }
            }
            ASTNode::WithStatement(with_stmt) => {
                let result_reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(result_reg);
                self.gen_with_statement_common(with_stmt, ctx, Some(result_reg))?;
                Ok(Some(result_reg))
            }
            ASTNode::EmptyStatement | ASTNode::DebuggerStatement => Ok(None),
            _ => Ok(None),
        }
    }

    fn statement_list_produces_value(body: &[ASTNode]) -> bool {
        body.iter().any(|s| !Self::statement_is_statically_empty(s))
    }

    fn statement_is_statically_empty(stmt: &ASTNode) -> bool {
        match stmt {
            ASTNode::EmptyStatement
            | ASTNode::VariableDeclaration(_)
            | ASTNode::FunctionDeclaration(_)
            | ASTNode::ClassDeclaration(_)
            | ASTNode::DebuggerStatement => true,
            ASTNode::BlockStatement(block) => !Self::statement_list_produces_value(&block.body),
            ASTNode::LabelledStatement(labelled) => Self::statement_is_statically_empty(&labelled.body),
            _ => false,
        }
    }

    fn gen_block_statement_as_expression(
        &mut self,
        block: &BlockStatement,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let result_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(result_reg);
        self.gen_block_with_target(block, ctx, result_reg)?;
        Ok(result_reg)
    }

    fn gen_block_with_target(
        &mut self,
        block: &BlockStatement,
        ctx: &mut JSContext,
        target_reg: u16,
    ) -> Result<(), String> {
        self.push_scope();
        self.pre_scan_let_const(&block.body)?;
        self.emit_tdz_for_block(&block.body)?;
        let disposal = self.begin_scope_disposal(&block.body);

        for (i, s) in block.body.iter().enumerate() {
            if let Some(&line) = block.lines.get(i) {
                self.emit_line(line);
            }
            self.gen_statement_with_target(s, ctx, target_reg)?;
        }
        self.end_scope_disposal(disposal);
        self.pop_scope();
        Ok(())
    }

    fn gen_statement_with_target(
        &mut self,
        stmt: &ASTNode,
        ctx: &mut JSContext,
        target_reg: u16,
    ) -> Result<(), String> {
        match stmt {
            ASTNode::BlockStatement(block) => {
                self.push_scope();
                self.pre_scan_let_const(&block.body)?;
                self.emit_tdz_for_block(&block.body)?;
                let disposal = self.begin_scope_disposal(&block.body);
                for (i, s) in block.body.iter().enumerate() {
                    if let Some(&line) = block.lines.get(i) {
                        self.emit_line(line);
                    }
                    self.gen_statement_with_target(s, ctx, target_reg)?;
                }
                self.end_scope_disposal(disposal);
                self.pop_scope();
            }
            ASTNode::WithStatement(stmt) => {
                self.gen_with_statement_common(stmt, ctx, Some(target_reg))?;
            }
            ASTNode::ExpressionStatement(es) => {
                let reg = self.gen_expression(&es.expression, ctx)?;
                if reg != target_reg {
                    self.emit(Opcode::Move);
                    self.emit_u16(target_reg);
                    self.emit_u16(reg);
                }
                self.free_register(reg);
            }
            ASTNode::IfStatement(if_stmt) => {
                self.gen_if_statement_with_target(if_stmt, ctx, target_reg)?;
            }
            ASTNode::BreakStatement(_) | ASTNode::ContinueStatement(_) => {
                self.gen_statement(stmt, ctx)?;
            }
            ASTNode::ReturnStatement(ret) => {
                if self.is_script_level {
                    return Err("SyntaxError: Illegal return statement".to_string());
                }
                if let Some(ref arg) = ret.argument {
                    let reg = self.gen_expression(arg, ctx)?;
                    if self.in_async_generator {
                        self.emit(Opcode::Await);
                        self.emit_u16(reg);
                    }
                    self.emit(Opcode::Return);
                    self.emit_u16(reg);
                } else {
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(0);
                    self.emit(Opcode::Return);
                    self.emit_u16(0);
                }
            }
            ASTNode::ThrowStatement(throw) => {
                let reg = self.gen_expression(&throw.argument, ctx)?;
                self.emit(Opcode::Throw);
                self.emit_u16(reg);
            }
            ASTNode::WhileStatement(w_stmt) => {
                self.gen_while_statement_with_result(w_stmt, ctx, Some(target_reg))?;
            }
            ASTNode::DoWhileStatement(dw_stmt) => {
                self.gen_do_while_statement_with_result(dw_stmt, ctx, Some(target_reg))?;
            }
            ASTNode::SwitchStatement(sw_stmt) => {
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(target_reg);
                self.gen_switch_statement_with_result(sw_stmt, ctx, Some(target_reg))?;
            }
            ASTNode::ForInStatement(fi_stmt) => {
                self.gen_for_in_statement_with_result(fi_stmt, ctx, Some(target_reg))?;
            }
            ASTNode::ForOfStatement(fo_stmt) => {
                self.gen_for_of_statement_with_result(fo_stmt, ctx, Some(target_reg))?;
            }
            ASTNode::TryStatement(try_stmt) => {
                self.gen_try_statement(try_stmt, ctx, Some(target_reg))?;
            }
            _ => {
                if let Some(reg) = self.gen_statement_as_expression(stmt, ctx)? {
                    if reg != target_reg {
                        self.emit(Opcode::Move);
                        self.emit_u16(target_reg);
                        self.emit_u16(reg);
                        self.free_register(reg);
                    }
                } else if !matches!(
                    stmt,
                    ASTNode::VariableDeclaration(_)
                        | ASTNode::FunctionDeclaration(_)
                        | ASTNode::ClassDeclaration(_)
                ) {
                    self.gen_statement(stmt, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn gen_if_statement_as_expression(
        &mut self,
        stmt: &IfStatement,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        if self.p2_fold_profile.allow_if_bool() {
            if let Expression::Literal(Literal::Boolean(test_bool)) = &stmt.test {
                if *test_bool {
                    return self
                        .gen_statement_as_expression(&stmt.consequent, ctx)
                        .map(|r| self.unwrap_completion(r));
                } else if let Some(alt) = &stmt.alternate {
                    return self
                        .gen_statement_as_expression(alt, ctx)
                        .map(|r| self.unwrap_completion(r));
                } else {
                    return Ok(self.emit_load_undefined());
                }
            }
        }

        let test_reg = self.gen_expression(&stmt.test, ctx)?;

        if self.opt_branch_result_prealloc {
            let result = if self.reserved_slots.contains(&test_reg) {
                let r = self.alloc_register();
                self.emit(Opcode::Move);
                self.emit_u16(r);
                self.emit_u16(test_reg);
                r
            } else {
                test_reg
            };

            self.emit(Opcode::JumpIfNot);
            self.emit_u16(test_reg);
            let else_jump = self.code.len();
            self.emit_i32(0);

            let cons_opt = self.gen_statement_as_expression(&stmt.consequent, ctx)?;
            let cons = self.unwrap_completion(cons_opt);
            if cons != result {
                self.emit(Opcode::Move);
                self.emit_u16(result);
                self.emit_u16(cons);
                self.free_register(cons);
            }

            self.emit(Opcode::Jump);
            let end_jump = self.code.len();
            self.emit_i32(0);

            let else_pos = self.code.len();
            self.patch_jump(else_jump, else_pos);

            let alt = if let Some(alt_stmt) = &stmt.alternate {
                let alt_opt = self.gen_statement_as_expression(alt_stmt, ctx)?;
                self.unwrap_completion(alt_opt)
            } else {
                self.emit_load_undefined()
            };
            if alt != result {
                self.emit(Opcode::Move);
                self.emit_u16(result);
                self.emit_u16(alt);
                self.free_register(alt);
            }

            let end_pos = self.code.len();
            self.patch_jump(end_jump, end_pos);
            return Ok(result);
        }

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(test_reg);
        let else_jump = self.code.len();
        self.emit_i32(0);

        let cons_opt = self.gen_statement_as_expression(&stmt.consequent, ctx)?;
        let cons = self.unwrap_completion(cons_opt);

        let result = if self.reserved_slots.contains(&test_reg) && cons != test_reg {
            let r = self.alloc_register();
            self.emit(Opcode::Move);
            self.emit_u16(r);
            self.emit_u16(cons);
            self.free_register(cons);
            r
        } else if cons != test_reg {
            self.emit(Opcode::Move);
            self.emit_u16(test_reg);
            self.emit_u16(cons);
            self.free_register(cons);
            test_reg
        } else {
            test_reg
        };

        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);

        let else_pos = self.code.len();
        self.patch_jump(else_jump, else_pos);

        let alt = if let Some(alt_stmt) = &stmt.alternate {
            let alt_opt = self.gen_statement_as_expression(alt_stmt, ctx)?;
            self.unwrap_completion(alt_opt)
        } else {
            self.emit_load_undefined()
        };
        if self.reserved_slots.contains(&test_reg) && alt != result && alt != test_reg {
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(alt);
            self.free_register(alt);
        } else if alt != result && alt != test_reg {
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(alt);
            self.free_register(alt);
        }

        let end_pos = self.code.len();
        self.patch_jump(end_jump, end_pos);

        Ok(result)
    }

    fn gen_if_statement_with_target(
        &mut self,
        stmt: &IfStatement,
        ctx: &mut JSContext,
        target_reg: u16,
    ) -> Result<(), String> {
        if self.p2_fold_profile.allow_if_bool() {
            if let Expression::Literal(Literal::Boolean(test_bool)) = &stmt.test {
                if *test_bool {
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(target_reg);
                    self.gen_statement_with_target(&stmt.consequent, ctx, target_reg)?;
                } else if let Some(alt) = &stmt.alternate {
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(target_reg);
                    self.gen_statement_with_target(alt, ctx, target_reg)?;
                }
                return Ok(());
            }
        }
        if self.p2_fold_profile.allow_aggressive_truthy() {
            if let Expression::Literal(lit) = &stmt.test {
                if let Some(truthy) = Self::literal_truthiness(lit) {
                    if truthy {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(target_reg);
                        self.gen_statement_with_target(&stmt.consequent, ctx, target_reg)?;
                    } else if let Some(alt) = &stmt.alternate {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(target_reg);
                        self.gen_statement_with_target(alt, ctx, target_reg)?;
                    }
                    return Ok(());
                }
            }
        }
        let test_reg = self.gen_expression(&stmt.test, ctx)?;
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(test_reg);
        self.free_register(test_reg);
        let else_jump = self.code.len();
        self.emit_i32(0);

        self.emit(Opcode::LoadUndefined);
        self.emit_u16(target_reg);
        self.gen_statement_with_target(&stmt.consequent, ctx, target_reg)?;

        let has_else = stmt.alternate.is_some();
        let end_jump = if has_else {
            self.emit(Opcode::Jump);
            let pos = self.code.len();
            self.emit_i32(0);
            Some(pos)
        } else {
            None
        };

        let else_pos = self.code.len();
        self.patch_jump(else_jump, else_pos);

        if let Some(alt) = &stmt.alternate {
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(target_reg);
            self.gen_statement_with_target(alt, ctx, target_reg)?;
        }

        if let Some(end_jump_pos) = end_jump {
            let end_pos = self.code.len();
            self.patch_jump(end_jump_pos, end_pos);
        }
        Ok(())
    }

    fn gen_try_statement_as_expression(
        &mut self,
        stmt: &TryStatement,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(reg);
        self.gen_try_statement(stmt, ctx, Some(reg))?;
        Ok(reg)
    }

    fn gen_switch_statement_as_expression(
        &mut self,
        stmt: &SwitchStatement,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(reg);
        self.gen_switch_statement_with_result(stmt, ctx, Some(reg))?;
        Ok(reg)
    }

    fn gen_if_statement(&mut self, stmt: &IfStatement, ctx: &mut JSContext) -> Result<(), String> {
        if self.p2_fold_profile.allow_if_bool() {
            if let Expression::Literal(Literal::Boolean(test_bool)) = &stmt.test {
                if *test_bool {
                    self.gen_statement(&stmt.consequent, ctx)?;
                } else if let Some(alt) = &stmt.alternate {
                    self.gen_statement(alt, ctx)?;
                }
                return Ok(());
            }
        }

        if self.p2_fold_profile.allow_aggressive_truthy() {
            if let Expression::Literal(lit) = &stmt.test {
                if let Some(truthy) = Self::literal_truthiness(lit) {
                    if truthy {
                        self.gen_statement(&stmt.consequent, ctx)?;
                    } else if let Some(alt) = &stmt.alternate {
                        self.gen_statement(alt, ctx)?;
                    }
                    return Ok(());
                }
            }
        }

        let test_reg = self.gen_expression(&stmt.test, ctx)?;
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(test_reg);
        self.free_register(test_reg);
        let else_jump = self.code.len();
        self.emit_i32(0);

        self.gen_statement(&stmt.consequent, ctx)?;

        let has_else = stmt.alternate.is_some();
        let end_jump = if has_else {
            self.emit(Opcode::Jump);
            let pos = self.code.len();
            self.emit_i32(0);
            Some(pos)
        } else {
            None
        };

        let else_pos = self.code.len();
        self.patch_jump(else_jump, else_pos);

        if let Some(alt) = &stmt.alternate {
            self.gen_statement(alt, ctx)?;
        }

        if let Some(end_jump_pos) = end_jump {
            let end_pos = self.code.len();
            self.patch_jump(end_jump_pos, end_pos);
        }

        Ok(())
    }

    fn gen_while_statement(
        &mut self,
        stmt: &WhileStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_while_statement_with_result(stmt, ctx, None)
    }

    fn gen_while_statement_with_result(
        &mut self,
        stmt: &WhileStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        let loop_start = self.code.len();

        let break_jump = if self.opt_fused_cmp_jump {
            if let Expression::BinaryExpression(bin) = &stmt.test {
                if let Some(fused) = Self::fused_op_from_binary_cmp(&bin.op, false) {
                    let left = self.gen_expression(&bin.left, ctx)?;
                    let right = self.gen_expression(&bin.right, ctx)?;
                    self.emit(fused);
                    self.emit_u16(left);
                    self.emit_u16(right);
                    self.free_register(left);
                    self.free_register(right);
                    let p = self.code.len();
                    self.emit_i32(0);
                    p
                } else {
                    let test_reg = self.gen_expression(&stmt.test, ctx)?;
                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(test_reg);
                    self.free_register(test_reg);
                    let p = self.code.len();
                    self.emit_i32(0);
                    p
                }
            } else {
                let test_reg = self.gen_expression(&stmt.test, ctx)?;
                self.emit(Opcode::JumpIfNot);
                self.emit_u16(test_reg);
                self.free_register(test_reg);
                let p = self.code.len();
                self.emit_i32(0);
                p
            }
        } else {
            let test_reg = self.gen_expression(&stmt.test, ctx)?;
            self.emit(Opcode::JumpIfNot);
            self.emit_u16(test_reg);
            self.free_register(test_reg);
            let p = self.code.len();
            self.emit_i32(0);
            p
        };

        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: self.handler_depth,
            continue_depth: self.handler_depth,
            break_patches: vec![break_jump],
            continue_patches: Vec::new(),
            continue_target: Some(loop_start),
        });

        if let Some(v_reg) = result_reg {
            self.gen_statement_with_target(&stmt.body, ctx, v_reg)?;
        } else {
            self.gen_statement(&stmt.body, ctx)?;
        }

        let frame = self.breakable_frames.pop().unwrap();

        self.emit_backward_jump(loop_start);

        let end_pos = self.code.len();
        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }
        for patch in frame.continue_patches {
            self.patch_jump(patch, frame.continue_target.unwrap());
        }

        Ok(())
    }

    fn gen_for_statement(
        &mut self,
        stmt: &ForStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_for_statement_with_result(stmt, ctx, None)
    }

    fn gen_for_statement_with_result(
        &mut self,
        stmt: &ForStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        let needs_block_scope = if let Some(ForInit::VariableDeclaration(decl)) = &stmt.init {
            decl.kind != VariableKind::Var
        } else {
            false
        };

        if needs_block_scope {
            self.push_scope();
            if let Some(ForInit::VariableDeclaration(decl)) = &stmt.init {
                for d in &decl.declarations {
                    self.pre_scan_binding(&d.id, decl.kind);
                }
                for d in &decl.declarations {
                    self.emit_tdz_for_binding(&d.id)?;
                }
            }
        }

        let using_init = matches!(
            &stmt.init,
            Some(ForInit::VariableDeclaration(decl)) if decl.kind.is_using()
        );
        let disposal = if using_init {
            let region = self.begin_disposal_region();
            *self.scope_dispose_stack.last_mut().unwrap() = Some(region.stack);
            Some(region)
        } else {
            None
        };

        if let Some(init) = &stmt.init {
            match init {
                ForInit::VariableDeclaration(decl) => {
                    self.gen_variable_declaration(decl, ctx)?;
                }
                ForInit::Expression(expr) => {
                    let r = self.gen_expression(expr, ctx)?;
                    self.free_register(r);
                }
            }
        }

        // Emit ResetPerIterVar for per-iteration bindings (let variables)
        // BEFORE the first test, as per spec CreatePerIterationEnvironment step
        if needs_block_scope {
            if let Some(ForInit::VariableDeclaration(decl)) = &stmt.init {
                for d in &decl.declarations {
                    if let BindingPattern::Identifier(name) = &d.id {
                        if let Some(slot) = self.lookup_var_slot(name) {
                            self.emit(Opcode::ResetPerIterVar);
                            self.emit_u16(slot);
                        }
                    }
                }
            }
        }

        let loop_start = self.code.len();

        let break_jump = if let Some(test) = &stmt.test {
            if self.opt_fused_cmp_jump {
                if let Expression::BinaryExpression(bin) = test {
                    if let Some(fused) = Self::fused_op_from_binary_cmp(&bin.op, false) {
                        let left = self.gen_expression(&bin.left, ctx)?;
                        let right = self.gen_expression(&bin.right, ctx)?;
                        self.emit(fused);
                        self.emit_u16(left);
                        self.emit_u16(right);
                        self.free_register(left);
                        self.free_register(right);
                        let bj = self.code.len();
                        self.emit_i32(0);
                        bj
                    } else {
                        let test_reg = self.gen_expression(test, ctx)?;
                        self.emit(Opcode::JumpIfNot);
                        self.emit_u16(test_reg);
                        self.free_register(test_reg);
                        let bj = self.code.len();
                        self.emit_i32(0);
                        bj
                    }
                } else {
                    let test_reg = self.gen_expression(test, ctx)?;
                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(test_reg);
                    self.free_register(test_reg);
                    let bj = self.code.len();
                    self.emit_i32(0);
                    bj
                }
            } else {
                let test_reg = self.gen_expression(test, ctx)?;
                self.emit(Opcode::JumpIfNot);
                self.emit_u16(test_reg);
                self.free_register(test_reg);
                let bj = self.code.len();
                self.emit_i32(0);
                bj
            }
        } else {
            0
        };

        let loop_start_for_continue = loop_start;
        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: self.handler_depth,
            continue_depth: self.handler_depth,
            break_patches: if stmt.test.is_some() {
                vec![break_jump]
            } else {
                Vec::new()
            },
            continue_patches: Vec::new(),
            continue_target: Some(loop_start_for_continue),
        });

        if let Some(v_reg) = result_reg {
            self.gen_statement_with_target(&stmt.body, ctx, v_reg)?;
        } else {
            self.gen_statement(&stmt.body, ctx)?;
        }

        // Emit ResetPerIterVar for per-iteration bindings (let variables)
        // This must happen BEFORE the update expression per the spec
        if needs_block_scope {
            if let Some(ForInit::VariableDeclaration(decl)) = &stmt.init {
                for d in &decl.declarations {
                    if let BindingPattern::Identifier(name) = &d.id {
                        if let Some(slot) = self.lookup_var_slot(name) {
                            self.emit(Opcode::ResetPerIterVar);
                            self.emit_u16(slot);
                        }
                    }
                }
            }
        }

        let update_pos = self.code.len();
        if let Some(update) = &stmt.update {
            let r = self.gen_expression(update, ctx)?;
            self.free_register(r);
        }
        if let Some(frame) = self.breakable_frames.last_mut() {
            frame.continue_target = Some(update_pos);
        }

        let frame = self.breakable_frames.pop().unwrap();

        self.emit_backward_jump(loop_start);

        let end_pos = self.code.len();
        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }
        for patch in frame.continue_patches {
            self.patch_jump(patch, frame.continue_target.unwrap());
        }

        self.end_scope_disposal(disposal);

        if needs_block_scope {
            self.pop_scope();
        }

        Ok(())
    }

    fn gen_for_in_statement(
        &mut self,
        stmt: &ForInStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_for_in_statement_with_result(stmt, ctx, None)
    }

    fn gen_for_in_statement_with_result(
        &mut self,
        stmt: &ForInStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        let needs_block_scope = match &stmt.left {
            ForInOfLeft::VariableDeclaration(decl) => decl.kind != VariableKind::Var,
            _ => false,
        };

        if needs_block_scope {
            self.push_scope();
            if let ForInOfLeft::VariableDeclaration(decl) = &stmt.left {
                if let Some(first) = decl.declarations.first() {
                    self.pre_scan_binding(&first.id, decl.kind);
                    self.emit_tdz_for_binding(&first.id)?;
                }
            }
        }

        let has_complex_left = match &stmt.left {
            ForInOfLeft::VariableDeclaration(decl) => decl
                .declarations
                .first()
                .map_or(false, |d| !matches!(d.id, BindingPattern::Identifier(_))),
            ForInOfLeft::AssignmentTarget(at) => !matches!(at, AssignmentTarget::Identifier(_)),
        };

        let mut dynamic_target = false;
        let var_slot = if has_complex_left {
            let tmp_slot = self.alloc_register();
            self.free_register(tmp_slot);
            tmp_slot
        } else {
            match &stmt.left {
                ForInOfLeft::VariableDeclaration(decl) => {
                    if let Some(first) = decl.declarations.first() {
                        if let BindingPattern::Identifier(name) = &first.id {
                            if decl.kind == VariableKind::Var {
                                self.declare_var(name, VariableKind::Var)
                            } else {
                                self.lookup_var_slot(name).unwrap_or(u16::MAX)
                            }
                        } else {
                            u16::MAX
                        }
                    } else {
                        u16::MAX
                    }
                }
                ForInOfLeft::AssignmentTarget(AssignmentTarget::Identifier(name)) => {
                    if let Some(slot) = self.lookup_var_slot(name) {
                        slot
                    } else if !self.is_strict {
                        self.declare_var(name, VariableKind::Var)
                    } else {
                        dynamic_target = true;
                        self.alloc_register()
                    }
                }
                _ => 255,
            }
        };

        let obj_reg = self.gen_expression(&stmt.right, ctx)?;

        let keys_reg = self.alloc_register();
        self.emit(Opcode::GetPropertyNames);
        self.emit_u16(keys_reg);
        self.emit_u16(obj_reg);

        let idx_reg = self.alloc_register();
        self.emit_int_const(idx_reg, 0);

        let loop_start = self.code.len();

        let len_key_reg = self.alloc_register();
        let len_atom = ctx.common_atoms.length;
        let len_const = self.add_constant(JSValue::new_string(len_atom));
        self.emit(Opcode::LoadConst);
        self.emit_u16(len_key_reg);
        self.emit_u32(len_const);
        let len_val_reg = self.alloc_register();
        self.emit(Opcode::GetProp);
        self.emit_u16(len_val_reg);
        self.emit_u16(keys_reg);
        self.emit_u16(len_key_reg);
        self.free_register(len_key_reg);

        let cmp_reg = self.alloc_register();
        self.emit(Opcode::Lt);
        self.emit_u16(cmp_reg);
        self.emit_u16(idx_reg);
        self.emit_u16(len_val_reg);
        self.free_register(len_val_reg);

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(cmp_reg);
        let exit_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(cmp_reg);

        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: self.handler_depth,
            continue_depth: self.handler_depth,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            continue_target: Some(0),
        });

        let key_val_reg = self.alloc_register();
        self.emit(Opcode::GetField);
        self.emit_u16(key_val_reg);
        self.emit_u16(keys_reg);
        self.emit_u16(idx_reg);

        let still_present_reg = self.alloc_register();
        self.emit(Opcode::HasProperty);
        self.emit_u16(still_present_reg);
        self.emit_u16(obj_reg);
        self.emit_u16(key_val_reg);
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(still_present_reg);
        let skip_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(still_present_reg);

        if needs_block_scope {
            self.emit_reset_per_iter_bindings();
        }
        if has_complex_left {
            match &stmt.left {
                ForInOfLeft::VariableDeclaration(decl) => {
                    if let Some(first) = decl.declarations.first() {
                        self.gen_binding_pattern(&first.id, key_val_reg, decl.kind, ctx)?;
                    } else {
                        return Err("for-in without variable".to_string());
                    }
                }
                ForInOfLeft::AssignmentTarget(at) => {
                    self.gen_assignment_target_destructure(at, key_val_reg, ctx)?;
                }
            };
        } else {
            let loop_var_name = match &stmt.left {
                ForInOfLeft::VariableDeclaration(decl) => decl.declarations.first().and_then(|d| {
                    if let BindingPattern::Identifier(n) = &d.id {
                        Some(n.clone())
                    } else {
                        None
                    }
                }),
                ForInOfLeft::AssignmentTarget(AssignmentTarget::Identifier(n)) => Some(n.clone()),
                _ => None,
            };
            if var_slot != key_val_reg {
                self.emit(Opcode::Move);
                self.emit_u16(var_slot);
                self.emit_u16(key_val_reg);
            }

            // Mark the variable as initialized so closures in the body don't
            // incorrectly emit CheckTdz
            if let Some(ref name) = loop_var_name {
                for scope in self.scope_stack.iter_mut().rev() {
                    if let Some(entry) = scope.get_mut(name) {
                        entry.initialized = true;
                        break;
                    }
                }
            }

            if let Some(ref name) = loop_var_name {
                if self.is_global_binding(name) || dynamic_target {
                    let atom = ctx.intern(name);
                    let idx = self.add_constant(JSValue::new_string(atom));
                    let src = if var_slot != key_val_reg {
                        var_slot
                    } else {
                        key_val_reg
                    };
                    self.emit(Opcode::SetGlobal);
                    self.emit_u32(idx);
                    self.emit_u16(src);
                }
            }
            if dynamic_target {
                self.free_register(var_slot);
            }
        }
        self.free_register(key_val_reg);

        if let Some(v_reg) = result_reg {
            self.gen_statement_with_target(&stmt.body, ctx, v_reg)?;
        } else {
            self.gen_statement(&stmt.body, ctx)?;
        }

        let continue_target = self.code.len();
        if let Some(frame) = self.breakable_frames.last_mut() {
            frame.continue_target = Some(continue_target);
        }
        self.patch_jump(skip_patch, continue_target);

        self.emit(Opcode::IncLocal);
        self.emit_u16(idx_reg);

        self.emit_backward_jump(loop_start);

        let frame = self.breakable_frames.pop().unwrap();
        let end_pos = self.code.len();

        self.patch_jump(exit_patch, end_pos);

        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }

        for patch in frame.continue_patches {
            self.patch_jump(patch, continue_target);
        }

        self.free_register(keys_reg);
        self.free_register(idx_reg);
        self.free_register(obj_reg);

        if needs_block_scope {
            self.pop_scope();
        }

        Ok(())
    }

    fn gen_for_of_statement(
        &mut self,
        stmt: &ForOfStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_for_of_statement_with_result(stmt, ctx, None)
    }

    fn gen_for_of_statement_with_result(
        &mut self,
        stmt: &ForOfStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        let needs_block_scope = match &stmt.left {
            ForInOfLeft::VariableDeclaration(decl) => decl.kind != VariableKind::Var,
            _ => false,
        };

        if needs_block_scope {
            self.push_scope();
            if let ForInOfLeft::VariableDeclaration(decl) = &stmt.left {
                if let Some(first) = decl.declarations.first() {
                    self.pre_scan_binding(&first.id, decl.kind);
                    self.emit_tdz_for_binding(&first.id)?;
                }
            }
        }

        let iterable_reg = self.gen_expression(&stmt.right, ctx)?;
        let iter_reg = self.alloc_register();
        let await_state_reg = if stmt.is_await {
            Some(self.alloc_register())
        } else {
            None
        };
        let close_state_reg = if stmt.is_await {
            Some(self.alloc_register())
        } else {
            None
        };
        let close_await_reg = if stmt.is_await {
            Some(self.alloc_register())
        } else {
            None
        };
        let close_done_reg = if stmt.is_await {
            Some(self.alloc_register())
        } else {
            None
        };
        if stmt.is_await {
            self.emit(Opcode::GetAsyncIterator);
        } else {
            self.emit(Opcode::GetIterator);
        }
        self.emit_u16(iter_reg);
        self.emit_u16(iterable_reg);
        self.free_register(iterable_reg);

        if let Some(state_reg) = await_state_reg {
            self.emit(Opcode::LoadInt8);
            self.emit_u16(state_reg);
            self.emit_u8(0);
        }
        if let Some(state_reg) = close_state_reg {
            self.emit(Opcode::LoadInt8);
            self.emit_u16(state_reg);
            self.emit_u8(0);
        }

        let loop_start = self.code.len();

        let val_reg = self.alloc_register();
        let done_reg = self.alloc_register();
        if stmt.is_await {
            self.emit(Opcode::IteratorNextAwait);
        } else {
            self.emit(Opcode::IteratorNext);
        }
        self.emit_u16(val_reg);
        self.emit_u16(done_reg);
        self.emit_u16(iter_reg);
        if let Some(state_reg) = await_state_reg {
            self.emit_u16(state_reg);
        }

        self.emit(Opcode::JumpIf);
        self.emit_u16(done_reg);
        let exit_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(done_reg);

        let close_iterators = true;
        let outer_depth = self.handler_depth;
        let mut close_region: Option<(usize, usize)> = None;
        if close_iterators {
            self.emit(Opcode::Try);
            self.handler_depth += 1;
            let catch_pc_pos = self.code.len();
            self.emit_i32(0);
            let finally_pc_pos = self.code.len();
            self.emit_i32(0);
            close_region = Some((catch_pc_pos, finally_pc_pos));
        }

        let using_disposal = match &stmt.left {
            ForInOfLeft::VariableDeclaration(decl)
                if decl
                    .declarations
                    .first()
                    .map_or(false, |_| decl.kind.is_using()) =>
            {
                Some(self.begin_disposal_region())
            }
            _ => None,
        };

        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: outer_depth,
            continue_depth: self.handler_depth,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            continue_target: Some(0),
        });

        if needs_block_scope {
            self.emit_reset_per_iter_bindings();
        }
        match &stmt.left {
            ForInOfLeft::VariableDeclaration(decl) => {
                if let Some(first) = decl.declarations.first() {
                    if decl.kind.is_using() {
                        if let Some(region) = &using_disposal {
                            self.emit_add_disposable_resource_to(
                                val_reg,
                                decl.kind.is_await_using(),
                                region.stack,
                            );
                        }
                    }
                    self.gen_binding_pattern(&first.id, val_reg, decl.kind, ctx)?;
                } else {
                    return Err("for-of without variable".to_string());
                }
            }
            ForInOfLeft::AssignmentTarget(target) => {
                self.gen_assignment_target_destructure(target, val_reg, ctx)?;
            }
        }
        self.free_register(val_reg);

        if let Some(v_reg) = result_reg {
            self.gen_statement_with_target(&stmt.body, ctx, v_reg)?;
        } else {
            self.gen_statement(&stmt.body, ctx)?;
        }

        let continue_target = self.code.len();
        if let Some(frame) = self.breakable_frames.last_mut() {
            frame.continue_target = Some(continue_target);
        }

        if using_disposal.is_some() {
            self.end_scope_disposal(using_disposal);
        }

        if close_iterators {
            self.emit(Opcode::Catch);
            self.handler_depth -= 1;
        }

        self.emit_backward_jump(loop_start);

        let frame = self.breakable_frames.pop().unwrap();

        if let Some((catch_pc_pos, finally_pc_pos)) = close_region {
            let close_start = self.code.len();
            let close_bytes = (close_start as i32).to_le_bytes();
            self.code[catch_pc_pos..catch_pc_pos + 4].copy_from_slice(&close_bytes);
            self.code[finally_pc_pos..finally_pc_pos + 4].copy_from_slice(&close_bytes);
            self.emit(Opcode::Finally);
            if stmt.is_await {
                self.emit(Opcode::AsyncIteratorCloseFinally);
                self.emit_u16(iter_reg);
                self.emit_u16(close_state_reg.unwrap());
                self.emit_u16(close_await_reg.unwrap());
                self.emit_u16(close_done_reg.unwrap());
            } else {
                self.emit(Opcode::IteratorCloseFinally);
                self.emit_u16(iter_reg);
            }
            self.emit(Opcode::EndFinally);
        }

        let end_pos = self.code.len();

        self.patch_jump(exit_patch, end_pos);

        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }
        for patch in frame.continue_patches {
            self.patch_jump(patch, continue_target);
        }

        self.free_register(iter_reg);
        if let Some(state_reg) = await_state_reg {
            self.free_register(state_reg);
        }
        if let Some(state_reg) = close_state_reg {
            self.free_register(state_reg);
        }
        if let Some(reg) = close_await_reg {
            self.free_register(reg);
        }
        if let Some(reg) = close_done_reg {
            self.free_register(reg);
        }

        if needs_block_scope {
            self.pop_scope();
        }

        Ok(())
    }

    fn gen_do_while_statement(
        &mut self,
        stmt: &DoWhileStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_do_while_statement_with_result(stmt, ctx, None)
    }

    fn gen_do_while_statement_with_result(
        &mut self,
        stmt: &DoWhileStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        let loop_start = self.code.len();

        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: self.handler_depth,
            continue_depth: self.handler_depth,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            continue_target: Some(0),
        });

        if let Some(v_reg) = result_reg {
            self.gen_statement_with_target(&stmt.body, ctx, v_reg)?;
        } else {
            self.gen_statement(&stmt.body, ctx)?;
        }

        let continue_target = self.code.len();
        if let Some(frame) = self.breakable_frames.last_mut() {
            frame.continue_target = Some(continue_target);
        }

        let test_reg = self.gen_expression(&stmt.test, ctx)?;
        self.emit_conditional_backward_jump(test_reg, loop_start, true);
        self.free_register(test_reg);

        let frame = self.breakable_frames.pop().unwrap();

        let end_pos = self.code.len();
        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }
        for patch in frame.continue_patches {
            self.patch_jump(patch, frame.continue_target.unwrap());
        }

        Ok(())
    }

    fn gen_switch_statement(
        &mut self,
        stmt: &SwitchStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.gen_switch_statement_with_result(stmt, ctx, None)
    }

    fn gen_switch_statement_with_result(
        &mut self,
        stmt: &SwitchStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        // Evaluate the switch expression in the enclosing scope (BEFORE the
        // CaseBlock lexical environment is created).
        let disc = self.gen_expression(&stmt.discriminant, ctx)?;

        // Create the lexical scope for the entire CaseBlock. Per spec, this
        // is done BEFORE case selector evaluation so that `let`/`const`
        // declarations inside cases are visible (as TDZ) during selector
        // evaluation.
        self.push_scope();
        for case in &stmt.cases {
            self.pre_scan_let_const(&case.consequent)?;
            self.emit_tdz_for_block(&case.consequent)?;
        }

        let mut test_jumps: Vec<(usize, usize)> = Vec::new();
        for (case_idx, case) in stmt.cases.iter().enumerate() {
            if let Some(test_expr) = &case.test {
                let test_reg = self.gen_expression(test_expr, ctx)?;
                let eq_reg = self.alloc_register();
                self.emit(Opcode::StrictEq);
                self.emit_u16(eq_reg);
                self.emit_u16(disc);
                self.emit_u16(test_reg);
                self.free_register(test_reg);
                self.emit(Opcode::JumpIf);
                self.emit_u16(eq_reg);
                test_jumps.push((self.code.len(), case_idx));
                self.emit_i32(0);
                self.free_register(eq_reg);
            }
        }

        self.emit(Opcode::Jump);
        let default_jump_placeholder = self.code.len();
        self.emit_i32(0);

        self.breakable_frames.push(BreakableFrame {
            labels: std::mem::take(&mut self.pending_labels),
            is_label: false,
            handler_depth: self.handler_depth,
            continue_depth: self.handler_depth,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            continue_target: None,
        });

        let mut body_starts = vec![0usize; stmt.cases.len()];
        for (case_idx, case) in stmt.cases.iter().enumerate() {
            body_starts[case_idx] = self.code.len();
            if let Some(v_reg) = result_reg {
                for s in &case.consequent {
                    self.gen_statement_with_target(s, ctx, v_reg)?;
                }
            } else {
                for s in &case.consequent {
                    self.gen_statement(s, ctx)?;
                }
            }
        }
        self.pop_scope();

        let frame = self.breakable_frames.pop().unwrap();
        let end_pos = self.code.len();

        for patch in frame.break_patches {
            self.patch_jump(patch, end_pos);
        }

        let default_case_idx = stmt.cases.iter().position(|c| c.test.is_none());
        let default_target = default_case_idx
            .map(|idx| body_starts[idx])
            .unwrap_or(end_pos);
        self.patch_jump(default_jump_placeholder, default_target);

        for (placeholder, case_idx) in test_jumps {
            self.patch_jump(placeholder, body_starts[case_idx]);
        }

        self.free_register(disc);
        Ok(())
    }

    fn gen_throw_statement(
        &mut self,
        stmt: &ThrowStatement,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let reg = self.gen_expression(&stmt.argument, ctx)?;
        self.emit(Opcode::Throw);
        self.emit_u16(reg);
        self.free_register(reg);
        Ok(())
    }

    fn gen_try_statement(
        &mut self,
        stmt: &TryStatement,
        ctx: &mut JSContext,
        result_reg: Option<u16>,
    ) -> Result<(), String> {
        self.emit(Opcode::Try);
        self.handler_depth += 1;
        let catch_pc_pos = self.code.len();
        self.emit_i32(0);
        let finally_pc_pos = self.code.len();
        self.emit_i32(0);

        self.push_scope();
        self.pre_scan_let_const(&stmt.block.body)?;
        self.emit_tdz_for_block(&stmt.block.body)?;
        let block_disposal = self.begin_scope_disposal(&stmt.block.body);
        if let Some(v_reg) = result_reg {
            for s in &stmt.block.body {
                self.gen_statement_with_target(s, ctx, v_reg)?;
            }
        } else {
            for s in &stmt.block.body {
                self.gen_statement(s, ctx)?;
            }
        }
        self.end_scope_disposal(block_disposal);
        self.pop_scope();

        self.emit(Opcode::Catch);
        self.handler_depth -= 1;

        self.emit(Opcode::Jump);
        let end_jump_pos = self.code.len();
        self.emit_i32(0);

        let has_catch = stmt.handler.is_some();
        let mut after_catch_jump_pos: Option<usize> = None;
        let has_finally = stmt.finalizer.is_some();
        let mut catch_inner_finally_placeholder: Option<usize> = None;

        if let Some(handler) = &stmt.handler {
            let catch_start = self.code.len();

            let catch_bytes = (catch_start as i32).to_le_bytes();
            self.code[catch_pc_pos..catch_pc_pos + 4].copy_from_slice(&catch_bytes);

            // Wrap catch body in Try-Finally so exceptions inside catch run finally
            if has_finally {
                self.emit(Opcode::Try);
                self.handler_depth += 1;
                let _catch_inner_catch_pc = self.code.len();
                self.emit_i32(0); // placeholder for catch_pc (set to finally_start)
                catch_inner_finally_placeholder = Some(self.code.len());
                self.emit_i32(0); // placeholder for finally_pc (set to finally_start)
            }

            self.push_scope();
            if let Some(param) = &handler.param {
                if self.is_strict {
                    if let BindingPattern::Identifier(name) = param {
                        if name == "eval" || name == "arguments" {
                            return Err(format!(
                                "SyntaxError: Catch variable may not be '{}' in strict mode",
                                name
                            ));
                        }
                    }
                }
                match param {
                    BindingPattern::Identifier(name) => {
                        self.pre_scan_binding(param, VariableKind::Let);
                        self.emit_tdz_for_binding(param)?;
                        let slot = self.declare_var(name, VariableKind::Let);
                        self.emit(Opcode::LoadCaughtException);
                        self.emit_u16(slot);

                        if self.is_global_var(name) {
                            let atom = ctx.intern(name);
                            let idx = self.add_constant(JSValue::new_string(atom));
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(idx);
                            self.emit_u16(slot);
                        }
                    }
                    _ => {
                        self.pre_scan_binding(param, VariableKind::Let);
                        self.emit_tdz_for_binding(param)?;
                        let exc_reg = self.alloc_register();
                        self.emit(Opcode::LoadCaughtException);
                        self.emit_u16(exc_reg);
                        self.gen_binding_pattern(param, exc_reg, VariableKind::Let, ctx)?;
                        self.free_register(exc_reg);
                    }
                }
            }
            self.pre_scan_let_const(&handler.body.body)?;
            self.emit_tdz_for_block(&handler.body.body)?;
            let handler_disposal = self.begin_scope_disposal(&handler.body.body);
            if let Some(v_reg) = result_reg {
                for s in &handler.body.body {
                    self.gen_statement_with_target(s, ctx, v_reg)?;
                }
            } else {
                for s in &handler.body.body {
                    self.gen_statement(s, ctx)?;
                }
            }
            self.end_scope_disposal(handler_disposal);
            self.pop_scope();

            if has_finally {
                self.emit(Opcode::Catch);
                self.handler_depth -= 1;
            }

            self.emit(Opcode::Jump);
            after_catch_jump_pos = Some(self.code.len());
            self.emit_i32(0);
        }

        let finally_start = if let Some(finalizer) = &stmt.finalizer {
            let fs = self.code.len();

            // Patch the catch-inner handler's placeholders
            if let Some(fp_placeholder) = catch_inner_finally_placeholder {
                let finally_bytes = (fs as i32).to_le_bytes();
                self.code[fp_placeholder - 4..fp_placeholder].copy_from_slice(&finally_bytes);
                self.code[fp_placeholder..fp_placeholder + 4].copy_from_slice(&finally_bytes);
            }

            let finally_bytes = (fs as i32).to_le_bytes();
            self.code[finally_pc_pos..finally_pc_pos + 4].copy_from_slice(&finally_bytes);

            if !has_catch {
                self.code[catch_pc_pos..catch_pc_pos + 4].copy_from_slice(&finally_bytes);
            }

            self.patch_jump(end_jump_pos, fs);

            self.emit(Opcode::Finally);

            // Save the prior completion value before running finally body.
            // Per spec: if finally completes normally, the prior completion
            // (from try or catch) wins — not the finally body's value.
            let saved_prior = result_reg.map(|_| self.alloc_register());
            if let Some(sr) = saved_prior {
                self.emit(Opcode::Move);
                self.emit_u16(sr);
                self.emit_u16(result_reg.unwrap());
            }

            // Set result to undefined before the finally body so that
            // abrupt completions (break/continue/return/throw) inside
            // the finally result in an empty completion value.
            if let Some(v_reg) = result_reg {
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(v_reg);
            }

            self.push_scope();
            self.pre_scan_let_const(&finalizer.body)?;
            self.emit_tdz_for_block(&finalizer.body)?;
            let finalizer_disposal = self.begin_scope_disposal(&finalizer.body);
            if let Some(v_reg) = result_reg {
                for s in &finalizer.body {
                    self.gen_statement_with_target(s, ctx, v_reg)?;
                }
            } else {
                for s in &finalizer.body {
                    self.gen_statement(s, ctx)?;
                }
            }
            self.end_scope_disposal(finalizer_disposal);
            self.pop_scope();
            self.emit(Opcode::EndFinally);

            // Restore the prior completion — any non-abrupt writes to v_reg
            // by the finally body are discarded per spec.
            if let Some(sr) = saved_prior {
                self.emit(Opcode::Move);
                self.emit_u16(result_reg.unwrap());
                self.emit_u16(sr);
                self.free_register(sr);
            }

            fs
        } else {
            0
        };

        let end_pos = self.code.len();

        if !has_finally {
            self.patch_jump(end_jump_pos, end_pos);
        }

        if let Some(pos) = after_catch_jump_pos {
            let catch_jump_target = if has_finally { finally_start } else { end_pos };
            self.patch_jump(pos, catch_jump_target);
        }

        Ok(())
    }

    fn gen_function_like_with_name(
        &mut self,
        expr: &Expression,
        name: &str,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        match expr {
            Expression::FunctionExpression(func) => self.gen_function_expression(
                &func.params,
                &func.body,
                Some(name),
                ctx,
                func.generator,
                func.is_async,
                false,
                false,
                false,
            ),
            Expression::ArrowFunction(arrow) => {
                let body = match &arrow.body {
                    ArrowBody::Expression(expr) => BlockStatement {
                        body: vec![ASTNode::ReturnStatement(ReturnStatement {
                            argument: Some((**expr).clone()),
                        })],
                        lines: vec![],
                    },
                    ArrowBody::Block(block) => block.clone(),
                };
                self.gen_function_expression(
                    &arrow.params,
                    &body,
                    Some(name),
                    ctx,
                    false,
                    arrow.is_async,
                    true,
                    false,
                    false,
                )
            }
            Expression::ClassExpression(ce) => {
                let class_slot = self.alloc_register();
                self.gen_class_body(class_slot, name, &ce.super_class, &ce.body, ctx)?;
                Ok(class_slot)
            }
            _ => unreachable!(),
        }
    }

    fn gen_expression_with_name(
        &mut self,
        expr: &Expression,
        binding: &BindingPattern,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let needs_name = match binding {
            BindingPattern::Identifier(binding_name) => match expr {
                Expression::FunctionExpression(func) if func.name.is_none() => {
                    Some(binding_name.as_str())
                }
                Expression::ArrowFunction(_) => Some(binding_name.as_str()),
                Expression::ClassExpression(ce) if ce.name.is_none() => Some(binding_name.as_str()),
                _ => None,
            },
            _ => None,
        };
        if let Some(name) = needs_name {
            self.gen_function_like_with_name(expr, name, ctx)
        } else {
            self.gen_expression(expr, ctx)
        }
    }

    fn gen_expression_with_assign_name(
        &mut self,
        expr: &Expression,
        target_name: &str,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let needs_name = match expr {
            Expression::FunctionExpression(func) if func.name.is_none() => Some(target_name),
            Expression::ArrowFunction(_) => Some(target_name),
            Expression::ClassExpression(ce) if ce.name.is_none() => Some(target_name),
            _ => None,
        };
        if let Some(name) = needs_name {
            self.gen_function_like_with_name(expr, name, ctx)
        } else {
            self.gen_expression(expr, ctx)
        }
    }

    fn gen_yield_star(
        &mut self,
        yield_expr: &YieldExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        if self.in_async_generator {
            return self.gen_yield_star_async(yield_expr, ctx);
        }

        let argument = match &yield_expr.argument {
            Some(arg) => self.gen_expression(arg, ctx)?,
            None => self.emit_load_undefined(),
        };

        let iter_reg = self.alloc_register();
        self.emit(Opcode::GetIterator);
        self.emit_u16(iter_reg);
        self.emit_u16(argument);
        self.free_register(argument);

        let value_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(value_reg);

        let done_reg = self.alloc_register();
        self.emit(Opcode::LoadFalse);
        self.emit_u16(done_reg);

        let returning_reg = self.alloc_register();
        self.emit(Opcode::LoadFalse);
        self.emit_u16(returning_reg);

        let loop_start = self.code.len();
        self.emit(Opcode::YieldStarStep);
        self.emit_u16(iter_reg);
        self.emit_u16(value_reg);
        self.emit_u16(done_reg);
        self.emit_u16(returning_reg);

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(done_reg);
        let yield_patch = self.code.len();
        self.emit_i32(0);

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(returning_reg);
        let exit_patch = self.code.len();
        self.emit_i32(0);

        self.emit(Opcode::Return);
        self.emit_u16(value_reg);

        let yield_pos = self.code.len();
        self.patch_jump(yield_patch, yield_pos);

        self.emit(Opcode::YieldResult);
        self.emit_u16(value_reg);

        self.emit_backward_jump(loop_start);

        let exit_pos = self.code.len();
        self.patch_jump(exit_patch, exit_pos);

        self.free_register(iter_reg);
        self.free_register(done_reg);
        self.free_register(returning_reg);

        Ok(value_reg)
    }

    fn gen_yield_star_async(
        &mut self,
        yield_expr: &YieldExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let iterable_reg = match &yield_expr.argument {
            Some(arg) => self.gen_expression(arg, ctx)?,
            None => {
                let r = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(r);
                r
            }
        };

        let iter_reg = self.alloc_register();
        self.emit(Opcode::GetAsyncIterator);
        self.emit_u16(iter_reg);
        self.emit_u16(iterable_reg);
        self.free_register(iterable_reg);

        let value_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(value_reg);

        let done_reg = self.alloc_register();
        self.emit(Opcode::LoadFalse);
        self.emit_u16(done_reg);

        let returning_reg = self.alloc_register();
        self.emit(Opcode::LoadFalse);
        self.emit_u16(returning_reg);

        let state_reg = self.alloc_register();
        self.emit(Opcode::LoadInt);
        self.emit_u16(state_reg);
        self.emit_i32(0);

        let await_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(await_reg);

        let loop_start = self.code.len();
        self.emit(Opcode::AsyncYieldStarStep);
        self.emit_u16(iter_reg);
        self.emit_u16(value_reg);
        self.emit_u16(done_reg);
        self.emit_u16(returning_reg);
        self.emit_u16(state_reg);
        self.emit_u16(await_reg);

        self.emit(Opcode::JumpIf);
        self.emit_u16(done_reg);
        let exit_patch = self.code.len();
        self.emit_i32(0);

        self.emit(Opcode::YieldResult);
        self.emit_u16(value_reg);

        self.emit_backward_jump(loop_start);

        let exit_pos = self.code.len();
        self.patch_jump(exit_patch, exit_pos);

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(returning_reg);
        let end_patch = self.code.len();
        self.emit_i32(0);

        self.emit(Opcode::Return);
        self.emit_u16(value_reg);

        let end_pos = self.code.len();
        self.patch_jump(end_patch, end_pos);

        self.free_register(iter_reg);
        self.free_register(done_reg);
        self.free_register(returning_reg);
        self.free_register(state_reg);
        self.free_register(await_reg);

        Ok(value_reg)
    }

    fn with_local_tdz(&self, name: &str) -> bool {
        if let Some(entry) = self.lookup_var(name) {
            return entry.kind != VariableKind::Var && !entry.initialized;
        }
        if let Some(parent) = self.parent_vars.get(name) {
            return !parent.is_inherited
                && parent.kind != VariableKind::Var
                && !parent.initialized;
        }
        false
    }

    fn emit_with_resolve(&mut self, name: &str, ctx: &mut JSContext) -> u16 {
        let atom = ctx.intern(name);
        let dst = self.alloc_register();
        self.emit(Opcode::WithResolve);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        dst
    }

    fn emit_jump_if_nullish(&mut self, reg: u16) -> usize {
        self.emit(Opcode::JumpIfNullish);
        self.emit_u16(reg);
        let patch = self.code.len();
        self.emit_i32(0);
        patch
    }

    fn emit_with_get_resolved(
        &mut self,
        dst: u16,
        object: u16,
        name: &str,
        ctx: &mut JSContext,
    ) {
        let atom = ctx.intern(name);
        self.emit(Opcode::WithGetResolved);
        self.emit_u16(dst);
        self.emit_u16(object);
        self.emit_u32(atom.0);
    }

    fn emit_with_set_resolved(
        &mut self,
        object: u16,
        value: u16,
        name: &str,
        ctx: &mut JSContext,
    ) {
        let atom = ctx.intern(name);
        self.emit(Opcode::WithSetResolved);
        self.emit_u16(object);
        self.emit_u16(value);
        self.emit_u32(atom.0);
    }

    fn emit_with_get_var(
        &mut self,
        name: &str,
        kind: u8,
        index: u16,
        flags: u8,
        ctx: &mut JSContext,
    ) -> u16 {
        let atom = ctx.intern(name);
        let dst = self.alloc_register();
        self.emit(Opcode::WithGetVar);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        self.emit_u8(kind);
        self.emit_u16(index);
        self.emit_u8(flags);
        dst
    }

    fn gen_with_global_read(&mut self, name: &str, ctx: &mut JSContext) -> Result<u16, String> {
        let atom = ctx.intern(name);
        let idx = self.add_constant(JSValue::new_string(atom));
        let dst = self.alloc_register();
        self.emit(Opcode::WithHas);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(dst);
        let has_jump = self.code.len();
        self.emit_i32(0);
        self.emit(Opcode::WithGet);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);
        let fallback = self.code.len();
        self.patch_jump(has_jump, fallback);
        self.emit(Opcode::GetGlobal);
        self.emit_u16(dst);
        self.emit_u32(idx);
        if !self.suppress_ref_error {
            self.emit(Opcode::CheckRef);
            self.emit_u16(dst);
            self.emit_u32(idx);
        }
        let end = self.code.len();
        self.patch_jump(end_jump, end);
        Ok(dst)
    }

    fn gen_with_read(&mut self, name: &str, ctx: &mut JSContext) -> Result<u16, String> {
        match self.resolve_var(name, ctx) {
            Some(VarLocation::Local(slot)) => {
                if self.is_global_binding(name) {
                    let flags = if self.suppress_ref_error { 0 } else { 2 };
                    Ok(self.emit_with_get_var(name, 0, 0, flags, ctx))
                } else {
                    let tdz = self.with_local_tdz(name);
                    Ok(self.emit_with_get_var(name, 1, slot, if tdz { 1 } else { 0 }, ctx))
                }
            }
            Some(VarLocation::Upvalue(slot)) => {
                let tdz = self.with_local_tdz(name);
                Ok(self.emit_with_get_var(name, 2, slot, if tdz { 1 } else { 0 }, ctx))
            }
            None => {
                let flags = if self.suppress_ref_error { 0 } else { 2 };
                Ok(self.emit_with_get_var(name, 0, 0, flags, ctx))
            }
        }
    }

    fn gen_with_global_set(
        &mut self,
        name: &str,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let atom = ctx.intern(name);
        let idx = self.add_constant(JSValue::new_string(atom));
        let cond = self.alloc_register();
        self.emit(Opcode::WithHas);
        self.emit_u16(cond);
        self.emit_u32(atom.0);
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(cond);
        let has_jump = self.code.len();
        self.emit_i32(0);
        self.emit(Opcode::WithSet);
        self.emit_u32(atom.0);
        self.emit_u16(value_reg);
        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);
        let fallback = self.code.len();
        self.patch_jump(has_jump, fallback);
        if self.is_eval {
            self.emit(Opcode::SetGlobalVar);
        } else {
            self.emit(Opcode::SetGlobal);
        }
        self.emit_u32(idx);
        self.emit_u16(value_reg);
        let end = self.code.len();
        self.patch_jump(end_jump, end);
        self.free_register(cond);
        Ok(())
    }

    fn gen_with_store(
        &mut self,
        name: &str,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match self.resolve_var(name, ctx) {
            Some(VarLocation::Local(slot)) => {
                if self.is_global_binding(name) {
                    let atom = ctx.intern(name);
                    self.emit(Opcode::WithSetVar);
                    self.emit_u32(atom.0);
                    self.emit_u16(value_reg);
                    self.emit_u8(0);
                    self.emit_u16(0);
                    self.emit_u8(0);
                    Ok(())
                } else {
                    let atom = ctx.intern(name);
                    self.emit(Opcode::WithSetVar);
                    self.emit_u32(atom.0);
                    self.emit_u16(value_reg);
                    self.emit_u8(1);
                    self.emit_u16(slot);
                    self.emit_u8(0);
                    Ok(())
                }
            }
            Some(VarLocation::Upvalue(slot)) => {
                let atom = ctx.intern(name);
                self.emit(Opcode::WithSetVar);
                self.emit_u32(atom.0);
                self.emit_u16(value_reg);
                self.emit_u8(2);
                self.emit_u16(slot);
                self.emit_u8(0);
                Ok(())
            }
            None => {
                let atom = ctx.intern(name);
                self.emit(Opcode::WithSetVar);
                self.emit_u32(atom.0);
                self.emit_u16(value_reg);
                self.emit_u8(0);
                self.emit_u16(0);
                self.emit_u8(0);
                Ok(())
            }
        }
    }

    fn gen_with_delete(&mut self, name: &str, ctx: &mut JSContext) -> Result<u16, String> {
        let atom = ctx.intern(name);
        let dst = self.alloc_register();
        self.emit(Opcode::WithHas);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        self.emit(Opcode::JumpIfNot);
        self.emit_u16(dst);
        let has_jump = self.code.len();
        self.emit_i32(0);
        self.emit(Opcode::WithDelete);
        self.emit_u16(dst);
        self.emit_u32(atom.0);
        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);
        let fallback = self.code.len();
        self.patch_jump(has_jump, fallback);
        let resolved = self.resolve_var(name, ctx);
        match resolved {
            Some(VarLocation::Local(slot)) => {
                if self.is_global_var(name) {
                    let idx = self.add_constant(JSValue::new_string(atom));
                    self.emit(Opcode::DeleteGlobal);
                    self.emit_u16(dst);
                    self.emit_u32(idx);
                } else {
                    let _ = slot;
                    self.emit(Opcode::LoadFalse);
                    self.emit_u16(dst);
                }
            }
            Some(VarLocation::Upvalue(_)) => {
                self.emit(Opcode::LoadFalse);
                self.emit_u16(dst);
            }
            None => {
                let idx = self.add_constant(JSValue::new_string(atom));
                self.emit(Opcode::DeleteGlobal);
                self.emit_u16(dst);
                self.emit_u32(idx);
            }
        }
        let end = self.code.len();
        self.patch_jump(end_jump, end);
        Ok(dst)
    }

    fn gen_expression(&mut self, expr: &Expression, ctx: &mut JSContext) -> Result<u16, String> {
        match expr {
            Expression::Identifier(id) => {
                if id.name == "arguments"
                    && !self.is_script_level
                    && !self.is_arrow
                    && self.lookup_var("arguments").is_none()
                {
                    self.function_uses_arguments = true;
                    let r = self.alloc_register();
                    self.emit(Opcode::GetArguments);
                    self.emit_u16(r);
                    return Ok(r);
                }
                if !self.with_scopes.is_empty() {
                    let applies = match self.lookup_var_scope_index(&id.name) {
                        Some(index) => self.with_applies_to_local(&id.name, index),
                        None => self.with_applies_to_outer(&id.name),
                    };
                    if applies {
                        return self.gen_with_read(&id.name, ctx);
                    }
                }
                match self.resolve_var(&id.name, ctx) {
                    Some(VarLocation::Local(slot)) => {
                        if self.is_global_binding(&id.name) {
                            let atom = ctx.intern(&id.name);
                            let idx = self.add_constant(JSValue::new_string(atom));
                            let r = self.alloc_register();
                            self.emit(Opcode::GetGlobal);
                            self.emit_u16(r);
                            self.emit_u32(idx);
                            return Ok(r);
                        }

                        if let Some(entry) = self.lookup_var(&id.name) {
                            if entry.kind != VariableKind::Var && !entry.initialized {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(slot);
                            }
                        }
                        Ok(slot)
                    }
                    Some(VarLocation::Upvalue(slot)) => {
                        let r = self.alloc_register();
                        self.emit(Opcode::GetUpvalue);
                        self.emit_u16(r);
                        self.emit_u16(slot);

                        // Check TDZ for upvalues: check parent_vars since the variable
                        // is not in the nested function's own scope
                        let mut needs_tdz = false;
                        if let Some(entry) = self.lookup_var(&id.name) {
                            needs_tdz = entry.kind != VariableKind::Var && !entry.initialized;
                        } else if let Some(parent_var) = self.parent_vars.get(&id.name) {
                            // Only check TDZ for direct scope variables (not inherited upvalues).
                            // Inherited upvalues may come from pre-declared let bindings
                            // that are initialized after the closure is created.
                            if !parent_var.is_inherited {
                                // Check if this variable is in an explicitly pushed scope
                                // (like for-in/for-of heads) vs the root function scope.
                                // Variables in the root scope that are pre-declared by
                                // pre_scan_let_const should not trigger TDZ for closures
                                // created before the let statement.
                                needs_tdz =
                                    parent_var.kind != VariableKind::Var && !parent_var.initialized;
                            }
                        }
                        if needs_tdz {
                            self.emit(Opcode::CheckTdz);
                            self.emit_u16(r);
                        }
                        Ok(r)
                    }
                    None => {
                        let atom = ctx.intern(&id.name);
                        let idx = self.add_constant(JSValue::new_string(atom));
                        let r = self.alloc_register();
                        self.emit(Opcode::GetGlobal);
                        self.emit_u16(r);
                        self.emit_u32(idx);
                        // Emit CheckRef for unresolvable references so that reading an
                        // undeclared identifier throws a ReferenceError.  The only
                        // exception is the `typeof` operator, which never throws.
                        if !self.suppress_ref_error {
                            self.emit(Opcode::CheckRef);
                            self.emit_u16(r);
                            self.emit_u32(idx);
                        }
                        Ok(r)
                    }
                }
            }
            Expression::Literal(lit) => {
                let r = self.alloc_register();
                match lit {
                    Literal::Number(n) => {
                        if *n >= i32::MIN as f64 && *n <= i32::MAX as f64 && n.fract() == 0.0 {
                            let val = *n as i32;
                            if val >= i8::MIN as i32 && val <= i8::MAX as i32 {
                                self.emit(Opcode::LoadInt8);
                                self.emit_u16(r);
                                self.emit_u8(val as u8);
                            } else {
                                self.emit_int_const(r, val);
                            }
                        } else {
                            let idx = self.add_constant(JSValue::new_float(*n));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(r);
                            self.emit_u32(idx);
                        }
                    }
                    Literal::LegacyOctal(n) => {
                        if self.is_strict {
                            return Err(
                                "SyntaxError: Octal literals are not allowed in strict mode"
                                    .to_string(),
                            );
                        }
                        if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                            let val = *n as i32;
                            if val >= i8::MIN as i32 && val <= i8::MAX as i32 {
                                self.emit(Opcode::LoadInt8);
                                self.emit_u16(r);
                                self.emit_u8(val as u8);
                            } else {
                                self.emit_int_const(r, val);
                            }
                        } else {
                            let idx = self.add_constant(JSValue::new_float(*n as f64));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(r);
                            self.emit_u32(idx);
                        }
                    }
                    Literal::String(s, _, _) => {
                        let idx = self.intern_and_add_const(ctx, s);
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(r);
                        self.emit_u32(idx);
                    }
                    Literal::Boolean(b) => {
                        if *b {
                            self.emit(Opcode::LoadTrue);
                        } else {
                            self.emit(Opcode::LoadFalse);
                        }
                        self.emit_u16(r);
                    }
                    Literal::Null => {
                        self.emit(Opcode::LoadNull);
                        self.emit_u16(r);
                    }
                    Literal::Undefined => {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(r);
                    }
                    Literal::BigInt(s) => {
                        let atom = ctx.atom_table_mut().intern(s);
                        let idx = self.add_constant(JSValue::new_string(atom));
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(r);
                        self.emit_u32(idx);

                        let bigint_atom = ctx.common_atoms.bigint;
                        let bigint_idx = self.add_constant(JSValue::new_string(bigint_atom));
                        let func_reg = self.alloc_register();
                        self.emit(Opcode::GetGlobal);
                        self.emit_u16(func_reg);
                        self.emit_u32(bigint_idx);
                        self.emit(Opcode::Call);
                        self.emit_u16(r);
                        self.emit_u16(func_reg);
                        self.emit_u16(1);
                        self.emit_u16(r);
                        self.free_register(func_reg);
                    }
                }
                Ok(r)
            }
            Expression::BinaryExpression(bin) => self.gen_binary_expression(bin, ctx),
            Expression::LogicalExpression(logical) => self.gen_logical_expression(logical, ctx),
            Expression::UnaryExpression(unary) => self.gen_unary_expression(unary, ctx),
            Expression::UpdateExpression(update) => self.gen_update_expression(update, ctx),
            Expression::AssignmentExpression(assign) => self.gen_assignment_expression(assign, ctx),
            Expression::ConditionalExpression(cond) => self.gen_conditional_expression(cond, ctx),
            Expression::MemberExpression(member) => self.gen_member_expression(member, ctx),
            Expression::ChainExpression(chain) => self.gen_chain_expression(chain, ctx),
            Expression::OptionalMemberExpression(opt) => {
                let obj_reg = self.gen_expression(&opt.object, ctx)?;
                let result = self.alloc_register();

                let undef_reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(undef_reg);
                let is_undef = self.alloc_register();
                self.emit(Opcode::StrictEq);
                self.emit_u16(is_undef);
                self.emit_u16(obj_reg);
                self.emit_u16(undef_reg);
                self.free_register(undef_reg);
                self.emit(Opcode::JumpIf);
                self.emit_u16(is_undef);
                let short_jump = self.code.len();
                self.emit_i32(0);
                self.free_register(is_undef);

                let null_reg = self.alloc_register();
                self.emit(Opcode::LoadNull);
                self.emit_u16(null_reg);
                let is_null = self.alloc_register();
                self.emit(Opcode::StrictEq);
                self.emit_u16(is_null);
                self.emit_u16(obj_reg);
                self.emit_u16(null_reg);
                self.free_register(null_reg);
                self.emit(Opcode::JumpIf);
                self.emit_u16(is_null);
                let short_jump_2 = self.code.len();
                self.emit_i32(0);
                self.free_register(is_null);

                match &opt.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        self.emit(Opcode::GetNamedProp);
                        self.emit_u16(result);
                        self.emit_u16(obj_reg);
                        self.emit_u32(atom.0);
                    }
                    MemberProperty::Computed(expr) => {
                        let key_reg = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::GetField);
                        self.emit_u16(result);
                        self.emit_u16(obj_reg);
                        self.emit_u16(key_reg);
                        self.free_register(key_reg);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key_reg = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key_reg);
                        self.emit_u32(key_idx);
                        self.emit(Opcode::GetPrivate);
                        self.emit_u16(result);
                        self.emit_u16(obj_reg);
                        self.emit_u16(key_reg);
                        self.emit_u16(owner_reg);
                        if owner_temp {
                            self.free_register(owner_reg);
                        }
                        self.free_register(key_reg);
                    }
                }
                self.emit(Opcode::Jump);
                let end_jump = self.code.len();
                self.emit_i32(0);

                let short_pos = self.code.len();
                self.patch_jump(short_jump, short_pos);
                self.patch_jump(short_jump_2, short_pos);
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(result);

                let end_pos = self.code.len();
                self.patch_jump(end_jump, end_pos);
                self.free_register(obj_reg);
                Ok(result)
            }
            Expression::CallExpression(call) => self.gen_call_expression(call, ctx),
            Expression::NewExpression(new) => self.gen_new_expression(new, ctx),
            Expression::ArrayExpression(arr) => self.gen_array_expression(arr, ctx),
            Expression::ObjectExpression(obj) => self.gen_object_expression(obj, ctx),
            Expression::TaggedTemplateExpression(tagged) => {
                let cooked = ArrayExpression {
                    elements: tagged
                        .quasi
                        .quasis
                        .iter()
                        .map(|quasi| {
                            Some(ArrayElement::Expression(Expression::Literal(
                                if quasi.invalid_escape {
                                    Literal::Undefined
                                } else {
                                    Literal::String(quasi.value.clone(), false, false)
                                },
                            )))
                        })
                        .collect(),
                    trailing_comma_after_spread: false,
                    parenthesized: false,
                };
                let raw = ArrayExpression {
                    elements: tagged
                        .quasi
                        .quasis
                        .iter()
                        .map(|quasi| {
                            Some(ArrayElement::Expression(Expression::Literal(
                                Literal::String(quasi.raw.clone(), false, false),
                            )))
                        })
                        .collect(),
                    trailing_comma_after_spread: false,
                    parenthesized: false,
                };
                let template_object = Expression::CallExpression(CallExpression {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "__v16_template_object__".to_string(),
                        parenthesized: false,
                    })),
                    arguments: vec![
                        Argument::Expression(Expression::Literal(Literal::Number(
                            tagged.site as f64,
                        ))),
                        Argument::Expression(Expression::ArrayExpression(cooked)),
                        Argument::Expression(Expression::ArrayExpression(raw)),
                    ],
                });
                let mut arguments = vec![Argument::Expression(template_object)];
                arguments.extend(
                    tagged
                        .quasi
                        .expressions
                        .iter()
                        .cloned()
                        .map(Argument::Expression),
                );
                let call = Expression::CallExpression(CallExpression {
                    callee: Box::new((*tagged.tag).clone()),
                    arguments,
                });
                self.gen_expression(&call, ctx)
            }
            Expression::TemplateLiteral(tpl) => {
                let first_quasi = tpl
                    .quasis
                    .first()
                    .map(|quasi| quasi.value.as_str())
                    .unwrap_or("");
                let result = self.alloc_register();
                let first_idx = self.intern_and_add_const(ctx, first_quasi);
                self.emit(Opcode::LoadConst);
                self.emit_u16(result);
                self.emit_u32(first_idx);

                for (i, expr) in tpl.expressions.iter().enumerate() {
                    let expr_reg = self.gen_expression(expr, ctx)?;
                    self.emit(Opcode::Add);
                    self.emit_u16(result);
                    self.emit_u16(result);
                    self.emit_u16(expr_reg);
                    self.free_register(expr_reg);

                    if let Some(quasi) = tpl.quasis.get(i + 1) {
                        let quasi_idx = self.intern_and_add_const(ctx, &quasi.value);
                        let quasi_reg = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(quasi_reg);
                        self.emit_u32(quasi_idx);
                        self.emit(Opcode::Add);
                        self.emit_u16(result);
                        self.emit_u16(result);
                        self.emit_u16(quasi_reg);
                        self.free_register(quasi_reg);
                    }
                }

                Ok(result)
            }
            Expression::FunctionExpression(func) => self.gen_function_expression(
                &func.params,
                &func.body,
                func.name.as_deref(),
                ctx,
                func.generator,
                func.is_async,
                false,
                false,
                func.name.is_some(),
            ),
            Expression::ArrowFunction(arrow) => {
                let body = match &arrow.body {
                    ArrowBody::Expression(expr) => BlockStatement {
                        body: vec![ASTNode::ReturnStatement(ReturnStatement {
                            argument: Some((**expr).clone()),
                        })],
                        lines: vec![],
                    },
                    ArrowBody::Block(block) => block.clone(),
                };
                self.gen_function_expression(
                    &arrow.params,
                    &body,
                    None,
                    ctx,
                    false,
                    arrow.is_async,
                    true,
                    false,
                    false,
                )
            }
            Expression::This => {
                if self.in_derived_constructor {
                    self.emit(Opcode::CheckThisInitialized);
                }
                Ok(0)
            }
            Expression::Super => {
                let r = self.alloc_register();
                self.emit(Opcode::GetSuper);
                self.emit_u16(r);
                Ok(r)
            }
            Expression::PrivateIdentifier(name) => {
                let idx = self.intern_and_add_const(ctx, name);
                let r = self.alloc_register();
                self.emit(Opcode::LoadConst);
                self.emit_u16(r);
                self.emit_u32(idx);
                Ok(r)
            }
            Expression::YieldExpression(yield_expr) => {
                if yield_expr.delegate {
                    return self.gen_yield_star(yield_expr, ctx);
                }
                let arg_reg = if let Some(arg) = &yield_expr.argument {
                    self.gen_expression(arg, ctx)?
                } else {
                    let r = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(r);
                    r
                };
                let result_reg = self.alloc_register();
                self.emit(Opcode::Move);
                self.emit_u16(result_reg);
                self.emit_u16(arg_reg);
                self.free_register(arg_reg);
                self.emit(Opcode::Yield);
                self.emit_u16(result_reg);
                Ok(result_reg)
            }
            Expression::AwaitExpression(await_expr) => {
                let arg_reg = self.gen_expression(&await_expr.argument, ctx)?;
                let result_reg = self.alloc_register();
                self.emit(Opcode::Move);
                self.emit_u16(result_reg);
                self.emit_u16(arg_reg);
                self.free_register(arg_reg);
                self.emit(Opcode::Await);
                self.emit_u16(result_reg);
                Ok(result_reg)
            }
            Expression::RegExpLiteral(re) => {
                if let Err(e) = crate::regexp::Regex::with_flags(&re.pattern, &re.flags) {
                    return Err(format!("SyntaxError: {}", e));
                }
                let pattern_idx = self.intern_and_add_const(ctx, &re.pattern);
                let flags_idx = self.intern_and_add_const(ctx, &re.flags);
                let dst = self.alloc_register();
                self.emit(Opcode::NewRegExp);
                self.emit_u16(dst);
                self.emit_u32(pattern_idx);
                self.emit_u32(flags_idx);
                Ok(dst)
            }
            Expression::SequenceExpression(seq) => {
                let mut last_r = None;
                for (i, e) in seq.expressions.iter().enumerate() {
                    let r = self.gen_expression(e, ctx)?;
                    if i == seq.expressions.len() - 1 {
                        last_r = Some(r);
                    } else {
                        self.free_register(r);
                    }
                }

                Ok(last_r.unwrap_or_else(|| {
                    let r = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(r);
                    r
                }))
            }
            Expression::ClassExpression(ce) => {
                let class_name = ce.name.as_deref().unwrap_or("");
                let class_slot = self.alloc_register();
                self.gen_class_body(class_slot, class_name, &ce.super_class, &ce.body, ctx)?;
                Ok(class_slot)
            }
            Expression::MetaProperty(meta) => {
                if meta.meta == "new" && meta.property == "target" {
                    let r = self.alloc_register();
                    if self.in_field_initializer {
                        self.emit(Opcode::LoadUndefined);
                    } else {
                        self.emit(Opcode::GetNewTarget);
                    }
                    self.emit_u16(r);
                    return Ok(r);
                }
                let call = Expression::CallExpression(CallExpression {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "__v16_import_meta__".to_string(),
                        parenthesized: false,
                    })),
                    arguments: Vec::new(),
                });
                self.gen_expression(&call, ctx)
            }
            _ => {
                let r = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(r);
                Ok(r)
            }
        }
    }

    fn gen_binary_expression(
        &mut self,
        bin: &BinaryExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let left = self.gen_expression(&bin.left, ctx)?;

        let right_imm8 = if let Expression::Literal(Literal::Number(n)) = &*bin.right {
            if n.fract() == 0.0 {
                let v = *n as i64;
                if v >= i8::MIN as i64 && v <= i8::MAX as i64 {
                    Some(v as i8)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(imm) = right_imm8 {
            match bin.op {
                BinaryOp::Sub => {
                    let dst = self.alloc_register();
                    self.emit(Opcode::SubImm8);
                    self.emit_u16(dst);
                    self.emit_u16(left);
                    self.emit_u8(imm as u8);
                    self.free_register(left);
                    return Ok(dst);
                }
                BinaryOp::Lte => {
                    let dst = self.alloc_register();
                    self.emit(Opcode::LteImm8);
                    self.emit_u16(dst);
                    self.emit_u16(left);
                    self.emit_u8(imm as u8);
                    self.free_register(left);
                    return Ok(dst);
                }
                _ => {}
            }
        }

        let left = if self.reserved_slots.contains(&left) {
            let dst = self.alloc_register();
            self.emit(Opcode::Move);
            self.emit_u16(dst);
            self.emit_u16(left);
            dst
        } else {
            left
        };

        let right = self.gen_expression(&bin.right, ctx)?;

        let op = match bin.op {
            BinaryOp::Add => Opcode::Add,
            BinaryOp::Sub => Opcode::Sub,
            BinaryOp::Mul => Opcode::Mul,
            BinaryOp::Div => Opcode::Div,
            BinaryOp::Mod => Opcode::Mod,
            BinaryOp::Pow => Opcode::Pow,
            BinaryOp::BitAnd => Opcode::BitAnd,
            BinaryOp::BitOr => Opcode::BitOr,
            BinaryOp::BitXor => Opcode::BitXor,
            BinaryOp::Shl => Opcode::Shl,
            BinaryOp::Shr => Opcode::Shr,
            BinaryOp::UShr => Opcode::UShr,
            BinaryOp::Lt => Opcode::Lt,
            BinaryOp::Lte => Opcode::Lte,
            BinaryOp::Gt => Opcode::Gt,
            BinaryOp::Gte => Opcode::Gte,
            BinaryOp::Eq => Opcode::Eq,
            BinaryOp::Neq => Opcode::Neq,
            BinaryOp::StrictEq => Opcode::StrictEq,
            BinaryOp::StrictNeq => Opcode::StrictNeq,
            BinaryOp::In => {
                if matches!(&*bin.left, Expression::PrivateIdentifier(_)) {
                    Opcode::HasPrivate
                } else {
                    Opcode::HasProperty
                }
            }
            BinaryOp::InstanceOf => Opcode::InstanceOf,
        };

        let dst = self.alloc_register();
        let mut private_owner = None;
        if bin.op == BinaryOp::In {
            if let Expression::PrivateIdentifier(name) = &*bin.left {
                private_owner = Some(self.gen_private_owner(name, ctx)?);
            }
        }
        self.emit(op);
        self.emit_u16(dst);

        if bin.op == BinaryOp::In {
            self.emit_u16(right);
            self.emit_u16(left);
            if let Some((owner_reg, owner_temp)) = private_owner {
                self.emit_u16(owner_reg);
                if owner_temp {
                    self.free_register(owner_reg);
                }
            }
        } else {
            self.emit_u16(left);
            self.emit_u16(right);
        }

        self.free_register(left);
        self.free_register(right);
        Ok(dst)
    }

    fn gen_logical_expression(
        &mut self,
        logical: &LogicalExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        if self.p2_fold_profile.allow_logical_bool_left() {
            if let Expression::Literal(Literal::Boolean(left_bool)) = &*logical.left {
                match logical.op {
                    LogicalOp::And => {
                        if *left_bool {
                            return self.gen_expression(&logical.right, ctx);
                        }
                        let dst = self.alloc_register();
                        self.emit_bool_const(dst, false);
                        return Ok(dst);
                    }
                    LogicalOp::Or => {
                        if *left_bool {
                            let dst = self.alloc_register();
                            self.emit_bool_const(dst, true);
                            return Ok(dst);
                        }
                        return self.gen_expression(&logical.right, ctx);
                    }
                    LogicalOp::NullishCoalescing => {}
                }
            }
        }

        if self.p2_fold_profile.allow_aggressive_truthy() {
            if let Expression::Literal(lit) = &*logical.left {
                if let Some(left_truthy) = Self::literal_truthiness(lit) {
                    match logical.op {
                        LogicalOp::And => {
                            if left_truthy {
                                return self.gen_expression(&logical.right, ctx);
                            }
                            return self.gen_expression(&logical.left, ctx);
                        }
                        LogicalOp::Or => {
                            if left_truthy {
                                return self.gen_expression(&logical.left, ctx);
                            }
                            return self.gen_expression(&logical.right, ctx);
                        }
                        LogicalOp::NullishCoalescing => {}
                    }
                }
            }
        }

        if self.p2_fold_profile.allow_nullish_literal_left() {
            if let LogicalOp::NullishCoalescing = logical.op {
                if let Expression::Literal(lit) = &*logical.left {
                    return match lit {
                        Literal::Null | Literal::Undefined => {
                            self.gen_expression(&logical.right, ctx)
                        }
                        _ => self.gen_expression(&logical.left, ctx),
                    };
                }
            }
        }

        match logical.op {
            LogicalOp::And => {
                if self.opt_branch_result_prealloc {
                    let left = self.gen_expression(&logical.left, ctx)?;
                    let result = if self.reserved_slots.contains(&left) {
                        let dst = self.alloc_register();
                        self.emit(Opcode::Move);
                        self.emit_u16(dst);
                        self.emit_u16(left);
                        dst
                    } else {
                        left
                    };

                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(left);
                    let short_jump = self.code.len();
                    self.emit_i32(0);

                    let right = self.gen_expression(&logical.right, ctx)?;
                    if right != result {
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(right);
                        self.free_register(right);
                    }

                    let end_pos = self.code.len();
                    self.patch_jump(short_jump, end_pos);
                    return Ok(result);
                }

                let left = self.gen_expression(&logical.left, ctx)?;
                self.emit(Opcode::JumpIfNot);
                self.emit_u16(left);
                let short_jump = self.code.len();
                self.emit_i32(0);

                let right = self.gen_expression(&logical.right, ctx)?;

                let result = if self.reserved_slots.contains(&left) && right != left {
                    let dst = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(dst);
                    self.emit_u16(right);
                    self.free_register(right);
                    dst
                } else if right != left {
                    self.emit(Opcode::Move);
                    self.emit_u16(left);
                    self.emit_u16(right);
                    self.free_register(right);
                    left
                } else {
                    left
                };

                if result != left {
                    self.emit(Opcode::Jump);
                    let end_jump = self.code.len();
                    self.emit_i32(0);
                    let patch_pos = self.code.len();
                    self.patch_jump(short_jump, patch_pos);
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(left);
                    let end_pos = self.code.len();
                    self.patch_jump(end_jump, end_pos);
                } else {
                    let end_pos = self.code.len();
                    self.patch_jump(short_jump, end_pos);
                }
                Ok(result)
            }
            LogicalOp::Or => {
                if self.opt_branch_result_prealloc {
                    let left = self.gen_expression(&logical.left, ctx)?;
                    let result = if self.reserved_slots.contains(&left) {
                        let dst = self.alloc_register();
                        self.emit(Opcode::Move);
                        self.emit_u16(dst);
                        self.emit_u16(left);
                        dst
                    } else {
                        left
                    };

                    self.emit(Opcode::JumpIf);
                    self.emit_u16(left);
                    let short_jump = self.code.len();
                    self.emit_i32(0);

                    let right = self.gen_expression(&logical.right, ctx)?;
                    if right != result {
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(right);
                        self.free_register(right);
                    }

                    let end_pos = self.code.len();
                    self.patch_jump(short_jump, end_pos);
                    return Ok(result);
                }

                let left = self.gen_expression(&logical.left, ctx)?;
                self.emit(Opcode::JumpIf);
                self.emit_u16(left);
                let short_jump = self.code.len();
                self.emit_i32(0);

                let right = self.gen_expression(&logical.right, ctx)?;

                let result = if self.reserved_slots.contains(&left) && right != left {
                    let dst = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(dst);
                    self.emit_u16(right);
                    self.free_register(right);
                    dst
                } else if right != left {
                    self.emit(Opcode::Move);
                    self.emit_u16(left);
                    self.emit_u16(right);
                    self.free_register(right);
                    left
                } else {
                    left
                };

                if result != left {
                    self.emit(Opcode::Jump);
                    let end_jump = self.code.len();
                    self.emit_i32(0);
                    let patch_pos = self.code.len();
                    self.patch_jump(short_jump, patch_pos);
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(left);
                    let end_pos = self.code.len();
                    self.patch_jump(end_jump, end_pos);
                } else {
                    let end_pos = self.code.len();
                    self.patch_jump(short_jump, end_pos);
                }
                Ok(result)
            }
            LogicalOp::NullishCoalescing => {
                let left = self.gen_expression(&logical.left, ctx)?;
                let result = if self.reserved_slots.contains(&left) {
                    let dst = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(dst);
                    self.emit_u16(left);
                    dst
                } else {
                    left
                };

                let eval_right_jump = if self.opt_jump_if_nullish {
                    self.emit(Opcode::JumpIfNullish);
                    self.emit_u16(left);
                    let patch = self.code.len();
                    self.emit_i32(0);
                    patch
                } else {
                    let check_reg = self.alloc_register();
                    self.emit(Opcode::LoadNull);
                    self.emit_u16(check_reg);
                    self.emit(Opcode::Eq);
                    self.emit_u16(check_reg);
                    self.emit_u16(left);
                    self.emit_u16(check_reg);
                    self.emit(Opcode::JumpIf);
                    self.emit_u16(check_reg);
                    let patch = self.code.len();
                    self.emit_i32(0);
                    self.free_register(check_reg);
                    patch
                };

                self.emit(Opcode::Jump);
                let end_jump = self.code.len();
                self.emit_i32(0);

                let eval_right_pos = self.code.len();
                self.patch_jump(eval_right_jump, eval_right_pos);
                let right = self.gen_expression(&logical.right, ctx)?;
                if right != result {
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(right);
                    self.free_register(right);
                }

                let end_pos = self.code.len();
                self.patch_jump(end_jump, end_pos);
                Ok(result)
            }
        }
    }

    fn gen_unary_expression(
        &mut self,
        unary: &UnaryExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let saved = self.suppress_ref_error;
        if matches!(unary.op, UnaryOp::TypeOf) {
            if matches!(&*unary.argument, Expression::Identifier(_)) {
                self.suppress_ref_error = true;
            }
        }
        if matches!(unary.op, UnaryOp::Delete) {
            if matches!(&*unary.argument, Expression::Identifier(_)) {
                self.suppress_ref_error = true;
            }
        }
        if matches!(unary.op, UnaryOp::Delete) {
            self.suppress_ref_error = saved;
            return self.gen_delete_expression(0, &unary.argument, ctx);
        }
        let arg = self.gen_expression(&unary.argument, ctx)?;
        self.suppress_ref_error = saved;
        match unary.op {
            UnaryOp::Minus => {
                let dst = self.alloc_register();
                self.emit(Opcode::Neg);
                self.emit_u16(dst);
                self.emit_u16(arg);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::Not => {
                let dst = self.alloc_register();
                self.emit(Opcode::Not);
                self.emit_u16(dst);
                self.emit_u16(arg);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::TypeOf => {
                let dst = self.alloc_register();
                self.emit(Opcode::TypeOf);
                self.emit_u16(dst);
                self.emit_u16(arg);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::BitNot => {
                let dst = self.alloc_register();
                self.emit(Opcode::BitNot);
                self.emit_u16(dst);
                self.emit_u16(arg);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::Plus => {
                let dst = self.alloc_register();
                self.emit(Opcode::Pos);
                self.emit_u16(dst);
                self.emit_u16(arg);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::Void => {
                let dst = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(dst);
                self.free_register(arg);
                return Ok(dst);
            }
            UnaryOp::Delete => {
                return self.gen_delete_expression(arg, &unary.argument, ctx);
            }
        }
    }

    fn gen_delete_expression(
        &mut self,
        arg_reg: u16,
        arg_expr: &Expression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        match arg_expr {
            Expression::Identifier(id) => {
                self.free_register(arg_reg);
                if !self.with_scopes.is_empty() {
                    let applies = match self.lookup_var_scope_index(&id.name) {
                        Some(index) => self.with_applies_to_local(&id.name, index),
                        None => self.with_applies_to_outer(&id.name),
                    };
                    if applies {
                        return self.gen_with_delete(&id.name, ctx);
                    }
                }
                let dst = self.alloc_register();
                if id.name == "arguments" && !self.is_script_level && !self.is_arrow {
                    self.emit(Opcode::LoadFalse);
                    self.emit_u16(dst);
                } else if self.resolve_var(&id.name, ctx).is_some() {
                    self.emit(Opcode::LoadFalse);
                    self.emit_u16(dst);
                } else {
                    let atom = ctx.intern(&id.name);
                    let idx = self.add_constant(JSValue::new_string(atom));
                    self.emit(Opcode::DeleteGlobal);
                    self.emit_u16(dst);
                    self.emit_u32(idx);
                }
                Ok(dst)
            }
            Expression::MemberExpression(member) => {
                if matches!(&*member.object, Expression::Super) {
                    if let MemberProperty::Computed(expr) = &member.property {
                        self.gen_expression(expr, ctx)?;
                    }
                    self.free_register(arg_reg);
                    let msg_idx = self.add_constant(JSValue::new_string(
                        ctx.intern("Cannot delete super reference"),
                    ));
                    self.emit(Opcode::ThrowReferenceError);
                    self.emit_u32(msg_idx);
                    let dst = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(dst);
                    return Ok(dst);
                }
                let obj = self.gen_expression(&member.object, ctx)?;
                let dst = self.alloc_register();
                match &member.property {
                    MemberProperty::Identifier(name) => {
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        self.emit(Opcode::DeleteProp);
                        self.emit_u16(dst);
                        self.emit_u16(obj);
                        self.emit_u16(key);
                        self.free_register(key);
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::DeleteProp);
                        self.emit_u16(dst);
                        self.emit_u16(obj);
                        self.emit_u16(key);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        self.emit(Opcode::DeleteProp);
                        self.emit_u16(dst);
                        self.emit_u16(obj);
                        self.emit_u16(key);
                        self.free_register(key);
                    }
                }
                self.free_register(obj);
                Ok(dst)
            }
            Expression::Literal(Literal::Number(n)) if n.is_nan() || n.is_infinite() => {
                self.free_register(arg_reg);
                let name = if n.is_nan() { "NaN" } else { "Infinity" };
                let atom = ctx.intern(name);
                let idx = self.add_constant(JSValue::new_string(atom));
                let dst = self.alloc_register();
                self.emit(Opcode::DeleteGlobal);
                self.emit_u16(dst);
                self.emit_u32(idx);
                Ok(dst)
            }
            _ => {
                self.free_register(arg_reg);
                let dst = self.alloc_register();
                self.emit(Opcode::LoadTrue);
                self.emit_u16(dst);
                Ok(dst)
            }
        }
    }

    fn gen_update_expression(
        &mut self,
        update: &UpdateExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        self.gen_update_expression_impl(update, ctx, false)
    }

    fn gen_update_expression_impl(
        &mut self,
        update: &UpdateExpression,
        ctx: &mut JSContext,
        bypass_with_target: bool,
    ) -> Result<u16, String> {
        match &*update.argument {
            Expression::Identifier(id) => {
                if self.binding_is_const(&id.name) {
                    self.emit_throw_const_assignment(&id.name, ctx);
                    let dst = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(dst);
                    return Ok(dst);
                }
                if self.binding_is_function_name(&id.name) {
                    return self.gen_immutable_binding_update(update, &id.name, ctx);
                }
                let with_target = if bypass_with_target || self.with_scopes.is_empty() {
                    false
                } else {
                    match self.lookup_var_scope_index(&id.name) {
                        Some(index) => self.with_applies_to_local(&id.name, index),
                        None => self.with_applies_to_outer(&id.name),
                    }
                };
                if with_target {
                    let object = self.emit_with_resolve(&id.name, ctx);
                    let fallback_jump = self.emit_jump_if_nullish(object);
                    let current = self.alloc_register();
                    self.emit_with_get_resolved(current, object, &id.name, ctx);
                    let old_val = if update.prefix {
                        current
                    } else {
                        let old = self.alloc_register();
                        self.emit(Opcode::Move);
                        self.emit_u16(old);
                        self.emit_u16(current);
                        self.emit(Opcode::ToNumeric);
                        self.emit_u16(old);
                        self.emit_u16(old);
                        old
                    };
                    let op = match update.op {
                        UpdateOp::Increment => Opcode::IncLocal,
                        UpdateOp::Decrement => Opcode::DecLocal,
                    };
                    self.emit(op);
                    self.emit_u16(current);
                    self.emit_with_set_resolved(object, current, &id.name, ctx);
                    let result = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(old_val);
                    if !update.prefix {
                        self.free_register(old_val);
                    }
                    self.free_register(current);
                    self.emit(Opcode::Jump);
                    let skip_fallback = self.code.len();
                    self.emit_i32(0);
                    let fallback_pos = self.code.len();
                    self.patch_jump(fallback_jump, fallback_pos);
                    let plain = self.gen_update_expression_impl(update, ctx, true)?;
                    if plain != result {
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(plain);
                        self.free_register(plain);
                    }
                    let end_pos = self.code.len();
                    self.patch_jump(skip_fallback, end_pos);
                    self.free_register(object);
                    return Ok(result);
                }
                match self.resolve_var(&id.name, ctx) {
                    Some(VarLocation::Local(slot)) => {
                        if let Some(entry) = self.lookup_var(&id.name) {
                            if entry.kind != VariableKind::Var && !entry.initialized {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(slot);
                            }
                        }
                        let op = match update.op {
                            UpdateOp::Increment => Opcode::IncLocal,
                            UpdateOp::Decrement => Opcode::DecLocal,
                        };

                        if update.prefix {
                            self.emit(op);
                            self.emit_u16(slot);
                            if self.is_global_binding(&id.name) {
                                let atom = ctx.intern(&id.name);
                                let idx = self.add_constant(JSValue::new_string(atom));
                                self.emit(Opcode::SetGlobal);
                                self.emit_u32(idx);
                                self.emit_u16(slot);
                            }
                            Ok(slot)
                        } else {
                            let old_val = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(old_val);
                            self.emit_u16(slot);
                            self.emit(Opcode::ToNumeric);
                            self.emit_u16(old_val);
                            self.emit_u16(old_val);
                            self.emit(op);
                            self.emit_u16(slot);
                            if self.is_global_binding(&id.name) {
                                let atom = ctx.intern(&id.name);
                                let idx = self.add_constant(JSValue::new_string(atom));
                                self.emit(Opcode::SetGlobal);
                                self.emit_u32(idx);
                                self.emit_u16(slot);
                            }
                            Ok(old_val)
                        }
                    }
                    Some(VarLocation::Upvalue(slot)) => {
                        let temp = self.alloc_register();
                        self.emit(Opcode::GetUpvalue);
                        self.emit_u16(temp);
                        self.emit_u16(slot);

                        if let Some(entry) = self.lookup_var(&id.name) {
                            if entry.kind != VariableKind::Var && !entry.initialized {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(temp);
                            }
                        }

                        let op = match update.op {
                            UpdateOp::Increment => Opcode::IncLocal,
                            UpdateOp::Decrement => Opcode::DecLocal,
                        };

                        if update.prefix {
                            self.emit(op);
                            self.emit_u16(temp);
                            self.emit(Opcode::SetUpvalue);
                            self.emit_u16(slot);
                            self.emit_u16(temp);
                            Ok(temp)
                        } else {
                            let old_val = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(old_val);
                            self.emit_u16(temp);
                            self.emit(op);
                            self.emit_u16(temp);
                            self.emit(Opcode::SetUpvalue);
                            self.emit_u16(slot);
                            self.emit_u16(temp);
                            Ok(old_val)
                        }
                    }
                    None => {
                        let atom = ctx.intern(&id.name);
                        let const_idx = self.add_constant(JSValue::new_string(atom));
                        let r = self.alloc_register();
                        self.emit(Opcode::GetGlobal);
                        self.emit_u16(r);
                        self.emit_u32(const_idx);
                        self.emit(Opcode::CheckRef);
                        self.emit_u16(r);
                        self.emit_u32(const_idx);
                        let op = match update.op {
                            UpdateOp::Increment => Opcode::IncLocal,
                            UpdateOp::Decrement => Opcode::DecLocal,
                        };
                        if update.prefix {
                            self.emit(op);
                            self.emit_u16(r);
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(const_idx);
                            self.emit_u16(r);
                            Ok(r)
                        } else {
                            let old = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(old);
                            self.emit_u16(r);
                            self.emit(op);
                            self.emit_u16(r);
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(const_idx);
                            self.emit_u16(r);
                            self.free_register(r);
                            Ok(old)
                        }
                    }
                }
            }
            Expression::MemberExpression(member) => {
                let is_super = matches!(&*member.object, Expression::Super);
                let obj = if is_super {
                    let base = self.alloc_register();
                    self.emit(Opcode::GetSuperBase);
                    self.emit_u16(base);
                    base
                } else {
                    self.gen_expression(&member.object, ctx)?
                };
                let op = match update.op {
                    UpdateOp::Increment => Opcode::IncLocal,
                    UpdateOp::Decrement => Opcode::DecLocal,
                };

                match &member.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        if is_super {
                            let key = self.alloc_register();
                            let key_idx = self.add_constant(JSValue::new_string(atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key);
                            self.emit_u32(key_idx);
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetSuperProp);
                            self.emit_u16(tmp);
                            self.emit_u16(obj);
                            self.emit_u16(0);
                            self.emit_u16(key);
                            if update.prefix {
                                self.emit(op);
                                self.emit_u16(tmp);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(key);
                                self.free_register(obj);
                                Ok(tmp)
                            } else {
                                let old_val = self.alloc_register();
                                self.emit(Opcode::Move);
                                self.emit_u16(old_val);
                                self.emit_u16(tmp);
                                self.emit(Opcode::ToNumeric);
                                self.emit_u16(old_val);
                                self.emit_u16(old_val);
                                self.emit(op);
                                self.emit_u16(tmp);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(key);
                                self.free_register(obj);
                                self.free_register(tmp);
                                Ok(old_val)
                            }
                        } else {
                        let tmp = self.alloc_register();
                        self.emit(Opcode::GetNamedProp);
                        self.emit_u16(tmp);
                        self.emit_u16(obj);
                        self.emit_u32(atom.0);

                        if update.prefix {
                            self.emit(op);
                            self.emit_u16(tmp);
                            self.emit(Opcode::SetNamedProp);
                            self.emit_u16(obj);
                            self.emit_u16(tmp);
                            self.emit_u32(atom.0);
                            self.free_register(obj);
                            Ok(tmp)
                        } else {
                            let old_val = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(old_val);
                            self.emit_u16(tmp);
                            self.emit(Opcode::ToNumeric);
                            self.emit_u16(old_val);
                            self.emit_u16(old_val);
                            self.emit(op);
                            self.emit_u16(tmp);
                            self.emit(Opcode::SetNamedProp);
                            self.emit_u16(obj);
                            self.emit_u16(tmp);
                            self.emit_u32(atom.0);
                            self.free_register(obj);
                            self.free_register(tmp);
                            Ok(old_val)
                        }
                        }
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        if is_super {
                            self.emit(Opcode::GetSuperBase);
                            self.emit_u16(obj);
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetSuperProp);
                            self.emit_u16(tmp);
                            self.emit_u16(obj);
                            self.emit_u16(0);
                            self.emit_u16(key);
                            if update.prefix {
                                self.emit(op);
                                self.emit_u16(tmp);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(obj);
                                self.free_register(key);
                                Ok(tmp)
                            } else {
                                let old_val = self.alloc_register();
                                self.emit(Opcode::Move);
                                self.emit_u16(old_val);
                                self.emit_u16(tmp);
                                self.emit(op);
                                self.emit_u16(tmp);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(obj);
                                self.free_register(key);
                                self.free_register(tmp);
                                Ok(old_val)
                            }
                        } else {
                        let tmp = self.alloc_register();
                        self.emit(Opcode::GetField);
                        self.emit_u16(tmp);
                        self.emit_u16(obj);
                        self.emit_u16(key);

                        if update.prefix {
                            self.emit(op);
                            self.emit_u16(tmp);
                            self.emit(Opcode::SetField);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(tmp);
                            self.free_register(obj);
                            self.free_register(key);
                            Ok(tmp)
                        } else {
                            let old_val = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(old_val);
                            self.emit_u16(tmp);
                            self.emit(op);
                            self.emit_u16(tmp);
                            self.emit(Opcode::SetField);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(tmp);
                            self.free_register(obj);
                            self.free_register(key);
                            self.free_register(tmp);
                            Ok(old_val)
                        }
                        }
                    }
                    MemberProperty::PrivateIdentifier(_) => {
                        self.free_register(obj);
                        let r = self.alloc_register();
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(r);
                        Ok(r)
                    }
                }
            }
            _ => {
                let r = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(r);
                Ok(r)
            }
        }
    }

    fn assign_op_to_reg_opcode(op: &AssignOp) -> Option<Opcode> {
        match op {
            AssignOp::AddAssign => Some(Opcode::Add),
            AssignOp::SubAssign => Some(Opcode::Sub),
            AssignOp::MulAssign => Some(Opcode::Mul),
            AssignOp::DivAssign => Some(Opcode::Div),
            AssignOp::ModAssign => Some(Opcode::Mod),
            AssignOp::PowAssign => Some(Opcode::Pow),
            AssignOp::BitAndAssign => Some(Opcode::BitAnd),
            AssignOp::BitOrAssign => Some(Opcode::BitOr),
            AssignOp::BitXorAssign => Some(Opcode::BitXor),
            AssignOp::ShlAssign => Some(Opcode::Shl),
            AssignOp::ShrAssign => Some(Opcode::Shr),
            AssignOp::UShrAssign => Some(Opcode::UShr),
            _ => None,
        }
    }

    fn read_binding_temp(&mut self, name: &str, ctx: &mut JSContext) -> u16 {
        match self.resolve_var(name, ctx) {
            Some(VarLocation::Local(slot)) => {
                let tmp = self.alloc_register();
                self.emit(Opcode::Move);
                self.emit_u16(tmp);
                self.emit_u16(slot);
                tmp
            }
            Some(VarLocation::Upvalue(slot)) => {
                let tmp = self.alloc_register();
                self.emit(Opcode::GetUpvalue);
                self.emit_u16(tmp);
                self.emit_u16(slot);
                tmp
            }
            None => {
                let idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
                let tmp = self.alloc_register();
                self.emit(Opcode::GetGlobal);
                self.emit_u16(tmp);
                self.emit_u32(idx);
                if !self.suppress_ref_error {
                    self.emit(Opcode::CheckRef);
                    self.emit_u16(tmp);
                    self.emit_u32(idx);
                }
                tmp
            }
        }
    }

    fn gen_immutable_binding_assignment(
        &mut self,
        assign: &AssignmentExpression,
        name: &str,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let current = self.read_binding_temp(name, ctx);
        if matches!(
            assign.op,
            AssignOp::LogicalAndAssign | AssignOp::LogicalOrAssign | AssignOp::NullishAssign
        ) {
            let jump_to_end = match assign.op {
                AssignOp::LogicalAndAssign => {
                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(current);
                    let patch = self.code.len();
                    self.emit_i32(0);
                    patch
                }
                AssignOp::LogicalOrAssign => {
                    self.emit(Opcode::JumpIf);
                    self.emit_u16(current);
                    let patch = self.code.len();
                    self.emit_i32(0);
                    patch
                }
                AssignOp::NullishAssign => {
                    let null_reg = self.alloc_register();
                    self.emit(Opcode::LoadNull);
                    self.emit_u16(null_reg);
                    let cmp_reg = self.alloc_register();
                    self.emit(Opcode::StrictEq);
                    self.emit_u16(cmp_reg);
                    self.emit_u16(current);
                    self.emit_u16(null_reg);
                    self.emit(Opcode::JumpIf);
                    self.emit_u16(cmp_reg);
                    let jump_to_assign = self.code.len();
                    self.emit_i32(0);

                    let undef_reg = self.alloc_register();
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(undef_reg);
                    self.emit(Opcode::StrictEq);
                    self.emit_u16(cmp_reg);
                    self.emit_u16(current);
                    self.emit_u16(undef_reg);
                    self.emit(Opcode::JumpIfNot);
                    self.emit_u16(cmp_reg);
                    let jump_to_end = self.code.len();
                    self.emit_i32(0);

                    let assign_pos = self.code.len();
                    self.patch_jump(jump_to_assign, assign_pos);

                    self.free_register(undef_reg);
                    self.free_register(cmp_reg);
                    self.free_register(null_reg);
                    jump_to_end
                }
                _ => unreachable!(),
            };
            let rhs = if assign.lhs_parenthesized {
                self.gen_expression(&assign.right, ctx)?
            } else {
                self.gen_expression_with_assign_name(&assign.right, name, ctx)?
            };
            if current != rhs {
                self.emit(Opcode::Move);
                self.emit_u16(current);
                self.emit_u16(rhs);
                self.free_register(rhs);
            }
            if self.is_strict {
                self.emit_throw_const_assignment(name, ctx);
            }
            let end_pos = self.code.len();
            self.patch_jump(jump_to_end, end_pos);
            return Ok(current);
        }

        let val = if assign.lhs_parenthesized {
            self.gen_expression(&assign.right, ctx)?
        } else {
            self.gen_expression_with_assign_name(&assign.right, name, ctx)?
        };
        if let Some(opcode) = Self::assign_op_to_reg_opcode(&assign.op) {
            self.emit(opcode);
            self.emit_u16(current);
            self.emit_u16(current);
            self.emit_u16(val);
            self.free_register(val);
        } else if current != val {
            self.emit(Opcode::Move);
            self.emit_u16(current);
            self.emit_u16(val);
        }
        if self.is_strict {
            self.emit_throw_const_assignment(name, ctx);
        }
        Ok(current)
    }

    fn gen_immutable_binding_update(
        &mut self,
        update: &UpdateExpression,
        name: &str,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let current = self.read_binding_temp(name, ctx);
        if self.is_strict {
            self.emit_throw_const_assignment(name, ctx);
            return Ok(current);
        }
        let op = match update.op {
            UpdateOp::Increment => Opcode::IncLocal,
            UpdateOp::Decrement => Opcode::DecLocal,
        };
        if update.prefix {
            self.emit(op);
            self.emit_u16(current);
            Ok(current)
        } else {
            let old_val = self.alloc_register();
            self.emit(Opcode::Move);
            self.emit_u16(old_val);
            self.emit_u16(current);
            self.emit(Opcode::ToNumeric);
            self.emit_u16(old_val);
            self.emit_u16(old_val);
            self.emit(op);
            self.emit_u16(current);
            Ok(old_val)
        }
    }

    fn gen_assignment_expression(
        &mut self,
        assign: &AssignmentExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        self.gen_assignment_expression_impl(assign, ctx, false)
    }

    fn gen_assignment_expression_impl(
        &mut self,
        assign: &AssignmentExpression,
        ctx: &mut JSContext,
        bypass_with_target: bool,
    ) -> Result<u16, String> {
        let is_compound = Self::assign_op_to_reg_opcode(&assign.op);

        match &assign.left {
            AssignmentTarget::Identifier(name) => {
                if self.is_strict && Self::is_strict_reserved_word(name) {
                    return Err(format!(
                        "SyntaxError: Assignment to '{}' in strict mode is not allowed",
                        name
                    ));
                }
                if self.binding_is_const(name) {
                    let val = self.gen_expression_with_assign_name(&assign.right, name, ctx)?;
                    self.emit_throw_const_assignment(name, ctx);
                    return Ok(val);
                }
                if self.binding_is_function_name(name) {
                    return self.gen_immutable_binding_assignment(assign, name, ctx);
                }

                let with_target = if bypass_with_target || self.with_scopes.is_empty() {
                    false
                } else {
                    match self.lookup_var_scope_index(name) {
                        Some(index) => self.with_applies_to_local(name, index),
                        None => self.with_applies_to_outer(name),
                    }
                };

                if with_target
                    && matches!(
                        assign.op,
                        AssignOp::LogicalAndAssign
                            | AssignOp::LogicalOrAssign
                            | AssignOp::NullishAssign
                    )
                {
                    let object = self.emit_with_resolve(name, ctx);
                    let fallback_jump = self.emit_jump_if_nullish(object);
                    let current = self.alloc_register();
                    self.emit_with_get_resolved(current, object, name, ctx);
                    let jump_to_end = match assign.op {
                        AssignOp::LogicalAndAssign => {
                            self.emit(Opcode::JumpIfNot);
                            self.emit_u16(current);
                            let patch = self.code.len();
                            self.emit_i32(0);
                            patch
                        }
                        AssignOp::LogicalOrAssign => {
                            self.emit(Opcode::JumpIf);
                            self.emit_u16(current);
                            let patch = self.code.len();
                            self.emit_i32(0);
                            patch
                        }
                        AssignOp::NullishAssign => {
                            let null_reg = self.alloc_register();
                            self.emit(Opcode::LoadNull);
                            self.emit_u16(null_reg);
                            let cmp_reg = self.alloc_register();
                            self.emit(Opcode::StrictEq);
                            self.emit_u16(cmp_reg);
                            self.emit_u16(current);
                            self.emit_u16(null_reg);
                            self.emit(Opcode::JumpIf);
                            self.emit_u16(cmp_reg);
                            let jump_to_assign = self.code.len();
                            self.emit_i32(0);

                            let undef_reg = self.alloc_register();
                            self.emit(Opcode::LoadUndefined);
                            self.emit_u16(undef_reg);
                            self.emit(Opcode::StrictEq);
                            self.emit_u16(cmp_reg);
                            self.emit_u16(current);
                            self.emit_u16(undef_reg);
                            self.emit(Opcode::JumpIfNot);
                            self.emit_u16(cmp_reg);
                            let jump_to_end = self.code.len();
                            self.emit_i32(0);

                            let assign_pos = self.code.len();
                            self.patch_jump(jump_to_assign, assign_pos);

                            self.free_register(undef_reg);
                            self.free_register(cmp_reg);
                            self.free_register(null_reg);
                            jump_to_end
                        }
                        _ => unreachable!(),
                    };

                    let rhs = self.gen_expression(&assign.right, ctx)?;
                    if current != rhs {
                        self.emit(Opcode::Move);
                        self.emit_u16(current);
                        self.emit_u16(rhs);
                        self.free_register(rhs);
                    }
                    self.emit_with_set_resolved(object, current, name, ctx);
                    let result = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(current);
                    self.free_register(current);
                    self.emit(Opcode::Jump);
                    let skip_fallback = self.code.len();
                    self.emit_i32(0);
                    let fallback_pos = self.code.len();
                    self.patch_jump(fallback_jump, fallback_pos);
                    let plain = self.gen_assignment_expression_impl(assign, ctx, true)?;
                    if plain != result {
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(plain);
                        self.free_register(plain);
                    }
                    let end_pos = self.code.len();
                    self.patch_jump(jump_to_end, end_pos);
                    self.patch_jump(skip_fallback, end_pos);
                    self.free_register(object);
                    return Ok(result);
                }

                if with_target {
                    if let Some(op) = is_compound {
                        let object = self.emit_with_resolve(name, ctx);
                        let fallback_jump = self.emit_jump_if_nullish(object);
                        let current = self.alloc_register();
                        self.emit_with_get_resolved(current, object, name, ctx);
                        let rhs = self.gen_expression(&assign.right, ctx)?;
                        self.emit(op);
                        self.emit_u16(current);
                        self.emit_u16(current);
                        self.emit_u16(rhs);
                        self.free_register(rhs);
                        self.emit_with_set_resolved(object, current, name, ctx);
                        let result = self.alloc_register();
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(current);
                        self.free_register(current);
                        self.emit(Opcode::Jump);
                        let skip_fallback = self.code.len();
                        self.emit_i32(0);
                        let fallback_pos = self.code.len();
                        self.patch_jump(fallback_jump, fallback_pos);
                        let plain = self.gen_assignment_expression_impl(assign, ctx, true)?;
                        if plain != result {
                            self.emit(Opcode::Move);
                            self.emit_u16(result);
                            self.emit_u16(plain);
                            self.free_register(plain);
                        }
                        let end_pos = self.code.len();
                        self.patch_jump(skip_fallback, end_pos);
                        self.free_register(object);
                        return Ok(result);
                    }
                    let object = self.emit_with_resolve(name, ctx);
                    let fallback_jump = self.emit_jump_if_nullish(object);
                    let val = self.gen_expression_with_assign_name(&assign.right, name, ctx)?;
                    self.emit_with_set_resolved(object, val, name, ctx);
                    let result = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(result);
                    self.emit_u16(val);
                    self.free_register(val);
                    self.emit(Opcode::Jump);
                    let skip_fallback = self.code.len();
                    self.emit_i32(0);
                    let fallback_pos = self.code.len();
                    self.patch_jump(fallback_jump, fallback_pos);
                    let plain = self.gen_assignment_expression_impl(assign, ctx, true)?;
                    if plain != result {
                        self.emit(Opcode::Move);
                        self.emit_u16(result);
                        self.emit_u16(plain);
                        self.free_register(plain);
                    }
                    let end_pos = self.code.len();
                    self.patch_jump(skip_fallback, end_pos);
                    self.free_register(object);
                    return Ok(result);
                }

                if matches!(
                    assign.op,
                    AssignOp::LogicalAndAssign
                        | AssignOp::LogicalOrAssign
                        | AssignOp::NullishAssign
                ) {
                    let resolved = self.resolve_var(name, ctx);
                    let global_atom = ctx.intern(name);
                    let global_idx = self.add_constant(JSValue::new_string(global_atom));

                    let current = match resolved {
                        Some(VarLocation::Local(slot)) => slot,
                        Some(VarLocation::Upvalue(slot)) => {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetUpvalue);
                            self.emit_u16(tmp);
                            self.emit_u16(slot);
                            tmp
                        }
                        None => {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetGlobal);
                            self.emit_u16(tmp);
                            self.emit_u32(global_idx);
                            if !self.suppress_ref_error {
                                self.emit(Opcode::CheckRef);
                                self.emit_u16(tmp);
                                self.emit_u32(global_idx);
                            }
                            tmp
                        }
                    };

                    let jump_to_end = match assign.op {
                        AssignOp::LogicalAndAssign => {
                            self.emit(Opcode::JumpIfNot);
                            self.emit_u16(current);
                            let patch = self.code.len();
                            self.emit_i32(0);
                            patch
                        }
                        AssignOp::LogicalOrAssign => {
                            self.emit(Opcode::JumpIf);
                            self.emit_u16(current);
                            let patch = self.code.len();
                            self.emit_i32(0);
                            patch
                        }
                        AssignOp::NullishAssign => {
                            let null_reg = self.alloc_register();
                            self.emit(Opcode::LoadNull);
                            self.emit_u16(null_reg);
                            let cmp_reg = self.alloc_register();
                            self.emit(Opcode::StrictEq);
                            self.emit_u16(cmp_reg);
                            self.emit_u16(current);
                            self.emit_u16(null_reg);
                            self.emit(Opcode::JumpIf);
                            self.emit_u16(cmp_reg);
                            let jump_to_assign = self.code.len();
                            self.emit_i32(0);

                            let undef_reg = self.alloc_register();
                            self.emit(Opcode::LoadUndefined);
                            self.emit_u16(undef_reg);
                            self.emit(Opcode::StrictEq);
                            self.emit_u16(cmp_reg);
                            self.emit_u16(current);
                            self.emit_u16(undef_reg);
                            self.emit(Opcode::JumpIfNot);
                            self.emit_u16(cmp_reg);
                            let jump_to_end = self.code.len();
                            self.emit_i32(0);

                            let assign_pos = self.code.len();
                            self.patch_jump(jump_to_assign, assign_pos);

                            self.free_register(undef_reg);
                            self.free_register(cmp_reg);
                            self.free_register(null_reg);
                            jump_to_end
                        }
                        _ => unreachable!(),
                    };

                    let rhs = if assign.lhs_parenthesized {
                        self.gen_expression(&assign.right, ctx)?
                    } else {
                        self.gen_expression_with_assign_name(&assign.right, name, ctx)?
                    };
                    if current != rhs {
                        self.emit(Opcode::Move);
                        self.emit_u16(current);
                        self.emit_u16(rhs);
                    }

                    match resolved {
                        Some(VarLocation::Local(slot)) => {
                            if slot != current {
                                self.emit(Opcode::Move);
                                self.emit_u16(slot);
                                self.emit_u16(current);
                            }
                            if self.is_global_binding(name) {
                                let idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
                                if self.is_eval {
                                    self.emit(Opcode::SetGlobalVar);
                                } else {
                                    self.emit(Opcode::SetGlobal);
                                }
                                self.emit_u32(idx);
                                self.emit_u16(slot);
                            }
                        }
                        Some(VarLocation::Upvalue(slot)) => {
                            self.emit(Opcode::SetUpvalue);
                            self.emit_u16(slot);
                            self.emit_u16(current);
                        }
                        None => {
                            let idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
                            if self.is_eval {
                                self.emit(Opcode::SetGlobalVar);
                            } else {
                                self.emit(Opcode::SetGlobal);
                            }
                            self.emit_u32(idx);
                            self.emit_u16(current);
                        }
                    }

                    let end_pos = self.code.len();
                    self.patch_jump(jump_to_end, end_pos);
                    if current != rhs {
                        self.free_register(rhs);
                    }
                    return Ok(current);
                }

                if let Some(opcode) = is_compound {
                    let resolved = self.resolve_var(name, ctx);
                    let mut needs_tdz = false;
                    if let Some(entry) = self.lookup_var(name) {
                        needs_tdz = entry.kind != VariableKind::Var && !entry.initialized;
                    } else if let Some(parent_var) = self.parent_vars.get(name) {
                        if !parent_var.is_inherited {
                            needs_tdz =
                                parent_var.kind != VariableKind::Var && !parent_var.initialized;
                        }
                    }
                    let current = match resolved {
                        Some(VarLocation::Local(slot)) => {
                            if needs_tdz {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(slot);
                            }
                            let current = self.alloc_register();
                            self.emit(Opcode::Move);
                            self.emit_u16(current);
                            self.emit_u16(slot);
                            current
                        }
                        Some(VarLocation::Upvalue(slot)) => {
                            let current = self.alloc_register();
                            self.emit(Opcode::GetUpvalue);
                            self.emit_u16(current);
                            self.emit_u16(slot);
                            if needs_tdz {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(current);
                            }
                            current
                        }
                        None => {
                            let atom = ctx.intern(name);
                            let idx = self.add_constant(JSValue::new_string(atom));
                            let current = self.alloc_register();
                            self.emit(Opcode::GetGlobal);
                            self.emit_u16(current);
                            self.emit_u32(idx);
                            if !self.suppress_ref_error {
                                self.emit(Opcode::CheckRef);
                                self.emit_u16(current);
                                self.emit_u32(idx);
                            }
                            current
                        }
                    };
                    let val = self.gen_expression(&assign.right, ctx)?;
                    self.emit(opcode);
                    self.emit_u16(current);
                    self.emit_u16(current);
                    self.emit_u16(val);
                    self.free_register(val);
                    match resolved {
                        Some(VarLocation::Local(slot)) => {
                            if slot != current {
                                self.emit(Opcode::Move);
                                self.emit_u16(slot);
                                self.emit_u16(current);
                            }
                            if self.is_global_binding(name) {
                                let idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
                                if self.is_eval {
                                    self.emit(Opcode::SetGlobalVar);
                                } else {
                                    self.emit(Opcode::SetGlobal);
                                }
                                self.emit_u32(idx);
                                self.emit_u16(current);
                            }
                        }
                        Some(VarLocation::Upvalue(slot)) => {
                            self.emit(Opcode::SetUpvalueDirect);
                            self.emit_u16(slot);
                            self.emit_u16(current);
                        }
                        None => {
                            let idx = self.add_constant(JSValue::new_string(ctx.intern(name)));
                            if self.is_eval {
                                self.emit(Opcode::SetGlobalVar);
                            } else {
                                self.emit(Opcode::SetGlobal);
                            }
                            self.emit_u32(idx);
                            self.emit_u16(current);
                        }
                    }
                    return Ok(current);
                }

                let val = if assign.lhs_parenthesized {
                    self.gen_expression(&assign.right, ctx)?
                } else {
                    self.gen_expression_with_assign_name(&assign.right, name, ctx)?
                };
                match self.resolve_var(name, ctx) {
                    Some(VarLocation::Local(slot)) => {
                        // Check TDZ for let/const variables on assignment
                        if let Some(entry) = self.lookup_var(name) {
                            if entry.kind != VariableKind::Var && !entry.initialized {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(slot);
                            }
                        }
                        if let Some(opcode) = is_compound {
                            self.emit(opcode);
                            self.emit_u16(slot);
                            self.emit_u16(slot);
                            self.emit_u16(val);
                        } else if slot != val {
                            self.emit(Opcode::Move);
                            self.emit_u16(slot);
                            self.emit_u16(val);
                        }

                        if self.is_global_binding(name) {
                            let atom = ctx.intern(name);
                            let idx = self.add_constant(JSValue::new_string(atom));
                            if self.is_eval {
                                self.emit(Opcode::SetGlobalVar);
                            } else {
                                self.emit(Opcode::SetGlobal);
                            }
                            self.emit_u32(idx);
                            self.emit_u16(slot);
                        }
                        Ok(slot)
                    }
                    Some(VarLocation::Upvalue(slot)) => {
                        // Check TDZ for upvalue let/const variables
                        let mut needs_tdz = false;
                        if let Some(entry) = self.lookup_var(name) {
                            needs_tdz = entry.kind != VariableKind::Var && !entry.initialized;
                        } else if let Some(parent_var) = self.parent_vars.get(name) {
                            if !parent_var.is_inherited {
                                needs_tdz =
                                    parent_var.kind != VariableKind::Var && !parent_var.initialized;
                            }
                        }
                        if let Some(opcode) = is_compound {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetUpvalue);
                            self.emit_u16(tmp);
                            self.emit_u16(slot);
                            if needs_tdz {
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(tmp);
                            }
                            self.emit(opcode);
                            self.emit_u16(tmp);
                            self.emit_u16(tmp);
                            self.emit_u16(val);
                            self.emit(Opcode::SetUpvalue);
                            self.emit_u16(slot);
                            self.emit_u16(tmp);
                            self.free_register(val);
                            return Ok(tmp);
                        } else {
                            if needs_tdz {
                                let tmp = self.alloc_register();
                                self.emit(Opcode::GetUpvalue);
                                self.emit_u16(tmp);
                                self.emit_u16(slot);
                                self.emit(Opcode::CheckTdz);
                                self.emit_u16(tmp);
                                self.free_register(tmp);
                            }
                            self.emit(Opcode::SetUpvalueDirect);
                            self.emit_u16(slot);
                            self.emit_u16(val);
                        }
                        Ok(val)
                    }
                    None => {
                        let atom = ctx.intern(name);
                        let idx = self.add_constant(JSValue::new_string(atom));
                        if let Some(opcode) = is_compound {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetGlobal);
                            self.emit_u16(tmp);
                            self.emit_u32(idx);
                            if !self.suppress_ref_error {
                                self.emit(Opcode::CheckRef);
                                self.emit_u16(tmp);
                                self.emit_u32(idx);
                            }
                            self.emit(opcode);
                            self.emit_u16(tmp);
                            self.emit_u16(tmp);
                            self.emit_u16(val);
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(idx);
                            self.emit_u16(tmp);
                            self.free_register(val);
                            return Ok(tmp);
                        } else {
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(idx);
                            self.emit_u16(val);
                            Ok(val)
                        }
                    }
                }
            }
            AssignmentTarget::MemberExpression(member) => {
                if matches!(
                    assign.op,
                    AssignOp::LogicalAndAssign
                        | AssignOp::LogicalOrAssign
                        | AssignOp::NullishAssign
                ) {
                    return self.gen_logical_member_assignment(member, assign, ctx);
                }
                let is_super = matches!(&*member.object, Expression::Super);
                let obj = if is_super {
                    let base = self.alloc_register();
                    self.emit(Opcode::GetSuperBase);
                    self.emit_u16(base);
                    base
                } else {
                    self.gen_expression(&member.object, ctx)?
                };
                match &member.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        if is_super {
                            let key = self.alloc_register();
                            let key_idx = self.add_constant(JSValue::new_string(atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key);
                            self.emit_u32(key_idx);
                            if let Some(opcode) = is_compound {
                                let tmp = self.alloc_register();
                                self.emit(Opcode::GetSuperProp);
                                self.emit_u16(tmp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                let val = self.gen_expression(&assign.right, ctx)?;
                                self.emit(opcode);
                                self.emit_u16(tmp);
                                self.emit_u16(tmp);
                                self.emit_u16(val);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(val);
                                self.free_register(key);
                                self.free_register(obj);
                                Ok(tmp)
                            } else {
                                let val = self.gen_expression(&assign.right, ctx)?;
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(val);
                                self.free_register(key);
                                self.free_register(obj);
                                Ok(val)
                            }
                        } else if let Some(opcode) = is_compound {
                            self.emit(Opcode::CheckObjectCoercible);
                            self.emit_u16(obj);
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetNamedProp);
                            self.emit_u16(tmp);
                            self.emit_u16(obj);
                            self.emit_u32(atom.0);
                            let val = self.gen_expression(&assign.right, ctx)?;
                            self.emit(opcode);
                            self.emit_u16(tmp);
                            self.emit_u16(tmp);
                            self.emit_u16(val);
                            self.emit(Opcode::SetNamedProp);
                            self.emit_u16(obj);
                            self.emit_u16(tmp);
                            self.emit_u32(atom.0);
                            self.free_register(val);
                            self.free_register(obj);
                            Ok(tmp)
                        } else {
                            let val = self.gen_expression(&assign.right, ctx)?;
                            self.emit(Opcode::SetNamedProp);
                            self.emit_u16(obj);
                            self.emit_u16(val);
                            self.emit_u32(atom.0);
                            self.free_register(obj);
                            Ok(val)
                        }
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        if is_super {
                            self.emit(Opcode::GetSuperBase);
                            self.emit_u16(obj);
                            if let Some(opcode) = is_compound {
                                let tmp = self.alloc_register();
                                self.emit(Opcode::GetSuperProp);
                                self.emit_u16(tmp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                let val = self.gen_expression(&assign.right, ctx)?;
                                self.emit(opcode);
                                self.emit_u16(tmp);
                                self.emit_u16(tmp);
                                self.emit_u16(val);
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(tmp);
                                self.free_register(val);
                                self.free_register(key);
                                self.free_register(obj);
                                Ok(tmp)
                            } else {
                                let val = self.gen_expression(&assign.right, ctx)?;
                                self.emit(Opcode::SetSuperProp);
                                self.emit_u16(obj);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.emit_u16(val);
                                self.free_register(key);
                                self.free_register(obj);
                                Ok(val)
                            }
                        } else if let Some(opcode) = is_compound {
                            self.emit(Opcode::CheckObjectCoercible);
                            self.emit_u16(obj);
                            self.emit(Opcode::ToPropertyKey);
                            self.emit_u16(key);
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetField);
                            self.emit_u16(tmp);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            let val = self.gen_expression(&assign.right, ctx)?;
                            self.emit(opcode);
                            self.emit_u16(tmp);
                            self.emit_u16(tmp);
                            self.emit_u16(val);
                            self.emit(Opcode::SetField);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(tmp);
                            self.free_register(val);
                            self.free_register(key);
                            self.free_register(obj);
                            Ok(tmp)
                        } else {
                            let val = self.gen_expression(&assign.right, ctx)?;
                            self.emit(Opcode::SetField);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(val);
                            self.free_register(key);
                            self.free_register(obj);
                            Ok(val)
                        }
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        if let Some(opcode) = is_compound {
                            self.emit(Opcode::CheckObjectCoercible);
                            self.emit_u16(obj);
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetPrivate);
                            self.emit_u16(tmp);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(owner_reg);
                            let val = self.gen_expression(&assign.right, ctx)?;
                            self.emit(opcode);
                            self.emit_u16(tmp);
                            self.emit_u16(tmp);
                            self.emit_u16(val);
                            self.emit(Opcode::SetPrivate);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(tmp);
                            self.emit_u16(owner_reg);
                            self.free_register(val);
                            self.free_register(key);
                            self.free_register(obj);
                            if owner_temp {
                                self.free_register(owner_reg);
                            }
                            Ok(tmp)
                        } else {
                            let val = self.gen_expression(&assign.right, ctx)?;
                            if assign.private_field_init {
                                self.emit(Opcode::DefinePrivateField);
                            } else {
                                self.emit(Opcode::SetPrivate);
                            }
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(val);
                            self.emit_u16(owner_reg);
                            self.free_register(key);
                            self.free_register(obj);
                            if owner_temp {
                                self.free_register(owner_reg);
                            }
                            Ok(val)
                        }
                    }
                }
            }
            _ => {
                let val = self.gen_expression(&assign.right, ctx)?;
                self.gen_assignment_target_destructure(&assign.left, val, ctx)?;
                Ok(val)
            }
        }
    }

    fn gen_logical_member_assignment(
        &mut self,
        member: &MemberExpression,
        assign: &AssignmentExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let obj = self.gen_expression(&member.object, ctx)?;
        let reference = match &member.property {
            MemberProperty::Identifier(name) => {
                self.emit(Opcode::CheckObjectCoercible);
                self.emit_u16(obj);
                DestructuringReference::Named {
                    object: obj,
                    atom: ctx.intern(name),
                }
            }
            MemberProperty::Computed(expr) => {
                let key = self.gen_expression(expr, ctx)?;
                self.emit(Opcode::CheckObjectCoercible);
                self.emit_u16(obj);
                self.emit(Opcode::ToPropertyKey);
                self.emit_u16(key);
                DestructuringReference::Computed { object: obj, key }
            }
            MemberProperty::PrivateIdentifier(name) => {
                let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                self.emit(Opcode::CheckObjectCoercible);
                self.emit_u16(obj);
                let key_idx = self.intern_and_add_const(ctx, name);
                let key = self.alloc_register();
                self.emit(Opcode::LoadConst);
                self.emit_u16(key);
                self.emit_u32(key_idx);
                DestructuringReference::Private {
                    object: obj,
                    key,
                    owner_reg,
                    owner_temp,
                }
            }
        };

        let old = self.alloc_register();
        match &reference {
            DestructuringReference::Named { object, atom } => {
                self.emit(Opcode::GetNamedProp);
                self.emit_u16(old);
                self.emit_u16(*object);
                self.emit_u32(atom.0);
            }
            DestructuringReference::Computed { object, key } => {
                self.emit(Opcode::GetField);
                self.emit_u16(old);
                self.emit_u16(*object);
                self.emit_u16(*key);
            }
            DestructuringReference::Private {
                object,
                key,
                owner_reg,
                ..
            } => {
                self.emit(Opcode::GetPrivate);
                self.emit_u16(old);
                self.emit_u16(*object);
                self.emit_u16(*key);
                self.emit_u16(*owner_reg);
            }
            DestructuringReference::Identifier { .. } => {}
        }

        let mut assign_jumps = Vec::new();
        match assign.op {
            AssignOp::LogicalAndAssign => {
                self.emit(Opcode::JumpIf);
                self.emit_u16(old);
                assign_jumps.push(self.code.len());
                self.emit_i32(0);
            }
            AssignOp::LogicalOrAssign => {
                self.emit(Opcode::JumpIfNot);
                self.emit_u16(old);
                assign_jumps.push(self.code.len());
                self.emit_i32(0);
            }
            AssignOp::NullishAssign => {
                let null_reg = self.alloc_register();
                self.emit(Opcode::LoadNull);
                self.emit_u16(null_reg);
                let cmp_reg = self.alloc_register();
                self.emit(Opcode::StrictEq);
                self.emit_u16(cmp_reg);
                self.emit_u16(old);
                self.emit_u16(null_reg);
                self.emit(Opcode::JumpIf);
                self.emit_u16(cmp_reg);
                assign_jumps.push(self.code.len());
                self.emit_i32(0);
                let undef_reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(undef_reg);
                self.emit(Opcode::StrictEq);
                self.emit_u16(cmp_reg);
                self.emit_u16(old);
                self.emit_u16(undef_reg);
                self.emit(Opcode::JumpIf);
                self.emit_u16(cmp_reg);
                assign_jumps.push(self.code.len());
                self.emit_i32(0);
                self.free_register(undef_reg);
                self.free_register(cmp_reg);
                self.free_register(null_reg);
            }
            _ => {}
        }

        self.emit(Opcode::Jump);
        let skip_patch = self.code.len();
        self.emit_i32(0);

        let assign_pos = self.code.len();
        for patch in assign_jumps {
            self.patch_jump(patch, assign_pos);
        }

        let rhs = self.gen_expression(&assign.right, ctx)?;
        self.gen_store_destructuring_reference(reference, rhs, ctx)?;
        let result = self.alloc_register();
        self.emit(Opcode::Move);
        self.emit_u16(result);
        self.emit_u16(rhs);
        self.free_register(rhs);
        self.emit(Opcode::Jump);
        let done_patch = self.code.len();
        self.emit_i32(0);

        let old_pos = self.code.len();
        self.patch_jump(skip_patch, old_pos);
        self.emit(Opcode::Move);
        self.emit_u16(result);
        self.emit_u16(old);
        let done_pos = self.code.len();
        self.patch_jump(done_patch, done_pos);
        self.free_register(old);
        Ok(result)
    }

    fn gen_destructuring_reference(
        &mut self,
        target: &AssignmentTarget,
        ctx: &mut JSContext,
    ) -> Result<Option<DestructuringReference>, String> {        match target {
            AssignmentTarget::Identifier(name) => Ok(Some(DestructuringReference::Identifier {
                name: name.clone(),
                location: self.resolve_var(name, ctx),
            })),
            AssignmentTarget::MemberExpression(member) => {
                let object = self.gen_expression(&member.object, ctx)?;
                match &member.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        Ok(Some(DestructuringReference::Named { object, atom }))
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        Ok(Some(DestructuringReference::Computed { object, key }))
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        Ok(Some(DestructuringReference::Private {
                            object,
                            key,
                            owner_reg,
                            owner_temp,
                        }))
                    }
                }
            }
            AssignmentTarget::ComputedMember(object_expr, key_expr) => {
                let object = self.gen_expression(object_expr, ctx)?;
                let key = self.gen_expression(key_expr, ctx)?;
                Ok(Some(DestructuringReference::Computed { object, key }))
            }
            _ => Ok(None),
        }
    }

    fn gen_store_destructuring_reference(
        &mut self,
        reference: DestructuringReference,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match reference {
            DestructuringReference::Identifier { name, location } => {
                if self.binding_is_const(&name) {
                    self.emit_throw_const_assignment(&name, ctx);
                    return Ok(());
                }
                if self.binding_is_function_name(&name) {
                    if self.is_strict {
                        self.emit_throw_const_assignment(&name, ctx);
                    }
                    return Ok(());
                }
                match location {
                    Some(VarLocation::Local(slot)) => {
                        if self.binding_needs_tdz_check(&name) {
                            self.emit(Opcode::CheckTdz);
                            self.emit_u16(slot);
                        }
                        if slot != value_reg {
                            self.emit(Opcode::Move);
                            self.emit_u16(slot);
                            self.emit_u16(value_reg);
                        }
                        if self.is_global_binding(&name) {
                            let global_idx = self.intern_and_add_const(ctx, &name);
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(global_idx);
                            self.emit_u16(slot);
                        }
                        Ok(())
                    }
                    Some(VarLocation::Upvalue(slot)) => {
                        if self.binding_needs_tdz_check(&name) {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetUpvalue);
                            self.emit_u16(tmp);
                            self.emit_u16(slot);
                            self.emit(Opcode::CheckTdz);
                            self.emit_u16(tmp);
                            self.free_register(tmp);
                        }
                        self.emit(Opcode::SetUpvalue);
                        self.emit_u16(slot);
                        self.emit_u16(value_reg);
                        Ok(())
                    }
                    None => {
                        let global_idx = self.intern_and_add_const(ctx, &name);
                        self.emit(Opcode::SetGlobal);
                        self.emit_u32(global_idx);
                        self.emit_u16(value_reg);
                        Ok(())
                    }
                }
            }
            DestructuringReference::Named { object, atom } => {
                self.emit(Opcode::SetNamedProp);
                self.emit_u16(object);
                self.emit_u16(value_reg);
                self.emit_u32(atom.0);
                self.free_register(object);
                Ok(())
            }
            DestructuringReference::Computed { object, key } => {
                self.emit(Opcode::SetField);
                self.emit_u16(object);
                self.emit_u16(key);
                self.emit_u16(value_reg);
                self.free_register(key);
                self.free_register(object);
                Ok(())
            }
            DestructuringReference::Private {
                object,
                key,
                owner_reg,
                owner_temp,
            } => {
                self.emit(Opcode::SetPrivate);
                self.emit_u16(object);
                self.emit_u16(key);
                self.emit_u16(value_reg);
                self.emit_u16(owner_reg);
                if owner_temp {
                    self.free_register(owner_reg);
                }
                self.free_register(key);
                self.free_register(object);
                Ok(())
            }
        }
    }

    fn gen_assignment_target_destructure(
        &mut self,
        target: &AssignmentTarget,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        match target {
            AssignmentTarget::Identifier(name) => {
                if self.binding_is_const(name) {
                    self.emit_throw_const_assignment(name, ctx);
                    return Ok(());
                }
                if self.binding_is_function_name(name) {
                    if self.is_strict {
                        self.emit_throw_const_assignment(name, ctx);
                    }
                    return Ok(());
                }
                match self.resolve_var(name, ctx) {
                    Some(VarLocation::Local(slot)) => {
                        if self.binding_needs_tdz_check(name) {
                            self.emit(Opcode::CheckTdz);
                            self.emit_u16(slot);
                        }
                        if slot != value_reg {
                            self.emit(Opcode::Move);
                            self.emit_u16(slot);
                            self.emit_u16(value_reg);
                        }
                        if self.is_global_binding(name) {
                            let global_idx = self.intern_and_add_const(ctx, name);
                            self.emit(Opcode::SetGlobal);
                            self.emit_u32(global_idx);
                            self.emit_u16(slot);
                        }
                        Ok(())
                    }
                    Some(VarLocation::Upvalue(slot)) => {
                        if self.binding_needs_tdz_check(name) {
                            let tmp = self.alloc_register();
                            self.emit(Opcode::GetUpvalue);
                            self.emit_u16(tmp);
                            self.emit_u16(slot);
                            self.emit(Opcode::CheckTdz);
                            self.emit_u16(tmp);
                            self.free_register(tmp);
                        }
                        self.emit(Opcode::SetUpvalue);
                        self.emit_u16(slot);
                        self.emit_u16(value_reg);
                        Ok(())
                    }
                    None => {
                        let global_idx = self.intern_and_add_const(ctx, name);
                        self.emit(Opcode::SetGlobal);
                        self.emit_u32(global_idx);
                        self.emit_u16(value_reg);
                        Ok(())
                    }
                }
            }
            AssignmentTarget::MemberExpression(member) => {
                let obj = self.gen_expression(&member.object, ctx)?;
                match &member.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        self.emit(Opcode::SetNamedProp);
                        self.emit_u16(obj);
                        self.emit_u16(value_reg);
                        self.emit_u32(atom.0);
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::SetField);
                        self.emit_u16(obj);
                        self.emit_u16(key);
                        self.emit_u16(value_reg);
                        self.free_register(key);
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        self.emit(Opcode::SetPrivate);
                        self.emit_u16(obj);
                        self.emit_u16(key);
                        self.emit_u16(value_reg);
                        self.emit_u16(owner_reg);
                        if owner_temp {
                            self.free_register(owner_reg);
                        }
                        self.free_register(key);
                    }
                }
                self.free_register(obj);
                Ok(())
            }
            AssignmentTarget::ComputedMember(obj_expr, key_expr) => {
                let obj = self.gen_expression(obj_expr, ctx)?;
                let key = self.gen_expression(key_expr, ctx)?;
                self.emit(Opcode::SetField);
                self.emit_u16(obj);
                self.emit_u16(key);
                self.emit_u16(value_reg);
                self.free_register(key);
                self.free_register(obj);
                Ok(())
            }
            AssignmentTarget::ArrayPattern(arr) => {
                self.gen_array_destructuring_target(arr, value_reg, ctx)
            }
            AssignmentTarget::ObjectPattern(obj) => {
                self.gen_object_destructuring_target(obj, value_reg, ctx)
            }
            AssignmentTarget::AssignmentPattern(ap) => {
                let binding_name = match &*ap.left {
                    AssignmentTarget::Identifier(name) => Some(name.as_str()),
                    _ => None,
                };
                self.gen_default_value_assignment(&ap.right, value_reg, binding_name, ctx)?;
                self.gen_assignment_target_destructure(&ap.left, value_reg, ctx)
            }
            AssignmentTarget::RestElement(_) => {
                Err("Rest element not valid in assignment target".to_string())
            }
            AssignmentTarget::OptionalMemberExpression(_) => {
                Err("Optional member expression not valid in assignment target".to_string())
            }
        }
    }

    fn gen_array_destructuring_target(
        &mut self,
        arr: &ArrayPattern,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.emit(Opcode::CheckObjectCoercible);
        self.emit_u16(value_reg);
        let iter_reg = self.alloc_register();
        self.emit(Opcode::GetIterator);
        self.emit_u16(iter_reg);
        self.emit_u16(value_reg);

        let done_reg = self.alloc_register();
        self.emit(Opcode::LoadFalse);
        self.emit_u16(done_reg);

        let (catch_pc_pos, finally_pc_pos) = self.begin_iterator_close_region();

        let len = arr.elements.len();
        for elem in arr.elements.iter() {
            if let Some(PatternElement::RestElement(rest)) = elem {
                let reference = self.gen_destructuring_reference(&rest.argument, ctx)?;
                let rest_reg = self.alloc_register();
                self.emit(Opcode::NewArray);
                self.emit_u16(rest_reg);
                self.emit_u16(0);

                let loop_start = self.code.len();
                self.emit(Opcode::JumpIf);
                self.emit_u16(done_reg);
                let done_exit = self.code.len();
                self.emit_i32(0);

                let val_for_rest = self.alloc_register();
                let done_for_rest = self.alloc_register();
                self.emit(Opcode::LoadTrue);
                self.emit_u16(done_reg);
                self.emit(Opcode::IteratorNext);
                self.emit_u16(val_for_rest);
                self.emit_u16(done_for_rest);
                self.emit_u16(iter_reg);
                self.emit(Opcode::Move);
                self.emit_u16(done_reg);
                self.emit_u16(done_for_rest);
                self.free_register(done_for_rest);

                self.emit(Opcode::JumpIf);
                self.emit_u16(done_reg);
                let next_done_exit = self.code.len();
                self.emit_i32(0);

                self.emit(Opcode::ArrayPush);
                self.emit_u16(rest_reg);
                self.emit_u16(val_for_rest);
                self.free_register(val_for_rest);
                self.emit_backward_jump(loop_start);

                let loop_end = self.code.len();
                self.patch_jump(done_exit, loop_end);
                self.patch_jump(next_done_exit, loop_end);

                if let Some(reference) = reference {
                    self.gen_store_destructuring_reference(reference, rest_reg, ctx)?;
                } else {
                    self.gen_assignment_target_destructure(&rest.argument, rest_reg, ctx)?;
                }
                self.free_register(rest_reg);
                break;
            }

            let val_reg = self.alloc_register();
            self.emit(Opcode::LoadUndefined);
            self.emit_u16(val_reg);

            let reference = match elem {
                Some(PatternElement::Pattern(target)) => {
                    self.gen_destructuring_reference(target, ctx)?
                }
                Some(PatternElement::AssignmentPattern(ap)) => {
                    self.gen_destructuring_reference(&ap.left, ctx)?
                }
                _ => None,
            };

            self.emit(Opcode::JumpIf);
            self.emit_u16(done_reg);
            let skip_next = self.code.len();
            self.emit_i32(0);

            let step_done_reg = self.alloc_register();
            self.emit(Opcode::LoadTrue);
            self.emit_u16(done_reg);
            if elem.is_none() {
                self.emit(Opcode::IteratorStep);
                self.emit_u16(step_done_reg);
                self.emit_u16(iter_reg);
            } else {
                self.emit(Opcode::IteratorNext);
                self.emit_u16(val_reg);
                self.emit_u16(step_done_reg);
                self.emit_u16(iter_reg);
            }
            self.emit(Opcode::Move);
            self.emit_u16(done_reg);
            self.emit_u16(step_done_reg);
            self.free_register(step_done_reg);

            let after_next = self.code.len();
            self.patch_jump(skip_next, after_next);

            if let Some(elem) = elem {
                match elem {
                    PatternElement::Pattern(target) => {
                        if let Some(reference) = reference {
                            self.gen_store_destructuring_reference(reference, val_reg, ctx)?;
                        } else {
                            self.gen_assignment_target_destructure(target, val_reg, ctx)?;
                        }
                    }
                    PatternElement::AssignmentPattern(ap) => {
                        let binding_name = match &*ap.left {
                            AssignmentTarget::Identifier(name) => Some(name.as_str()),
                            _ => None,
                        };
                        self.gen_default_value_assignment(
                            &ap.right,
                            val_reg,
                            binding_name,
                            ctx,
                        )?;
                        if let Some(reference) = reference {
                            self.gen_store_destructuring_reference(reference, val_reg, ctx)?;
                        } else {
                            self.gen_assignment_target_destructure(&ap.left, val_reg, ctx)?;
                        }
                    }
                    PatternElement::RestElement(_) => {}
                }
            }
            self.free_register(val_reg);
        }

        self.end_iterator_close_region(
            catch_pc_pos,
            finally_pc_pos,
            iter_reg,
            done_reg,
            len == 0,
        );
        self.free_register(done_reg);
        self.free_register(iter_reg);
        Ok(())
    }

    fn gen_object_destructuring_target(
        &mut self,
        obj: &ObjectPattern,
        value_reg: u16,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        self.emit(Opcode::CheckObjectCoercible);
        self.emit_u16(value_reg);
        let has_rest = obj
            .properties
            .iter()
            .any(|p| matches!(p, ObjectPatternProperty::RestElement(_)));
        let keys_reg = if has_rest {
            let r = self.alloc_register();
            self.emit(Opcode::NewArray);
            self.emit_u16(r);
            self.emit_u16(0);
            Some(r)
        } else {
            None
        };
        for prop in &obj.properties {
            match prop {
                ObjectPatternProperty::Property {
                    key,
                    value,
                    computed,
                    ..
                } => {
                    let key_reg = if *computed {
                        match key {
                            PropertyKey::Computed(expr) => {
                                let key_reg = self.gen_expression(expr, ctx)?;
                                self.emit(Opcode::ToPropertyKey);
                                self.emit_u16(key_reg);
                                key_reg
                            }
                            _ => return Err("Expected computed key".to_string()),
                        }
                    } else {
                        match key {
                            PropertyKey::Identifier(k)
                            | PropertyKey::Literal(Literal::String(k, _, _)) => {
                                let idx = self.intern_and_add_const(ctx, k);
                                let kreg = self.alloc_register();
                                self.emit(Opcode::LoadConst);
                                self.emit_u16(kreg);
                                self.emit_u32(idx);
                                kreg
                            }
                            PropertyKey::Literal(Literal::Number(n)) => {
                                let kreg = self.alloc_register();
                                if *n >= i32::MIN as f64
                                    && *n <= i32::MAX as f64
                                    && n.fract() == 0.0
                                {
                                    self.emit_int_const(kreg, *n as i32);
                                } else {
                                    let cidx = self.add_constant(JSValue::new_float(*n));
                                    self.emit(Opcode::LoadConst);
                                    self.emit_u16(kreg);
                                    self.emit_u32(cidx);
                                }
                                kreg
                            }
                            _ => {
                                return Err("SyntaxError: Invalid property key".to_string())
                            }
                        }
                    };

                    let reference = match value {
                        AssignmentTarget::AssignmentPattern(ap) => {
                            self.gen_destructuring_reference(&ap.left, ctx)?
                        }
                        target => self.gen_destructuring_reference(target, ctx)?,
                    };

                    let elem_reg = self.alloc_register();
                    self.emit(Opcode::GetProp);
                    self.emit_u16(elem_reg);
                    self.emit_u16(value_reg);
                    self.emit_u16(key_reg);
                    if let Some(kreg) = keys_reg {
                        self.emit(Opcode::ArrayPush);
                        self.emit_u16(kreg);
                        self.emit_u16(key_reg);
                    }
                    self.free_register(key_reg);

                    match value {
                        AssignmentTarget::AssignmentPattern(ap) => {
                            let binding_name = match &*ap.left {
                                AssignmentTarget::Identifier(name) => Some(name.as_str()),
                                _ => None,
                            };
                            self.gen_default_value_assignment(
                                &ap.right,
                                elem_reg,
                                binding_name,
                                ctx,
                            )?;
                            if let Some(reference) = reference {
                                self.gen_store_destructuring_reference(
                                    reference, elem_reg, ctx,
                                )?;
                            } else {
                                self.gen_assignment_target_destructure(
                                    &ap.left, elem_reg, ctx,
                                )?;
                            }
                        }
                        target => {
                            if let Some(reference) = reference {
                                self.gen_store_destructuring_reference(
                                    reference, elem_reg, ctx,
                                )?;
                            } else {
                                self.gen_assignment_target_destructure(target, elem_reg, ctx)?;
                            }
                        }
                    }
                    self.free_register(elem_reg);
                }
                ObjectPatternProperty::RestElement(rest) => {
                    let reference = self.gen_destructuring_reference(&rest.argument, ctx)?;
                    let rest_reg = self.alloc_register();
                    self.emit(Opcode::NewObject);
                    self.emit_u16(rest_reg);
                    if let Some(kreg) = keys_reg {
                        self.emit(Opcode::ObjectRest);
                        self.emit_u16(rest_reg);
                        self.emit_u16(value_reg);
                        self.emit_u16(kreg);
                    } else {
                        self.emit(Opcode::ObjectSpread);
                        self.emit_u16(rest_reg);
                        self.emit_u16(value_reg);
                    }
                    if let Some(reference) = reference {
                        self.gen_store_destructuring_reference(reference, rest_reg, ctx)?;
                    } else {
                        self.gen_assignment_target_destructure(&rest.argument, rest_reg, ctx)?;
                    }
                    self.free_register(rest_reg);
                }
            }
        }
        if let Some(kreg) = keys_reg {
            self.free_register(kreg);
        }
        Ok(())
    }

    fn gen_default_value_assignment(
        &mut self,
        default_expr: &Expression,
        value_reg: u16,
        binding_name: Option<&str>,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let cmp_reg = self.alloc_register();
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(cmp_reg);

        self.emit(Opcode::StrictNeq);
        self.emit_u16(cmp_reg);
        self.emit_u16(value_reg);
        self.emit_u16(cmp_reg);

        self.emit(Opcode::JumpIf);
        self.emit_u16(cmp_reg);
        let skip_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(cmp_reg);

        let default_reg = if let Some(name) = binding_name {
            self.gen_expression_with_assign_name(default_expr, name, ctx)?
        } else {
            self.gen_expression(default_expr, ctx)?
        };
        if default_reg != value_reg {
            self.emit(Opcode::Move);
            self.emit_u16(value_reg);
            self.emit_u16(default_reg);
        }
        self.free_register(default_reg);

        let after_pos = self.code.len();
        self.patch_jump(skip_patch, after_pos);

        Ok(())
    }

    fn gen_conditional_expression(
        &mut self,
        cond: &ConditionalExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        if self.p2_fold_profile.allow_ternary_bool() {
            if let Expression::Literal(Literal::Boolean(test_bool)) = &*cond.test {
                if *test_bool {
                    return self.gen_expression(&cond.consequent, ctx);
                }
                return self.gen_expression(&cond.alternate, ctx);
            }
        }

        if self.p2_fold_profile.allow_aggressive_truthy() {
            if let Expression::Literal(lit) = &*cond.test {
                if let Some(truthy) = Self::literal_truthiness(lit) {
                    if truthy {
                        return self.gen_expression(&cond.consequent, ctx);
                    }
                    return self.gen_expression(&cond.alternate, ctx);
                }
            }
        }

        let test = self.gen_expression(&cond.test, ctx)?;

        if self.opt_branch_result_prealloc {
            let result = if self.reserved_slots.contains(&test) {
                let dst = self.alloc_register();
                self.emit(Opcode::Move);
                self.emit_u16(dst);
                self.emit_u16(test);
                dst
            } else {
                test
            };

            self.emit(Opcode::JumpIfNot);
            self.emit_u16(test);
            let else_jump = self.code.len();
            self.emit_i32(0);

            let cons = self.gen_expression(&cond.consequent, ctx)?;
            if cons != result {
                self.emit(Opcode::Move);
                self.emit_u16(result);
                self.emit_u16(cons);
                self.free_register(cons);
            }

            self.emit(Opcode::Jump);
            let end_jump = self.code.len();
            self.emit_i32(0);

            let else_pos = self.code.len();
            self.patch_jump(else_jump, else_pos);

            let alt = self.gen_expression(&cond.alternate, ctx)?;
            if alt != result {
                self.emit(Opcode::Move);
                self.emit_u16(result);
                self.emit_u16(alt);
                self.free_register(alt);
            }

            let end_pos = self.code.len();
            self.patch_jump(end_jump, end_pos);
            return Ok(result);
        }

        self.emit(Opcode::JumpIfNot);
        self.emit_u16(test);
        let else_jump = self.code.len();
        self.emit_i32(0);

        let cons = self.gen_expression(&cond.consequent, ctx)?;

        let result = if self.reserved_slots.contains(&test) && cons != test {
            let dst = self.alloc_register();
            self.emit(Opcode::Move);
            self.emit_u16(dst);
            self.emit_u16(cons);
            self.free_register(cons);
            dst
        } else if cons != test {
            self.emit(Opcode::Move);
            self.emit_u16(test);
            self.emit_u16(cons);
            self.free_register(cons);
            test
        } else {
            test
        };

        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);

        let else_pos = self.code.len();
        self.patch_jump(else_jump, else_pos);

        let alt = self.gen_expression(&cond.alternate, ctx)?;
        if self.reserved_slots.contains(&test) && alt != result && alt != test {
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(alt);
            self.free_register(alt);
        } else if alt != result && alt != test {
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(alt);
            self.free_register(alt);
        }

        let end_pos = self.code.len();
        self.patch_jump(end_jump, end_pos);

        Ok(result)
    }

    fn flatten_chain(expr: &Expression, steps: &mut Vec<ChainStep>) -> Expression {
        match expr {
            Expression::MemberExpression(member) => {
                steps.push(ChainStep::Member {
                    property: member.property.clone(),
                    optional: false,
                });
                Self::flatten_chain(&member.object, steps)
            }
            Expression::OptionalMemberExpression(member) => {
                steps.push(ChainStep::Member {
                    property: member.property.clone(),
                    optional: true,
                });
                (*member.object).clone()
            }
            Expression::CallExpression(call) => {
                steps.push(ChainStep::Call {
                    arguments: call.arguments.clone(),
                    optional: false,
                });
                Self::flatten_chain(&call.callee, steps)
            }
            Expression::OptionalCallExpression(call) => {
                steps.push(ChainStep::Call {
                    arguments: call.arguments.clone(),
                    optional: true,
                });
                (*call.callee).clone()
            }
            other => other.clone(),
        }
    }

    fn gen_chain_expression(
        &mut self,
        chain: &Expression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let (value, this_reg) = self.gen_chain_expression_with_this(chain, ctx)?;
        self.free_register(this_reg);
        Ok(value)
    }

    fn gen_chain_expression_with_this(
        &mut self,
        chain: &Expression,
        ctx: &mut JSContext,
    ) -> Result<(u16, u16), String> {
        let mut steps = Vec::new();
        let base = Self::flatten_chain(chain, &mut steps);
        steps.reverse();

        let result = self.alloc_register();
        let base_is_super = matches!(base, Expression::Super);
        let mut base_this = None;
        if base_is_super {
            self.emit(Opcode::GetSuperBase);
            self.emit_u16(result);
        } else if let Expression::ChainExpression(inner) = &base {
            let (value, this) = self.gen_chain_expression_with_this(inner, ctx)?;
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(value);
            self.free_register(value);
            base_this = Some(this);
        } else if matches!(
            base,
            Expression::MemberExpression(_)
                | Expression::OptionalMemberExpression(_)
                | Expression::OptionalCallExpression(_)
        ) {
            let (value, this) = self.gen_chain_expression_with_this(&base, ctx)?;
            self.emit(Opcode::Move);
            self.emit_u16(result);
            self.emit_u16(value);
            self.free_register(value);
            base_this = Some(this);
        } else {
            let base_reg = self.gen_expression(&base, ctx)?;
            if base_reg != result {
                self.emit(Opcode::Move);
                self.emit_u16(result);
                self.emit_u16(base_reg);
                self.free_register(base_reg);
            }
        }

        let this_reg = match base_this {
            Some(reg) => reg,
            None => {
                let reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(reg);
                reg
            }
        };
        let mut has_this = base_this.is_some();

        let mut short_jumps = Vec::new();
        let mut first_member = true;

        for step in steps {
            match step {
                ChainStep::Member { property, optional } => {
                    if first_member && base_is_super {
                        first_member = false;
                        self.emit(Opcode::Move);
                        self.emit_u16(this_reg);
                        self.emit_u16(0);
                        has_this = true;
                        match property {
                            MemberProperty::Identifier(name) => {
                                let key = self.alloc_register();
                                let key_idx = self.intern_and_add_const(ctx, &name);
                                self.emit(Opcode::LoadConst);
                                self.emit_u16(key);
                                self.emit_u32(key_idx);
                                self.emit(Opcode::GetSuperProp);
                                self.emit_u16(result);
                                self.emit_u16(result);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.free_register(key);
                            }
                            MemberProperty::Computed(key_expr) => {
                                let key = self.gen_expression(&key_expr, ctx)?;
                                self.emit(Opcode::GetSuperProp);
                                self.emit_u16(result);
                                self.emit_u16(result);
                                self.emit_u16(0);
                                self.emit_u16(key);
                                self.free_register(key);
                            }
                            MemberProperty::PrivateIdentifier(_) => {
                                return Err(
                                    "Private identifiers cannot be accessed through super".to_string()
                                );
                            }
                        }
                        continue;
                    }
                    first_member = false;
                    let obj = self.alloc_register();
                    self.emit(Opcode::Move);
                    self.emit_u16(obj);
                    self.emit_u16(result);
                    if optional {
                        short_jumps.push(self.emit_jump_if_nullish(obj));
                    }
                    self.emit(Opcode::Move);
                    self.emit_u16(this_reg);
                    self.emit_u16(obj);
                    has_this = true;
                    match property {
                        MemberProperty::Identifier(name) => {
                            let atom = ctx.intern(&name);
                            self.emit(Opcode::GetNamedProp);
                            self.emit_u16(result);
                            self.emit_u16(obj);
                            self.emit_u32(atom.0);
                        }
                        MemberProperty::Computed(key_expr) => {
                            let key = self.gen_expression(&key_expr, ctx)?;
                            self.emit(Opcode::GetField);
                            self.emit_u16(result);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.free_register(key);
                        }
                        MemberProperty::PrivateIdentifier(name) => {
                            let (owner_reg, owner_temp) = self.gen_private_owner(&name, ctx)?;
                            let key_idx = self.intern_and_add_const(ctx, &name);
                            let key = self.alloc_register();
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key);
                            self.emit_u32(key_idx);
                            self.emit(Opcode::GetPrivate);
                            self.emit_u16(result);
                            self.emit_u16(obj);
                            self.emit_u16(key);
                            self.emit_u16(owner_reg);
                            if owner_temp {
                                self.free_register(owner_reg);
                            }
                            self.free_register(key);
                        }
                    }
                    self.free_register(obj);
                }
                ChainStep::Call { arguments, optional } => {
                    if optional {
                        short_jumps.push(self.emit_jump_if_nullish(result));
                    }
                    self.emit_chain_call(result, this_reg, has_this, &arguments, ctx)?;
                    self.emit(Opcode::LoadUndefined);
                    self.emit_u16(this_reg);
                    has_this = false;
                }
            }
        }

        self.emit(Opcode::Jump);
        let end_jump = self.code.len();
        self.emit_i32(0);
        let short_pos = self.code.len();
        for patch in short_jumps {
            self.patch_jump(patch, short_pos);
        }
        self.emit(Opcode::LoadUndefined);
        self.emit_u16(result);
        let end_pos = self.code.len();
        self.patch_jump(end_jump, end_pos);

        Ok((result, this_reg))
    }

    fn emit_chain_call(
        &mut self,
        dst: u16,
        this_reg: u16,
        has_this: bool,
        arguments: &[Argument],
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let has_spread = arguments.iter().any(|a| matches!(a, Argument::Spread(_)));
        if has_spread {
            let arr = self.alloc_register();
            self.emit(Opcode::NewArray);
            self.emit_u16(arr);
            self.emit_u16(0);
            for arg in arguments {
                match arg {
                    Argument::Expression(expr) => {
                        let val = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::ArrayPush);
                        self.emit_u16(arr);
                        self.emit_u16(val);
                        self.free_register(val);
                    }
                    Argument::Spread(expr) => {
                        let val = self.gen_expression(expr, ctx)?;
                        self.emit_spread_into_array(arr, val, ctx)?;
                    }
                }
            }
            if has_this {
                self.emit(Opcode::CallMethodSpread);
                self.emit_u16(dst);
                self.emit_u16(this_reg);
                self.emit_u16(dst);
                self.emit_u16(arr);
            } else {
                self.emit(Opcode::CallSpread);
                self.emit_u16(dst);
                self.emit_u16(dst);
                self.emit_u16(arr);
            }
            self.free_register(arr);
            return Ok(());
        }

        let mut arg_regs = Vec::new();
        for arg in arguments {
            if let Argument::Expression(expr) = arg {
                arg_regs.push(self.gen_expression(expr, ctx)?);
            }
        }
        if has_this {
            self.emit(Opcode::CallMethod);
            self.emit_u16(dst);
            self.emit_u16(this_reg);
            self.emit_u16(dst);
            self.emit_u16(arg_regs.len() as u16);
            for reg in &arg_regs {
                self.emit_u16(*reg);
            }
        } else {
            match arg_regs.len() {
                0 => {
                    self.emit(Opcode::Call0);
                    self.emit_u16(dst);
                    self.emit_u16(dst);
                }
                1 => {
                    self.emit(Opcode::Call1);
                    self.emit_u16(dst);
                    self.emit_u16(dst);
                    self.emit_u16(arg_regs[0]);
                }
                2 => {
                    self.emit(Opcode::Call2);
                    self.emit_u16(dst);
                    self.emit_u16(dst);
                    self.emit_u16(arg_regs[0]);
                    self.emit_u16(arg_regs[1]);
                }
                _ => {
                    self.emit(Opcode::Call);
                    self.emit_u16(dst);
                    self.emit_u16(dst);
                    self.emit_u16(arg_regs.len() as u16);
                    for reg in &arg_regs {
                        self.emit_u16(*reg);
                    }
                }
            }
        }
        for reg in arg_regs {
            self.free_register(reg);
        }
        Ok(())
    }

    fn gen_member_expression(
        &mut self,
        member: &MemberExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let is_super = matches!(&*member.object, Expression::Super);
        let obj = if is_super {
            let base = self.alloc_register();
            self.emit(Opcode::GetSuperBase);
            self.emit_u16(base);
            base
        } else {
            self.gen_expression(&member.object, ctx)?
        };
        let dst = self.alloc_register();
        match &member.property {
            MemberProperty::Identifier(name) => {
                let atom = ctx.intern(name);
                if is_super {
                    let key = self.alloc_register();
                    let key_idx = self.add_constant(JSValue::new_string(atom));
                    self.emit(Opcode::LoadConst);
                    self.emit_u16(key);
                    self.emit_u32(key_idx);
                    self.emit(Opcode::GetSuperProp);
                    self.emit_u16(dst);
                    self.emit_u16(obj);
                    self.emit_u16(0);
                    self.emit_u16(key);
                    self.free_register(key);
                } else {
                    self.emit(Opcode::GetNamedProp);
                    self.emit_u16(dst);
                    self.emit_u16(obj);
                    self.emit_u32(atom.0);
                }
            }
            MemberProperty::Computed(expr) => {
                let key = self.gen_expression(expr, ctx)?;
                if is_super {
                    self.emit(Opcode::GetSuperProp);
                    self.emit_u16(dst);
                    self.emit_u16(obj);
                    self.emit_u16(0);
                    self.emit_u16(key);
                } else {
                    self.emit(Opcode::GetField);
                    self.emit_u16(dst);
                    self.emit_u16(obj);
                    self.emit_u16(key);
                }
            }
            MemberProperty::PrivateIdentifier(name) => {
                let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                let key_idx = self.intern_and_add_const(ctx, name);
                let key = self.alloc_register();
                self.emit(Opcode::LoadConst);
                self.emit_u16(key);
                self.emit_u32(key_idx);
                self.emit(Opcode::GetPrivate);
                self.emit_u16(dst);
                self.emit_u16(obj);
                self.emit_u16(key);
                self.emit_u16(owner_reg);
                if owner_temp {
                    self.free_register(owner_reg);
                }
                self.free_register(key);
            }
        }
        self.free_register(obj);
        Ok(dst)
    }

    fn gen_call_expression(
        &mut self,
        call: &CallExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {

        if self.opt_tiny_inline {
            if let Some(reg) = self.try_emit_tiny_inline_call(call, ctx)? {
                return Ok(reg);
            }
        }

        if self.opt_fused_getprop_call {
            if let Expression::MemberExpression(member) = &*call.callee {
                if !call
                    .arguments
                    .iter()
                    .any(|arg| matches!(arg, Argument::Spread(_)))
                {
                    if let MemberProperty::Identifier(name) = &member.property {
                        if matches!(&*member.object, Expression::Super) {
                            // super method calls need the prototype base and the
                            // current this, handled by the general path below.
                        } else {
                        let is_namespace = if let Expression::Identifier(ident) = &*member.object {
                            matches!(
                                ident.name.as_str(),
                                "Math"
                                    | "Object"
                                    | "JSON"
                                    | "Number"
                                    | "String"
                                    | "Array"
                                    | "RegExp"
                                    | "Error"
                                    | "Function"
                                    | "Promise"
                                    | "Symbol"
                                    | "Map"
                                    | "Set"
                                    | "WeakMap"
                                    | "WeakSet"
                                    | "BigInt"
                                    | "Reflect"
                                    | "Proxy"
                                    | "console"
                            )
                        } else {
                            false
                        };

                        if !is_namespace {
                            let obj_reg = self.gen_expression(&member.object, ctx)?;
                            let atom = ctx.intern(name);

                            let mut arg_regs = Vec::new();
                            for arg in &call.arguments {
                                if let Argument::Expression(expr) = arg {
                                    arg_regs.push(self.gen_expression(expr, ctx)?);
                                }
                            }

                            let dst = self.alloc_register();
                            self.emit(Opcode::CallNamedMethod);
                            self.emit_u16(dst);
                            self.emit_u16(obj_reg);
                            self.emit_u32(atom.0);
                            self.emit_u16(arg_regs.len() as u16);
                            for r in &arg_regs {
                                self.emit_u16(*r);
                            }

                            self.free_register(obj_reg);
                            for r in arg_regs {
                                self.free_register(r);
                            }
                            return Ok(dst);
                        }
                        }
                    }
                }
            }
        }

        let is_self_recursive_call = matches!(&*call.callee, Expression::Identifier(id)
            if self.with_scopes.is_empty()
                && self.current_function_name.as_ref().map_or(false, |n| n == &id.name)
                && self.lookup_var_slot(&id.name).is_none()
                && !self.upvalue_slots.contains_key(&id.name));

        let has_spread = call
            .arguments
            .iter()
            .any(|arg| matches!(arg, Argument::Spread(_)));

        let is_direct_eval = matches!(&*call.callee, Expression::Identifier(id) if id.name == "eval");

        if is_self_recursive_call && !has_spread {
            let mut arg_regs = Vec::new();
            for arg in &call.arguments {
                if let Argument::Expression(expr) = arg {
                    arg_regs.push(self.gen_expression(expr, ctx)?);
                }
            }
            let dst = self.alloc_register();
            if arg_regs.len() == 1 {
                self.emit(Opcode::CallCurrent1);
                self.emit_u16(dst);
                self.emit_u16(arg_regs[0]);
            } else {
                self.emit(Opcode::CallCurrent);
                self.emit_u16(dst);
                self.emit_u16(arg_regs.len() as u16);
                for r in &arg_regs {
                    self.emit_u16(*r);
                }
            }
            for r in arg_regs {
                self.free_register(r);
            }
            return Ok(dst);
        }

        if !has_spread {
            if let Expression::MemberExpression(member) = &*call.callee {
                if let Expression::Identifier(ident) = &*member.object {
                    if ident.name == "Math" {
                        if let MemberProperty::Identifier(name) = &member.property {
                            let math_op = match name.as_str() {
                                "sin" if call.arguments.len() == 1 => Some(Opcode::MathSin),
                                "cos" if call.arguments.len() == 1 => Some(Opcode::MathCos),
                                "sqrt" if call.arguments.len() == 1 => Some(Opcode::MathSqrt),
                                "abs" if call.arguments.len() == 1 => Some(Opcode::MathAbs),
                                "floor" if call.arguments.len() == 1 => Some(Opcode::MathFloor),
                                "ceil" if call.arguments.len() == 1 => Some(Opcode::MathCeil),
                                "round" if call.arguments.len() == 1 => Some(Opcode::MathRound),
                                "pow" if call.arguments.len() == 2 => Some(Opcode::MathPow),
                                "min" if call.arguments.len() == 2 => Some(Opcode::MathMin),
                                "max" if call.arguments.len() == 2 => Some(Opcode::MathMax),
                                _ => None,
                            };
                            if let Some(op) = math_op {
                                let mut arg_regs = Vec::new();
                                for arg in &call.arguments {
                                    if let Argument::Expression(expr) = arg {
                                        arg_regs.push(self.gen_expression(expr, ctx)?);
                                    }
                                }
                                let dst = self.alloc_register();
                                self.emit(op);
                                self.emit_u16(dst);
                                for r in &arg_regs {
                                    self.emit_u16(*r);
                                }
                                for r in arg_regs {
                                    self.free_register(r);
                                }
                                return Ok(dst);
                            }
                        }
                    }
                }
            }
        }

        let (func, obj, namespace_obj_reg) = match &*call.callee {
            Expression::MemberExpression(member) => {
                let is_super = matches!(&*member.object, Expression::Super);
                let obj_reg = if is_super {
                    let base = self.alloc_register();
                    self.emit(Opcode::GetSuperBase);
                    self.emit_u16(base);
                    base
                } else {
                    self.gen_expression(&member.object, ctx)?
                };
                let func_reg = match &member.property {
                    MemberProperty::Identifier(name) => {
                        let atom = ctx.intern(name);
                        if is_super {
                            let key = self.alloc_register();
                            let key_idx = self.add_constant(JSValue::new_string(atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key);
                            self.emit_u32(key_idx);
                            let f = self.alloc_register();
                            self.emit(Opcode::GetSuperProp);
                            self.emit_u16(f);
                            self.emit_u16(obj_reg);
                            self.emit_u16(0);
                            self.emit_u16(key);
                            self.free_register(key);
                            f
                        } else {
                            let f = self.alloc_register();
                            self.emit(Opcode::GetNamedProp);
                            self.emit_u16(f);
                            self.emit_u16(obj_reg);
                            self.emit_u32(atom.0);
                            f
                        }
                    }
                    MemberProperty::PrivateIdentifier(name) => {
                        let (owner_reg, owner_temp) = self.gen_private_owner(name, ctx)?;
                        let key_idx = self.intern_and_add_const(ctx, name);
                        let key = self.alloc_register();
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key);
                        self.emit_u32(key_idx);
                        let f = self.alloc_register();
                        self.emit(Opcode::GetPrivate);
                        self.emit_u16(f);
                        self.emit_u16(obj_reg);
                        self.emit_u16(key);
                        self.emit_u16(owner_reg);
                        if owner_temp {
                            self.free_register(owner_reg);
                        }
                        self.free_register(key);
                        f
                    }
                    MemberProperty::Computed(expr) => {
                        let key = self.gen_expression(expr, ctx)?;
                        let f = self.alloc_register();
                        if is_super {
                            self.emit(Opcode::GetSuperProp);
                            self.emit_u16(f);
                            self.emit_u16(obj_reg);
                            self.emit_u16(0);
                            self.emit_u16(key);
                        } else {
                            self.emit(Opcode::GetField);
                            self.emit_u16(f);
                            self.emit_u16(obj_reg);
                            self.emit_u16(key);
                        }
                        self.free_register(key);
                        f
                    }
                };
                let is_ns = matches!(&*member.object, Expression::Identifier(ident)
                if matches!(ident.name.as_str(),
                    "Math" | "JSON" | "Reflect" | "console"
                ));
                if is_super {
                    self.free_register(obj_reg);
                    (func_reg, Some(0), None)
                } else if is_ns {
                    (func_reg, None, Some(obj_reg))
                } else {
                    (func_reg, Some(obj_reg), None)
                }
            }
            Expression::ChainExpression(inner) => {
                let (func_reg, this_reg) = self.gen_chain_expression_with_this(inner, ctx)?;
                (func_reg, Some(this_reg), None)
            }
            other => {
                let func_reg = self.gen_expression(other, ctx)?;
                (func_reg, None, None)
            }
        };

        let dst = self.alloc_register();
        if has_spread {
            let arr_reg = self.alloc_register();
            self.emit(Opcode::NewArray);
            self.emit_u16(arr_reg);
            self.emit_u16(0);
            for arg in &call.arguments {
                match arg {
                    Argument::Expression(expr) => {
                        let val = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::ArrayPush);
                        self.emit_u16(arr_reg);
                        self.emit_u16(val);
                        self.free_register(val);
                    }
                    Argument::Spread(expr) => {
                        if self.in_default_derived_constructor
                            && matches!(expr, Expression::Identifier(id) if id.name == "__v16_super_args__")
                        {
                            let val = self.gen_expression(expr, ctx)?;
                            self.emit(Opcode::ArrayPushAll);
                            self.emit_u16(arr_reg);
                            self.emit_u16(val);
                            self.free_register(val);
                        } else {
                            let val = self.gen_expression(expr, ctx)?;
                            self.emit_spread_into_array(arr_reg, val, ctx)?;
                        }
                    }
                }
            }
            if is_direct_eval {
                self.emit_mark_direct_eval(ctx);
            }
            if let Some(obj_reg) = obj {
                self.emit(Opcode::CallMethodSpread);
                self.emit_u16(dst);
                self.emit_u16(obj_reg);
                self.emit_u16(func);
                self.emit_u16(arr_reg);
            } else {
                self.emit(Opcode::CallSpread);
                self.emit_u16(dst);
                self.emit_u16(func);
                self.emit_u16(arr_reg);
            }
            if let Some(obj_reg) = obj {
                self.free_register(obj_reg);
            }
            if let Some(reg) = namespace_obj_reg {
                self.free_register(reg);
            }
            self.free_register(func);
            self.free_register(arr_reg);
        } else {
            let mut arg_regs = Vec::new();
            for arg in &call.arguments {
                match arg {
                    Argument::Expression(expr) => {
                        arg_regs.push(self.gen_expression(expr, ctx)?);
                    }
                    _ => {}
                }
            }
            if is_direct_eval {
                self.emit_mark_direct_eval(ctx);
            }
            if let Some(obj_reg) = obj {
                self.emit(Opcode::CallMethod);
                self.emit_u16(dst);
                self.emit_u16(obj_reg);
                self.emit_u16(func);
                self.emit_u16(arg_regs.len() as u16);
                for r in &arg_regs {
                    self.emit_u16(*r);
                }
            } else {
                match arg_regs.len() {
                    0 => {
                        self.emit(Opcode::Call0);
                        self.emit_u16(dst);
                        self.emit_u16(func);
                    }
                    1 => {
                        self.emit(Opcode::Call1);
                        self.emit_u16(dst);
                        self.emit_u16(func);
                        self.emit_u16(arg_regs[0]);
                    }
                    2 => {
                        if self.opt_call23 {
                            self.emit(Opcode::Call2);
                            self.emit_u16(dst);
                            self.emit_u16(func);
                            self.emit_u16(arg_regs[0]);
                            self.emit_u16(arg_regs[1]);
                        } else {
                            self.emit(Opcode::Call);
                            self.emit_u16(dst);
                            self.emit_u16(func);
                            self.emit_u16(2);
                            self.emit_u16(arg_regs[0]);
                            self.emit_u16(arg_regs[1]);
                        }
                    }
                    3 => {
                        if self.opt_call23 {
                            self.emit(Opcode::Call3);
                            self.emit_u16(dst);
                            self.emit_u16(func);
                            self.emit_u16(arg_regs[0]);
                            self.emit_u16(arg_regs[1]);
                            self.emit_u16(arg_regs[2]);
                        } else {
                            self.emit(Opcode::Call);
                            self.emit_u16(dst);
                            self.emit_u16(func);
                            self.emit_u16(3);
                            self.emit_u16(arg_regs[0]);
                            self.emit_u16(arg_regs[1]);
                            self.emit_u16(arg_regs[2]);
                        }
                    }
                    _ => {
                        self.emit(Opcode::Call);
                        self.emit_u16(dst);
                        self.emit_u16(func);
                        self.emit_u16(arg_regs.len() as u16);
                        for r in &arg_regs {
                            self.emit_u16(*r);
                        }
                    }
                }
            }
            if let Some(obj_reg) = obj {
                self.free_register(obj_reg);
            }
            if let Some(reg) = namespace_obj_reg {
                self.free_register(reg);
            }
            self.free_register(func);
            for r in arg_regs {
                self.free_register(r);
            }
        }
        Ok(dst)
    }

    fn emit_spread_into_array(
        &mut self,
        arr_reg: u16,
        src: u16,
        _ctx: &mut JSContext,
    ) -> Result<(), String> {
        let iter_reg = self.alloc_register();
        self.emit(Opcode::GetIterator);
        self.emit_u16(iter_reg);
        self.emit_u16(src);
        self.free_register(src);

        let loop_start = self.code.len();

        let val_reg = self.alloc_register();
        let done_reg = self.alloc_register();
        self.emit(Opcode::IteratorNext);
        self.emit_u16(val_reg);
        self.emit_u16(done_reg);
        self.emit_u16(iter_reg);

        self.emit(Opcode::JumpIf);
        self.emit_u16(done_reg);
        let exit_patch = self.code.len();
        self.emit_i32(0);
        self.free_register(done_reg);

        self.emit(Opcode::ArrayPush);
        self.emit_u16(arr_reg);
        self.emit_u16(val_reg);
        self.free_register(val_reg);

        self.emit_backward_jump(loop_start);

        let end_pos = self.code.len();
        self.patch_jump(exit_patch, end_pos);

        self.free_register(iter_reg);
        Ok(())
    }

    fn gen_new_expression(
        &mut self,
        new: &NewExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let func = self.gen_expression(&new.callee, ctx)?;
        let has_spread = new
            .arguments
            .iter()
            .any(|arg| matches!(arg, Argument::Spread(_)));
        let dst = self.alloc_register();
        if has_spread {
            let arr_reg = self.alloc_register();
            self.emit(Opcode::NewArray);
            self.emit_u16(arr_reg);
            self.emit_u16(0);
            for arg in &new.arguments {
                match arg {
                    Argument::Expression(expr) => {
                        let val = self.gen_expression(expr, ctx)?;
                        self.emit(Opcode::ArrayPush);
                        self.emit_u16(arr_reg);
                        self.emit_u16(val);
                        self.free_register(val);
                    }
                    Argument::Spread(expr) => {
                        let val = self.gen_expression(expr, ctx)?;
                        self.emit_spread_into_array(arr_reg, val, ctx)?;
                    }
                }
            }
            self.emit(Opcode::CallNewSpread);
            self.emit_u16(dst);
            self.emit_u16(func);
            self.emit_u16(arr_reg);
            self.free_register(func);
            self.free_register(arr_reg);
        } else {
            let mut arg_regs = Vec::new();
            for arg in &new.arguments {
                match arg {
                    Argument::Expression(expr) => {
                        arg_regs.push(self.gen_expression(expr, ctx)?);
                    }
                    _ => {}
                }
            }
            self.emit(Opcode::CallNew);
            self.emit_u16(dst);
            self.emit_u16(func);
            self.emit_u16(arg_regs.len() as u16);
            for r in &arg_regs {
                self.emit_u16(*r);
            }
            self.free_register(func);
            for r in arg_regs {
                self.free_register(r);
            }
        }
        Ok(dst)
    }

    fn gen_array_expression(
        &mut self,
        arr: &ArrayExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let dst = self.alloc_register();
        let count = arr.elements.len() as u16;
        self.emit(Opcode::NewArray);
        self.emit_u16(dst);
        self.emit_u16(count);

        let has_holes = arr.elements.iter().any(|e| e.is_none());
        let has_spread = arr
            .elements
            .iter()
            .any(|e| matches!(e, Some(ArrayElement::Spread(_))));

        if !has_holes && !has_spread {
            for elem in &arr.elements {
                if let Some(ArrayElement::Expression(expr)) = elem {
                    let val = self.gen_expression(expr, ctx)?;
                    self.emit(Opcode::ArrayPush);
                    self.emit_u16(dst);
                    self.emit_u16(val);
                    self.free_register(val);
                }
            }
            return Ok(dst);
        }

        let idx_reg = self.alloc_register();
        self.emit_int_const(idx_reg, 0);

        for elem in &arr.elements {
            match elem {
                Some(ArrayElement::Expression(expr)) => {
                    let val = self.gen_expression(expr, ctx)?;
                    self.emit(Opcode::SetField);
                    self.emit_u16(dst);
                    self.emit_u16(idx_reg);
                    self.emit_u16(val);
                    self.free_register(val);
                    self.emit(Opcode::IncLocal);
                    self.emit_u16(idx_reg);
                }
                None => {
                    self.emit(Opcode::IncLocal);
                    self.emit_u16(idx_reg);
                }
                Some(ArrayElement::Spread(expr)) => {
                    let src = self.gen_expression(expr, ctx)?;

                    let iter_reg = self.alloc_register();
                    self.emit(Opcode::GetIterator);
                    self.emit_u16(iter_reg);
                    self.emit_u16(src);
                    self.free_register(src);

                    let loop_start = self.code.len();

                    let val_reg = self.alloc_register();
                    let done_reg = self.alloc_register();
                    self.emit(Opcode::IteratorNext);
                    self.emit_u16(val_reg);
                    self.emit_u16(done_reg);
                    self.emit_u16(iter_reg);

                    self.emit(Opcode::JumpIf);
                    self.emit_u16(done_reg);
                    let exit_patch = self.code.len();
                    self.emit_i32(0);
                    self.free_register(done_reg);

                    self.emit(Opcode::ArrayPush);
                    self.emit_u16(dst);
                    self.emit_u16(val_reg);
                    self.free_register(val_reg);

                    self.emit_backward_jump(loop_start);

                    let end_pos = self.code.len();
                    self.patch_jump(exit_patch, end_pos);

                    self.free_register(iter_reg);

                    let len_atom = ctx.common_atoms.length;
                    let len_const = self.add_constant(JSValue::new_string(len_atom));
                    let len_key = self.alloc_register();
                    self.emit(Opcode::LoadConst);
                    self.emit_u16(len_key);
                    self.emit_u32(len_const);
                    self.emit(Opcode::GetProp);
                    self.emit_u16(idx_reg);
                    self.emit_u16(dst);
                    self.emit_u16(len_key);
                    self.free_register(len_key);
                }
            }
        }
        let len_atom = ctx.common_atoms.length;
        let len_const = self.add_constant(JSValue::new_string(len_atom));
        let len_key = self.alloc_register();
        self.emit(Opcode::LoadConst);
        self.emit_u16(len_key);
        self.emit_u32(len_const);
        self.emit(Opcode::SetProp);
        self.emit_u16(dst);
        self.emit_u16(len_key);
        self.emit_u16(idx_reg);
        self.free_register(len_key);
        self.free_register(idx_reg);
        Ok(dst)
    }

    fn gen_property_function_value(
        &mut self,
        value: &Expression,
        is_method: bool,
        name: Option<&str>,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        if is_method {
            if let Expression::FunctionExpression(func) = value {
                return self.gen_function_expression(
                    &func.params,
                    &func.body,
                    name.or(func.name.as_deref()),
                    ctx,
                    func.generator,
                    func.is_async,
                    false,
                    true,
                    false,
                );
            }
        }
        let anonymous = match value {
            Expression::FunctionExpression(func) => func.name.is_none(),
            Expression::ArrowFunction(_) => true,
            Expression::ClassExpression(class) => class.name.is_none(),
            _ => false,
        };
        if let (Some(name), true) = (name, anonymous) {
            return self.gen_expression_with_assign_name(value, name, ctx);
        }
        self.gen_expression(value, ctx)
    }

    fn gen_object_expression(
        &mut self,
        obj: &ObjectExpression,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let dst = self.alloc_register();
        self.emit(Opcode::NewObject);
        self.emit_u16(dst);

        for prop in &obj.properties {
            match prop {
                Property::Property {
                    key,
                    value,
                    shorthand,
                    method,
                    getter,
                    setter,
                    ..
                } => {
                    if *shorthand && matches!(**value, Expression::AssignmentExpression(_)) {
                        return Err(
                            "SyntaxError: Invalid shorthand property initializer".to_string()
                        );
                    }
                    if *getter || *setter {
                        let func_reg = self.gen_property_function_value(value, *method, None, ctx)?;
                        let key_reg = match key {
                            PropertyKey::Identifier(name)
                            | PropertyKey::Literal(Literal::String(name, _, _)) => {
                                let atom = ctx.intern(name);
                                let idx = self.add_constant(JSValue::new_string(atom));
                                let k = self.alloc_register();
                                self.emit(Opcode::LoadConst);
                                self.emit_u16(k);
                                self.emit_u32(idx);
                                k
                            }
                            PropertyKey::Computed(expr) => self.gen_expression(expr, ctx)?,
                            _ => {
                                self.free_register(func_reg);
                                continue;
                            }
                        };
                        let getter_reg = if *getter { func_reg } else { u16::MAX };
                        let setter_reg = if *setter { func_reg } else { u16::MAX };
                        self.emit(Opcode::DefineAccessor);
                        self.emit_u16(dst);
                        self.emit_u16(key_reg);
                        self.emit_u16(getter_reg);
                        self.emit_u16(setter_reg);
                        self.free_register(key_reg);
                        self.free_register(func_reg);
                        continue;
                    }
                    match key {
                        PropertyKey::Identifier(name)
                        | PropertyKey::Literal(Literal::String(name, _, _))
                            if name == "__proto__"
                                && !*shorthand
                                && !*method
                                && !*getter
                                && !*setter =>
                        {
                            let val = self.gen_expression(value, ctx)?;
                            self.emit(Opcode::SetProto);
                            self.emit_u16(dst);
                            self.emit_u16(val);
                            self.free_register(val);
                        }
                        PropertyKey::Identifier(name)
                        | PropertyKey::Literal(Literal::String(name, _, _)) => {
                            let val =
                                self.gen_property_function_value(value, *method, Some(name), ctx)?;
                            let atom = ctx.intern(name);
                            self.emit(Opcode::SetNamedProp);
                            self.emit_u16(dst);
                            self.emit_u16(val);
                            self.emit_u32(atom.0);
                            if *method {
                                self.emit(Opcode::SetHomeObject);
                                self.emit_u16(val);
                                self.emit_u16(dst);
                            }
                            self.free_register(val);
                        }
                        _ => {
                            let key_reg = match key {
                                PropertyKey::Literal(Literal::Number(n)) => {
                                    let k = self.alloc_register();
                                    let nv = *n;
                                    if nv >= i32::MIN as f64
                                        && nv <= i32::MAX as f64
                                        && nv.fract() == 0.0
                                    {
                                        self.emit_int_const(k, nv as i32);
                                    } else {
                                        let idx = self.add_constant(JSValue::new_float(nv));
                                        self.emit(Opcode::LoadConst);
                                        self.emit_u16(k);
                                        self.emit_u32(idx);
                                    }
                                    k
                                }
                                PropertyKey::Computed(expr) => {
                                    let k = self.gen_expression(expr, ctx)?;
                                    let key_reg = self.alloc_register();
                                    self.emit(Opcode::Move);
                                    self.emit_u16(key_reg);
                                    self.emit_u16(k);
                                    self.free_register(k);
                                    self.emit(Opcode::ToPropertyKey);
                                    self.emit_u16(key_reg);
                                    key_reg
                                }
                                _ => self.alloc_register(),
                            };
                            let val =
                                self.gen_property_function_value(value, *method, None, ctx)?;
                            let needs_name = *method
                                || matches!(&**value, Expression::FunctionExpression(f) if f.name.is_none())
                                || matches!(&**value, Expression::ArrowFunction(_))
                                || matches!(&**value, Expression::ClassExpression(c) if c.name.is_none());
                            if needs_name {
                                self.emit(Opcode::SetFunctionName);
                                self.emit_u16(val);
                                self.emit_u16(key_reg);
                            }
                            self.emit(Opcode::SetField);
                            self.emit_u16(dst);
                            self.emit_u16(key_reg);
                            self.emit_u16(val);
                            if *method {
                                self.emit(Opcode::SetHomeObject);
                                self.emit_u16(val);
                                self.emit_u16(dst);
                            }
                            self.free_register(key_reg);
                            self.free_register(val);
                        }
                    }
                }
                Property::SpreadElement(expr) => {
                    let src = self.gen_expression(expr, ctx)?;
                    self.emit(Opcode::ObjectSpread);
                    self.emit_u16(dst);
                    self.emit_u16(src);
                    self.free_register(src);
                }
            }
        }
        Ok(dst)
    }

    fn gen_function_expression(
        &mut self,
        params: &[Parameter],
        body: &BlockStatement,
        name: Option<&str>,
        ctx: &mut JSContext,
        is_generator: bool,
        is_async: bool,
        is_arrow: bool,
        is_method: bool,
        is_named_expression: bool,
    ) -> Result<u16, String> {
        let mut nested_codegen = self.nested_codegen();
        nested_codegen.is_arrow = is_arrow;
        nested_codegen.in_derived_constructor = is_arrow && self.in_derived_constructor;
        nested_codegen.in_field_initializer = is_arrow && self.in_field_initializer;
        let has_prototype = is_generator || (!is_arrow && !is_method && !is_async);
        let function_kind = (if has_prototype { 1 } else { 0 })
            | if is_arrow { 4 } else { 0 }
            | if is_method { 32 } else { 0 };
        let mut tentative_arguments: Option<(u16, bool)> = None;
        if is_arrow {
            if self.is_arrow {
                if self.lookup_var("arguments").is_none()
                    && !self.upvalue_slots.contains_key("arguments")
                {
                    if let Some(parent_var) = self.parent_vars.get("arguments").copied() {
                        if !parent_var.closed {
                            let upvalue_slot = self.upvalues.len() as u16;
                            self.reserved_slots.insert(upvalue_slot);
                            let atom = ctx.atom_table_mut().intern("arguments");
                            let local_idx = if parent_var.is_inherited {
                                u16::MAX
                            } else {
                                parent_var.local_idx
                            };
                            self.upvalues.push((atom.0, local_idx));
                            self.upvalue_slots
                                .insert("arguments".to_string(), upvalue_slot);
                            tentative_arguments = Some((upvalue_slot, true));
                        }
                    }
                }
            } else if !self.is_script_level
                && self.lookup_var("arguments").is_none()
                && !(self.is_strict && !self.simple_params)
            {
                if let Some(slot) = self.arguments_slot {
                    tentative_arguments = Some((slot, false));
                } else {
                    let slot = self.alloc_register();
                    self.reserved_slots.insert(slot);
                    self.arguments_slot = Some(slot);
                    tentative_arguments = Some((slot, true));
                }
            }
        }
        let mut parent_vars: FxHashMap<String, ParentVar> = FxHashMap::default();
        for (scope_idx, scope) in self.scope_stack.iter().enumerate() {
            if self
                .scope_is_global
                .get(scope_idx)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            for (name, entry) in scope {
                // Skip pre-declared uninitialized variables from the root scope.
                // These are created by pre_scan_let_const before the actual let
                // statement, causing closures to incorrectly emit CheckTdz.
                if scope_idx == 0 && !entry.initialized {
                    continue;
                }
                parent_vars.insert(
                    name.clone(),
                    ParentVar {
                        local_idx: entry.slot,
                        kind: entry.kind,
                        initialized: entry.initialized,
                        is_inherited: false,
                        closed: false,
                        is_function_name: entry.is_function_name,
                        is_block_decl: false,
                    },
                );
            }
        }
        for (name, &slot) in &self.upvalue_slots {
            if !parent_vars.contains_key(name) {
                parent_vars.insert(
                    name.clone(),
                    ParentVar {
                        local_idx: slot,
                        kind: VariableKind::Let,
                        initialized: true,
                        is_inherited: true,
                        closed: false,
                        is_function_name: false,
                        is_block_decl: false,
                    },
                );
            }
        }
        if !self.is_arrow {
            if let Some((slot, _)) = tentative_arguments {
                if !parent_vars.contains_key("arguments") {
                    parent_vars.insert(
                        "arguments".to_string(),
                        ParentVar {
                            local_idx: slot,
                            kind: VariableKind::Var,
                            initialized: true,
                            is_inherited: false,
                            closed: false,
                            is_function_name: false,
                            is_block_decl: false,
                        },
                    );
                }
            }
        }
        // Include variables from closed scopes (popped from scope_stack but
        // still in parent_vars with the closed flag). These are out-of-scope
        // let/const variables that closures should NOT access.
        for (name, pv) in &self.parent_vars {
            if pv.closed && !parent_vars.contains_key(name) {
                parent_vars.insert(name.clone(), *pv);
            }
        }
        let (bytecode, upvalues) = nested_codegen
            .compile_function_with_parent_gen(
                params,
                body,
                ctx,
                parent_vars,
                name,
                false,
                is_generator,
                is_async,
                is_named_expression,
            )
            .map_err(|e| format!("nested function compile error: {}", e))?;

        if is_arrow {
            if nested_codegen.needs_parent_arguments {
                self.needs_parent_arguments = true;
                if !self.is_arrow {
                    if let Some((slot, _)) = tentative_arguments {
                        self.function_uses_arguments = true;
                        self.emit(Opcode::GetArguments);
                        self.emit_u16(slot);
                    }
                }
            } else if let Some((slot, is_new)) = tentative_arguments {
                if is_new {
                    if self.is_arrow {
                        self.upvalue_slots.remove("arguments");
                        self.upvalues.pop();
                        self.reserved_slots.remove(&slot);
                    } else {
                        self.reserved_slots.remove(&slot);
                        self.free_register(slot);
                        self.arguments_slot = None;
                    }
                }
            }
        }

        let func_name_atom = name.map(|n| ctx.atom_table_mut().intern(n).0).unwrap_or(0);
        let dst = if let Some(name) = name {
            self.lookup_var_slot(name)
                .unwrap_or_else(|| self.alloc_register())
        } else {
            self.alloc_register()
        };
        self.emit_embedded_function(
            &bytecode,
            &upvalues,
            func_name_atom,
            dst,
            is_generator,
            is_async,
            function_kind,
        );
        Ok(dst)
    }

    fn gen_class_declaration(
        &mut self,
        class: &ClassDeclaration,
        ctx: &mut JSContext,
    ) -> Result<(), String> {
        let class_name = &class.name;
        let class_slot = self
            .lookup_var_slot(class_name)
            .unwrap_or_else(|| self.declare_var(class_name, VariableKind::Let));
        let inner_slot = self.alloc_register();
        let class_name_atom =
            self.gen_class_body(inner_slot, class_name, &class.super_class, &class.body, ctx)?;
        self.emit(Opcode::Move);
        self.emit_u16(class_slot);
        self.emit_u16(inner_slot);
        self.free_register(inner_slot);
        if self.is_global_lexical(class_name) {
            self.emit_global_binding(6, class_name, class_slot, ctx);
        } else if self.is_global_scope() && !self.is_eval && self.depth == 0 {
            let class_idx = self.add_constant(JSValue::new_string(class_name_atom));
            self.emit(Opcode::SetGlobal);
            self.emit_u32(class_idx);
            self.emit_u16(class_slot);
        }
        Ok(())
    }

    fn gen_class_body(
        &mut self,
        class_slot: u16,
        class_name: &str,
        super_class: &Option<Box<Expression>>,
        body: &ClassBody,
        ctx: &mut JSContext,
    ) -> Result<crate::runtime::atom::Atom, String> {
        self.reserved_slots.insert(class_slot);
        let has_super = super_class.is_some();
        let is_null_heritage = super_class
            .as_ref()
            .map_or(false, |e| matches!(**e, Expression::Literal(Literal::Null)));
        if !class_name.is_empty() {
            self.push_scope();
            self.scope_stack.last_mut().unwrap().insert(
                class_name.to_string(),
                VarEntry {
                    slot: class_slot,
                    kind: VariableKind::Const,
                    initialized: false,
                    is_function_name: false,
                },
            );
            self.emit(Opcode::LoadTdz);
            self.emit_u16(class_slot);
        }
        let super_val_reg = match super_class {
            Some(expr) => Some(self.gen_expression(expr, ctx)?),
            None => None,
        };

        let ctor_method = body.elements.iter().find(|e| {
            if let ClassElement::Method(m) = e {
                m.kind == MethodKind::Constructor && !m.is_static
            } else {
                false
            }
        });

        let mut instance_private_names: Vec<String> = Vec::new();
        let mut static_private_names: Vec<String> = Vec::new();
        for element in &body.elements {
            let (name, is_static, is_private) = match element {
                ClassElement::Property(p) => {
                    let name = match &p.name {
                        PropertyKey::PrivateIdentifier(n) => Some(n.clone()),
                        PropertyKey::Identifier(n) if p.is_private => Some(n.clone()),
                        _ => None,
                    };
                    (name, p.is_static, p.is_private)
                }
                ClassElement::Method(m) => {
                    let name = match &m.name {
                        PropertyKey::PrivateIdentifier(n) => Some(n.clone()),
                        PropertyKey::Identifier(n) if m.is_private => Some(n.clone()),
                        _ => None,
                    };
                    (name, m.is_static, m.is_private)
                }
                _ => (None, false, false),
            };
            if !is_private {
                continue;
            }
            if let Some(name) = name {
                if is_static {
                    static_private_names.push(name);
                } else {
                    instance_private_names.push(name);
                }
            }
        }

        let proto_reg = self.alloc_register();
        self.reserved_slots.insert(proto_reg);
        self.push_scope();
        {
            let scope = self.scope_stack.last_mut().unwrap();
            for name in &instance_private_names {
                scope.insert(
                    name.clone(),
                    VarEntry {
                        slot: proto_reg,
                        kind: VariableKind::Let,
                        initialized: true,
                        is_function_name: false,
                    },
                );
            }
            for name in &static_private_names {
                scope.insert(
                    name.clone(),
                    VarEntry {
                        slot: class_slot,
                        kind: VariableKind::Let,
                        initialized: true,
                        is_function_name: false,
                    },
                );
            }
        }

        let mut class_parent_vars = FxHashMap::default();

        for (scope_idx, scope) in self.scope_stack.iter().enumerate() {
            if self
                .scope_is_global
                .get(scope_idx)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            for (var_name, entry) in scope {
                // Skip pre-declared uninitialized variables from the root scope
                if scope_idx == 0 && !entry.initialized {
                    continue;
                }
                class_parent_vars.insert(
                    var_name.clone(),
                    ParentVar {
                        local_idx: entry.slot,
                        kind: entry.kind,
                        initialized: entry.initialized,
                        is_inherited: false,
                        closed: false,
                        is_function_name: entry.is_function_name,
                        is_block_decl: false,
                    },
                );
            }
        }

        for (var_name, &slot) in &self.upvalue_slots {
            if !class_parent_vars.contains_key(var_name) {
                class_parent_vars.insert(
                    var_name.clone(),
                    ParentVar {
                        local_idx: slot,
                        kind: VariableKind::Let,
                        initialized: true,
                        is_inherited: true,
                        closed: false,
                        is_function_name: false,
                        is_block_decl: false,
                    },
                );
            }
        }

        if !class_name.is_empty() {
            class_parent_vars.insert(
                class_name.to_string(),
                ParentVar {
                    local_idx: class_slot,
                    kind: VariableKind::Const,
                    initialized: true,
                    is_inherited: false,
                    closed: false,
                    is_function_name: false,
                    is_block_decl: false,
                },
            );
        }

        let (ctor_bytecode, ctor_upvalues) = if let Some(ClassElement::Method(m)) = ctor_method {
            let mut nested = self.nested_codegen();
            nested.is_strict = true;
            nested.in_derived_constructor = has_super;
            nested.compile_function_with_parent(
                &m.params,
                &m.body,
                ctx,
                class_parent_vars.clone(),
                None,
                false,
            )?
        } else {
            let mut default_body = Vec::new();
            let mut default_lines = Vec::new();
            if has_super {
                let super_call = ASTNode::ExpressionStatement(ExpressionStatement {
                    expression: Expression::CallExpression(CallExpression {
                        callee: Box::new(Expression::Super),
                        arguments: vec![Argument::Spread(Expression::Identifier(Identifier {
                            name: "__v16_super_args__".to_string(),
                            parenthesized: false,
                        }))],
                    }),
                });
                default_body.push(super_call);
                default_lines.push(0);
            }
            let mut nested = self.nested_codegen();
            nested.is_strict = true;
            nested.in_derived_constructor = has_super;
            nested.in_default_derived_constructor = has_super;
            let default_params: Vec<Parameter> = if has_super {
                vec![Parameter::RestElement(RestElement {
                    argument: Box::new(AssignmentTarget::Identifier(
                        "__v16_super_args__".to_string(),
                    )),
                })]
            } else {
                Vec::new()
            };
            nested.compile_function_with_parent(
                &default_params,
                &BlockStatement {
                    body: default_body,
                    lines: default_lines,
                },
                ctx,
                class_parent_vars.clone(),
                None,
                false,
            )?
        };

        enum ElementPlan {
            Method {
                key: PropertyKey,
                bc: Bytecode,
                upvalues: Vec<(u32, u16)>,
                is_gen: bool,
                is_async: bool,
                kind: MethodKind,
                is_private: bool,
                is_static: bool,
            },
            Field {
                key: PropertyKey,
                init: Option<(Bytecode, Vec<(u32, u16)>)>,
                is_static: bool,
                is_private: bool,
            },
            Block {
                bc: Bytecode,
                upvalues: Vec<(u32, u16)>,
            },
        }

        let mut plans: Vec<ElementPlan> = Vec::with_capacity(body.elements.len());
        for element in &body.elements {
            match element {
                ClassElement::Method(m) => {
                    if m.kind == MethodKind::Constructor && !m.is_static {
                        continue;
                    }
                    let can_handle = matches!(
                        &m.name,
                        PropertyKey::Identifier(_)
                            | PropertyKey::Literal(Literal::String(_, _, _))
                            | PropertyKey::Literal(Literal::Number(_))
                            | PropertyKey::PrivateIdentifier(_)
                            | PropertyKey::Computed(_)
                    );
                    if !can_handle {
                        continue;
                    }
                    let mut nested = self.nested_codegen();
                    nested.is_strict = true;
                    nested.in_static_method = m.is_static;
                    let (method_bc, method_upvalues) = nested.compile_function_with_parent_gen(
                        &m.params,
                        &m.body,
                        ctx,
                        class_parent_vars.clone(),
                        None,
                        false,
                        m.generator,
                        m.is_async,
                        false,
                    )?;
                    plans.push(ElementPlan::Method {
                        key: m.name.clone(),
                        bc: method_bc,
                        upvalues: method_upvalues,
                        is_gen: m.generator,
                        is_async: m.is_async,
                        kind: m.kind.clone(),
                        is_private: m.is_private,
                        is_static: m.is_static,
                    });
                }
                ClassElement::Property(p) => {
                    let init = match &p.value {
                        Some(value) => {
                            let mut nested = self.nested_codegen();
                            nested.is_strict = true;
                            nested.in_field_initializer = true;
                            nested.field_init_name =
                                Self::class_field_key_name(&p.name, p.is_private);
                            let init_body = BlockStatement {
                                body: vec![ASTNode::ExpressionStatement(ExpressionStatement {
                                    expression: value.clone(),
                                })],
                                lines: vec![0],
                            };
                            let (init_bc, init_upvalues) = nested.compile_function_with_parent(
                                &[],
                                &init_body,
                                ctx,
                                class_parent_vars.clone(),
                                None,
                                true,
                            )?;
                            Some((init_bc, init_upvalues))
                        }
                        None => None,
                    };
                    plans.push(ElementPlan::Field {
                        key: p.name.clone(),
                        init,
                        is_static: p.is_static,
                        is_private: p.is_private,
                    });
                }
                ClassElement::StaticBlock(block) => {
                    let mut nested = self.nested_codegen();
                    nested.is_strict = true;
                    nested.in_field_initializer = true;
                    let (block_bc, block_upvalues) = nested.compile_function_with_parent(
                        &[],
                        &BlockStatement {
                            body: block.body.clone(),
                            lines: block.lines.clone(),
                        },
                        ctx,
                        class_parent_vars.clone(),
                        None,
                        false,
                    )?;
                    plans.push(ElementPlan::Block {
                        bc: block_bc,
                        upvalues: block_upvalues,
                    });
                }
            }
        }

        let ctor_reg = self.alloc_register();
        let class_name_atom = ctx.atom_table_mut().intern(class_name);
        let ctor_name_atom = class_name_atom.0;
        let has_instance_private_elements = !instance_private_names.is_empty();
        self.emit_embedded_function(
            &ctor_bytecode,
            &ctor_upvalues,
            ctor_name_atom,
            ctor_reg,
            false,
            false,
            3 | if has_super { 8 } else { 0 }
                | if has_instance_private_elements { 16 } else { 0 },
        );

        if let Some(super_val) = super_val_reg {
            let super_key_idx = self.add_constant(JSValue::new_string(ctx.common_atoms.__super__));
            let super_key = self.alloc_register();
            self.emit(Opcode::LoadConst);
            self.emit_u16(super_key);
            self.emit_u32(super_key_idx);
            self.emit(Opcode::SetProp);
            self.emit_u16(ctor_reg);
            self.emit_u16(super_key);
            self.emit_u16(super_val);
            if !is_null_heritage {
                self.emit(Opcode::SetProto);
                self.emit_u16(ctor_reg);
                self.emit_u16(super_val);
            }
            self.free_register(super_key);
        }

        self.emit(Opcode::NewObject);
        self.emit_u16(proto_reg);

        if let Some(super_val) = super_val_reg {
            if is_null_heritage {
                let null_reg = self.alloc_register();
                self.emit(Opcode::LoadNull);
                self.emit_u16(null_reg);
                self.emit(Opcode::SetProto);
                self.emit_u16(proto_reg);
                self.emit_u16(null_reg);
                self.free_register(null_reg);
            } else {
                let proto_key_idx =
                    self.add_constant(JSValue::new_string(ctx.common_atoms.prototype));
                let proto_key = self.alloc_register();
                self.emit(Opcode::LoadConst);
                self.emit_u16(proto_key);
                self.emit_u32(proto_key_idx);
                let super_proto = self.alloc_register();
                self.emit(Opcode::GetProp);
                self.emit_u16(super_proto);
                self.emit_u16(super_val);
                self.emit_u16(proto_key);
                self.emit(Opcode::CheckObjectOrNull);
                self.emit_u16(super_proto);
                self.emit(Opcode::SetProto);
                self.emit_u16(proto_reg);
                self.emit_u16(super_proto);
                self.free_register(super_proto);
                self.free_register(proto_key);
            }
            self.free_register(super_val);
        }

        let proto_key_idx = self.add_constant(JSValue::new_string(ctx.common_atoms.prototype));
        let proto_key = self.alloc_register();
        self.emit(Opcode::LoadConst);
        self.emit_u16(proto_key);
        self.emit_u32(proto_key_idx);
        self.emit(Opcode::DefineClassPrototype);
        self.emit_u16(ctor_reg);
        self.emit_u16(proto_reg);
        self.free_register(proto_key);

        enum StaticPendingKind {
            Field,
            PrivateField,
            Block,
        }

        struct StaticPending {
            kind: StaticPendingKind,
            key_reg: u16,
            init_reg: u16,
            has_init: bool,
        }

        let mut static_pending: Vec<StaticPending> = Vec::new();

        for plan in plans {
            match plan {
                ElementPlan::Method {
                    key,
                    bc,
                    upvalues,
                    is_gen,
                    is_async,
                    kind,
                    is_private,
                    is_static,
                } => {
                    let home = if is_static { ctor_reg } else { proto_reg };
                    let key_reg = self.alloc_register();
                    let method_name_atom = match &key {
                        PropertyKey::Identifier(n)
                        | PropertyKey::Literal(Literal::String(n, _, _)) => {
                            let method_atom = ctx.atom_table_mut().intern(n.as_str());
                            let method_idx = self.add_constant(JSValue::new_string(method_atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key_reg);
                            self.emit_u32(method_idx);
                            method_atom.0
                        }
                        PropertyKey::Literal(Literal::Number(n)) => {
                            let s = Self::class_key_number_string(*n);
                            let method_atom = ctx.atom_table_mut().intern(&s);
                            let method_idx = self.add_constant(JSValue::new_string(method_atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key_reg);
                            self.emit_u32(method_idx);
                            method_atom.0
                        }
                        PropertyKey::PrivateIdentifier(n) => {
                            let method_atom = ctx.atom_table_mut().intern(n.as_str());
                            let method_idx = self.add_constant(JSValue::new_string(method_atom));
                            self.emit(Opcode::LoadConst);
                            self.emit_u16(key_reg);
                            self.emit_u32(method_idx);
                            method_atom.0
                        }
                        PropertyKey::Computed(expr) => {
                            let k = self.gen_expression(expr, ctx)?;
                            self.emit(Opcode::Move);
                            self.emit_u16(key_reg);
                            self.emit_u16(k);
                            self.free_register(k);
                            0
                        }
                        _ => {
                            self.free_register(key_reg);
                            continue;
                        }
                    };
                    let method_reg = self.alloc_register();
                    self.emit_embedded_function(
                        &bc,
                        &upvalues,
                        method_name_atom,
                        method_reg,
                        is_gen,
                        is_async,
                        (if is_gen { 1 } else { 0 }) | 32,
                    );
                    match kind {
                        MethodKind::Get if is_private => {
                            self.emit(Opcode::DefinePrivateAccessor);
                            self.emit_u16(home);
                            self.emit_u16(key_reg);
                            self.emit_u16(method_reg);
                            self.emit_u16(u16::MAX);
                            self.emit_u16(home);
                        }
                        MethodKind::Set if is_private => {
                            self.emit(Opcode::DefinePrivateAccessor);
                            self.emit_u16(home);
                            self.emit_u16(key_reg);
                            self.emit_u16(u16::MAX);
                            self.emit_u16(method_reg);
                            self.emit_u16(home);
                        }
                        MethodKind::Get => {
                            self.emit(Opcode::DefineClassAccessor);
                            self.emit_u16(home);
                            self.emit_u16(key_reg);
                            self.emit_u16(method_reg);
                            self.emit_u16(u16::MAX);
                        }
                        MethodKind::Set => {
                            self.emit(Opcode::DefineClassAccessor);
                            self.emit_u16(home);
                            self.emit_u16(key_reg);
                            self.emit_u16(u16::MAX);
                            self.emit_u16(method_reg);
                        }
                        _ => {
                            if is_private {
                                self.emit(Opcode::DefinePrivateMethod);
                                self.emit_u16(home);
                                self.emit_u16(key_reg);
                                self.emit_u16(method_reg);
                                self.emit_u16(home);
                            } else {
                                self.emit(Opcode::SetMethodProp);
                                self.emit_u16(home);
                                self.emit_u16(key_reg);
                                self.emit_u16(method_reg);
                            }
                        }
                    }
                    if is_private && is_static {
                        self.emit(Opcode::AddPrivateBrand);
                        self.emit_u16(ctor_reg);
                        self.emit_u16(ctor_reg);
                    }
                    self.free_register(key_reg);
                    self.free_register(method_reg);
                }
                ElementPlan::Field {
                    key,
                    init,
                    is_static,
                    is_private,
                } => {
                    let (init_reg, init_is_fn) = self.emit_field_initializer(&init);
                    if !is_static {
                        if init_is_fn {
                            self.emit(Opcode::SetHomeObject);
                            self.emit_u16(init_reg);
                            self.emit_u16(proto_reg);
                        }
                        if is_private {
                            let name = Self::private_field_name(&key);
                            let name_atom = ctx.atom_table_mut().intern(&name).0;
                            self.emit(Opcode::AddInstancePrivateField);
                            self.emit_u16(ctor_reg);
                            self.emit_u16(proto_reg);
                            self.emit_u32(name_atom);
                            self.emit_u16(init_reg);
                        } else {
                            let key_reg = self.emit_field_key_reg(&key, ctx)?;
                            self.emit(Opcode::AddInstanceField);
                            self.emit_u16(ctor_reg);
                            self.emit_u16(key_reg);
                            self.emit_u16(init_reg);
                            self.free_register(key_reg);
                        }
                        self.free_register(init_reg);
                    } else if is_private {
                        if init_is_fn {
                            self.emit(Opcode::SetHomeObject);
                            self.emit_u16(init_reg);
                            self.emit_u16(ctor_reg);
                        }
                        let name = Self::private_field_name(&key);
                        let name_atom = ctx.atom_table_mut().intern(&name);
                        let key_reg = self.alloc_register();
                        let key_idx = self.add_constant(JSValue::new_string(name_atom));
                        self.emit(Opcode::LoadConst);
                        self.emit_u16(key_reg);
                        self.emit_u32(key_idx);
                        static_pending.push(StaticPending {
                            kind: StaticPendingKind::PrivateField,
                            key_reg,
                            init_reg,
                            has_init: init_is_fn,
                        });
                    } else {
                        if init_is_fn {
                            self.emit(Opcode::SetHomeObject);
                            self.emit_u16(init_reg);
                            self.emit_u16(ctor_reg);
                        }
                        let key_reg = self.emit_field_key_reg(&key, ctx)?;
                        static_pending.push(StaticPending {
                            kind: StaticPendingKind::Field,
                            key_reg,
                            init_reg,
                            has_init: init_is_fn,
                        });
                    }
                }
                ElementPlan::Block { bc, upvalues } => {
                    let block_reg = self.alloc_register();
                    self.emit_embedded_function(&bc, &upvalues, 0, block_reg, false, false, 0);
                    self.emit(Opcode::SetHomeObject);
                    self.emit_u16(block_reg);
                    self.emit_u16(ctor_reg);
                    static_pending.push(StaticPending {
                        kind: StaticPendingKind::Block,
                        key_reg: 0,
                        init_reg: block_reg,
                        has_init: true,
                    });
                }
            }
        }

        if class_slot != ctor_reg {
            self.emit(Opcode::Move);
            self.emit_u16(class_slot);
            self.emit_u16(ctor_reg);
        }

        for pending in static_pending {
            match pending.kind {
                StaticPendingKind::Block => {
                    self.emit(Opcode::CallMethod);
                    self.emit_u16(pending.init_reg);
                    self.emit_u16(ctor_reg);
                    self.emit_u16(pending.init_reg);
                    self.emit_u16(0);
                }
                StaticPendingKind::Field => {
                    let val_reg = self.alloc_register();
                    if pending.has_init {
                        self.emit(Opcode::CallMethod);
                        self.emit_u16(val_reg);
                        self.emit_u16(ctor_reg);
                        self.emit_u16(pending.init_reg);
                        self.emit_u16(0);
                    } else {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(val_reg);
                    }
                    self.emit(Opcode::DefineField);
                    self.emit_u16(ctor_reg);
                    self.emit_u16(pending.key_reg);
                    self.emit_u16(val_reg);
                    self.free_register(val_reg);
                    self.free_register(pending.key_reg);
                }
                StaticPendingKind::PrivateField => {
                    let val_reg = self.alloc_register();
                    if pending.has_init {
                        self.emit(Opcode::CallMethod);
                        self.emit_u16(val_reg);
                        self.emit_u16(ctor_reg);
                        self.emit_u16(pending.init_reg);
                        self.emit_u16(0);
                    } else {
                        self.emit(Opcode::LoadUndefined);
                        self.emit_u16(val_reg);
                    }
                    self.emit(Opcode::AddPrivateBrand);
                    self.emit_u16(ctor_reg);
                    self.emit_u16(ctor_reg);
                    self.emit(Opcode::DefinePrivateField);
                    self.emit_u16(ctor_reg);
                    self.emit_u16(pending.key_reg);
                    self.emit_u16(val_reg);
                    self.emit_u16(ctor_reg);
                    self.free_register(val_reg);
                    self.free_register(pending.key_reg);
                }
            }
            self.free_register(pending.init_reg);
        }

        self.pop_scope();
        if !class_name.is_empty() {
            self.pop_scope();
        }

        Ok(class_name_atom)
    }

    fn class_key_number_string(n: f64) -> String {
        if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
            (n as i32).to_string()
        } else {
            f64_to_js_string(n)
        }
    }

    fn class_field_key_name(key: &PropertyKey, is_private: bool) -> Option<String> {
        match key {
            PropertyKey::PrivateIdentifier(n) => Some(n.clone()),
            PropertyKey::Identifier(n) if is_private => {
                if n.starts_with('#') {
                    Some(n.clone())
                } else {
                    Some(format!("#{}", n))
                }
            }
            PropertyKey::Identifier(n) => Some(n.clone()),
            PropertyKey::Literal(Literal::String(s, _, _)) => Some(s.clone()),
            PropertyKey::Literal(Literal::Number(n)) => Some(Self::class_key_number_string(*n)),
            _ => None,
        }
    }

    fn private_field_name(key: &PropertyKey) -> String {
        match key {
            PropertyKey::PrivateIdentifier(n) => n.clone(),
            PropertyKey::Identifier(n) => {
                if n.starts_with('#') {
                    n.clone()
                } else {
                    format!("#{}", n)
                }
            }
            _ => String::new(),
        }
    }

    fn emit_field_key_reg(
        &mut self,
        key: &PropertyKey,
        ctx: &mut JSContext,
    ) -> Result<u16, String> {
        let key_reg = self.alloc_register();
        match key {
            PropertyKey::Identifier(n) => {
                let atom = ctx.atom_table_mut().intern(n.as_str());
                let idx = self.add_constant(JSValue::new_string(atom));
                self.emit(Opcode::LoadConst);
                self.emit_u16(key_reg);
                self.emit_u32(idx);
            }
            PropertyKey::Literal(Literal::String(s, _, _)) => {
                let atom = ctx.atom_table_mut().intern(s.as_str());
                let idx = self.add_constant(JSValue::new_string(atom));
                self.emit(Opcode::LoadConst);
                self.emit_u16(key_reg);
                self.emit_u32(idx);
            }
            PropertyKey::Literal(Literal::Number(n)) => {
                let s = Self::class_key_number_string(*n);
                let atom = ctx.atom_table_mut().intern(&s);
                let idx = self.add_constant(JSValue::new_string(atom));
                self.emit(Opcode::LoadConst);
                self.emit_u16(key_reg);
                self.emit_u32(idx);
            }
            PropertyKey::Computed(expr) => {
                let k = self.gen_expression(expr, ctx)?;
                self.emit(Opcode::Move);
                self.emit_u16(key_reg);
                self.emit_u16(k);
                self.free_register(k);
                self.emit(Opcode::ToPropertyKey);
                self.emit_u16(key_reg);
            }
            _ => {
                self.free_register(key_reg);
                return Err("SyntaxError: Invalid property key".to_string());
            }
        }
        Ok(key_reg)
    }

    fn emit_field_initializer(&mut self, init: &Option<(Bytecode, Vec<(u32, u16)>)>) -> (u16, bool) {
        match init {
            Some((bc, upvalues)) => {
                let init_reg = self.alloc_register();
                self.emit_embedded_function(bc, upvalues, 0, init_reg, false, false, 0);
                (init_reg, true)
            }
            None => {
                let init_reg = self.alloc_register();
                self.emit(Opcode::LoadUndefined);
                self.emit_u16(init_reg);
                (init_reg, false)
            }
        }
    }
}

pub(crate) fn detect_simple_constructor_props(
    bytecode: &crate::compiler::opcode::Bytecode,
) -> bool {
    let code = &bytecode.code;
    if code.is_empty() {
        return false;
    }
    let mut pc = 0usize;
    let mut found_return = false;
    let mut has_simple_prop = false;
    let pc_limit = bytecode.param_count;
    while pc < code.len() {
        let op = crate::compiler::opcode::Opcode::from_u8_unchecked(code[pc]);
        let size = crate::compiler::opcode::Opcode::instruction_size(op);
        if size == 0 || pc + size > code.len() {
            break;
        }
        match op {
            crate::compiler::opcode::Opcode::SetNamedProp => {
                let obj = u16::from_le_bytes([code[pc + 1], code[pc + 2]]);
                let val = u16::from_le_bytes([code[pc + 3], code[pc + 4]]);
                if val > pc_limit {
                    return false;
                }
                if obj == 0 && val >= 1 {
                    has_simple_prop = true;
                } else {
                    return false;
                }
            }
            crate::compiler::opcode::Opcode::LoadUndefined => {
                let dst = u16::from_le_bytes([code[pc + 1], code[pc + 2]]);
                if dst != 0 {
                    return false;
                }
            }
            crate::compiler::opcode::Opcode::Return => {
                let src = u16::from_le_bytes([code[pc + 1], code[pc + 2]]);
                if src == 0 {
                    found_return = true;
                }
                break;
            }
            _ => {
                if op != crate::compiler::opcode::Opcode::Nop {
                    return false;
                }
            }
        }
        pc += size;
    }
    found_return && has_simple_prop
}

fn f64_to_js_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    let abs = n.abs();
    if abs == 0.0 {
        if n.is_sign_negative() {
            return "0".to_string();
        }
        return "0".to_string();
    }
    if abs < 1e-6 || abs >= 1e21 {
        let s = format!("{:e}", n);
        s
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::vm::VM;
    use crate::runtime::{JSContext, JSRuntime};

    fn run_expr(code: &str) -> i64 {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };

        let mut codegen = CodeGenerator::with_opt_level(OptLevel::default());
        let reg = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((reg + 1) as u32);

        if reg != 0 {
            bytecode.code.push(Opcode::Move as u8);
            bytecode.code.extend_from_slice(&0u16.to_le_bytes());
            bytecode.code.extend_from_slice(&reg.to_le_bytes());
        }
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        result.get_int()
    }

    fn run_func(code: &str) -> i64 {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);

        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let func = match ast.body.first() {
            Some(ASTNode::FunctionDeclaration(fd)) => fd.clone(),
            _ => panic!("expected function declaration"),
        };

        let mut codegen = CodeGenerator::with_opt_level(OptLevel::default());
        let (bytecode, _upvalues) = codegen
            .compile_function(&func.params, &func.body, &mut ctx)
            .unwrap();

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        result.get_int()
    }

    fn compile_func_bytecode(code: &str, opt_level: OptLevel) -> Bytecode {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let func = match ast.body.first() {
            Some(ASTNode::FunctionDeclaration(fd)) => fd.clone(),
            _ => panic!("expected function declaration"),
        };
        let mut codegen = CodeGenerator::with_opt_level(opt_level);
        let (bc, _) = codegen
            .compile_function(&func.params, &func.body, &mut ctx)
            .unwrap();
        bc
    }

    fn compile_script_bytecode_len(code: &str, opt_level: OptLevel) -> usize {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let block = crate::compiler::ast::BlockStatement {
            body: ast.body,
            lines: ast.lines,
        };
        let mut codegen = CodeGenerator::with_opt_level(opt_level);
        let (bc, _) = codegen.compile_script(&block, &mut ctx).unwrap();
        bc.code.len()
    }

    #[test]
    fn test_literal_int() {
        assert_eq!(run_expr("42"), 42);
    }

    #[test]
    fn test_add_sub() {
        assert_eq!(run_expr("1 + 2"), 3);
        assert_eq!(run_expr("10 - 3"), 7);
    }

    #[test]
    fn test_mul_div() {
        assert_eq!(run_expr("3 * 4"), 12);
        assert_eq!(run_expr("20 / 4"), 5);
    }

    #[test]
    fn test_complex_arithmetic() {
        assert_eq!(run_expr("1 + 2 * 3"), 7);
        assert_eq!(run_expr("(1 + 2) * 3"), 9);
    }

    #[test]
    fn test_variable_and_assignment() {
        assert_eq!(
            run_func("function f() { var x = 10; x = x + 5; return x; }"),
            15
        );
    }

    #[test]
    fn test_if_statement() {
        assert_eq!(
            run_func("function f() { var x = 3; if (x < 5) { return 1; } return 0; }"),
            1
        );
        assert_eq!(
            run_func("function f() { var x = 10; if (x < 5) { return 1; } return 0; }"),
            0
        );
    }

    #[test]
    fn test_if_else() {
        assert_eq!(
            run_func("function f() { var x = 2; if (x > 5) { return 10; } else { return 20; } }"),
            20
        );
    }

    #[test]
    fn test_while_loop() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; while (i < 10) { sum = sum + i; i = i + 1; } return sum; }"
            ),
            45
        );
    }

    #[test]
    fn test_for_loop() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; for (var i = 0; i <= 10; i = i + 1) { sum = sum + i; } return sum; }"
            ),
            55
        );
    }

    #[test]
    fn test_ternary() {
        assert_eq!(run_expr("1 < 2 ? 10 : 20"), 10);
        assert_eq!(run_expr("1 > 2 ? 10 : 20"), 20);
    }

    #[test]
    fn test_unary_neg() {
        assert_eq!(run_expr("-5"), -5);
        assert_eq!(run_expr("-(-3)"), 3);
    }

    #[test]
    fn test_logical_and() {
        assert_eq!(run_expr("1 && 1 ? 1 : 0"), 1);
    }

    #[test]
    fn test_p2_profile_raw_disables_all_literal_folds() {
        let code = "function f() { if (true) { return 1; } return 2; }";
        let bc = compile_func_bytecode(code, OptLevel::O0);
        assert!(bc.code.contains(&(Opcode::JumpIfNot as u8)));
    }

    #[test]
    fn test_p2_profile_ternary_keeps_if_unfolded() {
        let code = "function f() { if (true) { return 1; } return 2; }";
        let bc = compile_func_bytecode(code, OptLevel::O1);
        assert!(bc.code.contains(&(Opcode::JumpIfNot as u8)));
    }

    #[test]
    fn test_p2_profile_stable_folds_if_true() {
        let code = "function f() { if (true) { return 1; } return 2; }";
        let bc = compile_func_bytecode(code, OptLevel::O2);
        assert!(!bc.code.contains(&(Opcode::JumpIfNot as u8)));
    }

    #[test]
    fn test_p2_profile_aggressive_folds_string_truthy_conditional() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let ast = crate::compiler::parser::Parser::new("'x' ? 11 : 22")
            .parse()
            .unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };
        let mut codegen = CodeGenerator::with_opt_level(OptLevel::O3);
        let reg = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((reg + 1) as u32);
        if reg != 0 {
            bytecode.code.push(Opcode::Move as u8);
            bytecode.code.extend_from_slice(&0u16.to_le_bytes());
            bytecode.code.extend_from_slice(&reg.to_le_bytes());
        }
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        assert_eq!(result.get_int(), 11);
    }

    #[test]
    fn test_prefix_increment() {
        assert_eq!(run_func("function f() { var x = 5; return ++x; }"), 6);
        assert_eq!(
            run_func("function f() { var x = 5; var y = ++x; return x + y; }"),
            12
        );
    }

    #[test]
    fn test_postfix_increment() {
        assert_eq!(run_func("function f() { var x = 5; return x++; }"), 5);
        assert_eq!(
            run_func("function f() { var x = 5; var y = x++; return x + y; }"),
            11
        );
    }

    #[test]
    fn test_prefix_decrement() {
        assert_eq!(run_func("function f() { var x = 5; return --x; }"), 4);
        assert_eq!(
            run_func("function f() { var x = 5; var y = --x; return x + y; }"),
            8
        );
    }

    #[test]
    fn test_postfix_decrement() {
        assert_eq!(run_func("function f() { var x = 5; return x--; }"), 5);
        assert_eq!(
            run_func("function f() { var x = 5; var y = x--; return x + y; }"),
            9
        );
    }

    #[test]
    fn test_do_while_loop() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; do { sum = sum + i; i = i + 1; } while (i < 10); return sum; }"
            ),
            45
        );
    }

    #[test]
    fn test_break_in_while() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; while (i < 100) { sum = sum + i; i = i + 1; if (i > 5) break; } return sum; }"
            ),
            15
        );
    }

    #[test]
    fn test_break_in_for() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; for (var i = 0; i < 100; i = i + 1) { sum = sum + i; if (i == 5) break; } return sum; }"
            ),
            15
        );
    }

    #[test]
    fn test_break_in_do_while() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; do { sum = sum + i; i = i + 1; if (i > 5) break; } while (i < 100); return sum; }"
            ),
            15
        );
    }

    #[test]
    fn test_continue_in_while() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; while (i < 5) { i = i + 1; if (i == 3) continue; sum = sum + i; } return sum; }"
            ),
            12
        );
    }

    #[test]
    fn test_continue_in_for() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; for (var i = 0; i < 5; i = i + 1) { if (i == 3) continue; sum = sum + i; } return sum; }"
            ),
            7
        );
    }

    #[test]
    fn test_tiny_inline_getter_reduces_bytecode() {
        let code = r#"
            function car(p) { return p.car; }
            function main(x) { return car(x); }
        "#;

        let inline_len = compile_script_bytecode_len(code, OptLevel::O2);
        let noninline_len = compile_script_bytecode_len(code, OptLevel::O0);

        assert!(
            inline_len < noninline_len,
            "tiny getter inline should reduce bytecode: inline={}, noninline={}",
            inline_len,
            noninline_len
        );
    }

    #[test]
    fn test_tiny_inline_nested_getter_reduces_bytecode() {
        let code = r#"
            function cadr(p) { return p.cdr.car; }
            function main(x) { return cadr(x); }
        "#;

        let inline_len = compile_script_bytecode_len(code, OptLevel::O2);
        let noninline_len = compile_script_bytecode_len(code, OptLevel::O0);

        assert!(
            inline_len < noninline_len,
            "tiny nested getter inline should reduce bytecode: inline={}, noninline={}",
            inline_len,
            noninline_len
        );
    }

    #[test]
    fn test_continue_in_do_while() {
        assert_eq!(
            run_func(
                "function f() { var sum = 0; var i = 0; do { i = i + 1; if (i == 3) continue; sum = sum + i; } while (i < 5); return sum; }"
            ),
            12
        );
    }

    #[test]
    fn test_bitwise_codegen() {
        assert_eq!(run_expr("12 & 10"), 8);
        assert_eq!(run_expr("12 | 10"), 14);
        assert_eq!(run_expr("12 ^ 10"), 6);
        assert_eq!(run_expr("~8"), -9);
        assert_eq!(run_expr("1 << 4"), 16);
        assert_eq!(run_expr("16 >> 2"), 4);
        assert_eq!(run_expr("-1 >>> 1"), 2147483647);
    }

    #[test]
    fn test_mod_pow_codegen() {
        assert_eq!(run_expr("17 % 5"), 2);
        assert_eq!(run_expr("2 ** 10"), 1024);
    }

    #[test]
    fn test_switch_basic() {
        assert_eq!(
            run_func(
                "function f() { var x = 2; switch (x) { case 1: return 10; case 2: return 20; case 3: return 30; default: return 99; } }"
            ),
            20
        );
    }

    #[test]
    fn test_switch_default() {
        assert_eq!(
            run_func(
                "function f() { var x = 5; switch (x) { case 1: return 10; case 2: return 20; default: return 99; } }"
            ),
            99
        );
    }

    #[test]
    fn test_switch_fallthrough() {
        assert_eq!(
            run_func(
                "function f() { var x = 1; var r = 0; switch (x) { case 1: r = r + 1; case 2: r = r + 10; default: r = r + 100; } return r; }"
            ),
            111
        );
    }

    #[test]
    fn test_switch_break() {
        assert_eq!(
            run_func(
                "function f() { var x = 1; var r = 0; switch (x) { case 1: r = r + 1; break; case 2: r = r + 10; break; default: r = r + 100; } return r; }"
            ),
            1
        );
    }

    #[test]
    fn test_switch_break_in_nested_loop() {
        assert_eq!(
            run_func(
                "function f() { var r = 0; for (var i = 0; i < 3; i = i + 1) { switch (i) { case 0: r = r + 1; break; case 1: r = r + 10; break; default: r = r + 100; } } return r; }"
            ),
            111
        );
    }

    #[test]
    fn test_closure_basic() {
        assert_eq!(
            run_func(
                "function f() { var count = 0; function inc() { count = count + 1; return count; } return inc() + inc(); }"
            ),
            3
        );
    }

    #[test]
    fn test_closure_counter() {
        assert_eq!(
            run_func(
                "function f() { function makeCounter() { var count = 0; return function() { count = count + 1; return count; }; } var c = makeCounter(); return c() + c(); }"
            ),
            3
        );
    }

    #[test]
    fn test_closure_with_param() {
        assert_eq!(
            run_func(
                "function f() { function makeAdder(x) { return function(y) { return x + y; }; } var add5 = makeAdder(5); return add5(3); }"
            ),
            8
        );
    }

    #[test]
    fn test_nested_closure() {
        assert_eq!(
            run_func(
                "function f() { var a = 1; function g() { var b = 2; return function() { return a + b; }; } return g()(); }"
            ),
            3
        );
    }

    #[test]
    fn test_arrow_function() {
        assert_eq!(
            run_func(
                "function f() { var double = function(x) { return x * 2; }; return double(5); }"
            ),
            10
        );
    }

    #[test]
    fn test_delete_prop() {
        assert_eq!(
            run_func("function f() { var o = {x: 1, y: 2}; delete o.x; return o.y; }"),
            2
        );
    }

    #[test]
    fn test_delete_returns_true() {
        assert_eq!(
            run_func("function f() { var o = {x: 1}; return delete o.x; }"),
            1
        );
    }

    #[test]
    fn test_has_property_in() {
        assert_eq!(
            run_func("function f() { var o = {x: 1}; return 'x' in o; }"),
            1
        );
    }

    #[test]
    fn test_has_property_not_in() {
        assert_eq!(
            run_func("function f() { var o = {x: 1}; return 'y' in o; }"),
            0
        );
    }

    #[test]
    fn test_rest_params() {
        assert_eq!(
            run_func(
                "function outer() { function f(a, b, ...rest) { return rest.length; } return f(1, 2, 3, 4); }"
            ),
            2
        );
    }

    #[test]
    fn test_rest_params_values() {
        assert_eq!(
            run_func(
                "function outer() { function f(a, ...rest) { return rest[0] + rest[1]; } return f(1, 10, 20); }"
            ),
            30
        );
    }

    #[test]
    fn test_rest_params_empty() {
        assert_eq!(
            run_func(
                "function outer() { function f(a, b, ...rest) { return rest.length; } return f(1, 2); }"
            ),
            0
        );
    }

    #[test]
    fn test_object_spread() {
        assert_eq!(
            run_func(
                "function f() { var o = {a: 1, b: 2}; var n = {...o, c: 3}; return n.a + n.b + n.c; }"
            ),
            6
        );
    }

    #[test]
    fn test_array_spread() {
        assert_eq!(
            run_func(
                "function f() { var a = [1, 2]; var b = [...a, 3, 4]; return b[0] + b[1] + b[2] + b[3]; }"
            ),
            10
        );
    }

    #[test]
    fn test_array_spread_mixed() {
        assert_eq!(
            run_func("function f() { var a = [10, 20]; var b = [5, ...a]; return b.length; }"),
            3
        );
    }

    #[test]
    fn test_for_of_basic() {
        assert_eq!(
            run_func(
                "function f() { var s = 0; for (var x of [1, 2, 3]) { s = s + x; } return s; }"
            ),
            6
        );
    }

    #[test]
    fn test_for_of_destructuring_array() {
        assert_eq!(
            run_func(
                "function f() { var s = 0; for (var [a, b] of [[1,2],[3,4]]) { s = s + a + b; } return s; }"
            ),
            10
        );
    }

    #[test]
    fn test_for_of_destructuring_object() {
        assert_eq!(
            run_func(
                "function f() { var s = 0; for (var {x} of [{x:1},{x:2},{x:3}]) { s = s + x; } return s; }"
            ),
            6
        );
    }

    #[test]
    fn test_binding_destructuring_nested() {
        assert_eq!(
            run_func("function f() { var [[x], {y}] = [[1], {y: 2}]; return x + y; }"),
            3
        );
    }

    #[test]
    fn test_binding_destructuring_defaults() {
        assert_eq!(
            run_func("function f() { var [a = 10, b = 20] = [1]; return a + b; }"),
            21
        );
    }

    #[test]
    fn test_for_in_destructuring_array() {
        assert_eq!(
            run_func(
                "function f() { var o = {a:1,b:2}; var s = 0; for (var [k] in o) { s = s + 1; } return s; }"
            ),
            2
        );
    }

    #[test]
    fn test_class_basic() {
        assert_eq!(
            run_func(
                "function f() { class A { constructor() { this.x = 42; } getX() { return this.x; } } var a = new A(); return a.getX(); }"
            ),
            42
        );
    }

    #[test]
    fn test_class_with_static_method() {
        assert_eq!(
            run_func("function f() { class A { static foo() { return 10; } } return A.foo(); }"),
            10
        );
    }

    #[test]
    fn test_class_with_instance_field() {
        assert_eq!(
            run_func(
                "function f() { class A { x = 5; y = 7; sum() { return this.x + this.y; } } var a = new A(); return a.sum(); }"
            ),
            12
        );
    }

    #[test]
    fn test_class_inheritance() {
        assert_eq!(
            run_func(
                "function f() { class A { constructor() { this.x = 10; } foo() { return this.x; } } class B extends A { constructor() { super(); this.y = 20; } bar() { return this.y; } } var b = new B(); return b.foo() + b.bar(); }"
            ),
            30
        );
    }

    #[test]
    fn test_class_default_constructor_with_super() {
        assert_eq!(
            run_func(
                "function f() { class A { constructor() { this.x = 100; } } class B extends A {} var b = new B(); return b.x; }"
            ),
            100
        );
    }

    #[test]
    fn test_class_static_field() {
        assert_eq!(
            run_func("function f() { class A { static count = 3; } return A.count; }"),
            3
        );
    }

    #[test]
    fn test_for_in_basic() {
        assert_eq!(
            run_func(
                "function f() { var o = {a: 1, b: 2, c: 3}; var sum = 0; for (var k in o) { sum = sum + 1; } return sum; }"
            ),
            3
        );
    }

    #[test]
    fn test_for_in_sum_values() {
        assert_eq!(
            run_func(
                "function f() { var o = {a: 10, b: 20}; var sum = 0; for (var k in o) { sum = sum + o[k]; } return sum; }"
            ),
            30
        );
    }

    #[test]
    fn test_computed_prop_access() {
        assert_eq!(
            run_func("function f() { var o = {a: 10}; return o['a']; }"),
            10
        );
    }

    #[test]
    fn test_try_catch_throw() {
        assert_eq!(
            run_func("function f() { try { throw 42; return 0; } catch(e) { return e + 1; } }"),
            43
        );
    }

    #[test]
    fn test_try_no_throw() {
        assert_eq!(
            run_func("function f() { try { return 7; } catch(e) { return 99; } }"),
            7
        );
    }

    #[test]
    fn test_try_catch_finally() {
        assert_eq!(
            run_func(
                "function f() { var x = 0; try { throw 5; } catch(e) { x = e + 10; } finally { x = x + 1; } return x; }"
            ),
            16
        );
    }

    #[test]
    fn test_try_catch_nested() {
        assert_eq!(
            run_func(
                "function f() { try { try { throw 3; } catch(e) { throw e + 2; } } catch(e) { return e * 2; } }"
            ),
            10
        );
    }

    #[test]
    fn test_private_instance_field_get() {
        assert_eq!(
            run_func(
                "function f() { class A { #x = 42; getX() { return this.#x; } } return new A().getX(); }"
            ),
            42
        );
    }

    #[test]
    fn test_private_instance_field_set() {
        assert_eq!(
            run_func(
                "function f() { class A { #x = 10; setX(v) { this.#x = v; } getX() { return this.#x; } } var a = new A(); a.setX(99); return a.getX(); }"
            ),
            99
        );
    }

    #[test]
    fn test_private_instance_field_in() {
        assert_eq!(
            run_func(
                "function f() { class A { #x = 1; hasX() { return #x in this; } } return new A().hasX() ? 1 : 0; }"
            ),
            1
        );
    }

    #[test]
    fn test_private_static_field() {
        assert_eq!(
            run_func(
                "function f() { class A { static #x = 7; static getX() { return A.#x; } } return A.getX(); }"
            ),
            7
        );
    }

    #[test]
    fn test_generator_basic() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let code = "(function*() { yield 1; yield 2; })";
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };
        let mut codegen = CodeGenerator::new();
        let func_reg = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let call_dst = codegen.alloc_register();
        codegen.emit(Opcode::Call);
        codegen.emit_u16(call_dst);
        codegen.emit_u16(func_reg);
        codegen.emit_u16(0);

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((call_dst + 1) as u32);
        bytecode.code.push(Opcode::Move as u8);
        bytecode.code.extend_from_slice(&0u16.to_le_bytes());
        bytecode.code.extend_from_slice(&call_dst.to_le_bytes());
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        assert!(result.is_object(), "expected generator object");
    }

    #[test]
    fn test_async_generator_basic() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let code = "(async function*() { yield 1; yield 2; })";
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };
        let mut codegen = CodeGenerator::new();
        let func_reg = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let call_dst = codegen.alloc_register();
        codegen.emit(Opcode::Call);
        codegen.emit_u16(call_dst);
        codegen.emit_u16(func_reg);
        codegen.emit_u16(0);

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((call_dst + 1) as u32);
        bytecode.code.push(Opcode::Move as u8);
        bytecode.code.extend_from_slice(&0u16.to_le_bytes());
        bytecode.code.extend_from_slice(&call_dst.to_le_bytes());
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        assert!(result.is_object(), "expected async generator object");
        let obj = result.as_object();
        assert!(obj.is_generator(), "expected generator flag to be set");
    }

    #[test]
    fn test_promise_resolve_in_register_vm() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let code = "Promise.resolve(42)";
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };
        let mut codegen = CodeGenerator::new();
        let dst = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((dst + 1) as u32);
        bytecode.code.push(Opcode::Move as u8);
        bytecode.code.extend_from_slice(&0u16.to_le_bytes());
        bytecode.code.extend_from_slice(&dst.to_le_bytes());
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        assert!(result.is_object(), "expected promise object");
        assert!(
            crate::builtins::promise::is_promise(&result),
            "expected a Promise"
        );
        let promise_obj = result.as_object();
        let state = promise_obj
            .get(ctx.common_atoms.__promise_state__)
            .unwrap_or(JSValue::new_int(0))
            .get_int();
        assert_eq!(state, 1, "expected resolved promise");
        let value = promise_obj
            .get(ctx.common_atoms.__promise_result__)
            .unwrap_or(JSValue::undefined());
        assert_eq!(value.get_int(), 42, "expected promise result to be 42");
    }

    #[test]
    fn test_async_function_await_resolved_promise() {
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let code = "(async function() { return await Promise.resolve(42); })";
        let ast = crate::compiler::parser::Parser::new(code).parse().unwrap();
        let expr = match ast.body.first() {
            Some(ASTNode::ExpressionStatement(es)) => es.expression.clone(),
            _ => panic!("expected expression"),
        };
        let mut codegen = CodeGenerator::new();
        let func_reg = codegen.compile_expression(&expr, &mut ctx).unwrap();

        let call_dst = codegen.alloc_register();
        codegen.emit(Opcode::Call);
        codegen.emit_u16(call_dst);
        codegen.emit_u16(func_reg);
        codegen.emit_u16(0);

        let mut bytecode = Bytecode::new();
        let (code, constants, locals_count, _) = codegen.take_bytecode();
        bytecode.code = code;
        bytecode.constants = constants;
        bytecode.locals_count = locals_count.max((call_dst + 1) as u32);
        bytecode.code.push(Opcode::Move as u8);
        bytecode.code.extend_from_slice(&0u16.to_le_bytes());
        bytecode.code.extend_from_slice(&call_dst.to_le_bytes());
        bytecode.code.push(Opcode::End as u8);

        let mut vm = VM::new();
        let result = match vm.execute(&mut ctx, &bytecode).unwrap() {
            crate::runtime::vm::ExecutionOutcome::Complete(v) => v,
            crate::runtime::vm::ExecutionOutcome::Yield(v) => v,
        };
        assert!(
            result.is_object(),
            "expected promise object from async function call"
        );

        assert!(
            crate::builtins::promise::is_promise(&result),
            "expected a Promise"
        );
        let promise_obj = result.as_object();
        let state = promise_obj
            .get(ctx.common_atoms.__promise_state__)
            .unwrap_or(JSValue::new_int(0))
            .get_int();
        assert_eq!(state, 1, "expected resolved promise");
        let value = promise_obj
            .get(ctx.common_atoms.__promise_result__)
            .unwrap_or(JSValue::undefined());
        assert_eq!(value.get_int(), 42, "expected promise result to be 42");
    }

    #[test]
    fn test_deep_nesting_no_overflow() {
        let code = "1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1";
        assert_eq!(run_expr(code), 49);
    }

    #[test]
    fn test_very_deep_nesting_no_overflow() {
        let ones: Vec<&str> = std::iter::repeat("1").take(100).collect();
        let code = ones.join(" + ");
        assert_eq!(run_expr(&code), 100);
    }

    #[test]
    fn test_extreme_deep_nesting_no_overflow() {
        let ones: Vec<&str> = std::iter::repeat("1").take(300).collect();
        let code = ones.join(" + ");
        assert_eq!(run_expr(&code), 300);
    }
}
