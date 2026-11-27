#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Nop = 0,
    End = 1,
    Return = 2,

    LoadConst = 10,
    LoadInt = 11,
    LoadInt8 = 12,
    LoadTrue = 13,
    LoadFalse = 14,
    LoadNull = 15,
    LoadUndefined = 16,
    LoadTdz = 17,
    CheckTdz = 18,
    CheckRef = 19,
    CheckObjectCoercible = 91,

    Add = 20,
    Sub = 21,
    SubImm8 = 34,
    AddNum = 35,
    SubNum = 36,
    MulNum = 37,
    DivNum = 38,
    AddImm8 = 39,
    Mul = 22,
    Div = 23,
    Mod = 24,
    Pow = 25,
    Neg = 26,
    BitAnd = 27,
    BitOr = 28,
    BitXor = 29,
    BitNot = 30,
    Shl = 31,
    Shr = 32,
    UShr = 33,

    Lt = 50,
    Lte = 51,
    LteImm8 = 60,
    Gt = 52,
    Gte = 53,
    Eq = 54,
    Neq = 55,
    StrictEq = 56,
    StrictNeq = 57,
    InstanceOf = 58,
    NewRegExp = 59,

    Not = 70,
    TypeOf = 71,

    Jump = 80,
    JumpIf = 81,
    JumpIfNot = 82,
    JumpBreak = 199,
    EndFinally = 200,
    ResetPerIterVar = 201,
    JumpIfNullish = 176,
    Throw = 83,
    Try = 84,
    Catch = 85,
    Finally = 86,
    Jump8 = 87,
    JumpIf8 = 88,
    JumpIfNot8 = 89,

    Move = 90,

    GetLocal = 100,
    SetLocal = 101,
    GetGlobal = 102,
    SetGlobal = 103,
    GetUpvalue = 104,
    SetUpvalue = 105,
    IncLocal = 106,
    DecLocal = 107,

    NewObject = 120,
    NewArray = 121,
    GetField = 122,
    SetField = 123,
    GetProp = 124,
    SetProp = 125,
    Call = 126,
    CallMethod = 127,
    NewFunction = 128,
    CallNew = 129,
    DeleteProp = 130,
    HasProperty = 131,
    GatherRest = 132,
    ObjectSpread = 133,
    GetPropertyNames = 134,
    ArrayExtend = 135,
    SetProto = 136,
    GetSuper = 137,
    ArrayPush = 138,
    CallSpread = 139,
    CallMethodSpread = 140,
    CallNewSpread = 141,
    GetPrivate = 142,
    SetPrivate = 143,
    HasPrivate = 144,
    Yield = 145,
    NewGeneratorFunction = 146,
    NewAsyncFunction = 147,
    Await = 148,
    NewAsyncGeneratorFunction = 149,
    GetIterator = 150,
    IteratorNext = 151,
    GetArguments = 152,
    GetCurrentFunction = 153,
    CallCurrent = 154,
    CallCurrent1 = 155,
    GetNamedProp = 156,
    SetNamedProp = 157,
    Call0 = 174,
    Call1 = 175,
    Call2 = 177,
    Call3 = 178,
    CallNamedMethod = 179,

    MathSin = 180,
    MathCos = 181,
    MathSqrt = 182,
    MathAbs = 183,
    MathFloor = 184,
    MathCeil = 185,
    MathRound = 186,
    MathPow = 187,
    MathMin = 188,
    MathMax = 189,

    DefineAccessor = 190,

    DefinePrivateAccessor = 191,

    SetMethodProp = 192,
    DefineClassPrototype = 208,
    DefineClassAccessor = 209,
    WithEnter = 210,
    WithExit = 211,
    WithHas = 212,
    WithGet = 213,
    WithSet = 214,
    WithGetVar = 215,
    WithSetVar = 216,
    WithDelete = 217,
    WithResolve = 218,
    WithGetResolved = 219,
    WithSetResolved = 220,

    Pos = 193,
    DefineGlobal = 194,
    DeleteGlobal = 195,
    ThrowReferenceError = 196,
    ThrowConstAssignment = 221,
    SetGlobalVar = 197,
    InitGlobalVar = 198,
    IteratorNextSend = 202,
    IteratorClose = 203,
    IteratorStep = 222,
    IteratorCloseFinally = 223,
    ToPropertyKey = 224,
    GetAsyncIterator = 204,
    IteratorNextAwait = 205,
    ObjectRest = 206,
    IteratorNextSendAwait = 207,

    LtJumpIfNot = 158,
    LtJumpIf = 159,
    LteJumpIfNot = 160,
    LteJumpIf = 161,
    GtJumpIfNot = 162,
    GtJumpIf = 163,
    GteJumpIfNot = 164,
    GteJumpIf = 165,
    EqJumpIfNot = 166,
    EqJumpIf = 167,
    NeqJumpIfNot = 168,
    NeqJumpIf = 169,
    StrictEqJumpIfNot = 170,
    StrictEqJumpIf = 171,
    StrictNeqJumpIfNot = 172,
    StrictNeqJumpIf = 173,
    ParamInitMark = 225,
    GetSuperBase = 226,
    SetHomeObject = 227,
    GetSuperProp = 228,
    SetSuperProp = 229,
    CheckThisInitialized = 230,
    GetNewTarget = 231,
    MarkDirectEval = 232,
    YieldResult = 233,
    YieldStarStep = 234,
    DefinePrivateField = 235,
    DefinePrivateMethod = 236,
    AddPrivateBrand = 237,
    AddInstanceField = 238,
    AddInstancePrivateField = 239,
    DefineField = 240,
    GetEvalPrivateOwner = 241,
    CheckObjectOrNull = 242,
    ArrayPushAll = 243,
    LoadCaughtException = 244,
    SetFunctionName = 245,
    ToNumeric = 246,
    AsyncYieldStarStep = 247,
    AsyncIteratorCloseFinally = 248,
    AddDisposableResource = 249,
    DisposeResourcesFinally = 250,
    SetUpvalueDirect = 251,
    GlobalBinding = 252,
    WithGetVarBase = 253,
    CheckConstructor = 254,
}

impl Opcode {
    #[inline(always)]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Opcode::Nop),
            1 => Some(Opcode::End),
            2 => Some(Opcode::Return),
            10 => Some(Opcode::LoadConst),
            11 => Some(Opcode::LoadInt),
            12 => Some(Opcode::LoadInt8),
            13 => Some(Opcode::LoadTrue),
            14 => Some(Opcode::LoadFalse),
            15 => Some(Opcode::LoadNull),
            16 => Some(Opcode::LoadUndefined),
            17 => Some(Opcode::LoadTdz),
            18 => Some(Opcode::CheckTdz),
            19 => Some(Opcode::CheckRef),
            91 => Some(Opcode::CheckObjectCoercible),
            20 => Some(Opcode::Add),
            21 => Some(Opcode::Sub),
            34 => Some(Opcode::SubImm8),
            35 => Some(Opcode::AddNum),
            36 => Some(Opcode::SubNum),
            37 => Some(Opcode::MulNum),
            38 => Some(Opcode::DivNum),
            39 => Some(Opcode::AddImm8),
            22 => Some(Opcode::Mul),
            23 => Some(Opcode::Div),
            24 => Some(Opcode::Mod),
            25 => Some(Opcode::Pow),
            26 => Some(Opcode::Neg),
            27 => Some(Opcode::BitAnd),
            28 => Some(Opcode::BitOr),
            29 => Some(Opcode::BitXor),
            30 => Some(Opcode::BitNot),
            31 => Some(Opcode::Shl),
            32 => Some(Opcode::Shr),
            33 => Some(Opcode::UShr),
            50 => Some(Opcode::Lt),
            51 => Some(Opcode::Lte),
            60 => Some(Opcode::LteImm8),
            52 => Some(Opcode::Gt),
            53 => Some(Opcode::Gte),
            54 => Some(Opcode::Eq),
            55 => Some(Opcode::Neq),
            56 => Some(Opcode::StrictEq),
            57 => Some(Opcode::StrictNeq),
            58 => Some(Opcode::InstanceOf),
            59 => Some(Opcode::NewRegExp),
            70 => Some(Opcode::Not),
            71 => Some(Opcode::TypeOf),
            80 => Some(Opcode::Jump),
            81 => Some(Opcode::JumpIf),
            82 => Some(Opcode::JumpIfNot),
            199 => Some(Opcode::JumpBreak),
            200 => Some(Opcode::EndFinally),
            201 => Some(Opcode::ResetPerIterVar),
            176 => Some(Opcode::JumpIfNullish),
            83 => Some(Opcode::Throw),
            84 => Some(Opcode::Try),
            85 => Some(Opcode::Catch),
            86 => Some(Opcode::Finally),
            87 => Some(Opcode::Jump8),
            88 => Some(Opcode::JumpIf8),
            89 => Some(Opcode::JumpIfNot8),
            90 => Some(Opcode::Move),
            100 => Some(Opcode::GetLocal),
            101 => Some(Opcode::SetLocal),
            102 => Some(Opcode::GetGlobal),
            103 => Some(Opcode::SetGlobal),
            104 => Some(Opcode::GetUpvalue),
            105 => Some(Opcode::SetUpvalue),
            106 => Some(Opcode::IncLocal),
            107 => Some(Opcode::DecLocal),
            120 => Some(Opcode::NewObject),
            121 => Some(Opcode::NewArray),
            122 => Some(Opcode::GetField),
            123 => Some(Opcode::SetField),
            124 => Some(Opcode::GetProp),
            125 => Some(Opcode::SetProp),
            126 => Some(Opcode::Call),
            127 => Some(Opcode::CallMethod),
            128 => Some(Opcode::NewFunction),
            129 => Some(Opcode::CallNew),
            130 => Some(Opcode::DeleteProp),
            131 => Some(Opcode::HasProperty),
            132 => Some(Opcode::GatherRest),
            133 => Some(Opcode::ObjectSpread),
            134 => Some(Opcode::GetPropertyNames),
            135 => Some(Opcode::ArrayExtend),
            136 => Some(Opcode::SetProto),
            137 => Some(Opcode::GetSuper),
            138 => Some(Opcode::ArrayPush),
            139 => Some(Opcode::CallSpread),
            140 => Some(Opcode::CallMethodSpread),
            141 => Some(Opcode::CallNewSpread),
            142 => Some(Opcode::GetPrivate),
            143 => Some(Opcode::SetPrivate),
            144 => Some(Opcode::HasPrivate),
            145 => Some(Opcode::Yield),
            146 => Some(Opcode::NewGeneratorFunction),
            147 => Some(Opcode::NewAsyncFunction),
            148 => Some(Opcode::Await),
            149 => Some(Opcode::NewAsyncGeneratorFunction),
            150 => Some(Opcode::GetIterator),
            151 => Some(Opcode::IteratorNext),
            202 => Some(Opcode::IteratorNextSend),
            203 => Some(Opcode::IteratorClose),
            222 => Some(Opcode::IteratorStep),
            223 => Some(Opcode::IteratorCloseFinally),
            224 => Some(Opcode::ToPropertyKey),
            204 => Some(Opcode::GetAsyncIterator),
            205 => Some(Opcode::IteratorNextAwait),
            206 => Some(Opcode::ObjectRest),
            207 => Some(Opcode::IteratorNextSendAwait),
            152 => Some(Opcode::GetArguments),
            153 => Some(Opcode::GetCurrentFunction),
            154 => Some(Opcode::CallCurrent),
            155 => Some(Opcode::CallCurrent1),
            156 => Some(Opcode::GetNamedProp),
            157 => Some(Opcode::SetNamedProp),
            174 => Some(Opcode::Call0),
            175 => Some(Opcode::Call1),
            177 => Some(Opcode::Call2),
            178 => Some(Opcode::Call3),
            179 => Some(Opcode::CallNamedMethod),
            180 => Some(Opcode::MathSin),
            181 => Some(Opcode::MathCos),
            182 => Some(Opcode::MathSqrt),
            183 => Some(Opcode::MathAbs),
            184 => Some(Opcode::MathFloor),
            185 => Some(Opcode::MathCeil),
            186 => Some(Opcode::MathRound),
            187 => Some(Opcode::MathPow),
            188 => Some(Opcode::MathMin),
            189 => Some(Opcode::MathMax),
            190 => Some(Opcode::DefineAccessor),
            191 => Some(Opcode::DefinePrivateAccessor),
            192 => Some(Opcode::SetMethodProp),
            208 => Some(Opcode::DefineClassPrototype),
            209 => Some(Opcode::DefineClassAccessor),
            210 => Some(Opcode::WithEnter),
            211 => Some(Opcode::WithExit),
            212 => Some(Opcode::WithHas),
            213 => Some(Opcode::WithGet),
            214 => Some(Opcode::WithSet),
            215 => Some(Opcode::WithGetVar),
            216 => Some(Opcode::WithSetVar),
            217 => Some(Opcode::WithDelete),
            218 => Some(Opcode::WithResolve),
            219 => Some(Opcode::WithGetResolved),
            220 => Some(Opcode::WithSetResolved),
            193 => Some(Opcode::Pos),
            194 => Some(Opcode::DefineGlobal),
            195 => Some(Opcode::DeleteGlobal),
            196 => Some(Opcode::ThrowReferenceError),
            221 => Some(Opcode::ThrowConstAssignment),
            197 => Some(Opcode::SetGlobalVar),
            198 => Some(Opcode::InitGlobalVar),
            158 => Some(Opcode::LtJumpIfNot),
            159 => Some(Opcode::LtJumpIf),
            160 => Some(Opcode::LteJumpIfNot),
            161 => Some(Opcode::LteJumpIf),
            162 => Some(Opcode::GtJumpIfNot),
            163 => Some(Opcode::GtJumpIf),
            164 => Some(Opcode::GteJumpIfNot),
            165 => Some(Opcode::GteJumpIf),
            166 => Some(Opcode::EqJumpIfNot),
            167 => Some(Opcode::EqJumpIf),
            168 => Some(Opcode::NeqJumpIfNot),
            169 => Some(Opcode::NeqJumpIf),
            170 => Some(Opcode::StrictEqJumpIfNot),
            171 => Some(Opcode::StrictEqJumpIf),
            172 => Some(Opcode::StrictNeqJumpIfNot),
            173 => Some(Opcode::StrictNeqJumpIf),
            225 => Some(Opcode::ParamInitMark),
            226 => Some(Opcode::GetSuperBase),
            227 => Some(Opcode::SetHomeObject),
            228 => Some(Opcode::GetSuperProp),
            229 => Some(Opcode::SetSuperProp),
            230 => Some(Opcode::CheckThisInitialized),
            231 => Some(Opcode::GetNewTarget),
            232 => Some(Opcode::MarkDirectEval),
            233 => Some(Opcode::YieldResult),
            234 => Some(Opcode::YieldStarStep),
            235 => Some(Opcode::DefinePrivateField),
            236 => Some(Opcode::DefinePrivateMethod),
            237 => Some(Opcode::AddPrivateBrand),
            238 => Some(Opcode::AddInstanceField),
            239 => Some(Opcode::AddInstancePrivateField),
            240 => Some(Opcode::DefineField),
            241 => Some(Opcode::GetEvalPrivateOwner),
            242 => Some(Opcode::CheckObjectOrNull),
            243 => Some(Opcode::ArrayPushAll),
            244 => Some(Opcode::LoadCaughtException),
            245 => Some(Opcode::SetFunctionName),
            246 => Some(Opcode::ToNumeric),
            247 => Some(Opcode::AsyncYieldStarStep),
            248 => Some(Opcode::AsyncIteratorCloseFinally),
            249 => Some(Opcode::AddDisposableResource),
            250 => Some(Opcode::DisposeResourcesFinally),
            251 => Some(Opcode::SetUpvalueDirect),
            252 => Some(Opcode::GlobalBinding),
            253 => Some(Opcode::WithGetVarBase),
            254 => Some(Opcode::CheckConstructor),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn from_u8_unchecked(v: u8) -> Self {
        Self::from_u8(v).unwrap_or(Opcode::Nop)
    }

    pub fn instruction_size(op: Opcode) -> usize {
        match op {
            Opcode::Nop | Opcode::End | Opcode::Catch | Opcode::Finally | Opcode::EndFinally => 1,
            Opcode::ParamInitMark => 2,
            Opcode::MarkDirectEval => 8,
            Opcode::ResetPerIterVar => 3,
            Opcode::Return => 3,
            Opcode::LoadTrue
            | Opcode::LoadFalse
            | Opcode::LoadNull
            | Opcode::LoadUndefined
            | Opcode::LoadTdz
            | Opcode::CheckTdz
            | Opcode::IncLocal
            | Opcode::DecLocal
            | Opcode::Throw
            | Opcode::GatherRest
            | Opcode::GetSuper
            | Opcode::GetSuperBase
            | Opcode::GetNewTarget
            | Opcode::Yield
            | Opcode::YieldResult
            | Opcode::NewObject
            | Opcode::GetArguments
            | Opcode::GetCurrentFunction
            | Opcode::CheckObjectCoercible => 3,
            Opcode::CheckThisInitialized => 1,
            Opcode::Neg | Opcode::BitNot | Opcode::Not | Opcode::TypeOf | Opcode::Move => 5,
            Opcode::SetHomeObject => 5,
            Opcode::GetSuperProp => 9,
            Opcode::SetSuperProp => 9,
            Opcode::Add
            | Opcode::AddNum
            | Opcode::SubNum
            | Opcode::MulNum
            | Opcode::DivNum
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Mod
            | Opcode::Pow
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::UShr
            | Opcode::Lt
            | Opcode::Lte
            | Opcode::Gt
            | Opcode::Gte
            | Opcode::Eq
            | Opcode::Neq
            | Opcode::StrictEq
            | Opcode::StrictNeq
            | Opcode::InstanceOf => 7,
            Opcode::SubImm8 | Opcode::AddImm8 | Opcode::LteImm8 => 6,
            Opcode::NewRegExp => 11,
            Opcode::LoadConst | Opcode::LoadInt => 7,
            Opcode::LoadInt8 => 4,
            Opcode::Jump => 5,
            Opcode::JumpBreak => 7,
            Opcode::JumpIf | Opcode::JumpIfNot | Opcode::JumpIfNullish => 7,
            Opcode::Jump8 => 2,
            Opcode::JumpIf8 | Opcode::JumpIfNot8 => 4,
            Opcode::Try => 9,
            Opcode::GetLocal
            | Opcode::SetLocal
            | Opcode::GetGlobal
            | Opcode::SetGlobal
            | Opcode::CheckRef => 7,
            Opcode::GetUpvalue | Opcode::SetUpvalue | Opcode::SetUpvalueDirect => 5,
            Opcode::NewArray => 5,
            Opcode::GetField | Opcode::GetProp => 7,
            Opcode::SetField | Opcode::SetProp => 7,
            Opcode::GetNamedProp | Opcode::SetNamedProp => 9,

            Opcode::LtJumpIfNot
            | Opcode::LtJumpIf
            | Opcode::LteJumpIfNot
            | Opcode::LteJumpIf
            | Opcode::GtJumpIfNot
            | Opcode::GtJumpIf
            | Opcode::GteJumpIfNot
            | Opcode::GteJumpIf
            | Opcode::EqJumpIfNot
            | Opcode::EqJumpIf
            | Opcode::NeqJumpIfNot
            | Opcode::NeqJumpIf
            | Opcode::StrictEqJumpIfNot
            | Opcode::StrictEqJumpIf
            | Opcode::StrictNeqJumpIfNot
            | Opcode::StrictNeqJumpIf => 9,
            Opcode::Call | Opcode::CallNew => 7,
            Opcode::Call0 => 5,
            Opcode::Call1 => 7,
            Opcode::Call2 => 9,
            Opcode::Call3 => 11,
            Opcode::CallCurrent => 5,
            Opcode::CallCurrent1 => 5,
            Opcode::CallMethod => 9,
            Opcode::CallNamedMethod => 11,
            Opcode::MathSin
            | Opcode::MathCos
            | Opcode::MathSqrt
            | Opcode::MathAbs
            | Opcode::MathFloor
            | Opcode::MathCeil
            | Opcode::MathRound => 5,
            Opcode::MathPow | Opcode::MathMin | Opcode::MathMax => 7,
            Opcode::DefineAccessor => 9,
            Opcode::DefinePrivateAccessor => 11,
            Opcode::SetMethodProp => 7,
            Opcode::DefineClassPrototype => 5,
            Opcode::DefineClassAccessor => 9,
            Opcode::WithEnter => 3,
            Opcode::WithExit => 1,
            Opcode::WithHas | Opcode::WithGet | Opcode::WithDelete => 7,
            Opcode::WithSet => 7,
            Opcode::WithGetVar | Opcode::WithSetVar => 11,
            Opcode::WithGetVarBase => 12,
            Opcode::CheckConstructor => 3,
            Opcode::WithResolve => 7,
            Opcode::WithGetResolved | Opcode::WithSetResolved => 9,
            Opcode::Pos => 5,
            Opcode::DefineGlobal => 7,
            Opcode::DeleteGlobal => 7,
            Opcode::ThrowReferenceError => 5,
            Opcode::ThrowConstAssignment => 5,
            Opcode::SetGlobalVar => 7,
            Opcode::InitGlobalVar => 7,
            Opcode::GlobalBinding => 8,
            Opcode::NewFunction => 1,
            Opcode::DeleteProp => 7,
            Opcode::HasProperty => 7,
            Opcode::ObjectSpread => 5,
            Opcode::GetPropertyNames => 5,
            Opcode::ArrayExtend => 5,
            Opcode::SetProto => 5,
            Opcode::ArrayPush => 5,
            Opcode::CallSpread => 7,
            Opcode::CallMethodSpread => 9,
            Opcode::CallNewSpread => 7,
            Opcode::GetPrivate => 9,
            Opcode::SetPrivate => 9,
            Opcode::HasPrivate => 9,
            Opcode::DefinePrivateField => 9,
            Opcode::DefinePrivateMethod => 9,
            Opcode::AddPrivateBrand => 5,
            Opcode::AddInstanceField => 7,
            Opcode::AddInstancePrivateField => 11,
            Opcode::DefineField => 7,
            Opcode::GetEvalPrivateOwner => 7,
            Opcode::CheckObjectOrNull => 3,
            Opcode::ArrayPushAll => 5,
            Opcode::LoadCaughtException => 3,
            Opcode::SetFunctionName => 5,
            Opcode::ToNumeric => 5,
            Opcode::NewGeneratorFunction => 1,
            Opcode::NewAsyncFunction => 1,
            Opcode::NewAsyncGeneratorFunction => 1,
            Opcode::Await => 3,
            Opcode::GetIterator => 5,
            Opcode::IteratorNext => 7,
            Opcode::YieldStarStep => 9,
            Opcode::AsyncYieldStarStep => 13,
            Opcode::AsyncIteratorCloseFinally => 9,
            Opcode::AddDisposableResource => 7,
            Opcode::DisposeResourcesFinally => 12,
            Opcode::IteratorNextSend => 9,
            Opcode::IteratorClose => 3,
            Opcode::IteratorStep => 5,
            Opcode::IteratorCloseFinally => 3,
            Opcode::ToPropertyKey => 3,
            Opcode::GetAsyncIterator => 5,
            Opcode::IteratorNextAwait => 9,
            Opcode::ObjectRest => 7,
            Opcode::IteratorNextSendAwait => 9,
        }
    }

    pub fn instruction_size_at(code: &[u8], pc: usize) -> usize {
        let Some(byte) = code.get(pc) else {
            return 0;
        };
        match Opcode::from_u8_unchecked(*byte) {
            Opcode::NewFunction
            | Opcode::NewGeneratorFunction
            | Opcode::NewAsyncFunction
            | Opcode::NewAsyncGeneratorFunction => embedded_function_size(code, pc),
            Opcode::Call | Opcode::CallNew => 7 + read_u16_at(code, pc + 5) as usize * 2,
            Opcode::CallMethod => 9 + read_u16_at(code, pc + 7) as usize * 2,
            Opcode::CallCurrent => 5 + read_u16_at(code, pc + 3) as usize * 2,
            Opcode::CallNamedMethod => 11 + read_u16_at(code, pc + 9) as usize * 2,
            op => Opcode::instruction_size(op),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bytecode {
    pub code: Vec<u8>,
    pub constants: Vec<crate::value::JSValue>,
    pub locals_count: u32,
    pub param_count: u16,
    pub line_number_table: Option<std::sync::Arc<crate::compiler::location::LineNumberTable>>,
    pub ic_table: std::rc::Rc<std::cell::RefCell<crate::compiler::InlineCacheTable>>,

    pub shared_nested_bytecode: Option<std::sync::Arc<NestedBytecode>>,
    pub uses_arguments: bool,
    pub has_simple_parameters: bool,
    pub is_strict: bool,
    pub var_name_to_slot: std::rc::Rc<Vec<(u32, u16, u8)>>,
    pub nested_bytecodes: std::cell::RefCell<Vec<(u32, std::sync::Arc<NestedBytecode>)>>,
    pub is_simple_constructor: bool,
    pub simple_constructor_props: Vec<(crate::runtime::atom::Atom, u16, u16)>,
    pub cached_constructor_atoms: Vec<crate::runtime::atom::Atom>,
    pub cached_constructor_final_shape: Option<std::rc::Rc<crate::object::shape::Shape>>,
}

pub struct NestedBytecode {
    pub code: Vec<u8>,
    pub constants: Vec<crate::value::JSValue>,
    pub locals_count: u32,
    pub param_count: u16,
    pub uses_arguments: bool,
    pub has_simple_parameters: bool,
    pub is_strict: bool,
    pub function_kind: u8,
    pub var_name_to_slot: std::rc::Rc<Vec<(u32, u16, u8)>>,
    pub line_number_table: Option<std::sync::Arc<crate::compiler::location::LineNumberTable>>,

    pub parent_bytecode_span: u32,

    pub upvalue_count: u32,

    pub upvalue_descs: Vec<(u32, u32)>,

    pub func_name_atom: u32,

    pub source_text: Option<String>,

    pub ic_table: std::rc::Rc<std::cell::RefCell<crate::compiler::InlineCacheTable>>,

    pub nested_bytecodes: std::cell::RefCell<Vec<(u32, std::sync::Arc<NestedBytecode>)>>,
}

impl std::fmt::Debug for NestedBytecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedBytecode")
            .field("code_len", &self.code.len())
            .field("locals_count", &self.locals_count)
            .field("param_count", &self.param_count)
            .finish()
    }
}

fn read_u8_checked(data: &[u8], offset: &mut usize) -> Result<u8, String> {
    let byte = *data
        .get(*offset)
        .ok_or_else(|| "Invalid .jsc: truncated input".to_string())?;
    *offset += 1;
    Ok(byte)
}

fn read_u16_checked(data: &[u8], offset: &mut usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "Invalid .jsc: offset overflow".to_string())?;
    let bytes = data
        .get(*offset..end)
        .ok_or_else(|| "Invalid .jsc: truncated input".to_string())?;
    *offset = end;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_checked(data: &[u8], offset: &mut usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "Invalid .jsc: offset overflow".to_string())?;
    let bytes = data
        .get(*offset..end)
        .ok_or_else(|| "Invalid .jsc: truncated input".to_string())?;
    *offset = end;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64_checked(data: &[u8], offset: &mut usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "Invalid .jsc: offset overflow".to_string())?;
    let bytes = data
        .get(*offset..end)
        .ok_or_else(|| "Invalid .jsc: truncated input".to_string())?;
    *offset = end;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_u32_at(code: &[u8], offset: usize) -> u32 {
    let Some(end) = offset.checked_add(4) else {
        return 0;
    };
    let Some(bytes) = code.get(offset..end) else {
        return 0;
    };
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u16_at(code: &[u8], offset: usize) -> u16 {
    let Some(end) = offset.checked_add(2) else {
        return 0;
    };
    let Some(bytes) = code.get(offset..end) else {
        return 0;
    };
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn embedded_function_size(code: &[u8], start: usize) -> usize {
    let mut pc = start;
    if pc >= code.len() {
        return 0;
    }
    pc += 1;
    if pc + 2 > code.len() {
        return pc - start;
    }
    pc += 2;
    if pc + 1 > code.len() {
        return pc - start;
    }
    pc += 1;
    if pc + 1 > code.len() {
        return pc - start;
    }
    pc += 1;
    if pc + 1 > code.len() {
        return pc - start;
    }
    pc += 1;
    if pc + 1 > code.len() {
        return pc - start;
    }
    pc += 1;
    if pc + 2 > code.len() {
        return pc - start;
    }
    pc += 2;
    if pc + 4 > code.len() {
        return pc - start;
    }
    let bytecode_len = read_u32_at(code, pc) as usize;
    pc += 4;
    pc += bytecode_len;
    if pc + 4 > code.len() {
        return pc - start;
    }
    let constants_len = read_u32_at(code, pc) as usize;
    pc += 4;
    for _ in 0..constants_len {
        if pc >= code.len() {
            return pc - start;
        }
        let tag = code[pc];
        pc += 1;
        match tag {
            4 | 5 => {
                pc += 8;
            }
            6 => {
                pc += 4;
            }
            _ => {}
        }
    }
    if pc + 4 > code.len() {
        return pc - start;
    }
    let upvalue_count = read_u32_at(code, pc) as usize;
    pc += 4;
    pc += upvalue_count * 8;
    if pc + 4 > code.len() {
        return pc - start;
    }
    let line_entry_count = read_u32_at(code, pc) as usize;
    pc += 4;
    pc += line_entry_count * 8;
    if pc + 4 > code.len() {
        return pc - start;
    }
    pc += 4;
    if pc + 4 > code.len() {
        return pc - start;
    }
    let var_count = read_u32_at(code, pc) as usize;
    pc += 4;
    pc += var_count * 7;
    if pc + 2 > code.len() {
        return pc - start;
    }
    pc += 2;
    pc - start
}

impl Bytecode {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            locals_count: 0,
            param_count: 0,
            uses_arguments: false,
            has_simple_parameters: false,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            line_number_table: None,
            ic_table: std::rc::Rc::new(std::cell::RefCell::new(crate::compiler::InlineCacheTable::new())),
            shared_nested_bytecode: None,
            nested_bytecodes: std::cell::RefCell::new(Vec::new()),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn effective_ic_table(
        &self,
    ) -> std::rc::Rc<std::cell::RefCell<crate::compiler::InlineCacheTable>> {
        match &self.shared_nested_bytecode {
            Some(nested) => nested.ic_table.clone(),
            None => self.ic_table.clone(),
        }
    }

    #[inline(always)]
    pub fn effective_code_ptr(&self) -> *const u8 {
        if self.code.is_empty()
            && let Some(nested) = &self.shared_nested_bytecode
        {
            return nested.code.as_ptr();
        }
        self.code.as_ptr()
    }

    #[inline(always)]
    pub fn effective_code_len(&self) -> usize {
        if self.code.is_empty()
            && let Some(nested) = &self.shared_nested_bytecode
        {
            return nested.code.len();
        }
        self.code.len()
    }

    #[inline(always)]
    pub fn effective_const_ptr(&self) -> *const crate::value::JSValue {
        if self.constants.is_empty()
            && let Some(nested) = &self.shared_nested_bytecode
        {
            return nested.constants.as_ptr();
        }
        self.constants.as_ptr()
    }

    #[inline(always)]
    pub fn effective_const_len(&self) -> usize {
        if self.constants.is_empty()
            && let Some(nested) = &self.shared_nested_bytecode
        {
            return nested.constants.len();
        }
        self.constants.len()
    }

    pub fn disassemble(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        writeln!(out, "=== Register Bytecode ===").unwrap();
        writeln!(
            out,
            "locals_count: {}, param_count: {}",
            self.locals_count, self.param_count
        )
        .unwrap();
        let mut pc = 0usize;
        while pc < self.code.len() {
            let op_val = self.code[pc];
            let op = Opcode::from_u8_unchecked(op_val);
            let size = Opcode::instruction_size_at(&self.code, pc);
            let remaining = self.code.len().saturating_sub(pc + 1);

            let read_u8 = |offset: usize| -> u8 {
                if offset <= remaining {
                    self.code.get(pc + offset).copied().unwrap_or(0)
                } else {
                    0
                }
            };
            let read_i32 = |offset: usize| -> i32 {
                let bytes = [
                    read_u8(offset),
                    read_u8(offset + 1),
                    read_u8(offset + 2),
                    read_u8(offset + 3),
                ];
                i32::from_le_bytes(bytes)
            };
            let read_u32 = |offset: usize| -> u32 { read_i32(offset) as u32 };
            let read_u16 = |offset: usize| -> u16 {
                u16::from_le_bytes([read_u8(offset), read_u8(offset + 1)])
            };
            let read_i8 = |offset: usize| -> i8 { read_u8(offset) as i8 };

            let mut operands = String::new();
            match op {
                Opcode::Nop
                | Opcode::End
                | Opcode::Catch
                | Opcode::CheckThisInitialized
                | Opcode::Finally
                | Opcode::EndFinally => {}
                Opcode::ParamInitMark => {
                    let a = read_u8(1);
                    write!(operands, " {}", a).unwrap();
                }
                Opcode::MarkDirectEval => {
                    let flags = read_u8(1);
                    let start = read_u32(2);
                    let count = read_u16(6);
                    write!(operands, " {}, {}, {}", flags, start, count).unwrap();
                }
                Opcode::ResetPerIterVar => {
                    let a = read_u16(1);
                    write!(operands, " r{}", a).unwrap();
                }
                Opcode::Return
                | Opcode::LoadTrue
                | Opcode::LoadFalse
                | Opcode::LoadNull
                | Opcode::LoadUndefined
                | Opcode::IncLocal
                | Opcode::DecLocal => {
                    let a = read_u16(1);
                    write!(operands, " r{}", a).unwrap();
                }
                Opcode::LoadConst => {
                    let dst = read_u16(1);
                    let idx = read_u32(3);
                    write!(operands, " r{}, const[{}]", dst, idx).unwrap();
                }
                Opcode::LoadInt => {
                    let dst = read_u16(1);
                    let val = read_i32(3);
                    write!(operands, " r{}, {}", dst, val).unwrap();
                }
                Opcode::LoadInt8 => {
                    let dst = read_u16(1);
                    let val = read_u8(3) as i8;
                    write!(operands, " r{}, {}", dst, val).unwrap();
                }
                Opcode::Neg
                | Opcode::BitNot
                | Opcode::Not
                | Opcode::TypeOf
                | Opcode::Throw
                | Opcode::GatherRest
                | Opcode::GetSuper
                | Opcode::GetSuperBase
                | Opcode::GetNewTarget
                | Opcode::Yield
                | Opcode::YieldResult
                | Opcode::NewObject
                | Opcode::GetArguments
                | Opcode::GetCurrentFunction => {
                    let a = read_u16(1);
                    write!(operands, " r{}", a).unwrap();
                }
                Opcode::Add
                | Opcode::AddNum
                | Opcode::SubNum
                | Opcode::MulNum
                | Opcode::DivNum
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::Div
                | Opcode::Mod
                | Opcode::Pow
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::Shl
                | Opcode::Shr
                | Opcode::UShr
                | Opcode::Lt
                | Opcode::Lte
                | Opcode::Gt
                | Opcode::Gte
                | Opcode::Eq
                | Opcode::Neq
                | Opcode::StrictEq
                | Opcode::StrictNeq => {
                    let dst = read_u16(1);
                    let src1 = read_u16(3);
                    let src2 = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, src1, src2).unwrap();
                }
                Opcode::InstanceOf => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let ctor = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, obj, ctor).unwrap();
                }
                Opcode::NewRegExp => {
                    let dst = read_u16(1);
                    let pattern_idx = read_u32(3);
                    let flags_idx = read_u32(7);
                    write!(
                        operands,
                        " r{}, pattern={}, flags={}",
                        dst, pattern_idx, flags_idx
                    )
                    .unwrap();
                }
                Opcode::SubImm8 | Opcode::AddImm8 | Opcode::LteImm8 => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    let imm = read_i8(5);
                    write!(operands, " r{}, r{}, {}", dst, src, imm).unwrap();
                }
                Opcode::Jump => {
                    let off = read_i32(1);
                    let size = Opcode::instruction_size(Opcode::Jump) as i32;
                    write!(operands, " {} (pc {})", off, pc as i32 + size + off).unwrap();
                }
                Opcode::JumpBreak => {
                    let depth = read_u16(1);
                    let off = read_i32(3);
                    let size = Opcode::instruction_size(Opcode::JumpBreak) as i32;
                    write!(
                        operands,
                        " depth {} {} (pc {})",
                        depth,
                        off,
                        pc as i32 + size + off
                    )
                    .unwrap();
                }
                Opcode::JumpIf | Opcode::JumpIfNot | Opcode::JumpIfNullish => {
                    let src = read_u16(1);
                    let off = read_i32(3);
                    let size = Opcode::instruction_size(Opcode::JumpIf) as i32;
                    write!(
                        operands,
                        " r{}, {} (pc {})",
                        src,
                        off,
                        pc as i32 + size + off
                    )
                    .unwrap();
                }
                Opcode::Jump8 => {
                    let off = read_i8(1);
                    let size = Opcode::instruction_size(Opcode::Jump8) as i32;
                    write!(operands, " {} (pc {})", off, pc as i32 + size + off as i32).unwrap();
                }
                Opcode::JumpIf8 | Opcode::JumpIfNot8 => {
                    let src = read_u16(1);
                    let off = read_i8(3);
                    let size = Opcode::instruction_size(Opcode::JumpIf8) as i32;
                    write!(
                        operands,
                        " r{}, {} (pc {})",
                        src,
                        off,
                        pc as i32 + size + off as i32
                    )
                    .unwrap();
                }
                Opcode::Try => {
                    let catch = read_i32(1);
                    let finally = read_i32(5);
                    write!(operands, " catch={} finally={}", catch, finally).unwrap();
                }
                Opcode::Move => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::SetHomeObject => {
                    let func = read_u16(1);
                    let home = read_u16(3);
                    write!(operands, " r{}, r{}", func, home).unwrap();
                }
                Opcode::GetSuperProp => {
                    let dst = read_u16(1);
                    let base = read_u16(3);
                    let this = read_u16(5);
                    let key = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst, base, this, key).unwrap();
                }
                Opcode::SetSuperProp => {
                    let base = read_u16(1);
                    let this = read_u16(3);
                    let key = read_u16(5);
                    let value = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", base, this, key, value).unwrap();
                }
                Opcode::GetLocal | Opcode::GetGlobal => {
                    let dst = read_u16(1);
                    let idx = read_u32(3);
                    write!(operands, " r{}, {}", dst, idx).unwrap();
                }
                Opcode::SetLocal | Opcode::SetGlobal => {
                    let idx = read_u32(1);
                    let src = read_u16(5);
                    write!(operands, " {}, r{}", idx, src).unwrap();
                }
                Opcode::GetUpvalue | Opcode::SetUpvalue | Opcode::SetUpvalueDirect => {
                    let a = read_u16(1);
                    let b = read_u16(3);
                    write!(operands, " r{}, r{}", a, b).unwrap();
                }
                Opcode::NewArray => {
                    let dst = read_u16(1);
                    let count = read_u16(3);
                    write!(operands, " r{}, {}", dst, count).unwrap();
                }
                Opcode::GetField
                | Opcode::GetProp
                | Opcode::DeleteProp
                | Opcode::HasProperty => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let key = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, obj, key).unwrap();
                }
                Opcode::GetPrivate | Opcode::HasPrivate => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let key = read_u16(5);
                    let owner = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst, obj, key, owner).unwrap();
                }
                Opcode::GetNamedProp => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let atom = read_u32(5);
                    write!(operands, " r{}, r{}, atom({})", dst, obj, atom).unwrap();
                }
                Opcode::SetField | Opcode::SetProp => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let val = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", obj, key, val).unwrap();
                }
                Opcode::SetPrivate | Opcode::DefinePrivateField | Opcode::DefinePrivateMethod => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let val = read_u16(5);
                    let owner = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", obj, key, val, owner).unwrap();
                }
                Opcode::AddPrivateBrand => {
                    let obj = read_u16(1);
                    let owner = read_u16(3);
                    write!(operands, " r{}, r{}", obj, owner).unwrap();
                }
                Opcode::AddInstanceField => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let init = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", obj, key, init).unwrap();
                }
                Opcode::AddInstancePrivateField => {
                    let obj = read_u16(1);
                    let owner = read_u16(3);
                    let name = read_u32(5);
                    let init = read_u16(9);
                    write!(operands, " r{}, r{}, atom({}), r{}", obj, owner, name, init).unwrap();
                }
                Opcode::DefineField => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let val = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", obj, key, val).unwrap();
                }
                Opcode::GetEvalPrivateOwner => {
                    let dst = read_u16(1);
                    let name = read_u32(3);
                    write!(operands, " r{}, atom({})", dst, name).unwrap();
                }
                Opcode::CheckObjectOrNull => {
                    let reg = read_u16(1);
                    write!(operands, " r{}", reg).unwrap();
                }
                Opcode::ArrayPushAll => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::LoadCaughtException => {
                    let dst = read_u16(1);
                    write!(operands, " r{}", dst).unwrap();
                }
                Opcode::SetFunctionName => {
                    let func = read_u16(1);
                    let key = read_u16(3);
                    write!(operands, " r{}, r{}", func, key).unwrap();
                }
                Opcode::SetNamedProp => {
                    let obj = read_u16(1);
                    let val = read_u16(3);
                    let atom = read_u32(5);
                    write!(operands, " r{}, r{}, atom({})", obj, val, atom).unwrap();
                }
                Opcode::LtJumpIfNot
                | Opcode::LtJumpIf
                | Opcode::LteJumpIfNot
                | Opcode::LteJumpIf
                | Opcode::GtJumpIfNot
                | Opcode::GtJumpIf
                | Opcode::GteJumpIfNot
                | Opcode::GteJumpIf
                | Opcode::EqJumpIfNot
                | Opcode::EqJumpIf
                | Opcode::NeqJumpIfNot
                | Opcode::NeqJumpIf
                | Opcode::StrictEqJumpIfNot
                | Opcode::StrictEqJumpIf
                | Opcode::StrictNeqJumpIfNot
                | Opcode::StrictNeqJumpIf => {
                    let a = read_u16(3);
                    let b = read_u16(5);
                    let offset = read_i32(10);
                    let target = (pc as i64 + 14 + offset as i64) as usize;
                    write!(operands, " r{}, r{} -> {}", a, b, target).unwrap();
                }
                Opcode::Call | Opcode::CallNew => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    let argc = read_u16(5);
                    write!(operands, " r{}, r{}, argc={}", dst, func, argc).unwrap();
                }
                Opcode::Call0 => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    write!(operands, " r{}, r{}", dst, func).unwrap();
                }
                Opcode::Call1 => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    let arg = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, func, arg).unwrap();
                }
                Opcode::Call2 => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    let arg0 = read_u16(5);
                    let arg1 = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst, func, arg0, arg1).unwrap();
                }
                Opcode::Call3 => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    let arg0 = read_u16(5);
                    let arg1 = read_u16(7);
                    let arg2 = read_u16(9);
                    write!(
                        operands,
                        " r{}, r{}, r{}, r{}, r{}",
                        dst, func, arg0, arg1, arg2
                    )
                    .unwrap();
                }
                Opcode::CallCurrent => {
                    let dst = read_u16(1);
                    let argc = read_u16(3);
                    write!(operands, " r{}, argc={}", dst, argc).unwrap();
                }
                Opcode::CallCurrent1 => {
                    let dst = read_u16(1);
                    let arg = read_u16(3);
                    write!(operands, " r{}, r{}", dst, arg).unwrap();
                }
                Opcode::CallMethod => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let func = read_u16(5);
                    let argc = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, argc={}", dst, obj, func, argc).unwrap();
                }
                Opcode::CallNamedMethod => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let atom = read_u32(5);
                    let argc = read_u16(9);
                    write!(
                        operands,
                        " r{}, r{}, atom={}, argc={}",
                        dst, obj, atom, argc
                    )
                    .unwrap();
                }
                Opcode::NewFunction
                | Opcode::NewGeneratorFunction
                | Opcode::NewAsyncFunction
                | Opcode::NewAsyncGeneratorFunction => {
                    write!(operands, " <embedded>").unwrap();
                }
                Opcode::ObjectSpread
                | Opcode::GetPropertyNames
                | Opcode::ArrayExtend
                | Opcode::SetProto
                | Opcode::ArrayPush => {
                    let a = read_u16(1);
                    let b = read_u16(3);
                    write!(operands, " r{}, r{}", a, b).unwrap();
                }
                Opcode::CallSpread | Opcode::CallNewSpread => {
                    let dst = read_u16(1);
                    let func = read_u16(3);
                    let arr = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, func, arr).unwrap();
                }
                Opcode::CallMethodSpread => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let func = read_u16(5);
                    let arr = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst, obj, func, arr).unwrap();
                }
                Opcode::MathSin
                | Opcode::MathCos
                | Opcode::MathSqrt
                | Opcode::MathAbs
                | Opcode::MathFloor
                | Opcode::MathCeil
                | Opcode::MathRound => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::MathPow | Opcode::MathMin | Opcode::MathMax => {
                    let dst = read_u16(1);
                    let a = read_u16(3);
                    let b = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, a, b).unwrap();
                }
                Opcode::DefineAccessor => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let getter = read_u16(5);
                    let setter = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", obj, key, getter, setter).unwrap();
                }
                Opcode::DefinePrivateAccessor => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let getter = read_u16(5);
                    let setter = read_u16(7);
                    let owner = read_u16(9);
                    write!(
                        operands,
                        " r{}, r{}, r{}, r{}, r{}",
                        obj, key, getter, setter, owner
                    )
                    .unwrap();
                }
                Opcode::SetMethodProp => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let val = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", obj, key, val).unwrap();
                }
                Opcode::DefineClassPrototype => {
                    let ctor = read_u16(1);
                    let proto = read_u16(3);
                    write!(operands, " r{}, r{}", ctor, proto).unwrap();
                }
                Opcode::DefineClassAccessor => {
                    let obj = read_u16(1);
                    let key = read_u16(3);
                    let getter = read_u16(5);
                    let setter = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", obj, key, getter, setter).unwrap();
                }
                Opcode::WithEnter => {
                    let src = read_u16(1);
                    write!(operands, " r{}", src).unwrap();
                }
                Opcode::WithExit => {
                    write!(operands, "").unwrap();
                }
                Opcode::WithHas | Opcode::WithGet | Opcode::WithDelete => {
                    let dst = read_u16(1);
                    let atom = read_u32(3);
                    write!(operands, " r{}, atom({})", dst, atom).unwrap();
                }
                Opcode::WithSet => {
                    let atom = read_u32(1);
                    let val = read_u16(5);
                    write!(operands, " atom({}), r{}", atom, val).unwrap();
                }
                Opcode::WithResolve => {
                    let dst = read_u16(1);
                    let atom = read_u32(3);
                    write!(operands, " r{}, atom({})", dst, atom).unwrap();
                }
                Opcode::WithGetResolved => {
                    let dst = read_u16(1);
                    let obj = read_u16(3);
                    let atom = read_u32(5);
                    write!(operands, " r{}, r{}, atom({})", dst, obj, atom).unwrap();
                }
                Opcode::WithSetResolved => {
                    let obj = read_u16(1);
                    let val = read_u16(3);
                    let atom = read_u32(5);
                    write!(operands, " r{}, r{}, atom({})", obj, val, atom).unwrap();
                }
                Opcode::WithGetVar => {
                    let dst = read_u16(1);
                    let atom = read_u32(3);
                    let kind = read_u8(7);
                    let index = read_u16(8);
                    let flags = read_u8(10);
                    write!(
                        operands,
                        " r{}, atom({}), kind={}, idx={}, flags={}",
                        dst, atom, kind, index, flags
                    )
                    .unwrap();
                }
                Opcode::WithGetVarBase => {
                    let dst = read_u16(1);
                    let base = read_u16(3);
                    let atom = read_u32(5);
                    let kind = read_u8(9);
                    let index = read_u16(10);
                    let flags = read_u8(12);
                    write!(
                        operands,
                        " r{}, r{}, atom({}), kind={}, idx={}, flags={}",
                        dst, base, atom, kind, index, flags
                    )
                    .unwrap();
                }
                Opcode::CheckConstructor => {
                    let reg = read_u16(1);
                    write!(operands, " r{}", reg).unwrap();
                }
                Opcode::WithSetVar => {
                    let atom = read_u32(1);
                    let val = read_u16(5);
                    let kind = read_u8(7);
                    let index = read_u16(8);
                    let flags = read_u8(10);
                    write!(
                        operands,
                        " atom({}), r{}, kind={}, idx={}, flags={}",
                        atom, val, kind, index, flags
                    )
                    .unwrap();
                }
                Opcode::Pos | Opcode::ToNumeric => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::DefineGlobal => {
                    let idx = read_u32(1);
                    let src = read_u16(5);
                    write!(operands, " {}, r{}", idx, src).unwrap();
                }
                Opcode::DeleteGlobal => {
                    let dst = read_u16(1);
                    let idx = read_u32(3);
                    write!(operands, " r{}, {}", dst, idx).unwrap();
                }
                Opcode::ThrowReferenceError | Opcode::ThrowConstAssignment => {
                    let idx = read_u32(1);
                    write!(operands, " const[{}]", idx).unwrap();
                }
                Opcode::SetGlobalVar => {
                    let idx = read_u32(1);
                    let src = read_u16(5);
                    write!(operands, " {}, r{}", idx, src).unwrap();
                }
                Opcode::InitGlobalVar => {
                    let idx = read_u32(1);
                    let src = read_u16(5);
                    write!(operands, " {}, r{}", idx, src).unwrap();
                }
                Opcode::GlobalBinding => {
                    let mode = read_u8(1);
                    let idx = read_u32(2);
                    let src = read_u16(6);
                    write!(operands, " {}, {}, r{}", mode, idx, src).unwrap();
                }
                Opcode::Await => {
                    let src = read_u16(1);
                    write!(operands, " r{}", src).unwrap();
                }
                Opcode::LoadTdz | Opcode::CheckTdz => {
                    let a = read_u16(1);
                    write!(operands, " r{}", a).unwrap();
                }
                Opcode::CheckRef => {
                    let a = read_u16(1);
                    let idx = read_u32(3);
                    write!(operands, " r{}, const[{}]", a, idx).unwrap();
                }
                Opcode::CheckObjectCoercible => {
                    let a = read_u16(1);
                    write!(operands, " r{}", a).unwrap();
                }
                Opcode::GetIterator => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::IteratorNext => {
                    let dst_val = read_u16(1);
                    let dst_done = read_u16(3);
                    let iter = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst_val, dst_done, iter).unwrap();
                }
                Opcode::YieldStarStep => {
                    let iter = read_u16(1);
                    let value = read_u16(3);
                    let done = read_u16(5);
                    let returning = read_u16(7);
                    write!(
                        operands,
                        " r{}, r{}, r{}, r{}",
                        iter, value, done, returning
                    )
                    .unwrap();
                }
                Opcode::AsyncYieldStarStep => {
                    let iter = read_u16(1);
                    let value = read_u16(3);
                    let done = read_u16(5);
                    let returning = read_u16(7);
                    let state = read_u16(9);
                    let awaited = read_u16(11);
                    write!(
                        operands,
                        " r{}, r{}, r{}, r{}, r{}, r{}",
                        iter, value, done, returning, state, awaited
                    )
                    .unwrap();
                }
                Opcode::AsyncIteratorCloseFinally => {
                    let iter = read_u16(1);
                    let state = read_u16(3);
                    let awaited = read_u16(5);
                    let done = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", iter, state, awaited, done)
                    .unwrap();
                }
                Opcode::AddDisposableResource => {
                    let value = read_u16(1);
                    let method = read_u16(3);
                    let kind = read_u8(5);
                    let stack = read_u16(6);
                    write!(
                        operands,
                        " r{}, r{}, {}, r{}",
                        value, method, kind, stack
                    )
                    .unwrap();
                }
                Opcode::DisposeResourcesFinally => {
                    let stack = read_u16(1);
                    let state = read_u16(3);
                    let index = read_u16(5);
                    let awaited = read_u16(7);
                    let error = read_u16(9);
                    let flags = read_u16(11);
                    write!(
                        operands,
                        " r{}, r{}, r{}, r{}, r{}, r{}",
                        stack, state, index, awaited, error, flags
                    )
                    .unwrap();
                }
                Opcode::IteratorNextSend => {
                    let dst_val = read_u16(1);
                    let dst_done = read_u16(3);
                    let iter = read_u16(5);
                    let send = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst_val, dst_done, iter, send).unwrap();
                }
                Opcode::IteratorClose => {
                    let iter = read_u16(1);
                    write!(operands, " r{}", iter).unwrap();
                }
                Opcode::IteratorStep => {
                    let dst_done = read_u16(1);
                    let iter = read_u16(3);
                    write!(operands, " r{}, r{}", dst_done, iter).unwrap();
                }
                Opcode::IteratorCloseFinally => {
                    let iter = read_u16(1);
                    write!(operands, " r{}", iter).unwrap();
                }
                Opcode::ToPropertyKey => {
                    let reg = read_u16(1);
                    write!(operands, " r{}", reg).unwrap();
                }
                Opcode::GetAsyncIterator => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    write!(operands, " r{}, r{}", dst, src).unwrap();
                }
                Opcode::IteratorNextAwait => {
                    let dst_val = read_u16(1);
                    let dst_done = read_u16(3);
                    let iter = read_u16(5);
                    let state = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst_val, dst_done, iter, state).unwrap();
                }
                Opcode::ObjectRest => {
                    let dst = read_u16(1);
                    let src = read_u16(3);
                    let keys = read_u16(5);
                    write!(operands, " r{}, r{}, r{}", dst, src, keys).unwrap();
                }
                Opcode::IteratorNextSendAwait => {
                    let dst_val = read_u16(1);
                    let dst_done = read_u16(3);
                    let iter = read_u16(5);
                    let send = read_u16(7);
                    write!(operands, " r{}, r{}, r{}, r{}", dst_val, dst_done, iter, send).unwrap();
                }
            }
            writeln!(out, "{:04}  {:?}{}", pc, op, operands).unwrap();
            pc += size.max(1);
        }
        out
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();

        out.extend_from_slice(b"SAFC");

        out.extend_from_slice(&3u32.to_le_bytes());

        out.extend_from_slice(&self.locals_count.to_le_bytes());

        out.extend_from_slice(&self.param_count.to_le_bytes());

        out.push(self.uses_arguments as u8);

        out.push(self.has_simple_parameters as u8);

        out.extend_from_slice(&(self.code.len() as u64).to_le_bytes());

        out.extend_from_slice(&self.code);

        out.extend_from_slice(&(self.constants.len() as u64).to_le_bytes());

        for c in &self.constants {
            out.extend_from_slice(&c.raw_bits().to_le_bytes());
        }
        out
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 8 {
            return Err("Invalid .jsc: too short".to_string());
        }
        if &data[0..4] != b"SAFC" {
            return Err("Invalid .jsc: bad magic".to_string());
        }
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != 3 {
            return Err(format!("Unsupported .jsc version: {}", version));
        }
        let mut offset = 8usize;
        let locals_count = read_u32_checked(data, &mut offset)?;
        let param_count = read_u16_checked(data, &mut offset)?;
        let uses_arguments = read_u8_checked(data, &mut offset)? != 0;
        let has_simple_parameters = read_u8_checked(data, &mut offset)? != 0;
        let code_len = usize::try_from(read_u64_checked(data, &mut offset)?)
            .map_err(|_| "Invalid .jsc: code too long".to_string())?;
        let code_end = offset
            .checked_add(code_len)
            .ok_or_else(|| "Invalid .jsc: code too long".to_string())?;
        let code = data
            .get(offset..code_end)
            .ok_or_else(|| "Invalid .jsc: code truncated".to_string())?
            .to_vec();
        offset = code_end;
        let constants_count = usize::try_from(read_u64_checked(data, &mut offset)?)
            .map_err(|_| "Invalid .jsc: too many constants".to_string())?;
        let constants_bytes = constants_count
            .checked_mul(8)
            .ok_or_else(|| "Invalid .jsc: too many constants".to_string())?;
        let constants_end = offset
            .checked_add(constants_bytes)
            .ok_or_else(|| "Invalid .jsc: too many constants".to_string())?;
        if constants_end > data.len() {
            return Err("Invalid .jsc: constants truncated".to_string());
        }
        let mut constants = Vec::with_capacity(constants_count);
        for _ in 0..constants_count {
            let bits = read_u64_checked(data, &mut offset)?;
            let value = crate::value::JSValue::from_serialized_bits(bits)
                .ok_or_else(|| "Invalid .jsc: unsupported constant".to_string())?;
            constants.push(value);
        }
        Ok(Self {
            code,
            constants,
            locals_count,
            param_count,
            uses_arguments,
            has_simple_parameters,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            line_number_table: None,
            ic_table: std::rc::Rc::new(std::cell::RefCell::new(crate::compiler::InlineCacheTable::new())),
            shared_nested_bytecode: None,
            nested_bytecodes: std::cell::RefCell::new(Vec::new()),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_roundtrip() {
        let ops = [
            Opcode::Nop,
            Opcode::End,
            Opcode::Return,
            Opcode::LoadConst,
            Opcode::LoadInt,
            Opcode::LoadTrue,
            Opcode::Add,
            Opcode::Sub,
            Opcode::Mul,
            Opcode::Div,
            Opcode::Jump,
            Opcode::JumpBreak,
            Opcode::JumpIf,
            Opcode::JumpIfNot,
            Opcode::GetLocal,
            Opcode::SetLocal,
            Opcode::IncLocal,
            Opcode::DecLocal,
            Opcode::Call,
            Opcode::CallMethod,
            Opcode::NewFunction,
            Opcode::JumpIfNullish,
            Opcode::Call2,
            Opcode::Call3,
            Opcode::Try,
        ];
        for op in ops {
            let byte = op as u8;
            let recovered = Opcode::from_u8(byte).expect("roundtrip failed");
            assert_eq!(op, recovered, "Opcode {:?} did not round-trip", op);
        }
    }

    #[test]
    fn test_disassemble_basic_program() {
        let bytecode = Bytecode {
            code: vec![
                Opcode::LoadInt as u8,
                0x00,
                0x00,
                0x0A,
                0x00,
                0x00,
                0x00,
                Opcode::LoadInt as u8,
                0x01,
                0x00,
                0x14,
                0x00,
                0x00,
                0x00,
                Opcode::Add as u8,
                0x02,
                0x00,
                0x00,
                0x00,
                0x01,
                0x00,
                Opcode::Return as u8,
                0x02,
                0x00,
            ],
            constants: vec![],
            locals_count: 3,
            param_count: 0,
            line_number_table: None,
            ic_table: std::rc::Rc::new(std::cell::RefCell::new(crate::compiler::InlineCacheTable::new())),
            shared_nested_bytecode: None,
            uses_arguments: false,
            has_simple_parameters: false,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            nested_bytecodes: std::cell::RefCell::new(Vec::new()),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        };

        let output = bytecode.disassemble();
        assert!(output.contains("=== Register Bytecode ==="));
        assert!(
            output.contains("LoadInt r0, 10"),
            "Basic disasm wrong: {}",
            output
        );
        assert!(
            output.contains("LoadInt r1, 20"),
            "Basic disasm wrong: {}",
            output
        );
        assert!(
            output.contains("Add r2, r0, r1"),
            "Basic disasm wrong: {}",
            output
        );
        assert!(output.contains("Return"), "Basic disasm wrong: {}", output);
    }

    #[test]
    fn test_disassemble_jump_offsets() {
        let bytecode = Bytecode {
            code: vec![
                Opcode::LoadFalse as u8,
                0x00,
                0x00,
                Opcode::JumpIfNot as u8,
                0x00,
                0x00,
                0x05,
                0x00,
                0x00,
                0x00,
                Opcode::LoadTrue as u8,
                0x01,
                0x00,
                Opcode::Jump as u8,
                0x00,
                0x00,
                0x00,
                0x00,
            ],
            constants: vec![],
            locals_count: 1,
            param_count: 0,
            line_number_table: None,
            ic_table: std::rc::Rc::new(std::cell::RefCell::new(crate::compiler::InlineCacheTable::new())),
            shared_nested_bytecode: None,
            uses_arguments: false,
            has_simple_parameters: false,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            nested_bytecodes: std::cell::RefCell::new(Vec::new()),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        };

        let output = bytecode.disassemble();

        assert!(
            output.contains("JumpIfNot r0, 5 (pc 15)"),
            "JumpIfNot disasm wrong: {}",
            output
        );

        assert!(
            output.contains("Jump 0 (pc 18)"),
            "Jump disasm wrong: {}",
            output
        );
    }

    #[test]
    fn test_disassemble_call_and_object_ops() {
        let bytecode = Bytecode {
            code: vec![
                Opcode::NewObject as u8,
                0x00,
                0x00,
                Opcode::Call as u8,
                0x01,
                0x00,
                0x02,
                0x00,
                0x00,
                0x00,
                Opcode::SetField as u8,
                0x00,
                0x00,
                0x01,
                0x00,
                0x02,
                0x00,
            ],
            constants: vec![],
            locals_count: 3,
            param_count: 0,
            line_number_table: None,
            ic_table: std::rc::Rc::new(std::cell::RefCell::new(crate::compiler::InlineCacheTable::new())),
            shared_nested_bytecode: None,
            uses_arguments: false,
            has_simple_parameters: false,
            is_strict: false,
            var_name_to_slot: std::rc::Rc::new(Vec::new()),
            nested_bytecodes: std::cell::RefCell::new(Vec::new()),
            is_simple_constructor: false,
            simple_constructor_props: Vec::new(),
            cached_constructor_final_shape: None,
            cached_constructor_atoms: Vec::new(),
        };

        let output = bytecode.disassemble();
        assert!(
            output.contains("NewObject r0"),
            "NewObject disasm wrong: {}",
            output
        );
        assert!(
            output.contains("Call r1, r2, argc=0"),
            "Call disasm wrong: {}",
            output
        );
        assert!(
            output.contains("SetField r0, r1, r2"),
            "SetField disasm wrong: {}",
            output
        );
    }

    #[test]
    fn test_serialize_roundtrip_primitive_constants() {
        let mut bytecode = Bytecode::new();
        bytecode.code = vec![Opcode::Return as u8, 0x00, 0x00];
        bytecode.constants = vec![
            crate::value::JSValue::undefined(),
            crate::value::JSValue::null(),
            crate::value::JSValue::bool(true),
            crate::value::JSValue::new_int(-7),
            crate::value::JSValue::new_float(3.5),
            crate::value::JSValue::new_float(f64::NAN),
            crate::value::JSValue::new_string(crate::runtime::atom::Atom(42)),
        ];

        let data = bytecode.serialize();
        let decoded = Bytecode::deserialize(&data).expect("primitive constants must load");
        assert_eq!(decoded.constants.len(), bytecode.constants.len());
        for (decoded_value, original_value) in
            decoded.constants.iter().zip(bytecode.constants.iter())
        {
            assert_eq!(decoded_value.raw_bits(), original_value.raw_bits());
        }
    }

    #[test]
    fn test_deserialize_rejects_pointer_constants() {
        let object_ptr = Box::into_raw(Box::new(crate::object::object::JSObject::new()));
        let function_ptr = Box::into_raw(Box::new(crate::object::function::JSFunction::new()));
        let bigint_ptr = Box::into_raw(Box::new(crate::object::object::JSObject::new_bigint()));
        for value in [
            crate::value::JSValue::new_object(object_ptr),
            crate::value::JSValue::new_function(function_ptr),
            crate::value::JSValue::new_bigint(bigint_ptr),
        ] {
            let mut bytecode = Bytecode::new();
            bytecode.constants = vec![value];
            assert!(
                Bytecode::deserialize(&bytecode.serialize()).is_err(),
                "pointer constants must not load"
            );
        }
        // SAFETY: each pointer was produced by `Box::into_raw` above and is reclaimed exactly once.
        unsafe {
            drop(Box::from_raw(object_ptr));
            drop(Box::from_raw(function_ptr));
            drop(Box::from_raw(bigint_ptr));
        }
    }

    #[test]
    fn test_deserialize_rejects_truncated_input() {
        let mut bytecode = Bytecode::new();
        bytecode.code = vec![Opcode::Return as u8, 0x00, 0x00];
        bytecode.constants = vec![crate::value::JSValue::new_int(1)];
        let data = bytecode.serialize();
        for prefix_len in 0..data.len() {
            assert!(
                Bytecode::deserialize(&data[..prefix_len]).is_err(),
                "truncated prefix of {prefix_len} bytes was accepted"
            );
        }
        assert!(Bytecode::deserialize(&data).is_ok());
    }

    #[test]
    fn test_instruction_size_at_walks_compiled_programs_exactly() {
        use crate::{JSContext, JSRuntime};
        let mut rt = JSRuntime::new();
        let mut ctx = JSContext::new(&mut rt);
        let sources = [
            "var a = 0; for (var i = 0; i < 10; i++) { a += i; } a;",
            "function f(a, b) { return a + b; } f(1, 2);",
            "function f(a, b, c, d, e) { return a + b + c + d + e; } f(1, 2, 3, 4, 5);",
            "function r(a, b, c) { if (a > 0) { return r(a - 1, b, c); } return b + c; } r(2, 3, 4);",
            "function* g(x) { yield x; yield x + 1; } g(1);",
            "async function h(x) { return await x; } h(1);",
            "async function* i(x) { yield await x; } i(1);",
            "var fn = function named(x) { var v = arguments.length + x; return v; }; fn(1);",
            "class C { x = 1; y = 2; m(a) { return a + this.x; } static s() { return 0; } } new C();",
            "class D { constructor(a) { this.a = a; } m(a, b, c, d) { return a + b + c + d; } } var d = new D(1, 2); d.m(1, 2, 3, 4);",
            "var o = { m(a = 1, ...rest) { return a + rest.length; } }; o.m();",
            "var o2 = { m(a) { return a; } }; o2.m(...[1]);",
            "let d = (function (x) { return x * 2; })(21); d;",
            "try { throw 1; } catch (e) { e; } finally { 0; }",
            "switch (1) { case 1: break; default: break; }",
            "label: for (;;) { break label; }",
            "var { p = 1, ...q } = { r: 2 }; p + q.r;",
            "var [u = 1, ...v] = [2, 3]; u + v.length;",
            "Math.max(1, 2, 3, 4);",
            "with ({ a: 1 }) { a; }",
            "var re = /a(b)c/gi; re.test('abc');",
            "function* gen() { try { yield 1; } finally { 0; } } gen().next();",
        ];
        for src in sources {
            for opt in [crate::OptLevel::O0, crate::OptLevel::O2] {
                let (code, _constants) =
                    crate::compile_to_register_bytecode_with_opt_level(&mut ctx, src, opt)
                        .unwrap();
                let mut pc = 0usize;
                while pc < code.len() {
                    let size = Opcode::instruction_size_at(&code, pc);
                    assert!(
                        size > 0 && pc + size <= code.len(),
                        "instruction at {pc} in {src:?} ({opt:?}) has size {size} past the end"
                    );
                    pc += size;
                }
                assert_eq!(pc, code.len(), "walk desynced in {src:?} ({opt:?})");
            }
        }
    }

    #[test]
    fn test_instruction_size_at_reads_variable_call_arity() {
        fn encoded(op: Opcode, header: usize, argc_offset: usize, argc: u16) -> Vec<u8> {
            let mut buf = vec![0u8; header + argc as usize * 2 + 16];
            buf[0] = op as u8;
            buf[argc_offset..argc_offset + 2].copy_from_slice(&argc.to_le_bytes());
            buf
        }
        let cases = [
            (Opcode::Call, 7, 5),
            (Opcode::CallNew, 7, 5),
            (Opcode::CallMethod, 9, 7),
            (Opcode::CallCurrent, 5, 3),
            (Opcode::CallNamedMethod, 11, 9),
        ];
        for (op, header, argc_offset) in cases {
            for argc in 0..=8u16 {
                let buf = encoded(op, header, argc_offset, argc);
                assert_eq!(
                    Opcode::instruction_size_at(&buf, 0),
                    header + argc as usize * 2,
                    "{op:?} with argc={argc}"
                );
            }
        }
    }
}
