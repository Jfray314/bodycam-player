//! UStruct / UFunction binary data and Kismet bytecode (UE 5.5): disassembly, JSON and pseudo-code.
//! Script offsets (jump targets, FunctionOffsets) are *in-memory* offsets: object/property pointers count 8 bytes,
//! FNames 12, though serialized as 4 / variable / 8.
use crate::{R, Res, package::Package, props::Reader};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct Fld {
    pub ty: String,
    pub name: String,
    pub pflags: u64,
    pub dim: i32,
}

pub struct SData {
    pub sup: i32,
    pub fields: Vec<Fld>,
    pub mem_size: usize,
    pub script: Vec<u8>,
    pub func_flags: Option<u32>,
    /// bytes left in the export after everything we decoded
    pub tail: usize,
}

fn fld(pkg: &Package, r: &mut R) -> Res<Option<Fld>> {
    let tn = pkg.fname(r)?;
    if tn == "None" {
        return Ok(None);
    }
    let name = pkg.fname(r)?;
    r.u32()?; // field flags
    let dim = r.i32()?;
    r.i32()?; // element size
    let pflags = r.u64()?;
    r.u16()?; // rep index
    pkg.fname(r)?; // rep notify func
    r.u8()?; // replication condition
    let on = |r: &mut R| -> Res<String> { Ok(pkg.obj_name(r.i32()?).unwrap_or("None").to_string()) };
    let base = tn.strip_suffix("Property").unwrap_or(&tn).to_string();
    let ty = match tn.as_str() {
        "BoolProperty" => {
            r.take(6)?;
            base
        }
        "ByteProperty" => match on(r)?.as_str() {
            "None" => base,
            e => format!("Enum<{e}>"),
        },
        "ObjectProperty" | "WeakObjectProperty" | "LazyObjectProperty" | "SoftObjectProperty" | "InterfaceProperty" | "StructProperty" | "DelegateProperty" | "MulticastInlineDelegateProperty"
        | "MulticastDelegateProperty" | "FieldPathProperty" => format!("{base}<{}>", on(r)?),
        "ClassProperty" | "SoftClassProperty" => {
            on(r)?;
            format!("{base}<{}>", on(r)?)
        }
        "EnumProperty" => {
            let e = on(r)?;
            fld(pkg, r)?;
            format!("Enum<{e}>")
        }
        "ArrayProperty" | "SetProperty" | "OptionalProperty" => format!("{base}<{}>", fld(pkg, r)?.ok_or("null inner")?.ty),
        "MapProperty" => {
            let k = fld(pkg, r)?.ok_or("null key")?.ty;
            format!("Map<{k},{}>", fld(pkg, r)?.ok_or("null value")?.ty)
        }
        _ => base,
    };
    Ok(Some(Fld { ty, name, pflags, dim }))
}

pub fn read_struct(rd: &Reader, i: usize) -> Res<SData> {
    let pkg = rd.pkg;
    let e = &pkg.exports[i];
    let end = e.offset.checked_add(e.size).filter(|&x| x <= pkg.data.len()).ok_or("export range outside file")?;
    let mut r = R::new(&pkg.data[..end], e.offset);
    let cls = pkg.obj_name(e.class).unwrap_or_default();
    rd.read_props(&mut r, cls, &mut Map::new())?;
    if r.bool32()? {
        r.take(16)?;
    }
    let sup = r.i32()?;
    let nc = r.i32()?;
    r.take(usize::try_from(nc).map_err(|_| "bad child count")? * 4)?;
    let mut fields = vec![];
    for _ in 0..r.i32()? {
        fields.push(fld(pkg, &mut r)?.ok_or("null field")?);
    }
    let mem_size = r.i32()? as usize;
    let sz = r.i32()? as usize;
    let script = r.take(sz)?.to_vec();
    let mut func_flags = None;
    if cls.ends_with("Function") {
        let f = r.u32()?;
        if f & 0x40 != 0 {
            r.u16()?;
        }
        func_flags = Some(f);
        r.take(8)?; // EventGraphFunction, EventGraphCallOffset
    }
    Ok(SData { sup, fields, mem_size, script, func_flags, tail: end - r.p })
}

#[derive(Debug)]
pub enum A {
    E(Box<Ex>),
    L(Vec<Ex>),
    I(i64),
    F(f64),
    S(String),
    /// FName
    N(String),
    /// package index
    O(i32),
    /// FFieldPath: names + owner
    P(Vec<String>, i32),
    /// in-memory code offset
    Off(u32),
}

#[derive(Debug)]
pub struct Ex {
    pub off: u32,
    pub op: u8,
    pub a: Vec<A>,
}

struct Dec<'a> {
    r: R<'a>,
    pkg: &'a Package,
    mem: u32,
}

impl Dec<'_> {
    fn name(&mut self) -> Res<A> {
        self.mem += 12;
        Ok(A::N(self.pkg.fname(&mut self.r)?))
    }
    fn obj(&mut self) -> Res<A> {
        self.mem += 8;
        Ok(A::O(self.r.i32()?))
    }
    fn prop(&mut self) -> Res<A> {
        self.mem += 8;
        let n = self.r.i32()?;
        if !(0..64).contains(&n) {
            return Err(format!("bad field path count {n}"));
        }
        let p = (0..n).map(|_| self.pkg.fname(&mut self.r)).collect::<Res<_>>()?;
        Ok(A::P(p, self.r.i32()?))
    }
    fn off(&mut self) -> Res<A> {
        self.mem += 4;
        Ok(A::Off(self.r.u32()?))
    }
    fn int(&mut self, n: u32) -> Res<A> {
        self.mem += n;
        Ok(A::I(match n {
            1 => self.r.u8()? as i64,
            2 => self.r.u16()? as i64,
            4 => self.r.i32()? as i64,
            _ => self.r.i64()?,
        }))
    }
    fn f64s(&mut self, n: usize, dbl: bool) -> Res<Vec<A>> {
        self.mem += (n * if dbl { 8 } else { 4 }) as u32;
        (0..n).map(|_| Ok(A::F(if dbl { self.r.f64()? } else { self.r.f32()? as f64 }))).collect()
    }
    fn ex(&mut self) -> Res<A> {
        Ok(A::E(Box::new(self.expr()?)))
    }
    /// expressions up to and including terminator opcode `end`
    fn list(&mut self, end: u8) -> Res<A> {
        let mut v = vec![];
        loop {
            if self.r.d.get(self.r.p) == Some(&end) {
                self.r.p += 1;
                self.mem += 1;
                return Ok(A::L(v));
            }
            v.push(self.expr()?);
        }
    }
    fn cstr(&mut self, wide: bool) -> Res<A> {
        let mut s = vec![];
        loop {
            let c = if wide { self.r.u16()? } else { self.r.u8()? as u16 };
            self.mem += if wide { 2 } else { 1 };
            if c == 0 {
                return Ok(A::S(String::from_utf16_lossy(&s)));
            }
            s.push(c);
        }
    }
    fn expr(&mut self) -> Res<Ex> {
        let off = self.mem;
        let op = self.r.u8()?;
        self.mem += 1;
        let a = match op {
            0x00 | 0x01 | 0x02 | 0x33 | 0x48 | 0x6C => vec![self.prop()?],
            0x04 | 0x4E | 0x4F | 0x51 | 0x5D | 0x67 | 0x6D => vec![self.ex()?],
            0x06 | 0x4C | 0x5B => vec![self.off()?],
            0x07 => vec![self.off()?, self.ex()?],
            0x09 => vec![self.int(2)?, self.int(1)?, self.ex()?],
            0x0C => vec![self.int(4)?],
            0x0F => vec![self.prop()?, self.ex()?, self.ex()?],
            0x42 | 0x64 => vec![self.prop()?, self.ex()?],
            0x10 => vec![self.prop()?, self.int(1)?],
            0x12 | 0x19 | 0x1A => vec![self.ex()?, self.off()?, self.prop()?, self.ex()?],
            0x13 | 0x2E | 0x52 | 0x54 | 0x55 => vec![self.obj()?, self.ex()?],
            0x14 | 0x43 | 0x44 | 0x5F | 0x60 | 0x5C | 0x62 | 0x6B => vec![self.ex()?, self.ex()?],
            0x18 => vec![self.off()?, self.ex()?],
            0x1B | 0x45 => vec![self.name()?, self.list(0x16)?],
            0x1C | 0x46 | 0x68 => vec![self.obj()?, self.list(0x16)?],
            0x1D => vec![self.int(4)?],
            0x1E => self.f64s(1, false)?,
            0x37 => self.f64s(1, true)?,
            0x1F => vec![self.cstr(false)?],
            0x34 => vec![self.cstr(true)?],
            0x20 => vec![self.obj()?],
            0x21 | 0x4B => vec![self.name()?],
            0x22 | 0x23 => self.f64s(3, true)?,
            0x41 => self.f64s(3, false)?,
            0x2B => self.f64s(10, true)?,
            0x24 | 0x2C => vec![self.int(1)?],
            0x35 | 0x36 => vec![self.int(8)?],
            0x29 => {
                let k = self.int(1)?;
                let n = if let A::I(k) = k { k } else { 0 };
                let mut v = vec![k];
                match n {
                    1 => v.extend([self.ex()?, self.ex()?, self.ex()?]),
                    2 | 3 => v.push(self.ex()?),
                    4 => v.extend([self.obj()?, self.ex()?, self.ex()?]),
                    _ => {}
                }
                v
            }
            0x2F => vec![self.obj()?, self.int(4)?, self.list(0x30)?],
            0x31 => vec![self.ex()?, self.list(0x32)?],
            0x38 => {
                let t = self.int(1)?;
                let obj = matches!(t, A::I(0x46));
                let mut v = vec![t];
                if obj {
                    v.push(self.obj()?);
                }
                v.push(self.ex()?);
                v
            }
            0x39 => vec![self.ex()?, self.int(4)?, self.list(0x3A)?],
            0x3B => vec![self.ex()?, self.int(4)?, self.list(0x3C)?],
            0x3D => vec![self.prop()?, self.int(4)?, self.list(0x3E)?],
            0x3F => vec![self.prop()?, self.prop()?, self.int(4)?, self.list(0x40)?],
            0x65 => vec![self.prop()?, self.int(4)?, self.list(0x66)?],
            0x61 => vec![self.name()?, self.ex()?, self.ex()?],
            0x63 => vec![self.obj()?, self.ex()?, self.list(0x16)?],
            0x69 => {
                let n = if let A::I(n) = self.int(2)? { n } else { 0 };
                let mut v = vec![self.off()?, self.ex()?];
                for _ in 0..n {
                    v.extend([self.ex()?, self.off()?, self.ex()?]);
                }
                v.push(self.ex()?);
                v
            }
            0x6A => {
                let t = self.int(1)?;
                let mut v = vec![];
                if matches!(t, A::I(4)) {
                    v.push(self.name()?);
                }
                v.insert(0, t);
                v
            }
            // no operands
            0x03 | 0x05 | 0x0B | 0x15 | 0x16 | 0x17 | 0x25..=0x28 | 0x2A | 0x2D | 0x30 | 0x32 | 0x3A | 0x3C | 0x3E | 0x40 | 0x4A | 0x4D | 0x50 | 0x53 | 0x5A | 0x5E | 0x66 => vec![],
            _ => return Err(format!("unknown opcode {op:#04x} at {off}")),
        };
        // EX_Return takes an expression (0x04); 0x03/0x05 are unused here
        Ok(Ex { off, op, a })
    }
}

/// Decode a script into top-level statements. Statements decoded before an error are returned with it.
pub fn decode(pkg: &Package, script: &[u8], mem_size: usize) -> (Vec<Ex>, Option<String>) {
    let mut d = Dec { r: R::new(script, 0), pkg, mem: 0 };
    let mut out = vec![];
    while d.r.p < script.len() {
        match d.expr() {
            Ok(e) => out.push(e),
            Err(e) => return (out, Some(e)),
        }
    }
    let err = (d.mem as usize != mem_size).then(|| format!("memory size {} != BytecodeBufferSize {mem_size}", d.mem));
    (out, err)
}

/// Jump / push-flow targets that do not land on a statement start.
pub fn bad_targets(stmts: &[Ex]) -> Vec<u32> {
    let starts: BTreeSet<u32> = stmts.iter().map(|s| s.off).collect();
    let mut bad = vec![];
    for s in stmts {
        if let (0x06 | 0x07 | 0x4C, Some(A::Off(t))) = (s.op, s.a.first()) {
            if !starts.contains(t) {
                bad.push(*t);
            }
        }
    }
    bad
}

pub fn opname(op: u8) -> &'static str {
    const N: &[(u8, &str)] = &[
        (0x00, "LocalVariable"), (0x01, "InstanceVariable"), (0x02, "DefaultVariable"), (0x04, "Return"), (0x06, "Jump"), (0x07, "JumpIfNot"),
        (0x09, "Assert"), (0x0B, "Nothing"), (0x0C, "NothingInt32"), (0x0F, "Let"), (0x10, "BitFieldConst"), (0x12, "ClassContext"),
        (0x13, "MetaCast"), (0x14, "LetBool"), (0x15, "EndParmValue"), (0x16, "EndFunctionParms"), (0x17, "Self"), (0x18, "Skip"),
        (0x19, "Context"), (0x1A, "Context_FailSilent"), (0x1B, "VirtualFunction"), (0x1C, "FinalFunction"), (0x1D, "IntConst"),
        (0x1E, "FloatConst"), (0x1F, "StringConst"), (0x20, "ObjectConst"), (0x21, "NameConst"), (0x22, "RotationConst"), (0x23, "VectorConst"),
        (0x24, "ByteConst"), (0x25, "IntZero"), (0x26, "IntOne"), (0x27, "True"), (0x28, "False"), (0x29, "TextConst"), (0x2A, "NoObject"),
        (0x2B, "TransformConst"), (0x2C, "IntConstByte"), (0x2D, "NoInterface"), (0x2E, "DynamicCast"), (0x2F, "StructConst"),
        (0x30, "EndStructConst"), (0x31, "SetArray"), (0x32, "EndArray"), (0x33, "PropertyConst"), (0x34, "UnicodeStringConst"),
        (0x35, "Int64Const"), (0x36, "UInt64Const"), (0x37, "DoubleConst"), (0x38, "PrimitiveCast"), (0x39, "SetSet"), (0x3A, "EndSet"),
        (0x3B, "SetMap"), (0x3C, "EndMap"), (0x3D, "SetConst"), (0x3E, "EndSetConst"), (0x3F, "MapConst"), (0x40, "EndMapConst"),
        (0x41, "Vector3fConst"), (0x42, "StructMemberContext"), (0x43, "LetMulticastDelegate"), (0x44, "LetDelegate"),
        (0x45, "LocalVirtualFunction"), (0x46, "LocalFinalFunction"), (0x48, "LocalOutVariable"), (0x4A, "DeprecatedOp4A"),
        (0x4B, "InstanceDelegate"), (0x4C, "PushExecutionFlow"), (0x4D, "PopExecutionFlow"), (0x4E, "ComputedJump"),
        (0x4F, "PopExecutionFlowIfNot"), (0x50, "Breakpoint"), (0x51, "InterfaceContext"), (0x52, "ObjToInterfaceCast"), (0x53, "EndOfScript"),
        (0x54, "CrossInterfaceCast"), (0x55, "InterfaceToObjCast"), (0x5A, "WireTracepoint"), (0x5B, "SkipOffsetConst"),
        (0x5C, "AddMulticastDelegate"), (0x5D, "ClearMulticastDelegate"), (0x5E, "Tracepoint"), (0x5F, "LetObj"), (0x60, "LetWeakObjPtr"),
        (0x61, "BindDelegate"), (0x62, "RemoveMulticastDelegate"), (0x63, "CallMulticastDelegate"), (0x64, "LetValueOnPersistentFrame"),
        (0x65, "ArrayConst"), (0x66, "EndArrayConst"), (0x67, "SoftObjectConst"), (0x68, "CallMath"), (0x69, "SwitchValue"),
        (0x6A, "InstrumentationEvent"), (0x6B, "ArrayGetByRef"), (0x6C, "ClassSparseDataVariable"), (0x6D, "FieldPathConst"),
    ];
    N.iter().find(|n| n.0 == op).map_or("Unknown", |n| n.1)
}

pub fn ex_json(pkg: &Package, e: &Ex) -> Value {
    let a: Vec<Value> = e.a.iter().map(|a| arg_json(pkg, a)).collect();
    json!({"off": e.off, "op": format!("EX_{}", opname(e.op)), "args": a})
}

fn arg_json(pkg: &Package, a: &A) -> Value {
    match a {
        A::E(e) => ex_json(pkg, e),
        A::L(v) => Value::Array(v.iter().map(|e| ex_json(pkg, e)).collect()),
        A::I(i) => json!(i),
        A::F(f) => json!(f),
        A::S(s) | A::N(s) => json!(s),
        A::O(i) => json!(pkg.path(*i)),
        A::P(p, o) => json!({"path": p, "owner": pkg.path(*o)}),
        A::Off(o) => json!({"label": o}),
    }
}

// ---------------------------------------------------------------- pseudo-code

struct Pr<'a> {
    pkg: &'a Package,
}

impl Pr<'_> {
    fn obj(&self, i: i32) -> String {
        self.pkg.obj_name(i).unwrap_or("null").to_string()
    }
    fn pn(a: &A) -> String {
        if let A::P(p, _) = a { p.last().cloned().unwrap_or_default() } else { "?".into() }
    }
    fn e(&self, a: &A) -> String {
        match a {
            A::E(e) => self.show(e),
            _ => "?".into(),
        }
    }
    fn l(&self, a: &A) -> Vec<String> {
        if let A::L(v) = a { v.iter().map(|e| self.show(e)).collect() } else { vec![] }
    }
    fn n(a: &A) -> String {
        match a {
            A::N(s) | A::S(s) => s.clone(),
            A::I(i) => i.to_string(),
            A::F(f) => f.to_string(),
            A::Off(o) => o.to_string(),
            _ => "?".into(),
        }
    }
    fn on(&self, a: &A) -> String {
        if let A::O(i) = a { self.obj(*i) } else { "?".into() }
    }
    /// the right-hand side of a context: a member name or call without the `self.` prefix
    fn member(&self, e: &Ex) -> String {
        match e.op {
            0x01 | 0x02 => Self::pn(&e.a[0]),
            _ => self.show(e),
        }
    }
    fn call(&self, f: i32, args: Vec<String>, math: bool) -> String {
        let n = self.obj(f);
        let infix = if math && args.len() == 2 && self.pkg.path(f).is_some_and(|p| p.contains("KismetMathLibrary")) {
            [("Add_", "+"), ("Subtract_", "-"), ("Multiply_", "*"), ("Divide_", "/"), ("GreaterEqual_", ">="), ("LessEqual_", "<="), ("Greater_", ">"), ("Less_", "<"), ("EqualEqual_", "=="), ("NotEqual_", "!="), ("BooleanAND", "&&"), ("BooleanOR", "||")]
                .iter()
                .find(|(p, _)| n.starts_with(p))
                .map(|x| x.1)
        } else {
            None
        };
        match infix {
            Some(op) => format!("({} {op} {})", args[0], args[1]),
            None if math && args.len() == 1 && n.starts_with("Not_PreBool") => format!("!{}", args[0]),
            None => format!("{n}({})", args.join(", ")),
        }
    }
    fn show(&self, e: &Ex) -> String {
        let a = &e.a;
        let v = |x: usize| Self::n(&a[x]);
        match e.op {
            0x00 | 0x48 => Self::pn(&a[0]),
            0x01 => format!("self.{}", Self::pn(&a[0])),
            0x02 => format!("default.{}", Self::pn(&a[0])),
            0x6C => format!("sparse.{}", Self::pn(&a[0])),
            0x33 => format!("prop({})", Self::pn(&a[0])),
            0x04 => match &a[0] {
                A::E(x) if x.op == 0x0B => "return".into(),
                x => format!("return {}", self.e(x)),
            },
            0x06 => format!("goto L{}", v(0)),
            0x07 => format!("if (!{}) goto L{}", self.e(&a[1]), v(0)),
            0x4C => format!("push_flow L{}", v(0)),
            0x4D => "pop_flow".into(),
            0x4E => format!("goto_computed {}", self.e(&a[0])),
            0x4F => format!("if (!{}) pop_flow", self.e(&a[0])),
            0x53 => "end_of_script".into(),
            0x09 => format!("assert({})", self.e(&a[2])),
            0x0B => "nothing".into(),
            0x0F | 0x14 | 0x43 | 0x44 | 0x5F | 0x60 => {
                let (l, r) = if e.op == 0x0F { (1, 2) } else { (0, 1) };
                format!("{} = {}", self.e(&a[l]), self.e(&a[r]))
            }
            0x64 => format!("persist.{} = {}", Self::pn(&a[0]), self.e(&a[1])),
            0x17 => "self".into(),
            0x12 | 0x19 | 0x1A => {
                let c = match &a[3] {
                    A::E(x) => self.member(x),
                    _ => "?".into(),
                };
                format!("{}{}{c}", self.e(&a[0]), if e.op == 0x1A { "?." } else { "." })
            }
            0x42 => format!("{}.{}", self.e(&a[1]), Self::pn(&a[0])),
            0x51 => self.e(&a[0]),
            0x18 => self.e(&a[1]),
            0x1B | 0x45 => format!("{}({})", v(0), self.l(&a[1]).join(", ")),
            0x1C | 0x46 | 0x68 => match &a[0] {
                A::O(f) => self.call(*f, self.l(&a[1]), e.op == 0x68),
                _ => "?".into(),
            },
            0x63 => format!("broadcast {}({})", self.e(&a[1]), self.l(&a[2]).join(", ")),
            0x5C => format!("{}.Add({})", self.e(&a[0]), self.e(&a[1])),
            0x62 => format!("{}.Remove({})", self.e(&a[0]), self.e(&a[1])),
            0x5D => format!("{}.Clear()", self.e(&a[0])),
            0x61 => format!("bind {}.{} -> {}", self.e(&a[2]), v(0), self.e(&a[1])),
            0x4B => format!("delegate({})", v(0)),
            0x1D | 0x2C | 0x35 | 0x36 => v(0),
            0x25 => "0".into(),
            0x26 => "1".into(),
            0x1E | 0x37 => {
                let s = v(0);
                if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") { s } else { s + ".0" }
            }
            0x1F | 0x34 => format!("{:?}", v(0)),
            0x20 => format!("obj({})", self.pkg.path(if let A::O(i) = a[0] { i } else { 0 }).unwrap_or_default()),
            0x21 => format!("'{}'", v(0)),
            0x24 => format!("byte({})", v(0)),
            0x22 => format!("Rot({}, {}, {})", v(0), v(1), v(2)),
            0x23 | 0x41 => format!("Vec({}, {}, {})", v(0), v(1), v(2)),
            0x2B => format!("Transform({})", a.iter().map(Self::n).collect::<Vec<_>>().join(", ")),
            0x27 => "true".into(),
            0x28 => "false".into(),
            0x2A | 0x2D => "null".into(),
            0x29 => format!("Text({})", a[1..].iter().map(|x| if let A::E(_) = x { self.e(x) } else { self.on(x) }).collect::<Vec<_>>().join(", ")),
            0x13 | 0x2E | 0x52 | 0x54 | 0x55 => format!("cast<{}>({})", self.on(&a[0]), self.e(&a[1])),
            0x38 => format!("prim_cast_{:#x}({})", v(0).parse::<i64>().unwrap_or(0), self.e(a.last().unwrap())),
            0x2F => format!("{}{{{}}}", self.on(&a[0]), self.l(&a[2]).join(", ")),
            0x31 => format!("{} = [{}]", self.e(&a[0]), self.l(&a[1]).join(", ")),
            0x39 => format!("{} = set[{}]", self.e(&a[0]), self.l(&a[2]).join(", ")),
            0x3B => format!("{} = map[{}]", self.e(&a[0]), self.l(&a[2]).join(", ")),
            0x65 | 0x3D | 0x3F => format!("[{}]", self.l(a.last().unwrap()).join(", ")),
            0x67 => format!("soft({})", self.e(&a[0])),
            0x6D => format!("fieldpath({})", self.e(&a[0])),
            0x6B => format!("{}[{}]", self.e(&a[0]), self.e(&a[1])),
            0x69 => {
                let cases: Vec<String> = a[2..a.len() - 1].chunks(3).map(|c| format!("{} => {}", self.e(&c[0]), self.e(&c[2]))).collect();
                format!("switch({}) {{ {}, default => {} }}", self.e(&a[1]), cases.join(", "), self.e(a.last().unwrap()))
            }
            0x5A | 0x5E | 0x50 | 0x6A => String::new(),
            op => format!("{}({})", opname(op), a.iter().map(|x| if let A::E(_) = x { self.e(x) } else { Self::n(x) }).collect::<Vec<_>>().join(", ")),
        }
    }
}

/// Event entry points of `ExecuteUbergraph_*` keyed by code offset: other functions of the package call it with a literal.
pub fn ubergraph_entries(rd: &Reader) -> BTreeMap<u32, Vec<String>> {
    let pkg = rd.pkg;
    let mut m: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (i, ex) in pkg.exports.iter().enumerate() {
        if !pkg.obj_name(ex.class).is_some_and(|c| c.ends_with("Function")) || ex.name.starts_with("ExecuteUbergraph") {
            continue;
        }
        let Ok(s) = read_struct(rd, i) else { continue };
        for st in decode(pkg, &s.script, s.mem_size).0 {
            if let (0x46 | 0x1C, Some(A::O(f)), Some(A::L(args))) = (st.op, st.a.first(), st.a.get(1)) {
                if pkg.obj_name(*f).is_some_and(|n| n.starts_with("ExecuteUbergraph")) {
                    match args.first() {
                        Some(Ex { op: 0x1D, a, .. }) => {
                            if let Some(A::I(o)) = a.first() {
                                m.entry(*o as u32).or_default().push(ex.name.clone());
                            }
                        }
                        Some(Ex { op: 0x25, .. }) => m.entry(0).or_default().push(ex.name.clone()),
                        Some(Ex { op: 0x26, .. }) => m.entry(1).or_default().push(ex.name.clone()),
                        _ => {}
                    }
                }
            }
        }
    }
    m
}

pub fn flag_names(f: u32) -> String {
    const N: &[(u32, &str)] = &[
        (0x1, "Final"), (0x40, "Net"), (0x80, "NetReliable"), (0x200, "Exec"), (0x400, "Native"), (0x800, "Event"), (0x2000, "Static"),
        (0x4000, "NetMulticast"), (0x8000, "Ubergraph"), (0x10000, "MulticastDelegate"), (0x20000, "Public"), (0x40000, "Private"),
        (0x80000, "Protected"), (0x100000, "Delegate"), (0x200000, "NetServer"), (0x400000, "HasOutParms"), (0x800000, "HasDefaults"),
        (0x1000000, "NetClient"), (0x4000000, "BlueprintCallable"), (0x8000000, "BlueprintEvent"), (0x10000000, "BlueprintPure"),
        (0x40000000, "Const"),
    ];
    N.iter().filter(|n| f & n.0 != 0).map(|n| n.1).collect::<Vec<_>>().join("|")
}

fn sig(d: &SData, name: &str) -> String {
    let t = |f: &Fld| format!("{}: {}", f.name, f.ty);
    let params: Vec<String> = d.fields.iter().filter(|f| f.pflags & 0x80 != 0 && f.pflags & 0x400 == 0).map(|f| if f.pflags & 0x100 != 0 { format!("out {}", t(f)) } else { t(f) }).collect();
    let ret = d.fields.iter().find(|f| f.pflags & 0x400 != 0).map(|f| format!(" -> {}", f.ty)).unwrap_or_default();
    format!("fn {name}({}){ret}", params.join(", "))
}

/// Readable listing of one function. `entries` (offset -> events) labels ubergraph entry points.
pub fn pseudo(pkg: &Package, d: &SData, name: &str, stmts: &[Ex], entries: &BTreeMap<u32, Vec<String>>) -> String {
    let pr = Pr { pkg };
    let mut targets = BTreeSet::new();
    for s in stmts {
        if let (0x06 | 0x07 | 0x4C, Some(A::Off(t))) = (s.op, s.a.first()) {
            targets.insert(*t);
        }
    }
    let mut o = format!("{}  // flags {:#x} {}\n", sig(d, name), d.func_flags.unwrap_or(0), flag_names(d.func_flags.unwrap_or(0)));
    let (temps, named): (Vec<&Fld>, Vec<&Fld>) = d.fields.iter().filter(|f| f.pflags & 0x80 == 0).partition(|f| ["CallFunc_", "K2Node_", "Temp_"].iter().any(|p| f.name.starts_with(p)));
    for f in named {
        o += &format!("  local {}: {}\n", f.name, f.ty);
    }
    if !temps.is_empty() {
        o += &format!("  // + {} compiler temporaries\n", temps.len());
    }
    for s in stmts {
        if let Some(ev) = entries.get(&s.off) {
            o += &format!("// ==== EVENT {} (entry {}) ====\n", ev.join(", "), s.off);
        }
        if targets.contains(&s.off) {
            o += &format!("L{}:\n", s.off);
        }
        let t = pr.show(s);
        if !t.is_empty() {
            o += &format!("  {t}\n");
        }
    }
    o
}

/// Fill `o` (the export JSON object) for a Function / DelegateFunction export. Returns Err for a partial decode.
pub fn export_fn(rd: &Reader, i: usize, o: &mut Map<String, Value>) -> Res<()> {
    let pkg = rd.pkg;
    let d = read_struct(rd, i)?;
    let name = &pkg.exports[i].name;
    let f = d.func_flags.unwrap_or(0);
    o.insert("flags".into(), json!({"value": f, "names": flag_names(f)}));
    o.insert("super".into(), json!(pkg.path(d.sup)));
    let fl = |p: &Fld| json!({"name": p.name, "type": p.ty, "flags": p.pflags});
    o.insert("params".into(), Value::Array(d.fields.iter().filter(|f| f.pflags & 0x80 != 0).map(fl).collect()));
    o.insert("locals".into(), Value::Array(d.fields.iter().filter(|f| f.pflags & 0x80 == 0).map(fl).collect()));
    let (stmts, err) = decode(pkg, &d.script, d.mem_size);
    o.insert("bytecode".into(), Value::Array(stmts.iter().map(|s| ex_json(pkg, s)).collect()));
    let entries = if name.starts_with("ExecuteUbergraph") { ubergraph_entries(rd) } else { BTreeMap::new() };
    o.insert("pseudo".into(), json!(pseudo(pkg, &d, name, &stmts, &entries)));
    let bad = bad_targets(&stmts);
    if let Some(e) = err {
        return Err(e);
    }
    if !bad.is_empty() {
        return Err(format!("jump targets not on instruction boundary: {bad:?}"));
    }
    Ok(())
}
