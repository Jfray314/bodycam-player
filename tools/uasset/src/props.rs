//! Unversioned property deserialization driven by a .usmap.
use crate::package::{Package, RF_CLASS_DEFAULT_OBJECT};
use crate::usmap::{Prop, Struct, Ty, Usmap};
use crate::{R, Res};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Zero,
    Normal,
    /// element of an array / set / map (no zero mask)
    Inner,
}

pub struct Reader<'a> {
    pub pkg: &'a Package,
    pub map: &'a Usmap,
    /// native/unsupported struct names hit while reading
    pub unsupported: RefCell<BTreeSet<String>>,
    /// Schemas synthesised from BlueprintGeneratedClass exports whose class is missing from the usmap.
    pub extra: HashMap<String, Struct>,
}

/// Native-serialized structs we know exist but do not decode. Reading one stops the export.
const UNSUPPORTED: &[&str] = &[
    "ExpressionInput", "ColorMaterialInput", "ScalarMaterialInput", "VectorMaterialInput", "Vector2MaterialInput",
    "MaterialAttributesInput", "FontData", "FontCharacter", "RawAnimSequenceTrack", "Spline", "UniqueNetIdRepl",
    "SectionEvaluationDataTree", "LevelSequenceObjectReferenceMap", "InstancedPropertyBag",
    "InstancedOverridablePropertyBag", "InstancedStructContainer", "InstancedStructArray", "CompressedRichCurve",
    "NameCurveKey", "StringCurveKey", "SmartName", "EdGraphPinType", "MaterialLayersFunctionsTree",
    "MaterialOverrideNanite", "StateTreeInstanceData", "UniversalObjectLocatorFragment", "AttributeCurve",
    "AnimationAttributeIdentifier", "WorldConditionQueryDefinition", "NavAgentSelector", "MidiEvent",
    "KConvexElem", "KLevelSetElem", "KSkinnedLevelSetElem", "SkeletalMeshSamplingLODBuiltData",
    "SkeletalMeshSamplingRegionBuiltData", "RawDistributionFloat", "RawDistributionVector", "FloatDistribution",
    "VectorDistribution",
];

fn num(x: f64) -> Value {
    serde_json::Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

impl<'a> Reader<'a> {
    pub fn new(pkg: &'a Package, map: &'a Usmap) -> Self {
        let mut rd = Reader { pkg, map, unsupported: Default::default(), extra: HashMap::new() };
        for (i, e) in pkg.exports.iter().enumerate() {
            let is_class = pkg.obj_name(e.class).is_some_and(|c| c.ends_with("GeneratedClass"));
            if is_class && !map.structs.contains_key(&e.name) {
                if let Ok(s) = rd.synth_class(i) {
                    rd.extra.insert(e.name.clone(), s);
                }
            }
        }
        rd
    }

    fn has(&self, name: &str) -> bool {
        self.map.structs.contains_key(name) || self.extra.contains_key(name)
    }

    /// Rebuild a schema from the FProperty list serialised inside a BlueprintGeneratedClass export.
    pub fn synth_class(&self, i: usize) -> Res<Struct> {
        let e = &self.pkg.exports[i];
        let end = e.offset.checked_add(e.size).filter(|&x| x <= self.pkg.data.len()).ok_or("export range outside file")?;
        let mut r = R::new(&self.pkg.data[..end], e.offset);
        let cls = self.pkg.obj_name(e.class).unwrap_or_default();
        self.read_props(&mut r, cls, &mut Map::new())?; // properties of the UClass object itself
        if r.bool32()? {
            r.take(16)?; // object guid
        }
        let sup = self.pkg.obj_name(r.i32()?).map(str::to_string);
        let n_children = r.i32()?;
        r.take(usize::try_from(n_children).map_err(|_| "bad child count")? * 4)?;
        let n = r.i32()?;
        let mut s = Struct { sup, ..Default::default() };
        for _ in 0..n {
            let Some(p) = self.field(&mut r)? else { return Err("null field".into()) };
            for j in 0..p.dim {
                s.props.insert(s.count + j as usize, (s.list.len(), j));
            }
            s.count += p.dim as usize;
            s.list.push(p);
        }
        Ok(s)
    }

    /// FField::SerializeSingleField for the property types that can appear in blueprint classes.
    fn field(&self, r: &mut R) -> Res<Option<Prop>> {
        let tn = self.pkg.fname(r)?;
        if tn == "None" {
            return Ok(None);
        }
        let name = self.pkg.fname(r)?;
        if std::env::var_os("UASSET_TRACE").is_some() {
            eprintln!("  field {tn} {name} @{}", r.p);
        }
        r.u32()?; // field flags
        let dim = r.i32()?;
        r.take(4 + 8 + 2)?; // element size, property flags, rep index
        self.pkg.fname(r)?; // rep notify func
        r.u8()?; // replication condition
        let inner = |r: &mut R| -> Res<Box<Ty>> { Ok(Box::new(self.field(r)?.ok_or("null inner field")?.ty)) };
        let ty = match tn.as_str() {
            "BoolProperty" => {
                r.take(6)?; // field size, byte offset, byte mask, field mask, native bool, ...
                Ty::Bool
            }
            "ByteProperty" => match self.pkg.obj_name(r.i32()?) {
                Some(en) => Ty::Enum(Box::new(Ty::Byte), Some(en.to_string())),
                None => Ty::Byte,
            },
            "IntProperty" => Ty::Int,
            "UInt32Property" => Ty::UInt32,
            "UInt16Property" => Ty::UInt16,
            "UInt64Property" => Ty::UInt64,
            "Int8Property" => Ty::Int8,
            "Int16Property" => Ty::Int16,
            "Int64Property" => Ty::Int64,
            "FloatProperty" => Ty::Float,
            "DoubleProperty" => Ty::Double,
            "NameProperty" => Ty::Name,
            "StrProperty" => Ty::Str,
            "TextProperty" => Ty::Text,
            "ObjectProperty" => {
                r.i32()?;
                Ty::Object
            }
            "ClassProperty" => {
                r.take(8)?;
                Ty::Object
            }
            "WeakObjectProperty" => {
                r.i32()?;
                Ty::WeakObject
            }
            "LazyObjectProperty" => {
                r.i32()?;
                Ty::LazyObject
            }
            "SoftObjectProperty" => {
                r.i32()?;
                Ty::SoftObject
            }
            "SoftClassProperty" => {
                r.take(8)?;
                Ty::SoftObject
            }
            "InterfaceProperty" => {
                r.i32()?;
                Ty::Interface
            }
            "StructProperty" => Ty::Struct(self.pkg.obj_name(r.i32()?).unwrap_or_default().to_string()),
            "DelegateProperty" => {
                r.i32()?;
                Ty::Delegate
            }
            "MulticastInlineDelegateProperty" | "MulticastDelegateProperty" => {
                r.i32()?;
                Ty::MulticastDelegate
            }
            "ArrayProperty" => {
                Ty::Array(inner(r)?)
            }
            "SetProperty" => Ty::Set(inner(r)?),
            "MapProperty" => {
                let k = inner(r)?;
                Ty::Map(k, inner(r)?)
            }
            "EnumProperty" => {
                let en = self.pkg.obj_name(r.i32()?).map(str::to_string);
                Ty::Enum(inner(r)?, en)
            }
            "OptionalProperty" => Ty::Optional(inner(r)?),
            t => return Err(format!("unsupported field type {t}")),
        };
        Ok(Some(Prop { name, ty, dim: dim.clamp(1, 255) as u8 }))
    }

    fn obj(&self, r: &mut R) -> Res<Value> {
        let i = r.i32()?;
        Ok(self.pkg.path(i).map(Value::String).unwrap_or(Value::Null))
    }

    fn fvec<const N: usize>(r: &mut R, dbl: bool, keys: [&str; N]) -> Res<Value> {
        let mut m = Map::new();
        for k in keys {
            m.insert(k.into(), if dbl { num(r.f64()?) } else { numf(r.f32()?) });
        }
        Ok(Value::Object(m))
    }

    /// Native binary structs. `None` means "not a native struct, use the reflected layout".
    fn native(&self, r: &mut R, name: &str) -> Res<Option<Value>> {
        const XYZ: [&str; 3] = ["x", "y", "z"];
        const XYZW: [&str; 4] = ["x", "y", "z", "w"];
        Ok(Some(match name {
            "Vector" | "Vector3d" | "Vector_NetQuantize" | "Vector_NetQuantize10" | "Vector_NetQuantize100" | "Vector_NetQuantizeNormal" => Self::fvec(r, true, XYZ)?,
            "Vector3f" => Self::fvec(r, false, XYZ)?,
            "Rotator" | "Rotator3d" => Self::fvec(r, true, ["pitch", "yaw", "roll"])?,
            "Rotator3f" => Self::fvec(r, false, ["pitch", "yaw", "roll"])?,
            "Vector2D" | "Vector2d" => Self::fvec(r, true, ["x", "y"])?,
            "Vector2f" => Self::fvec(r, false, ["x", "y"])?,
            "Vector4" | "Vector4d" | "Quat" | "Quat4d" | "Plane" | "Plane4d" | "Sphere" | "Sphere3d" => Self::fvec(r, true, XYZW)?,
            "Vector4f" | "Quat4f" | "Plane4f" | "Sphere3f" => Self::fvec(r, false, XYZW)?,
            "TwoVectors" => json!({"v1": Self::fvec(r, true, XYZ)?, "v2": Self::fvec(r, true, XYZ)?}),
            "DeprecateSlateVector2D" => Self::fvec(r, false, ["x", "y"])?,
            "Matrix" | "Matrix44d" => Value::Array((0..16).map(|_| r.f64().map(num)).collect::<Res<_>>()?),
            "Matrix44f" => Value::Array((0..16).map(|_| r.f32().map(numf)).collect::<Res<_>>()?),
            "Box" | "Box3d" => {
                json!({"min": Self::fvec(r, true, XYZ)?, "max": Self::fvec(r, true, XYZ)?, "valid": r.u8()? != 0})
            }
            "Box3f" => json!({"min": Self::fvec(r, false, XYZ)?, "max": Self::fvec(r, false, XYZ)?, "valid": r.u8()? != 0}),
            "Box2D" | "Box2d" => json!({"min": Self::fvec(r, true, ["x", "y"])?, "max": Self::fvec(r, true, ["x", "y"])?, "valid": r.u8()? != 0}),
            "Box2f" => json!({"min": Self::fvec(r, false, ["x", "y"])?, "max": Self::fvec(r, false, ["x", "y"])?, "valid": r.u8()? != 0}),
            "LinearColor" => Self::fvec(r, false, ["r", "g", "b", "a"])?,
            "Color" => {
                let b = r.take(4)?;
                json!({"r": b[2], "g": b[1], "b": b[0], "a": b[3]})
            }
            "Guid" => {
                let g: Vec<String> = (0..4).map(|_| r.u32().map(|u| format!("{u:08X}"))).collect::<Res<_>>()?;
                Value::String(g.concat())
            }
            "GameplayTagContainer" => {
                let n = r.i32()?;
                if n < 0 || n as usize > r.d.len() - r.p {
                    return Err(format!("bad GameplayTagContainer count {n}"));
                }
                Value::Array((0..n).map(|_| self.pkg.fname(r).map(Value::String)).collect::<Res<_>>()?)
            }
            "SoftObjectPath" | "SoftClassPath" | "StringAssetReference" | "StringClassReference" => self.soft_path(r)?,
            "IntPoint" | "Int32Point" | "IntVector2" | "Int32Vector2" => json!({"x": r.i32()?, "y": r.i32()?}),
            "UintVector2" | "Uint32Point" => json!({"x": r.u32()?, "y": r.u32()?}),
            "IntVector" => json!({"x": r.i32()?, "y": r.i32()?, "z": r.i32()?}),
            "IntVector4" => json!({"x": r.i32()?, "y": r.i32()?, "z": r.i32()?, "w": r.i32()?}),
            "FrameNumber" => json!(r.i32()?),
            "DateTime" | "Timespan" => json!(r.i64()?),
            "PerPlatformFloat" | "PerPlatformInt" | "PerPlatformBool" | "PerPlatformFrameRate" => {
                if !r.bool32()? {
                    return Err(format!("non-cooked {name}"));
                }
                match name {
                    "PerPlatformFloat" => json!(numf(r.f32()?)),
                    "PerPlatformInt" => json!(r.i32()?),
                    "PerPlatformBool" => json!(r.bool32()?),
                    _ => json!({"numerator": r.i32()?, "denominator": r.i32()?}),
                }
            }
            "KeyHandleMap" => json!({}), // editor-only lookup; nothing is serialized in cooked data
            "FloatRange" | "Int32Range" | "DoubleRange" | "MovieSceneFrameRange" => {
                let mut b = vec![];
                for _ in 0..2 {
                    let t = r.u8()?;
                    let v = match name {
                        "FloatRange" => numf(r.f32()?),
                        "DoubleRange" => num(r.f64()?),
                        _ => json!(r.i32()?),
                    };
                    b.push(json!({"type": t, "value": v}));
                }
                json!({"lower": b[0], "upper": b[1]})
            }
            "RichCurveKey" => {
                let (im, tm, tw) = (r.u8()?, r.u8()?, r.u8()?);
                let mut f = || r.f32().map(numf);
                json!({"interp_mode": im, "tangent_mode": tm, "tangent_weight_mode": tw,
                    "time": f()?, "value": f()?, "arrive_tangent": f()?, "arrive_tangent_weight": f()?,
                    "leave_tangent": f()?, "leave_tangent_weight": f()?})
            }
            "SimpleCurveKey" => json!({"time": numf(r.f32()?), "value": numf(r.f32()?)}),
            "InstancedStruct" => {
                let ver = r.u8()?;
                let idx = r.i32()?;
                let size = r.i32()? as usize;
                let _ = ver;
                let start = r.p;
                let Some(sn) = self.pkg.obj_name(idx).map(str::to_string) else { return Ok(Some(Value::Null)) };
                let end = start + size;
                let v = self.read_struct(&mut R::new(&r.d[..end.min(r.d.len())], start), &sn, Mode::Normal)?;
                r.p = end;
                json!({"struct": sn, "value": v})
            }
            _ => return Ok(None),
        }))
    }

    fn soft_path(&self, r: &mut R) -> Res<Value> {
        let (p, a, s) = (self.pkg.fname(r)?, self.pkg.fname(r)?, r.fstring()?);
        Ok(Value::String(match (p.as_str(), s.is_empty()) {
            ("None", true) => String::new(),
            (_, true) => format!("{p}.{a}"),
            _ => format!("{p}.{a}:{s}"),
        }))
    }

    fn header(r: &mut R) -> Res<Vec<(usize, bool)>> {
        let mut frags = vec![];
        let mut zeros = 0;
        loop {
            let f = r.u16()?;
            let (skip, has_zero, vals, last) = ((f & 0x7f) as usize, f & 0x80 != 0, (f >> 9) as usize, f & 0x100 != 0);
            if has_zero {
                zeros += vals;
            }
            frags.push((skip, has_zero, vals));
            if last {
                break;
            }
        }
        let mut mask = vec![];
        if zeros > 0 {
            let bytes = if zeros <= 8 { 1 } else if zeros <= 16 { 2 } else { zeros.div_ceil(32) * 4 };
            mask = r.take(bytes)?.to_vec();
        }
        let (mut idx, mut zi, mut out) = (0, 0, vec![]);
        for (skip, has_zero, vals) in frags {
            idx += skip;
            for _ in 0..vals {
                let zero = has_zero && {
                    let z = mask[zi / 8] >> (zi % 8) & 1 != 0;
                    zi += 1;
                    z
                };
                out.push((idx, !zero));
                idx += 1;
            }
        }
        Ok(out)
    }

    /// Reads an unversioned property block of struct `sname` into `out` (partial results kept on error).
    pub fn read_props(&self, r: &mut R, sname: &str, out: &mut Map<String, Value>) -> Res<()> {
        for (idx, nz) in Self::header(r)? {
            let Some((p, elem)) = self.map.prop(&self.extra, sname, idx) else {
                if nz {
                    return Err(format!("{sname}: no property at schema index {idx}"));
                }
                continue;
            };
            if std::env::var_os("UASSET_TRACE").is_some() {
                eprintln!("{:6} {sname}.{} [{idx}] nz={nz} {:?}", r.p, p.name, p.ty);
            }
            let v = self.read_val(r, &p.ty, if nz { Mode::Normal } else { Mode::Zero }).map_err(|e| format!("{sname}.{}: {e}", p.name))?;
            if p.dim > 1 {
                let a = out.entry(p.name.clone()).or_insert_with(|| Value::Array(vec![Value::Null; p.dim as usize]));
                a[elem as usize] = v;
            } else {
                out.insert(p.name.clone(), v);
            }
        }
        Ok(())
    }

    fn read_struct(&self, r: &mut R, name: &str, mode: Mode) -> Res<Value> {
        if mode == Mode::Zero {
            let z = [0u8; 256];
            let mut zr = R { d: &z, p: 0, zero: true };
            return Ok(self.native(&mut zr, name).unwrap_or(None).unwrap_or_else(|| json!({})));
        }
        if let Some(v) = self.native(r, name)? {
            return Ok(v);
        }
        if UNSUPPORTED.contains(&name) {
            self.unsupported.borrow_mut().insert(name.into());
            return Err(format!("unsupported native struct {name}"));
        }
        if !self.has(name) {
            return Err(format!("struct {name} not in usmap"));
        }
        let mut m = Map::new();
        self.read_props(r, name, &mut m)?;
        if name == "GameplayTag" {
            return Ok(m.remove("TagName").unwrap_or(json!("None")));
        }
        Ok(Value::Object(m))
    }

    fn read_val(&self, r: &mut R, ty: &Ty, mode: Mode) -> Res<Value> {
        let z = mode == Mode::Zero;
        macro_rules! n {
            ($m:ident) => {
                if z { json!(0) } else { json!(r.$m()?) }
            };
        }
        Ok(match ty {
            Ty::Bool => json!(!z && r.u8()? != 0),
            Ty::Byte | Ty::Int8 if z => json!(0),
            Ty::Byte => json!(r.u8()?),
            Ty::Int8 => json!(r.u8()? as i8),
            Ty::Int16 => n!(i16),
            Ty::UInt16 => n!(u16),
            Ty::Int => n!(i32),
            Ty::UInt32 => n!(u32),
            Ty::Int64 => n!(i64),
            Ty::UInt64 => n!(u64),
            Ty::Float => if z { json!(0.0) } else { numf(r.f32()?) },
            Ty::Double => if z { json!(0.0) } else { num(r.f64()?) },
            Ty::Name if z => json!("None"),
            Ty::Name => json!(self.pkg.fname(r)?),
            Ty::Str | Ty::Utf8Str | Ty::AnsiStr => if z { json!("") } else { json!(r.fstring()?) },
            Ty::Text => if z { json!("") } else { self.text(r)? },
            Ty::Object | Ty::Interface | Ty::WeakObject => if z { Value::Null } else { self.obj(r)? },
            Ty::LazyObject => if z { Value::Null } else { json!(r.take(16)?.iter().map(|b| format!("{b:02X}")).collect::<String>()) },
            Ty::SoftObject => if z { json!("") } else { self.soft_path(r)? },
            Ty::Delegate => if z { Value::Null } else { json!({"object": self.obj(r)?, "function": self.pkg.fname(r)?}) },
            Ty::MulticastDelegate => {
                if z { return Ok(json!([])) }
                let n = r.i32()?;
                if n < 0 || n as usize > r.d.len() - r.p { return Err(format!("bad delegate count {n}")) }
                Value::Array((0..n).map(|_| Ok(json!({"object": self.obj(r)?, "function": self.pkg.fname(r)?}))).collect::<Res<_>>()?)
            }
            Ty::FieldPath => {
                if z { return Ok(Value::Null) }
                let n = r.i32()?;
                if n < 0 || n as usize > r.d.len() - r.p { return Err(format!("bad field path count {n}")) }
                let p: Vec<_> = (0..n).map(|_| self.pkg.fname(r)).collect::<Res<_>>()?;
                json!({"path": p, "owner": self.obj(r)?})
            }
            Ty::Enum(inner, en) => {
                let v = self.read_val(r, inner, if z { Mode::Zero } else { Mode::Normal })?;
                let k = v.as_u64().or(v.as_i64().map(|i| i as u64)).unwrap_or(0);
                let m = en.as_ref().and_then(|e| self.map.enums.get(e)?.get(&k));
                match (en, m) {
                    (Some(e), Some(m)) => json!(format!("{e}::{m}")),
                    (Some(e), None) => json!(format!("{e}::{k}")),
                    _ => v,
                }
            }
            Ty::Struct(s) => self.read_struct(r, s, mode)?,
            Ty::Array(inner) | Ty::Set(inner) => {
                if z { return Ok(json!([])) }
                if let Ty::Set(_) = ty { for _ in 0..r.i32()?.clamp(0, 1 << 20) { self.read_val(r, inner, Mode::Inner)?; } }
                let n = r.i32()?;
                if n < 0 || n as usize > r.d.len() - r.p { return Err(format!("bad element count {n}")) }
                Value::Array((0..n).map(|_| self.read_val(r, inner, Mode::Inner)).collect::<Res<_>>()?)
            }
            Ty::Map(k, v) => {
                if z { return Ok(json!({})) }
                for _ in 0..r.i32()?.clamp(0, 1 << 20) { self.read_val(r, k, Mode::Inner)?; }
                let n = r.i32()?;
                if n < 0 || n as usize > r.d.len() - r.p { return Err(format!("bad map count {n}")) }
                let ents: Vec<(Value, Value)> = (0..n).map(|_| Ok((self.read_val(r, k, Mode::Inner)?, self.read_val(r, v, Mode::Inner)?))).collect::<Res<_>>()?;
                let keyed: Option<Map<String, Value>> = ents.iter().map(|(k, v)| {
                    let s = match k { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), _ => return None };
                    Some((s, v.clone()))
                }).collect();
                match keyed {
                    Some(m) if m.len() == ents.len() => Value::Object(m),
                    _ => Value::Array(ents.into_iter().map(|(k, v)| json!({"key": k, "value": v})).collect()),
                }
            }
            Ty::Optional(inner) => {
                if z { return Ok(Value::Null) }
                if r.u8()? != 0 { self.read_val(r, inner, Mode::Inner)? } else { Value::Null }
            }
            Ty::Unknown(k) => return Err(format!("unknown usmap property type {k}")),
        })
    }

    fn text(&self, r: &mut R) -> Res<Value> {
        let flags = r.u32()?;
        let hist = r.u8()? as i8;
        match hist {
            -1 => Ok(if r.bool32()? { json!(r.fstring()?) } else { json!("") }),
            0 => {
                let (ns, key, src) = (r.fstring()?, r.fstring()?, r.fstring()?);
                Ok(json!({"namespace": ns, "key": key, "text": src, "flags": flags}))
            }
            11 => Ok(json!({"string_table": self.pkg.fname(r)?, "key": r.fstring()?})),
            h => Err(format!("unsupported FText history type {h}")),
        }
    }

    /// Resolve an export's class to a usmap struct name, walking class super chain through local exports.
    fn class_struct(&self, mut idx: i32) -> Option<String> {
        for _ in 0..16 {
            let n = self.pkg.obj_name(idx)?;
            if self.has(n) {
                return Some(n.to_string());
            }
            if idx <= 0 {
                return None;
            }
            idx = self.pkg.exports[idx as usize - 1].sup;
        }
        None
    }

    /// Dump one export to JSON: status ok | partial | skipped.
    pub fn export(&self, i: usize) -> Value {
        let e = &self.pkg.exports[i];
        let class_name = self.pkg.obj_name(e.class).unwrap_or("None").to_string();
        let mut o = Map::new();
        o.insert("index".into(), json!(i));
        o.insert("name".into(), json!(e.name));
        o.insert("class".into(), json!(self.pkg.path(e.class)));
        o.insert("outer".into(), json!(self.pkg.path(e.outer)));
        o.insert("cdo".into(), json!(e.flags & RF_CLASS_DEFAULT_OBJECT != 0));
        let skip = ["Class", "Function", "DelegateFunction", "ScriptStruct", "UserDefinedStruct", "Enum", "UserDefinedEnum", "MetaData"];
        if skip.contains(&class_name.as_str()) || class_name.ends_with("GeneratedClass") {
            o.insert("status".into(), json!("skipped"));
            o.insert("note".into(), json!("struct-like export (binary UStruct data not decoded)"));
            o.insert("extra_len".into(), json!(e.size));
            return Value::Object(o);
        }
        let mut props = Map::new();
        let res = self.export_body(i, &class_name, &mut props, &mut o);
        o.insert("properties".into(), Value::Object(props));
        match res {
            Ok(extra) => {
                o.insert("status".into(), json!("ok"));
                o.insert("extra_len".into(), json!(extra));
            }
            Err(err) => {
                o.insert("status".into(), json!("partial"));
                o.insert("error".into(), json!(err));
            }
        }
        Value::Object(o)
    }

    fn export_body(&self, i: usize, class_name: &str, props: &mut Map<String, Value>, o: &mut Map<String, Value>) -> Res<usize> {
        let e = &self.pkg.exports[i];
        let end = e.offset.checked_add(e.size).filter(|&x| x <= self.pkg.data.len()).ok_or("export range outside file")?;
        // Unknown (never-loaded) classes fall back to Object: fine if the export serialises no properties.
        let known = self.class_struct(e.class);
        let sname = known.clone().unwrap_or_else(|| "Object".into());
        o.insert("struct".into(), json!(sname));
        let mut r = R::new(&self.pkg.data[..end], e.offset);
        self.read_props(&mut r, &sname, props).map_err(|err| match known {
            Some(_) => err,
            None => format!("class {} not in usmap ({err})", self.pkg.path(e.class).unwrap_or_default()),
        })?;
        if e.flags & RF_CLASS_DEFAULT_OBJECT != 0 {
            if end - r.p >= 4 && r.i32()? != 0 {
                o.insert("sparse_class_data".into(), json!(true));
            }
        } else if end - r.p >= 4 && r.bool32()? {
            r.take(16)?; // object guid
        }
        if class_name == "DataTable" {
            let rs = props.get("RowStruct").and_then(Value::as_str).and_then(|s| s.rsplit('.').next()).ok_or("DataTable without RowStruct")?.to_string();
            let n = r.i32()?;
            let mut rows = Map::new();
            for _ in 0..n {
                let name = self.pkg.fname(&mut r)?;
                rows.insert(name, self.read_struct(&mut r, &rs, Mode::Normal)?);
            }
            props.insert("_rows".into(), Value::Object(rows));
        }
        Ok(end - r.p)
    }
}

/// f32 as the shortest decimal that round-trips (35.0, not 35.000000357).
fn numf(x: f32) -> Value {
    num(x.to_string().parse().unwrap_or(x as f64))
}
