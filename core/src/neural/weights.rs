//! `ASNW` weight files exported by `ml/asmr_ml/export.py`.
//!
//! ```text
//! "ASNW" | version u32 | count u32 | count x { name_len u16, name, dtype u8 (0 f32, 1 f16),
//!                                              ndim u8, dims u32 x ndim, data (LE, row-major) }
//! ```
//! Configuration lives in the same file as named tensors (`cfg.*`), so the engine never
//! hard-codes model sizes. Parsing happens once, off the audio thread.

use std::fmt;

pub const VERSION: u32 = 1;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "asnw: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

pub struct Tensor {
    pub name: String,
    pub dims: Vec<usize>,
    pub data: Vec<f32>,
}

pub struct Weights {
    tensors: Vec<Tensor>,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.pos + n > self.b.len() {
            return err("truncated file");
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

/// IEEE half -> f32 (no `half` crate: we stay dependency-free).
fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match exp {
        0 if man == 0 => sign,
        0 => {
            // Subnormal: renormalise.
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        31 => sign | 0x7f80_0000 | (man << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

impl Weights {
    /// In-memory weights (used when composing a base model with a species adapter).
    pub fn from_tensors(tensors: Vec<Tensor>) -> Self {
        Self { tensors }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tensors.iter().map(|t| t.name.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.tensors.iter().any(|t| t.name == name)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader { b: bytes, pos: 0 };
        if r.take(4)? != b"ASNW" {
            return err("bad magic");
        }
        let version = r.u32()?;
        if version != VERSION {
            return err(format!("version {version}, engine reads {VERSION}"));
        }
        let count = r.u32()? as usize;
        let mut tensors = Vec::with_capacity(count);
        for _ in 0..count {
            let n = r.u16()? as usize;
            let name = String::from_utf8(r.take(n)?.to_vec()).map_err(|_| Error("bad name".into()))?;
            let dtype = r.u8()?;
            let ndim = r.u8()? as usize;
            let mut dims = Vec::with_capacity(ndim);
            for _ in 0..ndim {
                dims.push(r.u32()? as usize);
            }
            let len: usize = dims.iter().product();
            let data = match dtype {
                0 => r.take(len * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
                1 => r
                    .take(len * 2)?
                    .chunks_exact(2)
                    .map(|c| f16_to_f32(u16::from_le_bytes(c.try_into().unwrap())))
                    .collect(),
                d => return err(format!("{name}: dtype {d}")),
            };
            tensors.push(Tensor { name, dims, data });
        }
        if r.pos != bytes.len() {
            return err("trailing bytes");
        }
        Ok(Self { tensors })
    }

    pub fn tensor(&self, name: &str) -> Result<&Tensor, Error> {
        match self.tensors.iter().find(|t| t.name == name) {
            Some(t) => Ok(t),
            None => err(format!("missing tensor {name}")),
        }
    }

    /// Data of `name`, checking its shape (`usize::MAX` in `dims` matches any size).
    pub fn get(&self, name: &str, dims: &[usize]) -> Result<&[f32], Error> {
        let t = self.tensor(name)?;
        let ok = t.dims.len() == dims.len() && t.dims.iter().zip(dims).all(|(a, b)| *b == usize::MAX || a == b);
        if !ok {
            return err(format!("{name}: shape {:?}, expected {:?}", t.dims, dims));
        }
        Ok(&t.data)
    }

    pub fn scalar(&self, name: &str) -> Result<f32, Error> {
        let t = self.tensor(name)?;
        if t.data.len() != 1 {
            return err(format!("{name}: not a scalar"));
        }
        Ok(t.data[0])
    }

    pub fn dims(&self, name: &str) -> Result<&[usize], Error> {
        Ok(&self.tensor(name)?.dims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!((f16_to_f32(0x0001) - 5.960_464_5e-8).abs() < 1e-12);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
    }

    #[test]
    fn parse_roundtrip() {
        let mut b = b"ASNW".to_vec();
        b.extend(VERSION.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(3u16.to_le_bytes());
        b.extend(b"w.x");
        b.push(0);
        b.push(2);
        b.extend(1u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        b.extend(1.5f32.to_le_bytes());
        b.extend((-3.0f32).to_le_bytes());
        let w = Weights::parse(&b).unwrap();
        assert_eq!(w.get("w.x", &[1, 2]).unwrap(), &[1.5, -3.0]);
        assert!(w.get("w.x", &[2, 1]).is_err());
        assert!(Weights::parse(&b[..b.len() - 1]).is_err());
    }
}
