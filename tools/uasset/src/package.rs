//! FPackageFileSummary, name/import/export maps for cooked UE 5.5 legacy packages.
use crate::{R, Res};

pub struct Import {
    pub class: String,
    pub outer: i32,
    pub name: String,
}

pub struct Export {
    pub class: i32,
    pub sup: i32,
    pub outer: i32,
    pub name: String,
    pub flags: u32,
    pub size: usize,
    pub offset: usize,
}

pub struct Package {
    pub pkg_name: String,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// .uasset followed by .uexp; export offsets index straight into this.
    pub data: Vec<u8>,
}

pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;

struct Sum {
    pkg_name: String,
    name: (usize, usize),
    exports: (usize, usize),
    imports: (usize, usize),
    end: usize,
    name_offset: usize,
}

/// Parse the summary. `cells`/`meta` toggle the UE 5.5 optional header blocks (VERSE_CELLS, METADATA_SERIALIZATION_OFFSET);
/// the combination is chosen by checking the cursor lands exactly on the name table.
fn summary(d: &[u8], cells: bool, meta: bool) -> Res<Sum> {
    let mut r = R::new(d, 0);
    if r.u32()? != 0x9E2A83C1 {
        return Err("bad package magic".into());
    }
    let legacy = r.i32()?;
    if legacy >= -7 || legacy < -9 {
        return Err(format!("unsupported legacy file version {legacy}"));
    }
    r.i32()?; // ue3
    let (v4, v5, lic) = (r.i32()?, r.i32()?, r.i32()?);
    if (v4, v5, lic) != (0, 0, 0) {
        return Err(format!("versioned package (ue4={v4} ue5={v5}); only unversioned supported"));
    }
    let n = r.i32()?;
    r.take(n as usize * 20)?; // custom versions
    r.i32()?; // total header size
    let pkg_name = r.fstring()?;
    let flags = r.u32()?;
    if flags & 0x2000 == 0 {
        return Err("package does not use unversioned properties".into());
    }
    let filter_editor = flags & 0x8000_0000 != 0;
    let (nc, no) = (r.i32()? as usize, r.i32()? as usize);
    r.take(8)?; // soft object paths count/offset
    if !filter_editor {
        r.fstring()?; // localization id
    }
    r.take(8)?; // gatherable text
    let (ec, eo) = (r.i32()? as usize, r.i32()? as usize);
    let (ic, io) = (r.i32()? as usize, r.i32()? as usize);
    if cells {
        r.take(16)?;
    }
    if meta {
        r.take(4)?;
    }
    r.take(4)?; // depends offset
    r.take(8)?; // soft package refs
    r.take(4)?; // searchable names
    r.take(4)?; // thumbnail table
    r.take(16)?; // guid
    if !filter_editor {
        r.take(16)?; // persistent guid
    }
    let gens = r.i32()?;
    r.take(gens as usize * 8)?;
    for _ in 0..2 {
        r.take(10)?; // engine version
        r.fstring()?;
    }
    r.take(4)?; // compression flags
    if r.i32()? != 0 {
        return Err("compressed chunks unsupported".into());
    }
    r.take(4)?; // package source
    for _ in 0..r.i32()? {
        r.fstring()?;
    }
    r.take(4)?; // asset registry offset
    r.take(8)?; // bulk data start
    r.take(4)?; // world tile info
    let chunks = r.i32()?;
    r.take(chunks as usize * 4)?;
    r.take(8)?; // preload deps
    r.take(4)?; // names referenced from export data
    r.take(8)?; // payload toc
    r.take(4)?; // data resource offset
    Ok(Sum { pkg_name, name: (nc, no), exports: (ec, eo), imports: (ic, io), end: r.p, name_offset: no })
}

impl Package {
    pub fn load(path: &std::path::Path) -> Res<Package> {
        let mut data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let uexp = path.with_extension("uexp");
        if uexp.exists() {
            data.extend(std::fs::read(&uexp).map_err(|e| e.to_string())?);
        }
        Self::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Res<Package> {
        let mut last = String::new();
        let mut sum = None;
        for (c, m) in [(true, true), (false, true), (false, false)] {
            match summary(&data, c, m) {
                Ok(s) if s.end == s.name_offset => {
                    sum = Some(s);
                    break;
                }
                Ok(s) => last = format!("summary ends at {} but name table at {}", s.end, s.name_offset),
                Err(e) => last = e,
            }
        }
        let s = sum.ok_or(last)?;
        let mut r = R::new(&data, s.name.1);
        let mut names = Vec::with_capacity(s.name.0);
        for _ in 0..s.name.0 {
            names.push(r.fstring()?);
            r.take(4)?; // case-preserving/non-case hashes
        }
        let fname = |r: &mut R| -> Res<String> {
            let (i, n) = (r.i32()?, r.i32()?);
            let b = names.get(i as usize).ok_or_else(|| format!("name index {i} out of range"))?;
            Ok(if n > 0 { format!("{b}_{}", n - 1) } else { b.clone() })
        };
        let mut r = R::new(&data, s.imports.1);
        let mut imports = vec![];
        for _ in 0..s.imports.0 {
            fname(&mut r)?; // class package
            let class = fname(&mut r)?;
            let outer = r.i32()?;
            let name = fname(&mut r)?;
            r.take(4)?; // optional flag
            imports.push(Import { class, outer, name });
        }
        let mut r = R::new(&data, s.exports.1);
        let mut exports = vec![];
        for _ in 0..s.exports.0 {
            let (class, sup, _template, outer) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            let name = fname(&mut r)?;
            let flags = r.u32()?;
            let (size, offset) = (r.i64()? as usize, r.i64()? as usize);
            r.take(12 + 4 + 4 + 12 + 20)?; // forced/notfor*, inherited, package flags, 3 bools, 5 preload deps
            exports.push(Export { class, sup, outer, name, flags, size, offset });
        }
        Ok(Package { pkg_name: s.pkg_name, names, imports, exports, data })
    }

    pub fn fname(&self, r: &mut R) -> Res<String> {
        let (i, n) = (r.i32()?, r.i32()?);
        if r.zero {
            return Ok("None".into());
        }
        let b = self.names.get(i as usize).ok_or_else(|| format!("name index {i} out of range"))?;
        Ok(if n > 0 { format!("{b}_{}", n - 1) } else { b.clone() })
    }

    /// Object name of a package index (None for null / out of range).
    pub fn obj_name(&self, idx: i32) -> Option<&str> {
        match idx {
            0 => None,
            i if i > 0 => self.exports.get(i as usize - 1).map(|e| e.name.as_str()),
            i => self.imports.get((-i) as usize - 1).map(|e| e.name.as_str()),
        }
    }

    /// Full path like `/Script/Engine.Class` or `<this package>.Outer.Name`.
    pub fn path(&self, idx: i32) -> Option<String> {
        let mut parts = vec![];
        let mut cur = idx;
        for _ in 0..32 {
            let (name, outer) = match cur {
                0 => return Some(self.pkg_name.clone() + &Self::join(&parts)),
                i if i > 0 => self.exports.get(i as usize - 1).map(|e| (&e.name, e.outer))?,
                i => self.imports.get((-i) as usize - 1).map(|e| (&e.name, e.outer))?,
            };
            if cur < 0 && outer == 0 {
                return Some(name.clone() + &Self::join(&parts));
            }
            parts.push(name.as_str());
            cur = outer;
        }
        None
    }

    fn join(parts: &[&str]) -> String {
        parts.iter().rev().map(|p| format!(".{p}")).collect()
    }
}
