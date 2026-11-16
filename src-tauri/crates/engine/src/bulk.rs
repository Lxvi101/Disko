//! Directory listing with `getattrlistbulk(2)`: one system call returns names and metadata for a
//! whole batch of entries, instead of a `readdir` plus an `lstat` per entry.
//!
//! Adapted from 0.6's `bulkdir.rs`, trimmed to the attributes the engine needs and without a
//! `String` per entry: names land in one reusable buffer.

use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

const ATTR_CMN_NAME: u32 = 0x0000_0001;
const ATTR_CMN_DEVID: u32 = 0x0000_0002;
const ATTR_CMN_OBJTYPE: u32 = 0x0000_0008;
const ATTR_CMN_MODTIME: u32 = 0x0000_0400;
const ATTR_CMN_FILEID: u32 = 0x0200_0000;
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const ATTR_CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
const ATTR_DIR_MOUNTSTATUS: u32 = 0x0000_0004;
const DIR_MNTSTATUS_MNTPOINT: u32 = 0x1;
const ATTR_FILE_LINKCOUNT: u32 = 0x0000_0001;
const ATTR_FILE_ALLOCSIZE: u32 = 0x0000_0004;
const FSOPT_NOFOLLOW: u64 = 0x1;
const FSOPT_PACK_INVAL_ATTRS: u64 = 0x8;
const FSOPT_ATTR_CMN_EXTENDED: u64 = 0x20;
const ATTR_CMNEXT_CLONEID: u32 = 0x0000_0100;
const ATTR_CMNEXT_EXT_FLAGS: u32 = 0x0000_0200;
/// `ext_flags`: the file may share blocks with another file (an APFS clone).
#[allow(dead_code)]
pub const EF_MAY_SHARE_BLOCKS: u64 = 0x1;
/// `ext_flags`: the file shares all of its blocks with another file (a pure clone).
pub const EF_SHARES_ALL_BLOCKS: u64 = 0x40;

const VREG: u32 = 1;
const VDIR: u32 = 2;

const BUF: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    /// Symlinks, sockets, devices …: counted as files with their (tiny) allocation.
    Other,
}

impl Ent {
    /// A file with full clones (same `clone_id`, all blocks shared): count the family's data once.
    #[inline]
    pub fn is_clone(&self) -> bool {
        self.ext_flags & EF_SHARES_ALL_BLOCKS != 0 && self.clone_id != 0 && self.alloc > 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ent {
    name_at: u32,
    name_len: u16,
    pub kind: Kind,
    /// Mount point (the reported `dev` is then the covered volume's; lstat to learn the real one).
    pub mount: bool,
    pub dev: u64,
    pub ino: u64,
    pub mtime: i64,
    pub alloc: u64,
    pub nlink: u32,
    /// APFS clone bookkeeping: the data stream id (equal for pure clones) and EF_* flags.
    pub clone_id: u64,
    pub ext_flags: u64,
    /// Per-entry error (errno); such entries carry no usable metadata.
    pub err: i32,
}

/// One directory's listing. Reused between calls to avoid allocation.
#[derive(Default)]
pub struct Listing {
    pub names: Vec<u8>,
    pub ents: Vec<Ent>,
}

impl Listing {
    pub fn clear(&mut self) {
        self.names.clear();
        self.ents.clear();
    }
    #[inline]
    pub fn name(&self, e: &Ent) -> &str {
        let b = &self.names[e.name_at as usize..e.name_at as usize + e.name_len as usize];
        // Names were validated (or lossily converted) when stored.
        unsafe { std::str::from_utf8_unchecked(b) }
    }
}

thread_local! {
    static BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; BUF]);
}

#[inline]
fn rd<T: Copy>(buf: &[u8], at: &mut usize) -> T {
    let n = std::mem::size_of::<T>();
    let v = unsafe { std::ptr::read_unaligned(buf[*at..*at + n].as_ptr() as *const T) };
    *at += n;
    v
}

struct Fd(i32);
impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

/// List `dir` without following symlinks into `out` (cleared first). An error means the directory
/// itself could not be read; per-entry problems come back in `Ent::err`.
pub fn list(dir: &Path, out: &mut Listing) -> io::Result<()> {
    out.clear();
    let c = CString::new(dir.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = Fd(fd);

    let mut req: libc::attrlist = unsafe { std::mem::zeroed() };
    req.bitmapcount = 5;
    req.commonattr = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_DEVID | ATTR_CMN_OBJTYPE | ATTR_CMN_MODTIME | ATTR_CMN_FILEID | ATTR_CMN_ERROR;
    req.dirattr = ATTR_DIR_MOUNTSTATUS;
    req.fileattr = ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE;
    // With FSOPT_ATTR_CMN_EXTENDED the fork group carries extended common attributes (packed in
    // the fork group's place, after the file group, in bit order).
    // No ATTR_CMNEXT_PRIVATESIZE: the kernel computes it by walking extents (seconds for a big
    // sparse disk image) and pure clones share all their blocks anyway.
    req.forkattr = ATTR_CMNEXT_CLONEID | ATTR_CMNEXT_EXT_FLAGS;

    let res = BUFFER.with(|cell| {
        let mut buf = cell.borrow_mut();
        loop {
            let n = unsafe {
                libc::getattrlistbulk(fd.0, &mut req as *mut _ as *mut libc::c_void, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), FSOPT_NOFOLLOW | FSOPT_PACK_INVAL_ATTRS | FSOPT_ATTR_CMN_EXTENDED)
            };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n == 0 {
                return Ok(());
            }
            let mut base = 0usize;
            for _ in 0..n {
                let mut at = base;
                let len: u32 = rd(&buf, &mut at);
                let returned: libc::attribute_set_t = rd(&buf, &mut at);
                // ATTR_CMN_ERROR is packed straight after the returned set, not in bit order.
                let err: u32 = rd(&buf, &mut at);
                let name_at = at;
                let name_ref: libc::attrreference_t = rd(&buf, &mut at);
                let dev: i32 = rd(&buf, &mut at);
                let objtype: u32 = rd(&buf, &mut at);
                let mtime_s: i64 = rd(&buf, &mut at);
                let _mtime_ns: i64 = rd(&buf, &mut at);
                let ino: u64 = rd(&buf, &mut at);
                // Directory and file groups are only packed for objects of that type.
                let mount: u32 = if objtype == VDIR { rd(&buf, &mut at) } else { 0 };
                let (nlink, alloc): (u32, i64) = if objtype != VDIR { (rd(&buf, &mut at), rd(&buf, &mut at)) } else { (1, 0) };
                // Extended common attributes take the fork group's place, after the file group.
                let clone_id: u64 = rd(&buf, &mut at);
                let ext_flags: u64 = rd(&buf, &mut at);
                let ext_ok = returned.forkattr & ATTR_CMNEXT_EXT_FLAGS != 0 && returned.forkattr & ATTR_CMNEXT_CLONEID != 0;

                let start = name_at + name_ref.attr_dataoffset as usize;
                let end = (start + (name_ref.attr_length as usize).saturating_sub(1)).min(buf.len()); // drop the NUL
                let raw = &buf[start..end];
                let at_name = out.names.len() as u32;
                match std::str::from_utf8(raw) {
                    Ok(s) => out.names.extend_from_slice(s.as_bytes()),
                    Err(_) => out.names.extend_from_slice(String::from_utf8_lossy(raw).as_bytes()),
                }
                let name_len = (out.names.len() as u32 - at_name) as u16;

                let kind = match objtype {
                    VDIR => Kind::Dir,
                    VREG => Kind::File,
                    _ => Kind::Other,
                };
                let file_attrs = kind != Kind::Dir && returned.fileattr & ATTR_FILE_ALLOCSIZE != 0;
                let mount = kind == Kind::Dir && returned.dirattr & ATTR_DIR_MOUNTSTATUS != 0 && mount & DIR_MNTSTATUS_MNTPOINT != 0;
                out.ents.push(Ent {
                    name_at: at_name,
                    name_len,
                    kind,
                    mount,
                    dev: dev as u32 as u64,
                    ino,
                    mtime: mtime_s,
                    alloc: if file_attrs { alloc.max(0) as u64 } else { 0 },
                    nlink: if file_attrs { nlink } else { 1 },
                    clone_id: if ext_ok { clone_id } else { 0 },
                    ext_flags: if ext_ok { ext_flags } else { 0 },
                    err: err as i32,
                });
                base += len as usize;
            }
        }
    });
    match res {
        // Volumes without bulk listing (some network and FUSE file systems): readdir + lstat.
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EINVAL) | Some(libc::ENOSYS)) && out.ents.is_empty() => {
            list_slow(dir, out)
        }
        r => r,
    }
}

fn list_slow(dir: &Path, out: &mut Listing) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    out.clear();
    for ent in std::fs::read_dir(dir)?.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        let at = out.names.len() as u32;
        out.names.extend_from_slice(name.as_bytes());
        let len = name.len() as u16;
        match std::fs::symlink_metadata(ent.path()) {
            Ok(m) => {
                let ft = m.file_type();
                let kind = if ft.is_dir() { Kind::Dir } else if ft.is_file() { Kind::File } else { Kind::Other };
                let _ = ft.is_socket();
                out.ents.push(Ent {
                    name_at: at,
                    name_len: len,
                    kind,
                    mount: false,
                    dev: m.dev() as u64,
                    ino: m.ino(),
                    mtime: m.mtime(),
                    alloc: if kind == Kind::Dir { 0 } else { m.blocks() * 512 },
                    nlink: if kind == Kind::Dir { 1 } else { m.nlink() as u32 },
                    clone_id: 0,
                    ext_flags: 0,
                    err: 0,
                });
            }
            Err(e) => out.ents.push(Ent { name_at: at, name_len: len, kind: Kind::Other, mount: false, dev: 0, ino: 0, mtime: 0, alloc: 0, nlink: 1, clone_id: 0, ext_flags: 0, err: e.raw_os_error().unwrap_or(libc::EIO) }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn matches_lstat() {
        let dir = crate::testutil::tmp("bulk");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![7u8; 100_000]).unwrap();
        std::fs::write(dir.join("ünïcode name.txt"), b"hi").unwrap();
        std::os::unix::fs::symlink("a.bin", dir.join("link")).unwrap();
        std::fs::hard_link(dir.join("a.bin"), dir.join("hard")).unwrap();

        let mut l = Listing::default();
        list(&dir, &mut l).unwrap();
        let mut got: Vec<(String, Ent)> = l.ents.iter().map(|e| (l.name(e).to_string(), *e)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let names: Vec<&str> = got.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(names, ["a.bin", "hard", "link", "sub", "ünïcode name.txt"]);
        for (n, e) in &got {
            let m = std::fs::symlink_metadata(dir.join(n)).unwrap();
            assert_eq!(e.ino, m.ino(), "{}", n);
            assert_eq!(e.dev, m.dev() as u64, "{}", n);
            assert_eq!(e.mtime, m.mtime(), "{}", n);
            assert_eq!(e.err, 0);
            if e.kind != Kind::Dir {
                assert_eq!(e.alloc, m.blocks() * 512, "{}", n);
                assert_eq!(e.nlink as u64, m.nlink(), "{}", n);
            }
        }
        assert_eq!(got[0].1.kind, Kind::File);
        assert_eq!(got[2].1.kind, Kind::Other);
        assert_eq!(got[3].1.kind, Kind::Dir);
        assert!(!got[3].1.mount);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clones_are_recognisable() {
        let dir = crate::testutil::tmp("bulk-clone");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("orig.bin"), &data).unwrap();
        let st = std::process::Command::new("/bin/cp").arg("-c").arg(dir.join("orig.bin")).arg(dir.join("clone.bin")).status().unwrap();
        assert!(st.success());
        std::fs::write(dir.join("plain.bin"), &data).unwrap();
        let mut l = Listing::default();
        list(&dir, &mut l).unwrap();
        let get = |n: &str| *l.ents.iter().find(|e| l.name(e) == n).unwrap();
        let (o, c, p) = (get("orig.bin"), get("clone.bin"), get("plain.bin"));
        for (n, e) in [("orig.bin", o), ("clone.bin", c), ("plain.bin", p)] {
            let m = std::fs::symlink_metadata(dir.join(n)).unwrap();
            assert_eq!(e.alloc, m.blocks() * 512, "{}", n);
            assert_eq!(e.nlink as u64, m.nlink(), "{}", n);
            assert_eq!(e.ino, m.ino(), "{}", n);
        }
        assert!(o.ext_flags & EF_MAY_SHARE_BLOCKS != 0, "{:?}", o);
        assert!(c.ext_flags & EF_MAY_SHARE_BLOCKS != 0);
        assert_eq!(p.ext_flags & EF_MAY_SHARE_BLOCKS, 0);
        assert_eq!(o.clone_id, c.clone_id);
        assert_ne!(o.clone_id, p.clone_id);
        eprintln!("orig {:?}\nclone {:?}\nplain {:?}", o, c, p);
        assert!(o.is_clone() && c.is_clone() && !p.is_clone());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn large_directory_spans_several_calls() {
        let dir = crate::testutil::tmp("bulk-many");
        for i in 0..3000 {
            std::fs::write(dir.join(format!("file-with-a-fairly-long-name-{:05}.txt", i)), b"x").unwrap();
        }
        let mut l = Listing::default();
        list(&dir, &mut l).unwrap();
        assert_eq!(l.ents.len(), 3000);
        assert!(l.ents.iter().all(|e| l.name(e).starts_with("file-with")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
