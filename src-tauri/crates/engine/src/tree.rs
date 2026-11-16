//! The in-memory disk tree: struct-of-arrays, one entry per directory and per file of at least
//! `min_file` bytes. Smaller files are folded into their folder's own aggregate (`own_*`).
//!
//! Index 0 is the root. Children form a singly linked list (`first` → `next`); removed nodes are
//! unlinked and flagged dead, and their slots are dropped when the tree is compacted (snapshot
//! save/load).

use disko_core::{Category, Matched, TreeNode};
use std::collections::HashMap;

pub const NONE: u32 = u32::MAX;

pub const F_DIR: u8 = 1;
/// A kept (big) file.
pub const F_FILE: u8 = 2;
pub const F_DEAD: u8 = 4;
/// The matcher tagged this node (tag in `Tree::tags`).
pub const F_MATCHED: u8 = 8;

pub const E_NONE: u8 = 0;
pub const E_FDA: u8 = 1;
pub const E_PERM: u8 = 2;
pub const E_PROTECTED: u8 = 3;
pub const E_READ: u8 = 4;

pub const NO_CAT: u8 = u8::MAX;

pub fn err_text(e: u8) -> Option<&'static str> {
    match e {
        E_NONE => None,
        E_FDA => Some("Needs Full Disk Access"),
        E_PERM => Some("Permission denied"),
        E_PROTECTED => Some("Protected by macOS"),
        _ => Some("Couldn't read"),
    }
}

pub fn err_code(errno: i32, fda: bool) -> u8 {
    match errno {
        libc::EPERM if !fda => E_FDA,
        libc::EPERM => E_PROTECTED,
        libc::EACCES => E_PERM,
        _ => E_READ,
    }
}

pub fn cat_code(c: Category) -> u8 {
    Category::ALL.iter().position(|x| *x == c).map(|p| p as u8).unwrap_or(NO_CAT)
}

pub fn cat_of(code: u8) -> Option<Category> {
    Category::ALL.get(code as usize).copied()
}

/// Seconds since the epoch as stored (u32, 0 = unknown).
#[inline]
pub fn secs(t: i64) -> u32 {
    t.clamp(0, u32::MAX as i64) as u32
}

/// Views fold children smaller than this share of the view root into one "other" child.
pub const VIEW_MIN_SHARE: u64 = 1000;
/// … and never show more than this many children per folder.
pub const VIEW_MAX_CHILDREN: usize = 40;

#[derive(Default)]
pub struct Tree {
    /// Absolute path of the root ("/" for the whole disk).
    pub root: String,
    names: Vec<u8>,
    name_at: Vec<u32>,
    name_len: Vec<u16>,
    pub parent: Vec<u32>,
    first: Vec<u32>,
    next: Vec<u32>,
    pub flags: Vec<u8>,
    pub err: Vec<u8>,
    pub cat: Vec<u8>,
    /// Subtree totals.
    pub bytes: Vec<u64>,
    pub files: Vec<u32>,
    pub newest: Vec<u32>,
    /// This node's own share: for a folder its folded small files, for a kept file itself.
    pub own_bytes: Vec<u64>,
    pub own_files: Vec<u32>,
    pub own_newest: Vec<u32>,
    /// Matcher tag per matched node.
    pub tags: HashMap<u32, u32>,
    /// Hard-linked files (nlink > 1) → the folder whose listing counted them.
    pub links: HashMap<(u64, u64), u32>,
    /// Walk bookkeeping: listings + subfolders still outstanding per folder (0 = complete).
    pending: Vec<u32>,
    pub dead: usize,
    /// FSEvents id taken right before the walk (or of the last applied change).
    pub event_id: u64,
    /// FSEvents UUID of the scanned device when `event_id` was taken.
    pub uuid: String,
    /// Unix seconds when the data was taken.
    pub created: u64,
    pub min_file: u64,
    /// The whole walk finished (not a partial or cancelled tree).
    pub complete: bool,
}

impl Tree {
    pub fn new(root: &str, min_file: u64) -> Tree {
        let mut t = Tree { root: root.to_string(), min_file, ..Default::default() };
        t.push(NONE, root, F_DIR);
        t
    }

    pub fn reserve(&mut self, n: usize) {
        self.name_at.reserve(n);
        self.name_len.reserve(n);
        self.parent.reserve(n);
        self.first.reserve(n);
        self.next.reserve(n);
        self.flags.reserve(n);
        self.err.reserve(n);
        self.cat.reserve(n);
        self.bytes.reserve(n);
        self.files.reserve(n);
        self.newest.reserve(n);
        self.own_bytes.reserve(n);
        self.own_files.reserve(n);
        self.own_newest.reserve(n);
        self.names.reserve(n * 16);
    }

    pub fn is_empty(&self) -> bool {
        self.parent.len() <= 1 && !self.complete
    }

    /// Slots, including dead ones.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn live(&self) -> usize {
        self.parent.len() - self.dead
    }

    /// Approximate heap bytes held.
    pub fn heap_bytes(&self) -> usize {
        self.names.capacity()
            + self.name_at.capacity() * 4
            + self.name_len.capacity() * 2
            + (self.parent.capacity() + self.first.capacity() + self.next.capacity()) * 4
            + (self.flags.capacity() + self.err.capacity() + self.cat.capacity())
            + (self.bytes.capacity() + self.own_bytes.capacity()) * 8
            + (self.files.capacity() + self.newest.capacity() + self.own_files.capacity() + self.own_newest.capacity() + self.pending.capacity()) * 4
            + self.tags.capacity() * 12
            + self.links.capacity() * 24
    }

    /// Append a node without linking it to a parent.
    fn push(&mut self, parent: u32, name: &str, flags: u8) -> u32 {
        let idx = self.parent.len() as u32;
        self.name_at.push(self.names.len() as u32);
        let n = name.len().min(u16::MAX as usize);
        self.names.extend_from_slice(&name.as_bytes()[..n]);
        self.name_len.push(n as u16);
        self.parent.push(parent);
        self.first.push(NONE);
        self.next.push(NONE);
        self.flags.push(flags);
        self.err.push(E_NONE);
        self.cat.push(NO_CAT);
        self.bytes.push(0);
        self.files.push(0);
        self.newest.push(0);
        self.own_bytes.push(0);
        self.own_files.push(0);
        self.own_newest.push(0);
        idx
    }

    /// Add a child (totals are left at zero; use `add_up`).
    pub fn add_child(&mut self, parent: u32, name: &str, flags: u8) -> u32 {
        let idx = self.push(parent, name, flags);
        self.next[idx as usize] = self.first[parent as usize];
        self.first[parent as usize] = idx;
        idx
    }

    #[inline]
    pub fn name(&self, i: u32) -> &str {
        let at = self.name_at[i as usize] as usize;
        let b = &self.names[at..at + self.name_len[i as usize] as usize];
        unsafe { std::str::from_utf8_unchecked(b) }
    }

    #[inline]
    pub fn is_dir(&self, i: u32) -> bool {
        self.flags[i as usize] & F_DIR != 0
    }

    #[inline]
    pub fn is_dead(&self, i: u32) -> bool {
        self.flags[i as usize] & F_DEAD != 0
    }

    pub fn children(&self, i: u32) -> Children<'_> {
        Children { t: self, at: self.first[i as usize] }
    }

    pub fn path(&self, i: u32) -> String {
        let mut parts: Vec<u32> = Vec::new();
        let mut at = i;
        while at != 0 && at != NONE {
            parts.push(at);
            at = self.parent[at as usize];
        }
        let mut s = self.root.clone();
        for p in parts.iter().rev() {
            if !s.ends_with('/') {
                s.push('/');
            }
            s.push_str(self.name(*p));
        }
        s
    }

    pub fn child_named(&self, i: u32, name: &str) -> Option<u32> {
        self.children(i).find(|&c| self.name(c) == name)
    }

    /// Node at `path` (must be the root or below it).
    pub fn lookup(&self, path: &str) -> Option<u32> {
        let rest = strip_root(&self.root, path)?;
        let mut at = 0u32;
        for comp in rest.split('/').filter(|c| !c.is_empty()) {
            at = self.child_named(at, comp)?;
        }
        Some(at)
    }

    /// Deepest existing node on the way to `path` (for paths the tree doesn't hold yet).
    pub fn nearest(&self, path: &str) -> Option<u32> {
        let rest = strip_root(&self.root, path)?;
        let mut at = 0u32;
        for comp in rest.split('/').filter(|c| !c.is_empty()) {
            match self.child_named(at, comp) {
                Some(c) => at = c,
                None => break,
            }
        }
        Some(at)
    }

    /// Add to a node's totals and every ancestor's.
    pub fn add_up(&mut self, i: u32, dbytes: i64, dfiles: i64, newest: u32) {
        let mut at = i;
        while at != NONE {
            let a = at as usize;
            self.bytes[a] = (self.bytes[a] as i64 + dbytes).max(0) as u64;
            self.files[a] = (self.files[a] as i64 + dfiles).max(0) as u32;
            if newest > self.newest[a] {
                self.newest[a] = newest;
            }
            at = self.parent[a];
        }
    }

    /// Recompute `newest` of `i` and its ancestors after something below shrank.
    pub fn renew_up(&mut self, i: u32) {
        let mut at = i;
        while at != NONE {
            let mut n = self.own_newest[at as usize];
            for c in self.children(at) {
                n = n.max(self.newest[c as usize]);
            }
            if n == self.newest[at as usize] && at != i {
                break;
            }
            self.newest[at as usize] = n;
            at = self.parent[at as usize];
        }
    }

    /// Drop a node and everything below it, fixing ancestor totals. Returns the matched nodes that
    /// went away (paths).
    pub fn remove(&mut self, i: u32) -> Vec<String> {
        if i == 0 || self.is_dead(i) {
            return Vec::new();
        }
        let p = self.parent[i as usize];
        let (b, f, n) = (self.bytes[i as usize], self.files[i as usize], self.newest[i as usize]);
        // Unlink.
        let mut prev = NONE;
        let mut at = self.first[p as usize];
        while at != NONE && at != i {
            prev = at;
            at = self.next[at as usize];
        }
        if at == i {
            if prev == NONE {
                self.first[p as usize] = self.next[i as usize];
            } else {
                self.next[prev as usize] = self.next[i as usize];
            }
        }
        let mut gone = Vec::new();
        let mut stack = vec![i];
        let mut dead_matched = Vec::new();
        while let Some(x) = stack.pop() {
            if self.flags[x as usize] & F_MATCHED != 0 {
                dead_matched.push(x);
            }
            let mut c = self.first[x as usize];
            while c != NONE {
                stack.push(c);
                c = self.next[c as usize];
            }
        }
        for m in &dead_matched {
            gone.push(self.path(*m));
        }
        let mut stack = vec![i];
        while let Some(x) = stack.pop() {
            self.flags[x as usize] |= F_DEAD;
            self.dead += 1;
            self.tags.remove(&x);
            if (x as usize) < self.pending.len() {
                self.pending[x as usize] = 0;
            }
            let mut c = self.first[x as usize];
            while c != NONE {
                stack.push(c);
                c = self.next[c as usize];
            }
        }
        self.add_up(p, -(b as i64), -(f as i64), 0);
        if n >= self.newest[p as usize] {
            self.renew_up(p);
        }
        gone
    }

    pub fn set_pending(&mut self, i: u32, v: u32) {
        if self.pending.len() <= i as usize {
            self.pending.resize((i as usize + 1).max(self.pending.len() * 2), 0);
        }
        self.pending[i as usize] = v;
    }

    pub fn add_pending(&mut self, i: u32, v: u32) {
        let cur = self.pending.get(i as usize).copied().unwrap_or(0);
        self.set_pending(i, cur + v);
    }

    /// One outstanding item of `i` is done; bubble completion up. Matched folders that just
    /// completed are appended to `out`.
    pub fn finish(&mut self, i: u32, out: &mut Vec<Matched>) {
        let mut at = i;
        loop {
            let p = self.pending.get(at as usize).copied().unwrap_or(0);
            if p == 0 {
                return;
            }
            self.pending[at as usize] = p - 1;
            if p > 1 {
                return;
            }
            if self.flags[at as usize] & F_MATCHED != 0 && !self.is_dead(at) {
                out.push(self.matched(at));
            }
            at = self.parent[at as usize];
            if at == NONE {
                return;
            }
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending = Vec::new();
    }

    pub fn matched(&self, i: u32) -> Matched {
        Matched {
            tag: self.tags.get(&i).copied().unwrap_or(0),
            path: self.path(i),
            bytes: self.bytes[i as usize],
            files: self.files[i as usize] as u64,
            newest: (self.newest[i as usize] > 0).then_some(self.newest[i as usize] as i64),
        }
    }

    /// Nearest matched node at or above `i`.
    pub fn matched_ancestor(&self, i: u32) -> Option<u32> {
        let mut at = i;
        while at != NONE {
            if self.flags[at as usize] & F_MATCHED != 0 {
                return Some(at);
            }
            at = self.parent[at as usize];
        }
        None
    }

    /// Category inherited from the nearest categorised ancestor (not `i` itself).
    pub fn inherited_cat(&self, i: u32) -> u8 {
        let mut at = self.parent.get(i as usize).copied().unwrap_or(NONE);
        while at != NONE {
            if self.cat[at as usize] != NO_CAT {
                return self.cat[at as usize];
            }
            at = self.parent[at as usize];
        }
        NO_CAT
    }

    fn base_view(&self, i: u32, inherited: u8) -> TreeNode {
        let u = i as usize;
        let c = if self.cat[u] != NO_CAT { self.cat[u] } else { inherited };
        TreeNode {
            name: self.name(i).to_string(),
            path: self.path(i),
            kind: if self.flags[u] & F_DIR != 0 { "dir" } else { "file" }.into(),
            bytes: self.bytes[u],
            files: self.files[u] as u64,
            newest: (self.newest[u] > 0).then_some(self.newest[u] as i64),
            clutter: cat_of(c),
            error: err_text(self.err[u]).map(String::from),
            children: None,
        }
    }

    /// A flat node (no children).
    pub fn flat(&self, i: u32) -> TreeNode {
        self.base_view(i, self.inherited_cat(i))
    }

    /// `i` with `depth` levels of children, largest first, small ones folded into "other".
    pub fn view(&self, i: u32, depth: u32) -> TreeNode {
        let min = (self.bytes[i as usize] / VIEW_MIN_SHARE).max(1);
        self.view_rec(i, depth, self.inherited_cat(i), min, VIEW_MAX_CHILDREN)
    }

    /// `i` with all its children (one level, nothing folded except the small-file aggregate).
    pub fn list(&self, i: u32) -> TreeNode {
        self.view_rec(i, 1, self.inherited_cat(i), 0, usize::MAX)
    }

    fn view_rec(&self, i: u32, depth: u32, inherited: u8, min: u64, max: usize) -> TreeNode {
        let mut node = self.base_view(i, inherited);
        if depth == 0 || !self.is_dir(i) {
            return node;
        }
        let here = if self.cat[i as usize] != NO_CAT { self.cat[i as usize] } else { inherited };
        let mut kids: Vec<u32> = self.children(i).collect();
        kids.sort_unstable_by(|a, b| self.bytes[*b as usize].cmp(&self.bytes[*a as usize]).then_with(|| self.name(*a).cmp(self.name(*b))));
        let mut out = Vec::new();
        let (mut ob, mut of, mut on, mut oc) = (self.own_bytes[i as usize], self.own_files[i as usize] as u64, self.own_newest[i as usize], self.own_files[i as usize] as u64);
        for k in kids {
            let b = self.bytes[k as usize];
            if out.len() < max && (b >= min && b > 0 || (min == 0 && max == usize::MAX)) {
                out.push(self.view_rec(k, depth - 1, here, min, max));
            } else {
                ob += b;
                of += self.files[k as usize] as u64;
                on = on.max(self.newest[k as usize]);
                oc += 1;
            }
        }
        if oc > 0 {
            out.push(TreeNode {
                name: if oc == 1 { "1 small item".into() } else { format!("{} small items", oc) },
                path: node.path.clone(),
                kind: "other".into(),
                bytes: ob,
                files: of,
                newest: (on > 0).then_some(on as i64),
                clutter: cat_of(here),
                error: None,
                children: None,
            });
        }
        node.children = Some(out);
        node
    }

    /// Live nodes under `i` (including `i`), depth-first.
    pub fn subtree(&self, i: u32) -> Subtree<'_> {
        Subtree { t: self, stack: vec![i] }
    }

    /// Nodes in a compact preorder: `order[new] = old`. Parents come before children.
    pub fn preorder(&self) -> Vec<u32> {
        let mut order = Vec::with_capacity(self.live());
        let mut stack = vec![0u32];
        while let Some(x) = stack.pop() {
            order.push(x);
            let mut c = self.first[x as usize];
            while c != NONE {
                stack.push(c);
                c = self.next[c as usize];
            }
        }
        order
    }

    /// Rebuild without dead slots (and without wasted arena).
    pub fn compact(&mut self) {
        let order = self.preorder();
        let mut map = vec![NONE; self.len()];
        let mut t = Tree { root: self.root.clone(), min_file: self.min_file, event_id: self.event_id, uuid: self.uuid.clone(), created: self.created, complete: self.complete, ..Default::default() };
        t.reserve(order.len());
        for &old in &order {
            let o = old as usize;
            let p = if old == 0 { NONE } else { map[self.parent[o] as usize] };
            let new = if old == 0 { t.push(NONE, &self.root, self.flags[o]) } else { t.add_child(p, self.name(old), self.flags[o]) };
            map[o] = new;
            let n = new as usize;
            t.err[n] = self.err[o];
            t.cat[n] = self.cat[o];
            t.bytes[n] = self.bytes[o];
            t.files[n] = self.files[o];
            t.newest[n] = self.newest[o];
            t.own_bytes[n] = self.own_bytes[o];
            t.own_files[n] = self.own_files[o];
            t.own_newest[n] = self.own_newest[o];
        }
        for (k, v) in &self.tags {
            if map[*k as usize] != NONE {
                t.tags.insert(map[*k as usize], *v);
            }
        }
        for (k, v) in &self.links {
            if map[*v as usize] != NONE {
                t.links.insert(*k, map[*v as usize]);
            }
        }
        *self = t;
    }

    /// Recompute subtree totals from the `own_*` values (after loading).
    pub fn recompute_totals(&mut self) {
        // Preorder from load means parent < child, so one reverse pass suffices.
        for i in 0..self.len() {
            self.bytes[i] = self.own_bytes[i];
            self.files[i] = self.own_files[i];
            self.newest[i] = self.own_newest[i];
        }
        for i in (1..self.len()).rev() {
            if self.flags[i] & F_DEAD != 0 {
                continue;
            }
            let p = self.parent[i] as usize;
            debug_assert!(p < i);
            self.bytes[p] += self.bytes[i];
            self.files[p] += self.files[i];
            self.newest[p] = self.newest[p].max(self.newest[i]);
        }
    }

    /// Low-level append used by the snapshot loader (parent must already exist).
    pub fn load_node(&mut self, parent: u32, name: &str, flags: u8, err: u8, cat: u8, own: (u64, u32, u32)) -> u32 {
        let i = if parent == NONE {
            debug_assert!(self.len() == 1);
            0
        } else {
            self.add_child(parent, name, flags)
        };
        let u = i as usize;
        self.flags[u] = flags;
        self.err[u] = err;
        self.cat[u] = cat;
        self.own_bytes[u] = own.0;
        self.own_files[u] = own.1;
        self.own_newest[u] = own.2;
        i
    }
}

pub struct Children<'a> {
    t: &'a Tree,
    at: u32,
}

impl Iterator for Children<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        while self.at != NONE {
            let c = self.at;
            self.at = self.t.next[c as usize];
            if self.t.flags[c as usize] & F_DEAD == 0 {
                return Some(c);
            }
        }
        None
    }
}

pub struct Subtree<'a> {
    t: &'a Tree,
    stack: Vec<u32>,
}

impl Iterator for Subtree<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let x = self.stack.pop()?;
        let mut c = self.t.first[x as usize];
        while c != NONE {
            if self.t.flags[c as usize] & F_DEAD == 0 {
                self.stack.push(c);
            }
            c = self.t.next[c as usize];
        }
        Some(x)
    }
}

/// The part of `path` below `root` ("" for the root itself), or None when outside.
pub fn strip_root<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    if root == "/" {
        return path.strip_prefix('/');
    }
    let rest = path.strip_prefix(root)?;
    if rest.is_empty() || rest.starts_with('/') {
        Some(rest)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new("/r", 1 << 20);
        let a = t.add_child(0, "a", F_DIR);
        let b = t.add_child(a, "b", F_DIR);
        let f = t.add_child(b, "big.bin", F_FILE);
        t.own_bytes[f as usize] = 5 << 20;
        t.own_files[f as usize] = 1;
        t.own_newest[f as usize] = 100;
        t.add_up(f, 5 << 20, 1, 100);
        t.own_bytes[b as usize] = 1000;
        t.own_files[b as usize] = 3;
        t.own_newest[b as usize] = 200;
        t.add_up(b, 1000, 3, 200);
        let c = t.add_child(0, "c", F_DIR);
        t.own_bytes[c as usize] = 10;
        t.own_files[c as usize] = 1;
        t.own_newest[c as usize] = 50;
        t.add_up(c, 10, 1, 50);
        t
    }

    #[test]
    fn paths_and_lookup() {
        let t = sample();
        let f = t.lookup("/r/a/b/big.bin").unwrap();
        assert_eq!(t.path(f), "/r/a/b/big.bin");
        assert_eq!(t.lookup("/r"), Some(0));
        assert_eq!(t.lookup("/r/"), Some(0));
        assert_eq!(t.lookup("/rx/a"), None);
        assert_eq!(t.lookup("/r/a/zz"), None);
        assert_eq!(t.nearest("/r/a/zz/q"), t.lookup("/r/a"));
        assert_eq!(t.bytes[0], (5 << 20) + 1010);
        assert_eq!(t.files[0], 5);
        assert_eq!(t.newest[0], 200);
    }

    #[test]
    fn remove_fixes_ancestors() {
        let mut t = sample();
        let b = t.lookup("/r/a/b").unwrap();
        t.remove(b);
        assert_eq!(t.bytes[0], 10);
        assert_eq!(t.files[0], 1);
        assert_eq!(t.newest[0], 50);
        assert_eq!(t.lookup("/r/a/b"), None);
        assert_eq!(t.bytes[t.lookup("/r/a").unwrap() as usize], 0);
        t.compact();
        assert_eq!(t.len(), 3);
        assert_eq!(t.bytes[0], 10);
        assert!(t.lookup("/r/c").is_some());
    }

    #[test]
    fn view_folds_small_children() {
        let t = sample();
        let v = t.view(0, 2);
        let kids = v.children.unwrap();
        assert_eq!(kids[0].name, "a");
        // "c" (10 bytes) is below 1/1000 of the root → folded.
        assert_eq!(kids.last().unwrap().kind, "other");
        assert_eq!(kids.iter().map(|k| k.bytes).sum::<u64>(), v.bytes);
        let a = &kids[0].children.as_ref().unwrap()[0];
        assert_eq!(a.name, "b");
        assert!(a.children.is_none());
        let l = t.list(0);
        assert_eq!(l.children.unwrap().len(), 2);
    }

    #[test]
    fn root_slash() {
        let mut t = Tree::new("/", 1);
        let u = t.add_child(0, "Users", F_DIR);
        assert_eq!(t.path(u), "/Users");
        assert_eq!(t.lookup("/Users"), Some(u));
        assert_eq!(t.lookup("/"), Some(0));
        assert_eq!(t.path(0), "/");
    }
}
