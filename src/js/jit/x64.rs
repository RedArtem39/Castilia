//! x86-64 backend (System V and Windows x64 calling conventions).
//!
//! Register use inside generated code:
//! * `rbx` — `*mut JitCtx`
//! * `r12` — operand stack base (`*mut Value`), `r13` — stack depth (`sp`)
//! * `r14` — frame registers base (`*mut Value`)
//!
//! Values are 24 bytes: tag byte at +0, payload at +8 (see `Value::TAG_*`). Fast paths only touch
//! plain values (undefined, null, booleans, numbers); anything else goes to `helper_generic`.
use super::{helper_generic, ST_EXIT, ST_JUMP, ST_NEXT};
use crate::js::{
    ast::BinOp,
    value::Value,
    vm::{Code, Op},
};
use alloc::vec::Vec;

const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const RBX: u8 = 3;
const RSI: u8 = 6;
const RDI: u8 = 7;
const R12: u8 = 12;
const R13: u8 = 13;
const R14: u8 = 14;

// JitCtx field offsets
const CTX_SP: i32 = 8;
const CTX_CAP: i32 = 16;
const CTX_STACK: i32 = 0;
const CTX_REGS: i32 = 24;
const CTX_PC: i32 = 32;
const CTX_BUDGET: i32 = 64;
const CTX_TABLE: i32 = 72;
const CTX_BASE: i32 = 80;

const VSIZE: i32 = 24;
const PAYLOAD: i32 = Value::PAYLOAD_OFFSET as i32;
const TAG_BOOL: u8 = Value::TAG_BOOL;
const TAG_NUM: u8 = Value::TAG_NUM;
const LAST_PLAIN: u8 = Value::TAG_LAST_PLAIN;

#[derive(Clone, Copy)]
enum Target {
    Insn(usize),
    Label(usize),
}

struct Asm {
    b: Vec<u8>,
    /// (position of rel32, target)
    fix: Vec<(usize, Target)>,
    labels: Vec<usize>,
}

/// A memory operand: base register, optional `index*scale`, displacement.
#[derive(Clone, Copy)]
struct Mem {
    base: u8,
    index: Option<(u8, u8)>,
    disp: i32,
}

fn mem(base: u8, disp: i32) -> Mem {
    Mem { base, index: None, disp }
}

impl Asm {
    fn byte(&mut self, x: u8) {
        self.b.push(x);
    }
    fn bytes(&mut self, x: &[u8]) {
        self.b.extend_from_slice(x);
    }
    fn u32(&mut self, x: u32) {
        self.b.extend_from_slice(&x.to_le_bytes());
    }
    fn label(&mut self) -> usize {
        self.labels.push(usize::MAX);
        self.labels.len() - 1
    }
    fn bind(&mut self, l: usize) {
        self.labels[l] = self.b.len();
    }

    /// REX prefix for `reg` field and memory operand (w = 64-bit).
    fn rex_mem(&mut self, w: bool, reg: u8, m: Mem, force: bool) {
        let x = m.index.map_or(0, |(i, _)| (i >> 3) & 1);
        let r = 0x40 | (w as u8) << 3 | ((reg >> 3) & 1) << 2 | x << 1 | ((m.base >> 3) & 1);
        if r != 0x40 || force {
            self.byte(r);
        }
    }

    /// ModRM (+SIB) with a 32-bit displacement.
    fn modrm_mem(&mut self, reg: u8, m: Mem) {
        let reg = reg & 7;
        match m.index {
            None if m.base & 7 == 4 => {
                self.byte(0x80 | reg << 3 | 4);
                self.byte(0x24);
            }
            None => self.byte(0x80 | reg << 3 | (m.base & 7)),
            Some((i, scale)) => {
                self.byte(0x80 | reg << 3 | 4);
                let ss = match scale {
                    1 => 0,
                    2 => 1,
                    4 => 2,
                    _ => 3,
                };
                self.byte(ss << 6 | (i & 7) << 3 | (m.base & 7));
            }
        }
        self.u32(m.disp as u32);
    }

    fn op_mem(&mut self, w: bool, opcode: &[u8], reg: u8, m: Mem) {
        self.rex_mem(w, reg, m, false);
        self.bytes(opcode);
        self.modrm_mem(reg, m);
    }

    fn mov_load(&mut self, dst: u8, m: Mem) {
        self.op_mem(true, &[0x8b], dst, m);
    }
    fn mov_store(&mut self, m: Mem, src: u8) {
        self.op_mem(true, &[0x89], src, m);
    }
    fn mov_imm64(&mut self, dst: u8, v: u64) {
        self.byte(0x48 | (dst >> 3) & 1);
        self.byte(0xb8 | (dst & 7));
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn mov_rr(&mut self, dst: u8, src: u8) {
        self.byte(0x48 | ((src >> 3) & 1) << 2 | (dst >> 3) & 1);
        self.byte(0x89);
        self.byte(0xc0 | (src & 7) << 3 | (dst & 7));
    }
    /// `mov byte [m], imm8`
    fn mov_byte_imm(&mut self, m: Mem, v: u8) {
        self.rex_mem(false, 0, m, false);
        self.byte(0xc6);
        self.modrm_mem(0, m);
        self.byte(v);
    }
    /// `cmp byte [m], imm8`
    fn cmp_byte_imm(&mut self, m: Mem, v: u8) {
        self.rex_mem(false, 7, m, false);
        self.byte(0x80);
        self.modrm_mem(7, m);
        self.byte(v);
    }
    /// `movzx eax, byte [m]`
    fn movzx_eax(&mut self, m: Mem) {
        self.rex_mem(false, RAX, m, false);
        self.bytes(&[0x0f, 0xb6]);
        self.modrm_mem(RAX, m);
    }
    fn sse_mem(&mut self, prefix: u8, op: u8, xmm: u8, m: Mem) {
        self.byte(prefix);
        self.rex_mem(false, xmm, m, false);
        self.bytes(&[0x0f, op]);
        self.modrm_mem(xmm, m);
    }
    fn movsd_load(&mut self, xmm: u8, m: Mem) {
        self.sse_mem(0xf2, 0x10, xmm, m);
    }
    fn movsd_store(&mut self, m: Mem, xmm: u8) {
        self.sse_mem(0xf2, 0x11, xmm, m);
    }
    fn sse_rr(&mut self, prefix: u8, op: u8, dst: u8, src: u8) {
        self.byte(prefix);
        self.bytes(&[0x0f, op, 0xc0 | (dst & 7) << 3 | (src & 7)]);
    }
    /// `movq xmm, rax`
    fn movq_xmm_rax(&mut self, xmm: u8) {
        self.bytes(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (xmm & 7) << 3]);
    }
    fn setcc(&mut self, cc: u8, reg8: u8) {
        self.bytes(&[0x0f, 0x90 | cc, 0xc0 | reg8]);
    }
    fn jcc(&mut self, cc: u8, t: Target) {
        self.bytes(&[0x0f, 0x80 | cc]);
        self.fix.push((self.b.len(), t));
        self.u32(0);
    }
    fn jmp(&mut self, t: Target) {
        self.byte(0xe9);
        self.fix.push((self.b.len(), t));
        self.u32(0);
    }
    fn inc_r13(&mut self) {
        self.bytes(&[0x49, 0xff, 0xc5]);
    }
    fn dec_r13(&mut self) {
        self.bytes(&[0x49, 0xff, 0xcd]);
    }
    /// `rax = r13 * 3` (stack slot index scaled for `[r12 + rax*8]`).
    fn sp3(&mut self) {
        self.mov_rr(RAX, R13);
        self.bytes(&[0x48, 0x8d, 0x04, 0x40]); // lea rax, [rax + rax*2]
    }
}

/// Operand stack slot `k` from the top (0 = top) after `sp3`.
fn top(k: i32) -> Mem {
    Mem { base: R12, index: Some((RAX, 8)), disp: -VSIZE * (k + 1) }
}
/// The free slot just above the top after `sp3`.
fn push_slot() -> Mem {
    Mem { base: R12, index: Some((RAX, 8)), disp: 0 }
}
fn reg_slot(r: u32) -> Option<Mem> {
    let d = (r as i64) * VSIZE as i64;
    if d > i32::MAX as i64 - 64 {
        return None;
    }
    Some(mem(R14, d as i32))
}

// condition codes
const CC_B: u8 = 0x2;
const CC_AE: u8 = 0x3;
const CC_E: u8 = 0x4;
const CC_NE: u8 = 0x5;
const CC_BE: u8 = 0x6;
const CC_A: u8 = 0x7;
const CC_S: u8 = 0x8;
const CC_P: u8 = 0xa;
const CC_NP: u8 = 0xb;

/// Compile a function. Returns the machine code and the per-instruction offsets.
/// Share of instructions with an inline fast path; below this the helper round trips cost more
/// than native code saves.
const MIN_INLINE_SHARE: f64 = 0.4;

fn inlinable(op: &Op) -> bool {
    matches!(
        op,
        Op::Int(_) | Op::Undef | Op::Null | Op::True | Op::False | Op::Const(_) | Op::GetReg(_) | Op::SetReg(_) | Op::PutReg(_) | Op::IncReg(_) | Op::DecReg(_) | Op::Pop | Op::Dup | Op::Pos | Op::Inc | Op::Dec | Op::Neg | Op::Not | Op::Jump(_) | Op::JumpIfFalse(_) | Op::JumpIfTrue(_) | Op::Nop
    ) || matches!(op, Op::Bin(b) if matches!(b, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::StrictEq | BinOp::StrictNe | BinOp::Eq | BinOp::Ne))
}

pub fn compile(code: &Code) -> Option<(Vec<u8>, Vec<u32>)> {
    let n = code.ops.len();
    if n == 0 || n > 200_000 {
        return None;
    }
    let share = code.ops.iter().filter(|o| inlinable(o)).count() as f64 / n as f64;
    if share < MIN_INLINE_SHARE {
        return None;
    }
    let mut a = Asm { b: Vec::with_capacity(n * 24 + 256), fix: Vec::new(), labels: Vec::new() };
    let dispatch = a.label();
    let exit = a.label();
    let mut table = alloc::vec![0u32; n];

    // ---- prologue ----
    a.byte(0x53); // push rbx
    a.bytes(&[0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57]); // push r12..r15
    a.bytes(&[0x48, 0x83, 0xec, 0x20]); // sub rsp, 32 (shadow space; keeps 16-byte alignment)
    if cfg!(windows) {
        a.mov_rr(RBX, RCX);
    } else {
        a.mov_rr(RBX, RDI);
    }
    reload(&mut a);
    a.jmp(Target::Label(dispatch));

    // ---- per-instruction code ----
    for (pc, &op) in code.ops.iter().enumerate() {
        table[pc] = a.b.len() as u32;
        let slow = a.label();
        let mut has_slow = true;
        let next = Target::Insn(pc + 1);
        match op {
            Op::Nop | Op::Debugger => has_slow = false,
            Op::Int(_) | Op::Undef | Op::Null | Op::True | Op::False | Op::Const(_)
                if !matches!(op, Op::Const(i) if !matches!(code.consts.get(i as usize), Some(Value::Num(_)))) =>
            {
                // capacity check
                a.op_mem(true, &[0x3b], R13, mem(RBX, CTX_CAP)); // cmp r13, [rbx+cap]
                a.jcc(CC_AE, Target::Label(slow));
                a.sp3();
                let (tag, payload): (u8, u64) = match op {
                    Op::Int(i) => (TAG_NUM, (i as f64).to_bits()),
                    Op::Const(i) => match &code.consts[i as usize] {
                        Value::Num(x) => (TAG_NUM, x.to_bits()),
                        _ => unreachable!(),
                    },
                    Op::Undef => (Value::TAG_UNDEFINED, 0),
                    Op::Null => (Value::TAG_NULL, 0),
                    Op::True => (TAG_BOOL, 1),
                    _ => (TAG_BOOL, 0),
                };
                a.mov_byte_imm(push_slot(), tag);
                a.mov_imm64(RCX, payload);
                a.mov_store(Mem { disp: PAYLOAD, ..push_slot() }, RCX);
                a.inc_r13();
            }
            Op::GetReg(r) if reg_slot(r).is_some() => {
                let rs = reg_slot(r).unwrap();
                a.cmp_byte_imm(rs, LAST_PLAIN);
                a.jcc(CC_A, Target::Label(slow));
                a.op_mem(true, &[0x3b], R13, mem(RBX, CTX_CAP));
                a.jcc(CC_AE, Target::Label(slow));
                a.sp3();
                a.mov_load(RCX, rs);
                a.mov_store(push_slot(), RCX);
                a.mov_load(RCX, Mem { disp: rs.disp + PAYLOAD, ..rs });
                a.mov_store(Mem { disp: PAYLOAD, ..push_slot() }, RCX);
                a.inc_r13();
            }
            Op::SetReg(r) | Op::PutReg(r) if reg_slot(r).is_some() => {
                let rs = reg_slot(r).unwrap();
                // the old register value must be plain (no drop needed)
                a.cmp_byte_imm(rs, LAST_PLAIN);
                a.jcc(CC_A, Target::Label(slow));
                a.sp3();
                if matches!(op, Op::PutReg(_)) {
                    // copying (not moving) requires a plain value
                    a.cmp_byte_imm(top(0), LAST_PLAIN);
                    a.jcc(CC_A, Target::Label(slow));
                }
                for q in 0..3 {
                    a.mov_load(RCX, Mem { disp: -VSIZE + q * 8, ..push_slot() });
                    a.mov_store(Mem { disp: rs.disp + q * 8, ..rs }, RCX);
                }
                if matches!(op, Op::SetReg(_)) {
                    a.dec_r13();
                }
            }
            Op::IncReg(r) | Op::DecReg(r) if reg_slot(r).is_some() => {
                let rs = reg_slot(r).unwrap();
                a.cmp_byte_imm(rs, TAG_NUM);
                a.jcc(CC_NE, Target::Label(slow));
                a.movsd_load(0, Mem { disp: rs.disp + PAYLOAD, ..rs });
                a.mov_imm64(RAX, 1.0f64.to_bits());
                a.movq_xmm_rax(1);
                a.sse_rr(0xf2, if matches!(op, Op::IncReg(_)) { 0x58 } else { 0x5c }, 0, 1);
                a.movsd_store(Mem { disp: rs.disp + PAYLOAD, ..rs }, 0);
            }
            Op::Pop => {
                a.sp3();
                a.cmp_byte_imm(top(0), LAST_PLAIN);
                a.jcc(CC_A, Target::Label(slow));
                a.dec_r13();
            }
            Op::Dup => {
                a.op_mem(true, &[0x3b], R13, mem(RBX, CTX_CAP));
                a.jcc(CC_AE, Target::Label(slow));
                a.sp3();
                a.cmp_byte_imm(top(0), LAST_PLAIN);
                a.jcc(CC_A, Target::Label(slow));
                a.mov_load(RCX, top(0));
                a.mov_store(push_slot(), RCX);
                a.mov_load(RCX, Mem { disp: -VSIZE + PAYLOAD, ..push_slot() });
                a.mov_store(Mem { disp: PAYLOAD, ..push_slot() }, RCX);
                a.inc_r13();
            }
            Op::Pos | Op::Inc | Op::Dec | Op::Neg => {
                a.sp3();
                a.cmp_byte_imm(top(0), TAG_NUM);
                a.jcc(CC_NE, Target::Label(slow));
                let p = Mem { disp: -VSIZE + PAYLOAD, ..push_slot() };
                match op {
                    Op::Pos => {}
                    Op::Neg => {
                        a.mov_load(RCX, p);
                        a.mov_imm64(RDX, 1u64 << 63);
                        a.bytes(&[0x48, 0x31, 0xd1]); // xor rcx, rdx
                        a.mov_store(p, RCX);
                    }
                    _ => {
                        a.movsd_load(0, p);
                        a.mov_imm64(RCX, 1.0f64.to_bits());
                        a.bytes(&[0x66, 0x48, 0x0f, 0x6e, 0xc9]); // movq xmm1, rcx
                        a.sse_rr(0xf2, if op == Op::Inc { 0x58 } else { 0x5c }, 0, 1);
                        a.movsd_store(p, 0);
                    }
                }
            }
            Op::Not => {
                a.sp3();
                a.cmp_byte_imm(top(0), TAG_BOOL);
                a.jcc(CC_NE, Target::Label(slow));
                let p = Mem { disp: -VSIZE + PAYLOAD, ..push_slot() };
                a.movzx_ecx(p);
                a.bytes(&[0x80, 0xf1, 0x01]); // xor cl, 1
                a.store_cl(p);
            }
            Op::Bin(bop) if matches!(bop, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::StrictEq | BinOp::StrictNe | BinOp::Eq | BinOp::Ne) => {
                a.sp3();
                a.cmp_byte_imm(top(1), TAG_NUM);
                a.jcc(CC_NE, Target::Label(slow));
                a.cmp_byte_imm(top(0), TAG_NUM);
                a.jcc(CC_NE, Target::Label(slow));
                let pa = Mem { disp: top(1).disp + PAYLOAD, ..top(1) };
                let pb = Mem { disp: top(0).disp + PAYLOAD, ..top(0) };
                a.movsd_load(0, pa);
                a.movsd_load(1, pb);
                match bop {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        let opc = match bop {
                            BinOp::Add => 0x58,
                            BinOp::Sub => 0x5c,
                            BinOp::Mul => 0x59,
                            _ => 0x5e,
                        };
                        a.sse_rr(0xf2, opc, 0, 1);
                        a.movsd_store(pa, 0);
                    }
                    _ => {
                        // result in cl (rax keeps the slot index)
                        match bop {
                            // a < b  ⇔  b > a ; a <= b ⇔ b >= a (unordered → false)
                            BinOp::Lt => {
                                a.sse_rr(0x66, 0x2e, 1, 0);
                                a.setcc(CC_A, RCX);
                            }
                            BinOp::Le => {
                                a.sse_rr(0x66, 0x2e, 1, 0);
                                a.setcc(CC_AE, RCX);
                            }
                            BinOp::Gt => {
                                a.sse_rr(0x66, 0x2e, 0, 1);
                                a.setcc(CC_A, RCX);
                            }
                            BinOp::Ge => {
                                a.sse_rr(0x66, 0x2e, 0, 1);
                                a.setcc(CC_AE, RCX);
                            }
                            BinOp::StrictEq | BinOp::Eq => {
                                a.sse_rr(0x66, 0x2e, 0, 1);
                                a.setcc(CC_E, RCX);
                                a.setcc(CC_NP, RDX);
                                a.bytes(&[0x20, 0xd1]); // and cl, dl
                            }
                            _ => {
                                a.sse_rr(0x66, 0x2e, 0, 1);
                                a.setcc(CC_NE, RCX);
                                a.setcc(CC_P, RDX);
                                a.bytes(&[0x08, 0xd1]); // or cl, dl
                            }
                        }
                        a.mov_byte_imm(top(1), TAG_BOOL);
                        a.store_cl(pa);
                    }
                }
                a.dec_r13();
            }
            Op::Jump(t) => {
                if (t as usize) <= pc {
                    // back-edge: count work, leave to the helper when the chunk is used up
                    a.bytes(&[0x48, 0xff, 0x4b, CTX_BUDGET as u8]); // dec qword [rbx+64]
                    a.jcc(CC_S, Target::Label(slow));
                }
                a.jmp(Target::Insn(t as usize));
            }
            Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => {
                let want_true = matches!(op, Op::JumpIfTrue(_));
                let is_num = a.label();
                let taken = Target::Insn(t as usize);
                a.sp3();
                a.cmp_byte_imm(top(0), TAG_BOOL);
                a.jcc(CC_NE, Target::Label(is_num));
                a.movzx_eax(Mem { disp: top(0).disp + PAYLOAD, ..top(0) });
                a.dec_r13();
                a.bytes(&[0x85, 0xc0]); // test eax, eax
                a.jcc(if want_true { CC_NE } else { CC_E }, taken);
                a.jmp(next);
                a.bind(is_num);
                a.cmp_byte_imm(top(0), TAG_NUM);
                a.jcc(CC_NE, Target::Label(slow));
                a.movsd_load(0, Mem { disp: top(0).disp + PAYLOAD, ..top(0) });
                a.dec_r13();
                a.sse_rr(0x66, 0x57, 1, 1); // xorpd xmm1, xmm1
                a.sse_rr(0x66, 0x2e, 0, 1); // ucomisd xmm0, xmm1
                // falsy: equal to zero (ZF=1) or NaN (PF=1, which also sets ZF)
                a.jcc(if want_true { CC_NE } else { CC_E }, taken);
                if !want_true {
                    // NaN is also falsy
                    a.jcc(CC_P, taken);
                } else {
                    // NE with PF set means NaN: not taken
                }
                a.jmp(next);
            }
            _ => {
                // generic: the interpreter executes this instruction
                has_slow = false;
                emit_generic(&mut a, pc, dispatch, exit);
            }
        }
        if has_slow {
            a.jmp(next);
            a.bind(slow);
            emit_generic(&mut a, pc, dispatch, exit);
        }
    }
    // falling off the end can't happen (functions end in Return), but stay safe
    let end_at = a.b.len();
    a.mov_imm64(RAX, n as u64);
    a.mov_store(mem(RBX, CTX_PC), RAX);
    a.bytes(&[0xb8]);
    a.u32(ST_NEXT);
    a.jmp(Target::Label(exit));

    // ---- dispatch: jump to the native code of instruction ctx.pc ----
    a.bind(dispatch);
    a.mov_load(RAX, mem(RBX, CTX_PC));
    a.mov_load(RCX, mem(RBX, CTX_TABLE));
    a.bytes(&[0x8b, 0x04, 0x81]); // mov eax, [rcx + rax*4]
    a.op_mem(true, &[0x03], RAX, mem(RBX, CTX_BASE)); // add rax, [rbx+base]
    a.bytes(&[0xff, 0xe0]); // jmp rax

    // ---- exit: status in eax ----
    a.bind(exit);
    a.mov_store(mem(RBX, CTX_SP), R13);
    a.bytes(&[0x48, 0x83, 0xc4, 0x20]); // add rsp, 32
    a.bytes(&[0x41, 0x5f, 0x41, 0x5e, 0x41, 0x5d, 0x41, 0x5c, 0x5b, 0xc3]); // pop r15..r12, rbx; ret

    // ---- resolve jumps ----
    for (at, t) in core::mem::take(&mut a.fix) {
        let dest = match t {
            Target::Insn(i) if i < n => table[i] as usize,
            Target::Insn(_) => end_at,
            Target::Label(l) => a.labels[l],
        };
        let rel = dest as i64 - (at as i64 + 4);
        a.b[at..at + 4].copy_from_slice(&(rel as i32).to_le_bytes());
    }
    Some((a.b, table))
}

/// Call `helper_generic(ctx, pc)` and continue according to its status.
fn emit_generic(a: &mut Asm, pc: usize, dispatch: usize, exit: usize) {
    a.mov_store(mem(RBX, CTX_SP), R13);
    if cfg!(windows) {
        a.mov_rr(RCX, RBX);
        a.byte(0xba); // mov edx, imm32
    } else {
        a.mov_rr(RDI, RBX);
        a.byte(0xbe); // mov esi, imm32
    }
    a.u32(pc as u32);
    a.mov_imm64(RAX, helper_generic as *const () as usize as u64);
    a.bytes(&[0xff, 0xd0]); // call rax
    reload(a);
    a.bytes(&[0x83, 0xf8, ST_NEXT as u8]); // cmp eax, NEXT
    a.jcc(CC_E, Target::Insn(pc + 1));
    a.bytes(&[0x83, 0xf8, ST_JUMP as u8]); // cmp eax, JUMP
    a.jcc(CC_E, Target::Label(dispatch));
    let _ = ST_EXIT;
    a.jmp(Target::Label(exit));
}

/// Reload the cached stack/register pointers from the context.
fn reload(a: &mut Asm) {
    a.mov_load(R12, mem(RBX, CTX_STACK));
    a.mov_load(R13, mem(RBX, CTX_SP));
    a.mov_load(R14, mem(RBX, CTX_REGS));
}

impl Asm {
    /// `movzx ecx, byte [m]`
    fn movzx_ecx(&mut self, m: Mem) {
        self.rex_mem(false, RCX, m, false);
        self.bytes(&[0x0f, 0xb6]);
        self.modrm_mem(RCX, m);
    }
    /// `mov byte [m], cl`
    fn store_cl(&mut self, m: Mem) {
        self.rex_mem(false, RCX, m, false);
        self.byte(0x88);
        self.modrm_mem(RCX, m);
    }
}

#[allow(dead_code)]
const _UNUSED: (u8, u8, u8) = (CC_B, CC_BE, RSI);
