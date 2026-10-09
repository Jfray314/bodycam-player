//! Reader for cooked UE 5.5 legacy packages (.uasset + .uexp) with unversioned properties.
pub mod package;
pub mod props;
pub mod usmap;

pub type Res<T> = Result<T, String>;

/// Little-endian cursor over a byte slice. `zero` makes FName/object reads yield defaults (zero-masked structs).
pub struct R<'a> {
    pub d: &'a [u8],
    pub p: usize,
    pub zero: bool,
}

macro_rules! num {
    ($n:ident, $t:ty) => {
        pub fn $n(&mut self) -> Res<$t> {
            Ok(<$t>::from_le_bytes(self.take(std::mem::size_of::<$t>())?.try_into().unwrap()))
        }
    };
}

impl<'a> R<'a> {
    pub fn new(d: &'a [u8], p: usize) -> Self {
        R { d, p, zero: false }
    }
    pub fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let e = self.p.checked_add(n).filter(|&e| e <= self.d.len()).ok_or_else(|| format!("read of {n} bytes past end at {}", self.p))?;
        let s = &self.d[self.p..e];
        self.p = e;
        Ok(s)
    }
    num!(u8, u8);
    num!(u16, u16);
    num!(i16, i16);
    num!(u32, u32);
    num!(i32, i32);
    num!(u64, u64);
    num!(i64, i64);
    num!(f32, f32);
    num!(f64, f64);
    pub fn bool32(&mut self) -> Res<bool> {
        Ok(self.u32()? != 0)
    }
    pub fn fstring(&mut self) -> Res<String> {
        let n = self.i32()?;
        if n == 0 {
            return Ok(String::new());
        }
        if n < 0 {
            let n = n.checked_neg().ok_or("bad fstring")? as usize;
            let b = self.take(n * 2)?;
            let u: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Ok(String::from_utf16_lossy(&u[..n - 1]));
        }
        let b = self.take(n as usize)?;
        Ok(String::from_utf8_lossy(&b[..n as usize - 1]).into_owned())
    }
}
