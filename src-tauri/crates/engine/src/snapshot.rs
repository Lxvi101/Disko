//! The one snapshot file: the tree in compact preorder, column-chunked and zstd-compressed,
//! written atomically (temp file + rename).
//!
//! Layout (all inside one zstd frame after the 8-byte magic):
//! header (version, root, created, event id, device UUID, min_file, complete, node count),
//! then chunks of up to 64Ki nodes, each as columns (parent delta, flags, error, category,
//! name lengths, names, own bytes, own files, own newest), then matcher tags and hard links.
//! Subtree totals are not stored; they are recomputed on load.

use crate::tree::*;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"DISKOSN\x01";
const VERSION: u32 = 1;
const CHUNK: usize = 1 << 16;

fn put_var(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_var(buf: &[u8], at: &mut usize) -> io::Result<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let b = *buf.get(*at).ok_or_else(bad)?;
        *at += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err(bad());
        }
    }
}

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "corrupt snapshot")
}

fn write_block(w: &mut impl Write, b: &[u8]) -> io::Result<()> {
    let mut len = Vec::with_capacity(10);
    put_var(&mut len, b.len() as u64);
    w.write_all(&len)?;
    w.write_all(b)
}

fn read_var(r: &mut impl Read) -> io::Result<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        v |= ((b[0] & 0x7f) as u64) << shift;
        if b[0] & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err(bad());
        }
    }
}

fn read_block(r: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
    let n = read_var(r)? as usize;
    if n > max {
        return Err(bad());
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_var(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn get_str(buf: &[u8], at: &mut usize) -> io::Result<String> {
    let n = get_var(buf, at)? as usize;
    let s = buf.get(*at..*at + n).ok_or_else(bad)?;
    *at += n;
    String::from_utf8(s.to_vec()).map_err(|_| bad())
}

/// Write `t` to `path` atomically. Returns the file size.
pub fn save(t: &Tree, path: &Path) -> io::Result<u64> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("snapshot"), std::process::id()));
    let res = (|| {
        let f = std::fs::File::create(&tmp)?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        w.write_all(MAGIC)?;
        let mut z = zstd::stream::write::Encoder::new(w, 3)?;
        write_body(t, &mut z)?;
        let mut w = z.finish()?;
        w.flush()?;
        let f = w.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        let size = f.metadata()?.len();
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(size)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn write_body(t: &Tree, w: &mut impl Write) -> io::Result<()> {
    let order = t.preorder();
    let mut map = vec![NONE; t.len()];
    for (new, &old) in order.iter().enumerate() {
        map[old as usize] = new as u32;
    }
    let mut h = Vec::new();
    put_var(&mut h, VERSION as u64);
    put_str(&mut h, &t.root);
    put_var(&mut h, t.created);
    put_var(&mut h, t.event_id);
    put_str(&mut h, &t.uuid);
    put_var(&mut h, t.min_file);
    h.push(t.complete as u8);
    put_var(&mut h, order.len() as u64);
    write_block(w, &h)?;

    let mut cols: [Vec<u8>; 9] = Default::default();
    for (ci, chunk) in order.chunks(CHUNK).enumerate() {
        for c in cols.iter_mut() {
            c.clear();
        }
        for (k, &old) in chunk.iter().enumerate() {
            let new = (ci * CHUNK + k) as u64;
            let o = old as usize;
            let p = if old == 0 { new } else { map[t.parent[o] as usize] as u64 };
            put_var(&mut cols[0], new - p);
            cols[1].push(t.flags[o] & !F_DEAD);
            cols[2].push(t.err[o]);
            cols[3].push(t.cat[o]);
            let name = if old == 0 { "" } else { t.name(old) };
            put_var(&mut cols[4], name.len() as u64);
            cols[5].extend_from_slice(name.as_bytes());
            put_var(&mut cols[6], t.own_bytes[o]);
            put_var(&mut cols[7], t.own_files[o] as u64);
            cols[8].extend_from_slice(&t.own_newest[o].to_le_bytes());
        }
        for c in &cols {
            write_block(w, c)?;
        }
    }
    let mut tail = Vec::new();
    let tags: Vec<(u32, u32)> = t.tags.iter().filter(|(k, _)| map[**k as usize] != NONE).map(|(k, v)| (map[*k as usize], *v)).collect();
    put_var(&mut tail, tags.len() as u64);
    for (k, v) in tags {
        put_var(&mut tail, k as u64);
        put_var(&mut tail, v as u64);
    }
    let links: Vec<(&(u64, u64), u32)> = t.links.iter().filter(|(_, v)| map[**v as usize] != NONE).map(|(k, v)| (k, map[*v as usize])).collect();
    put_var(&mut tail, links.len() as u64);
    for ((dev, ino), owner) in links {
        put_var(&mut tail, *dev);
        put_var(&mut tail, *ino);
        put_var(&mut tail, owner as u64);
    }
    tail.extend_from_slice(MAGIC);
    write_block(w, &tail)
}

pub fn load(path: &Path) -> io::Result<Tree> {
    let f = std::fs::File::open(path)?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad());
    }
    let mut z = zstd::stream::read::Decoder::with_buffer(r)?;
    let h = read_block(&mut z, 1 << 20)?;
    let mut at = 0;
    if get_var(&h, &mut at)? != VERSION as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot version"));
    }
    let root = get_str(&h, &mut at)?;
    let created = get_var(&h, &mut at)?;
    let event_id = get_var(&h, &mut at)?;
    let uuid = get_str(&h, &mut at)?;
    let min_file = get_var(&h, &mut at)?;
    let complete = *h.get(at).ok_or_else(bad)? != 0;
    at += 1;
    let n = get_var(&h, &mut at)? as usize;
    if n == 0 || n > u32::MAX as usize - 1 {
        return Err(bad());
    }
    let mut t = Tree::new(&root, min_file);
    t.created = created;
    t.event_id = event_id;
    t.uuid = uuid;
    t.complete = complete;
    t.reserve(n);
    let mut done = 0usize;
    while done < n {
        let m = (n - done).min(CHUNK);
        let max = 64 << 20;
        let cols: Vec<Vec<u8>> = (0..9).map(|_| read_block(&mut z, max)).collect::<io::Result<_>>()?;
        if cols[1].len() != m || cols[2].len() != m || cols[3].len() != m || cols[8].len() != m * 4 {
            return Err(bad());
        }
        let (mut a0, mut a4, mut a5, mut a6, mut a7) = (0, 0, 0, 0, 0);
        for k in 0..m {
            let new = (done + k) as u64;
            let delta = get_var(&cols[0], &mut a0)?;
            let len = get_var(&cols[4], &mut a4)? as usize;
            let name = std::str::from_utf8(cols[5].get(a5..a5 + len).ok_or_else(bad)?).map_err(|_| bad())?;
            a5 += len;
            let ob = get_var(&cols[6], &mut a6)?;
            let of = get_var(&cols[7], &mut a7)? as u32;
            let on = u32::from_le_bytes(cols[8][k * 4..k * 4 + 4].try_into().unwrap());
            let parent = if new == 0 {
                NONE
            } else {
                if delta == 0 || delta > new {
                    return Err(bad());
                }
                (new - delta) as u32
            };
            if parent != NONE && !t.is_dir(parent) {
                return Err(bad());
            }
            t.load_node(parent, name, cols[1][k], cols[2][k], cols[3][k], (ob, of, on));
        }
        done += m;
    }
    let tail = read_block(&mut z, 1 << 30)?;
    let mut at = 0;
    let nt = get_var(&tail, &mut at)?;
    for _ in 0..nt {
        let k = get_var(&tail, &mut at)? as u32;
        let v = get_var(&tail, &mut at)? as u32;
        if (k as usize) < t.len() {
            t.tags.insert(k, v);
        }
    }
    let nl = get_var(&tail, &mut at)?;
    t.links.reserve(nl as usize);
    for _ in 0..nl {
        let dev = get_var(&tail, &mut at)?;
        let ino = get_var(&tail, &mut at)?;
        let o = get_var(&tail, &mut at)? as u32;
        if (o as usize) < t.len() {
            t.links.insert((dev, ino), o);
        }
    }
    if tail.get(at..at + 8) != Some(&MAGIC[..]) {
        return Err(bad());
    }
    t.recompute_totals();
    Ok(t)
}
