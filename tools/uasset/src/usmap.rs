//! .usmap mapping file parser (versions 0..=4, uncompressed).
use crate::{R, Res};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Ty {
    Byte, Bool, Int, Float, Object, Name, Delegate, Double,
    Array(Box<Ty>), Struct(String), Str, Text, Interface, MulticastDelegate,
    WeakObject, LazyObject, SoftObject,
    UInt64, UInt32, UInt16, Int64, Int16, Int8,
    Map(Box<Ty>, Box<Ty>), Set(Box<Ty>), Enum(Box<Ty>, Option<String>),
    FieldPath, Optional(Box<Ty>), Utf8Str, AnsiStr,
    Unknown(u8),
}

#[derive(Debug)]
pub struct Prop {
    pub name: String,
    pub ty: Ty,
    pub dim: u8,
}

#[derive(Debug, Default)]
pub struct Struct {
    pub sup: Option<String>,
    pub count: usize,
    /// local schema index -> (index into `list`, element index within a static array)
    pub props: HashMap<usize, (usize, u8)>,
    pub list: Vec<Prop>,
}

#[derive(Debug, Default)]
pub struct Usmap {
    pub enums: HashMap<String, HashMap<u64, String>>,
    pub structs: HashMap<String, Struct>,
}

fn name(r: &mut R, lut: &[String]) -> Res<Option<String>> {
    let i = r.i32()?;
    if i < 0 {
        return Ok(None);
    }
    lut.get(i as usize).cloned().map(Some).ok_or_else(|| format!("usmap name index {i} out of range"))
}

fn ty(r: &mut R, lut: &[String]) -> Res<Ty> {
    let k = r.u8()?;
    Ok(match k {
        0 => Ty::Byte, 1 => Ty::Bool, 2 => Ty::Int, 3 => Ty::Float, 4 | 31 => Ty::Object, 5 => Ty::Name,
        6 => Ty::Delegate, 7 => Ty::Double,
        8 => Ty::Array(Box::new(ty(r, lut)?)),
        9 => Ty::Struct(name(r, lut)?.unwrap_or_default()),
        10 => Ty::Str, 11 => Ty::Text, 12 => Ty::Interface, 13 | 32 => Ty::MulticastDelegate,
        14 => Ty::WeakObject, 15 => Ty::LazyObject, 16 | 17 | 33 => Ty::SoftObject,
        18 => Ty::UInt64, 19 => Ty::UInt32, 20 => Ty::UInt16, 21 => Ty::Int64, 22 => Ty::Int16, 23 => Ty::Int8,
        24 => {
            let k = ty(r, lut)?;
            Ty::Map(Box::new(k), Box::new(ty(r, lut)?))
        }
        25 => Ty::Set(Box::new(ty(r, lut)?)),
        26 => {
            let inner = ty(r, lut)?;
            Ty::Enum(Box::new(inner), name(r, lut)?)
        }
        27 => Ty::FieldPath,
        28 => Ty::Optional(Box::new(ty(r, lut)?)),
        29 => Ty::Utf8Str, 30 => Ty::AnsiStr,
        k => Ty::Unknown(k),
    })
}

impl Usmap {
    pub fn load(path: &str) -> Res<Usmap> {
        let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &[u8]) -> Res<Usmap> {
        let mut r = R::new(raw, 0);
        if r.u16()? != 0x30C4 {
            return Err("bad usmap magic".into());
        }
        let ver = r.u8()?;
        if ver > 4 {
            return Err(format!("unsupported usmap version {ver}"));
        }
        if ver >= 1 && r.bool32()? {
            if ver >= 3 {
                r.take(2 + 2 + 2 + 4)?; // FEngineVersion: major, minor, patch, changelist
                r.fstring()?; // branch
            }
            r.take(8)?; // package file version (ue4, ue5)
            let n = r.u32()?;
            r.take(n as usize * 20)?; // custom versions
            r.u32()?; // net CL
        }
        let comp = r.u8()?;
        let (csize, dsize) = (r.u32()? as usize, r.u32()? as usize);
        if comp != 0 || csize != dsize {
            return Err(format!("usmap compression method {comp} unsupported (only none)"));
        }
        let body = r.take(csize)?;
        let mut r = R::new(body, 0);
        let n = r.u32()?;
        let mut lut = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let l = if ver >= 2 { r.u16()? as usize } else { r.u8()? as usize };
            lut.push(String::from_utf8_lossy(r.take(l)?).into_owned());
        }
        let mut m = Usmap::default();
        for _ in 0..r.u32()? {
            let en = name(&mut r, &lut)?.unwrap_or_default();
            let cnt = if ver >= 3 { r.u16()? as usize } else { r.u8()? as usize };
            let mut vals = HashMap::new();
            for j in 0..cnt {
                let v = if ver >= 4 { r.u64()? } else { j as u64 };
                vals.insert(v, name(&mut r, &lut)?.unwrap_or_default());
            }
            m.enums.entry(en).or_insert(vals);
        }
        for _ in 0..r.u32()? {
            let nm = name(&mut r, &lut)?.unwrap_or_default();
            let sup = name(&mut r, &lut)?;
            let count = r.u16()? as usize;
            let ser = r.u16()?;
            let mut s = Struct { sup, count, ..Default::default() };
            for _ in 0..ser {
                let idx = r.u16()? as usize;
                let dim = r.u8()?;
                let pn = name(&mut r, &lut)?.unwrap_or_default();
                let t = ty(&mut r, &lut)?;
                s.list.push(Prop { name: pn, ty: t, dim });
                for j in 0..dim {
                    s.props.insert(idx + j as usize, (s.list.len() - 1, j));
                }
            }
            m.structs.insert(nm, s);
        }
        Ok(m)
    }

    /// Property at global schema index `idx` of struct `name` (own properties first, then the super chain).
    pub fn prop<'a>(&'a self, extra: &'a HashMap<String, Struct>, name: &str, idx: usize) -> Option<(&'a Prop, u8)> {
        let (mut cur, mut idx) = (Some(name), idx);
        for _ in 0..64 {
            let s = self.structs.get(cur?).or_else(|| extra.get(cur?))?;
            if idx < s.count {
                let (i, e) = *s.props.get(&idx)?;
                return Some((&s.list[i], e));
            }
            idx -= s.count;
            cur = s.sup.as_deref();
        }
        None
    }
}
