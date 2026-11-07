//! Parsed instruction identities. Canonical spelling remains available for hashes and diagnostics.
use std::{fmt, ops::Deref};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizedOp {
    Const,
    Load,
    Store,
    Zext,
    Sext,
    Trunc,
    Ptrtoint,
    Measure,
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    Lshr,
    Ashr,
}

macro_rules! opcodes {
    ($($variant:ident => $spelling:literal,)*) => {
        #[derive(Clone, PartialEq, Eq)]
        pub enum Op {
            $($variant,)*
            Sized { kind: SizedOp, width: usize, lifted: bool, text: String },
            Unknown(String),
        }
        impl Op {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $spelling,)*
                    Self::Sized { text, .. } | Self::Unknown(text) => text,
                }
            }
        }
        impl From<&str> for Op {
            fn from(text: &str) -> Self {
                match text {
                    $($spelling => Self::$variant,)*
                    _ => Self::sized(text).unwrap_or_else(|| Self::Unknown(text.into())),
                }
            }
        }
    };
}
opcodes! {
    ActionCall => "action_call",
    Add => "add",
    Addr => "addr",
    Adj => "adj",
    Alloc => "alloc",
    And => "and",
    Ashr => "ashr",
    Boundscheck => "boundscheck",
    Br => "br",
    BudgetMint => "budget.mint",
    Call => "call",
    Ccx => "ccx",
    Cswap => "cswap",
    Ctl => "ctl",
    Cx => "cx",
    Cz => "cz",
    Effect => "effect",
    Fault => "fault",
    Faultif => "faultif",
    Free => "free",
    H => "h",
    Hostcall => "hostcall",
    IcmpEq => "icmp.eq",
    IcmpNe => "icmp.ne",
    IcmpSle => "icmp.sle",
    IcmpSlt => "icmp.slt",
    IcmpUle => "icmp.ule",
    IcmpUlt => "icmp.ult",
    Inttoptr => "inttoptr",
    Label => "label",
    LiftAdd => "lift.add",
    LiftAddr => "lift.addr",
    LiftAlloc => "lift.alloc",
    LiftAnd => "lift.and",
    LiftAshr => "lift.ashr",
    LiftBoundscheck => "lift.boundscheck",
    LiftCall => "lift.call",
    LiftFaultif => "lift.faultif",
    LiftFree => "lift.free",
    LiftIcmpEq => "lift.icmp.eq",
    LiftIcmpNe => "lift.icmp.ne",
    LiftIcmpSle => "lift.icmp.sle",
    LiftIcmpSlt => "lift.icmp.slt",
    LiftIcmpUle => "lift.icmp.ule",
    LiftIcmpUlt => "lift.icmp.ult",
    LiftLabel => "lift.label",
    LiftLshr => "lift.lshr",
    LiftMov => "lift.mov",
    LiftMul => "lift.mul",
    LiftOr => "lift.or",
    LiftOracleCall => "lift.oracle_call",
    LiftOracleUncall => "lift.oracle_uncall",
    LiftParmov => "lift.parmov",
    LiftPermCall => "lift.perm_call",
    LiftRet => "lift.ret",
    LiftRetval => "lift.retval",
    LiftSdiv => "lift.sdiv",
    LiftSelect => "lift.select",
    LiftShl => "lift.shl",
    LiftSrem => "lift.srem",
    LiftSub => "lift.sub",
    LiftUdiv => "lift.udiv",
    LiftUrem => "lift.urem",
    LiftXor => "lift.xor",
    Lshr => "lshr",
    Mcx => "mcx",
    Measure => "measure",
    Mov => "mov",
    Mul => "mul",
    Or => "or",
    OracleCall => "oracle_call",
    OracleUncall => "oracle_uncall",
    Parmov => "parmov",
    PermCall => "perm_call",
    Ptrtoint => "ptrtoint",
    Qalloc => "qalloc",
    Qborrow => "qborrow",
    QborrowRestore => "qborrow_restore",
    Qbranch => "qbranch",
    Qcopy => "qcopy",
    Qexch => "qexch",
    Qexchm => "qexchm",
    Qfree => "qfree",
    Qif => "qif",
    QifAdj => "qif_adj",
    Qinplace => "qinplace",
    Qjump => "qjump",
    Qjumpr => "qjumpr",
    Qload => "qload",
    Qloop => "qloop",
    QloopAdj => "qloop_adj",
    Qpred => "qpred",
    QregConcat => "qreg.concat",
    QregFree => "qreg.free",
    QregFromId => "qreg.from_id",
    QregId => "qreg.id",
    QregNew => "qreg.new",
    QregSlice => "qreg.slice",
    QregWidth => "qreg.width",
    Qregion => "qregion",
    Qreturn => "qreturn",
    Quncopy => "quncopy",
    Qunload => "qunload",
    Qstore => "qstore",
    Qunpred => "qunpred",
    Qxor => "qxor",
    Ret => "ret",
    Retval => "retval",
    Rx => "rx",
    Ry => "ry",
    Rz => "rz",
    S => "s",
    Sdg => "sdg",
    Sdiv => "sdiv",
    Select => "select",
    Shl => "shl",
    Srem => "srem",
    Sub => "sub",
    Swap => "swap",
    T => "t",
    Tdg => "tdg",
    Trap => "trap",
    Udiv => "udiv",
    Uncall => "uncall",
    Urem => "urem",
    X => "x",
    Xor => "xor",
}
impl Op {
    fn sized(text: &str) -> Option<Self> {
        let (lifted, base) = text
            .strip_prefix("lift.")
            .map_or((false, text), |s| (true, s));
        let (family, width) = base.rsplit_once(".i")?;
        let width = width.parse().ok()?;
        let kind = match family {
            "const" => SizedOp::Const,
            "load" => SizedOp::Load,
            "store" => SizedOp::Store,
            "zext" => SizedOp::Zext,
            "sext" => SizedOp::Sext,
            "trunc" => SizedOp::Trunc,
            "ptrtoint" => SizedOp::Ptrtoint,
            "measure" => SizedOp::Measure,
            "add" => SizedOp::Add,
            "sub" => SizedOp::Sub,
            "mul" => SizedOp::Mul,
            "and" => SizedOp::And,
            "or" => SizedOp::Or,
            "xor" => SizedOp::Xor,
            "shl" => SizedOp::Shl,
            "lshr" => SizedOp::Lshr,
            "ashr" => SizedOp::Ashr,
            _ => return None,
        };
        Some(Self::Sized {
            kind,
            width,
            lifted,
            text: text.into(),
        })
    }
}
impl From<String> for Op {
    fn from(text: String) -> Self {
        Self::from(text.as_str())
    }
}
impl Deref for Op {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
impl fmt::Debug for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl PartialEq<&str> for Op {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<str> for Op {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<String> for Op {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}
