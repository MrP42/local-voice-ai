//! Nur die Speaker-Projektion aus GGUF v2/v3; kein Modell- oder Inferenz-Loader.
use super::*;
use std::io::{Read, Seek, SeekFrom};

const WEIGHT: &str = "flow_lm.condition_provider.conditioners.speaker_wavs.output_proj.weight";

fn bad() -> VoiceError {
    VoiceError::InvalidFile("Phonon-Speaker-Projektion ungültig oder nicht unterstützt.".into())
}

struct Gguf<'a, R> { r: &'a mut R, len: u64 }

impl<R: Read + Seek> Gguf<'_, R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], VoiceError> {
        let mut b = [0; N];
        self.r.read_exact(&mut b).map_err(|_| bad())?;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, VoiceError> { Ok(u32::from_le_bytes(self.bytes()?)) }
    fn u64(&mut self) -> Result<u64, VoiceError> { Ok(u64::from_le_bytes(self.bytes()?)) }
    fn pos(&mut self) -> Result<u64, VoiceError> { self.r.stream_position().map_err(|_| bad()) }
    fn skip(&mut self, n: u64) -> Result<(), VoiceError> {
        let end = self.pos()?.checked_add(n).filter(|v| *v <= self.len).ok_or_else(bad)?;
        self.r.seek(SeekFrom::Start(end)).map_err(|_| bad())?;
        Ok(())
    }
    fn string(&mut self) -> Result<String, VoiceError> {
        let n = self.u64()?;
        if n > 1024 * 1024 || self.pos()?.checked_add(n).is_none_or(|v| v > self.len) { return Err(bad()); }
        let mut b = vec![0; n as usize];
        self.r.read_exact(&mut b).map_err(|_| bad())?;
        String::from_utf8(b).map_err(|_| bad())
    }
    fn value(&mut self, typ: u32, depth: u32) -> Result<(), VoiceError> {
        match typ {
            0 | 1 | 7 => self.skip(1),
            2 | 3 => self.skip(2),
            4 | 5 | 6 => self.skip(4),
            10 | 11 | 12 => self.skip(8),
            8 => { self.string()?; Ok(()) }
            9 if depth < 4 => {
                let typ = self.u32()?;
                let n = self.u64()?;
                if n > 1_000_000 { return Err(bad()); }
                for _ in 0..n { self.value(typ, depth + 1)?; }
                Ok(())
            }
            _ => Err(bad()),
        }
    }
}

pub(super) fn load_weight(path: &Path) -> Result<Vec<f32>, VoiceError> {
    let mut f = fs::File::open(path).map_err(|_| bad())?;
    let len = f.metadata().map_err(|_| bad())?.len();
    read_weight(&mut f, len)
}

fn read_weight<R: Read + Seek>(r: &mut R, len: u64) -> Result<Vec<f32>, VoiceError> {
    let mut g = Gguf { r, len };
    if &g.bytes::<4>()? != b"GGUF" || !matches!(g.u32()?, 2 | 3) { return Err(bad()); }
    let tensors = g.u64()?;
    let meta = g.u64()?;
    if tensors > 100_000 || meta > 100_000 { return Err(bad()); }
    let mut alignment = 32u64;
    for _ in 0..meta {
        let key = g.string()?;
        let typ = g.u32()?;
        if key == "general.alignment" {
            if typ != 4 { return Err(bad()); }
            alignment = u64::from(g.u32()?);
            if alignment == 0 || !alignment.is_power_of_two() || alignment > 4096 { return Err(bad()); }
        } else { g.value(typ, 0)?; }
        if g.pos()? > 64 * 1024 * 1024 { return Err(bad()); }
    }
    let mut target = None;
    let mut offsets = Vec::new();
    for _ in 0..tensors {
        let name = g.string()?;
        let n = g.u32()?;
        if !(1..=4).contains(&n) { return Err(bad()); }
        let mut shape = Vec::new();
        for _ in 0..n { shape.push(g.u64()?); }
        let typ = g.u32()?;
        let offset = g.u64()?;
        if offset % alignment != 0 { return Err(bad()); }
        offsets.push(offset);
        if name == WEIGHT {
            if target.is_some() || shape != [512, 512] || !matches!(typ, 0 | 30) { return Err(bad()); }
            target = Some((offset, typ));
        }
        if g.pos()? > 64 * 1024 * 1024 { return Err(bad()); }
    }
    let (offset, typ) = target.ok_or_else(bad)?;
    let start = g.pos()?.checked_add(alignment - 1).ok_or_else(bad)? / alignment * alignment;
    let size = 512 * 512 * if typ == 30 { 2 } else { 4 };
    let end = offset.checked_add(size).ok_or_else(bad)?;
    if start.checked_add(end).is_none_or(|v| v > len)
        || offsets.iter().filter(|&&o| o == offset).count() != 1
        || offsets.iter().any(|&o| o > offset && o < end)
        || offsets.iter().any(|&o| start.checked_add(o).is_none_or(|v| v >= len)) { return Err(bad()); }
    g.r.seek(SeekFrom::Start(start + offset)).map_err(|_| bad())?;
    let mut w = Vec::with_capacity(512 * 512);
    for _ in 0..512 * 512 {
        let v = if typ == 30 { f32::from_bits(u32::from(u16::from_le_bytes(g.bytes()?)) << 16) }
            else { f32::from_le_bytes(g.bytes()?) };
        if !v.is_finite() { return Err(bad()); }
        w.push(v);
    }
    Ok(w)
}

fn project(w: &[f32], x: &[f32], c: usize, t: usize) -> Result<Vec<f32>, VoiceError> {
    if c == 0 || !(1..=1024).contains(&t) || c.checked_mul(c) != Some(w.len())
        || c.checked_mul(t) != Some(x.len()) { return Err(bad()); }
    let mut out = vec![0.; t * c];
    for time in 0..t {
        for o in 0..c {
            let mut sum = 0f32;
            for i in 0..c { sum += w[o * c + i] * x[i * t + time]; }
            if !sum.is_finite() { return Err(bad()); }
            out[time * c + o] = sum;
        }
    }
    Ok(out)
}

pub(super) fn encode(bytes: &[u8], w: &[f32]) -> Result<Vec<u8>, VoiceError> {
    validate_voice_bytes(bytes)?;
    let n = u64::from_le_bytes(bytes[..8].try_into().map_err(|_| bad())?) as usize;
    let h: Value = serde_json::from_slice(&bytes[8..8+n]).map_err(|_| bad())?;
    let tensor = &h["speaker_wavs"];
    let t = tensor["shape"][2].as_u64().filter(|v| (1..=1024).contains(v)).ok_or_else(bad)? as usize;
    if tensor["dtype"] != "F32" { return Err(bad()); }
    let offset = tensor["data_offsets"][0].as_u64().ok_or_else(bad)? as usize;
    let data = bytes.get(8+n+offset..8+n+offset+t*512*4).ok_or_else(bad)?;
    let x = data.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
    let out = project(w, &x, 512, t)?;
    // Standard-Paddung: bei T=125 genau 72 Kopf- und 256000 Datenbytes.
    let h = format!(r#"{{"emb":{{"dtype":"F32","shape":[1,{t},512],"data_offsets":[0,{}]}}}}"#, out.len()*4);
    let mut h = h.into_bytes();
    while h.len() % 8 != 0 { h.push(b' '); }
    let mut bytes = (h.len() as u64).to_le_bytes().to_vec();
    bytes.extend(h);
    for v in out { bytes.extend(v.to_le_bytes()); }
    Ok(bytes)
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
