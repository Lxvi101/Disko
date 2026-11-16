//! Directory listing with `getattrlistbulk(2)`: one system call returns names and metadata for a
//! whole batch of entries, instead of a `readdir` plus an `lstat` per entry. On a home folder with a
//! few million files this roughly halves the scan.

use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

const ATTR_CMN_NAME: u32 = 0x0000_0001;
const ATTR_CMN_DEVID: u32 = 0x0000_0002;
const ATTR_CMN_OBJTYPE: u32 = 0x0000_0008;
const ATTR_CMN_MODTIME: u32 = 0x0000_0400;
const ATTR_CMN_CHGTIME: u32 = 0x0000_0800;
const ATTR_CMN_ACCESSMASK: u32 = 0x0002_0000;
const ATTR_CMN_FILEID: u32 = 0x0200_0000;
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const ATTR_CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
const ATTR_DIR_MOUNTSTATUS: u32 = 0x0000_0004;
const DIR_MNTSTATUS_MNTPOINT: u32 = 0x1;
const ATTR_FILE_LINKCOUNT: u32 = 0x0000_0001;
const ATTR_FILE_ALLOCSIZE: u32 = 0x0000_0004;
const ATTR_FILE_DATALENGTH: u32 = 0x0000_0200;
const FSOPT_NOFOLLOW: u64 = 0x1;
const FSOPT_PACK_INVAL_ATTRS: u64 = 0x8;

const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;

const BUF: usize = 256 * 1024;

pub struct Ent {
    pub name: String,
    /// 0 dir, 1 file, 2 symlink, 3 special, 4 unknown (matches the scanner's kinds)
    pub kind: u8,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub size: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub allocated: u64,
    pub nlink: u64,
    pub error: Option<String>,
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

#[inline]
fn ts(buf: &[u8], at: &mut usize) -> i64 {
    let s: i64 = rd(buf, at);
    let ns: i64 = rd(buf, at);
    s * 1_000_000_000 + ns
}

/// List `dir` without following symlinks. Errors mean the directory itself could not be read (or the
/// volume does not support bulk listing); per-entry problems come back in `Ent::error`.
pub fn list(dir: &Path) -> io::Result<Vec<Ent>> {
    let c = CString::new(dir.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let fd = Fd(fd);

    let mut req: libc::attrlist = unsafe { std::mem::zeroed() };
    req.bitmapcount = 5;
    req.commonattr = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_DEVID | ATTR_CMN_OBJTYPE | ATTR_CMN_MODTIME | ATTR_CMN_CHGTIME | ATTR_CMN_ACCESSMASK | ATTR_CMN_FILEID | ATTR_CMN_ERROR;
    req.dirattr = ATTR_DIR_MOUNTSTATUS;
    req.fileattr = ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE | ATTR_FILE_DATALENGTH;

    BUFFER.with(|cell| {
        let mut buf = cell.borrow_mut();
        let mut out = Vec::new();
        loop {
            let n = unsafe { libc::getattrlistbulk(fd.0, &mut req as *mut _ as *mut libc::c_void, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), FSOPT_NOFOLLOW | FSOPT_PACK_INVAL_ATTRS) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n == 0 {
                return Ok(out);
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
                let mtime_ns = ts(&buf, &mut at);
                let ctime_ns = ts(&buf, &mut at);
                let access: u32 = rd(&buf, &mut at);
                let ino: u64 = rd(&buf, &mut at);
                // Directory and file groups are only packed for objects of that type.
                let mount: u32 = if objtype == VDIR { rd(&buf, &mut at) } else { 0 };
                let (nlink, alloc, data): (u32, i64, i64) = if objtype != VDIR {
                    (rd(&buf, &mut at), rd(&buf, &mut at), rd(&buf, &mut at))
                } else {
                    (1, 0, 0)
                };

                let start = name_at + name_ref.attr_dataoffset as usize;
                let end = start + (name_ref.attr_length as usize).saturating_sub(1); // drop the NUL
                let name = String::from_utf8_lossy(&buf[start..end.min(buf.len())]).into_owned();

                let (kind, fmt) = match objtype {
                    VDIR => (0u8, libc::S_IFDIR as u32),
                    VREG => (1, libc::S_IFREG as u32),
                    VLNK => (2, libc::S_IFLNK as u32),
                    0 => (4, 0),
                    _ => (3, 0),
                };
                let dev = if kind == 0 && returned.dirattr & ATTR_DIR_MOUNTSTATUS != 0 && mount & DIR_MNTSTATUS_MNTPOINT != 0 {
                        // A mount point reports the covered volume; lstat sees the mounted one.
                        use std::os::unix::fs::MetadataExt;
                        std::fs::symlink_metadata(dir.join(&name)).map(|m| m.dev()).unwrap_or(u64::MAX)
                    } else {
                        dev as u32 as u64
                    };
                let file_attrs = kind != 0 && returned.fileattr & ATTR_FILE_ALLOCSIZE != 0;
                out.push(Ent {
                    name,
                    kind: if err != 0 && objtype == 0 { 4 } else { kind },
                    dev,
                    ino,
                    mode: fmt | (access & 0o7777),
                    size: if file_attrs { data.max(0) as u64 } else { 0 },
                    mtime_ns,
                    ctime_ns,
                    allocated: if file_attrs { alloc.max(0) as u64 } else { 0 },
                    nlink: if file_attrs { nlink as u64 } else { 1 },
                    error: (err != 0).then(|| io::Error::from_raw_os_error(err as i32).to_string()),
                });
                base += len as usize;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn matches_lstat() {
        let dir = std::env::temp_dir().join(format!("disko-bulk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![7u8; 100_000]).unwrap();
        std::fs::write(dir.join("ünïcode name.txt"), b"hi").unwrap();
        std::os::unix::fs::symlink("a.bin", dir.join("link")).unwrap();
        std::fs::hard_link(dir.join("a.bin"), dir.join("hard")).unwrap();

        let mut got = list(&dir).unwrap();
        got.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.bin", "hard", "link", "sub", "ünïcode name.txt"]);
        for e in &got {
            let m = std::fs::symlink_metadata(dir.join(&e.name)).unwrap();
            assert_eq!(e.ino, m.ino(), "{}", e.name);
            assert_eq!(e.dev, m.dev() as u64, "{}", e.name);
            assert_eq!(e.mtime_ns, m.mtime() * 1_000_000_000 + m.mtime_nsec(), "{}", e.name);
            assert_eq!(e.mode, m.mode() as u32, "{}", e.name);
            assert!(e.error.is_none());
            if e.kind != 0 {
                assert_eq!(e.size, m.size(), "{}", e.name);
                assert_eq!(e.allocated, m.blocks() * 512, "{}", e.name);
                assert_eq!(e.nlink, m.nlink(), "{}", e.name);
            }
        }
        assert_eq!(got[0].kind, 1);
        assert_eq!(got[2].kind, 2);
        assert_eq!(got[3].kind, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn large_directory_spans_several_calls() {
        let dir = std::env::temp_dir().join(format!("disko-bulk-many-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5000 {
            std::fs::write(dir.join(format!("file-with-a-fairly-long-name-{:05}.txt", i)), b"x").unwrap();
        }
        assert_eq!(list(&dir).unwrap().len(), 5000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
