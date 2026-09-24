import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import time

SCHEMA = 1

def absolute(path):
    return os.path.abspath(os.path.expanduser(str(path)))

def inside(path, root):
    return path == root or path.startswith(root.rstrip('/') + '/')

def classify(path, home=None):
    home = absolute(home or Path.home())
    p = Path(path)
    if not inside(path, home):
        return 'protected', 'Outside the current user home; audit only.'
    rel = p.relative_to(home)
    parts = rel.parts
    if not parts or any(x in parts for x in ('.ssh', '.gnupg')) or parts[0] == '.Trash':
        return 'protected', 'Home, credentials, or existing Trash; preserve.'
    if any(x in parts for x in ('.git', '.svn')):
        return 'keep', 'Version control history; use repository-aware maintenance.'
    if parts[:2] in [('Library', 'CloudStorage'), ('Library', 'Mobile Documents')]:
        return 'protected', 'Cloud-managed content; deletion may sync to other devices.'
    if any(x.endswith(('.photoslibrary', '.photolibrary', '.musiclibrary')) for x in parts):
        return 'keep', 'Managed media library; use its owning application.'
    rules = [
        (('Library', 'Caches'), 'rebuildable', 'App cache; quit the owning app first. May contain offline data.'),
        (('Library', 'Developer', 'Xcode', 'DerivedData'), 'rebuildable', 'Xcode build output and indexes; close Xcode, then rebuild as needed.'),
        (('Library', 'Developer', 'Xcode', 'Archives'), 'review', 'Release archives and symbols may be needed for distribution or crash reports.'),
        (('Library', 'Developer', 'CoreSimulator'), 'keep', 'Simulator devices can contain app data; manage through simctl or Xcode.'),
        (('Library', 'Logs'), 'review', 'Logs may be useful for debugging; review age and owning application.'),
        (('.cache',), 'rebuildable', 'Tool cache; verify offline assets and stop the owning tool first.'),
        (('.npm', '_cacache'), 'rebuildable', 'npm package download cache; packages may need downloading again.'),
        (('.npm', '_logs'), 'review', 'npm diagnostic logs.'),
        (('.cargo', 'registry', 'cache'), 'rebuildable', 'Downloaded Rust crates; future builds may require network access.'),
        (('Downloads',), 'review', 'Downloads are user files; verify installers, archives, and originals individually.'),
    ]
    for prefix, category, reason in rules:
        if parts[:len(prefix)] == prefix:
            return category, reason
    if parts[0] in ('.codex', '.agents', '.config', '.docker') or parts[:2] == ('.local', 'share'):
        return 'keep', 'Agent, tool, or application state; preserve configuration and history.'
    if parts[0] == 'Library':
        return 'keep', 'Application state, databases, backups, or settings; use the owning app to remove.'
    if 'node_modules' in parts:
        return 'review', 'Installed dependencies; verify lockfile and absence of local modifications before reinstalling.'
    if any(x in parts for x in ('.venv', 'venv')):
        return 'review', 'Python environment; verify reproducible dependencies and no user files.'
    return 'review', 'User or unrecognized data; size and age do not prove it is disposable.'

class Database(sqlite3.Connection):
    def __exit__(self, *args):
        try:
            return super().__exit__(*args)
        finally:
            self.close()


def connect(path, write=False):
    db = sqlite3.connect(path if write else Path(path).absolute().as_uri() + '?mode=ro', uri=not write, factory=Database)
    db.row_factory = sqlite3.Row
    return db

def scan(root, output, excludes=()):
    root, output = absolute(root), absolute(output)
    if os.path.islink(root) or not os.path.isdir(root):
        raise ValueError('Scan root must be a real directory, not a symlink.')
    root = os.path.realpath(root)
    if os.path.lexists(output):
        raise ValueError('Output already exists; choose a new scan database.')
    excludes = [str(Path.home()/'Library/Mobile Documents'), str(Path.home()/'Library/CloudStorage')] + [absolute(x) for x in excludes] + [output, output + '-journal', output + '-wal', output + '-shm']
    fd = os.open(output, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(fd)
    db = connect(output, True)
    db.executescript('''
    CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
    CREATE TABLE entries(id INTEGER PRIMARY KEY, parent INTEGER, path TEXT UNIQUE,
      kind TEXT, device INTEGER, inode INTEGER, mode INTEGER, size INTEGER,
      mtime_ns INTEGER, ctime_ns INTEGER, allocated INTEGER, total INTEGER,
      logical INTEGER, category TEXT, reason TEXT, error TEXT);
    CREATE INDEX parents ON entries(parent);
    CREATE TABLE inodes(device INTEGER, inode INTEGER, PRIMARY KEY(device,inode));
    ''')
    def meta(key, value):
        db.execute('INSERT OR REPLACE INTO meta VALUES (?,?)', (key, json.dumps(value)))
    meta('schema', SCHEMA)
    meta('root', root)
    meta('started', time.time())
    meta('complete', False)
    meta('home', str(Path.home()))
    meta('excludes', excludes)
    usage = os.statvfs(root)
    meta('volume', {'total': usage.f_blocks * usage.f_frsize, 'available': usage.f_bavail * usage.f_frsize})
    db.commit()
    device = os.lstat(root).st_dev
    stack = [(root, None)]
    count = 0
    try:
        while stack:
            path, parent = stack.pop()
            error = None
            try:
                s = os.lstat(path)
                kind = 'directory' if stat.S_ISDIR(s.st_mode) else 'file' if stat.S_ISREG(s.st_mode) else 'symlink' if stat.S_ISLNK(s.st_mode) else 'special'
                allocated = s.st_blocks * 512
                if s.st_nlink > 1 and kind == 'file':
                    cur = db.execute('INSERT OR IGNORE INTO inodes VALUES (?,?)', (s.st_dev, s.st_ino))
                    if cur.rowcount == 0:
                        allocated = 0
                if s.st_dev != device:
                    error, allocated = 'Different filesystem; not traversed.', 0
                if any(inside(path, x) for x in excludes):
                    error, allocated = 'Excluded from scan.', 0
                values = (s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
            except OSError as exc:
                kind, allocated, values, error = 'unknown', 0, (0,) * 6, str(exc)
            category, reason = classify(path)
            logical = values[3] if kind == 'file' and not error else 0
            cur = db.execute('INSERT INTO entries(parent,path,kind,device,inode,mode,size,mtime_ns,ctime_ns,allocated,total,logical,category,reason,error) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                             (parent,path,kind,*values,allocated,allocated,logical,category,reason,error))
            ident = cur.lastrowid
            if kind == 'directory' and error is None:
                try:
                    with os.scandir(path) as entries:
                        stack.extend((e.path, ident) for e in entries)
                except OSError as exc:
                    db.execute('UPDATE entries SET error=? WHERE id=?', (str(exc), ident))
            count += 1
            if count % 10000 == 0:
                db.commit()
                print(f'Scanned {count:,} entries', file=__import__('sys').stderr)
        print(f'Aggregating {count:,} entries', file=__import__('sys').stderr)
        # Reverse insertion order guarantees children have already been aggregated.
        for row in db.execute('SELECT id,parent FROM entries ORDER BY id DESC'):
            if row['parent'] is not None:
                db.execute('UPDATE entries SET total=total+(SELECT total FROM entries WHERE id=?),logical=logical+(SELECT logical FROM entries WHERE id=?) WHERE id=?', (row['id'],row['id'],row['parent']))
        meta('complete', True)
        meta('finished', time.time())
        meta('entries', count)
        db.commit()
    finally:
        db.close()
    return {'scan': output, 'entries': count, 'complete': True}

def metadata(db):
    data = {r['key']: json.loads(r['value']) for r in db.execute('SELECT * FROM meta')}
    if data.get('schema') != SCHEMA or not data.get('complete'):
        raise ValueError('Unsupported or interrupted scan; run a new scan.')
    return data

def report(path, limit=30, under=None, candidates=False):
    with connect(path) as db:
        meta = metadata(db)
        if candidates:
            rows = db.execute("SELECT * FROM entries WHERE category IN ('rebuildable','review') AND total>0 ORDER BY CASE WHEN category='rebuildable' THEN 0 ELSE 1 END,total DESC")
        elif under:
            rows = db.execute('SELECT * FROM entries WHERE parent=(SELECT id FROM entries WHERE path=?) ORDER BY total DESC LIMIT ?', (absolute(under), limit))
        else:
            rows = db.execute('SELECT * FROM entries ORDER BY total DESC LIMIT ?', (limit,))
        selected = []
        for r in rows:
            item = dict(r)
            if under and (str(Path(item['path']).parent) != absolute(under)):
                continue
            if candidates:
                if item['category'] not in ('rebuildable', 'review') or item['error'] or item['path'] == meta['root']:
                    continue
                if any(inside(item['path'], x['path']) or inside(x['path'], item['path']) for x in selected):
                    continue
                # Prefer individual app caches, downloads, and projects over their broad containers.
                if item['path'] in [str(Path.home()/x) for x in ('Downloads', 'Desktop', 'Documents', '.cache', 'Library/Caches', 'Library/Logs', 'Library/Developer/Xcode/DerivedData')]:
                    continue
                if item['total'] == 0:
                    continue
            selected.append(item)
            if len(selected) >= limit:
                break
        errors = [dict(r) for r in db.execute('SELECT path,error FROM entries WHERE error IS NOT NULL LIMIT 100')]
        return {'schema': SCHEMA, 'meta': meta, 'items': selected,
                'coverage_issues': errors, 'coverage_issue_count': db.execute('SELECT count(*) FROM entries WHERE error IS NOT NULL').fetchone()[0],
                'accounting': 'Allocated bytes from st_blocks, hard links counted once. APFS clones, snapshots and purgeable space prevent exact reclaim estimates. Directory rows overlap. Scan reads metadata only.'}

def fingerprint(path):
    """Hash recursive metadata without opening file contents or following symlinks."""
    path = absolute(path)
    h = hashlib.sha256()
    stack = [path]
    device = os.lstat(path).st_dev
    while stack:
        current = stack.pop()
        s = os.lstat(current)
        if s.st_dev != device:
            raise ValueError('Mounted filesystem inside selection; refusing cleanup.')
        if not (stat.S_ISDIR(s.st_mode) or stat.S_ISREG(s.st_mode) or stat.S_ISLNK(s.st_mode)):
            raise ValueError('Special filesystem object inside selection; refusing cleanup.')
        # Root ctime changes when moved into quarantine; omit it for restore validation.
        row = [os.path.relpath(current, path), s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns]
        h.update(json.dumps(row, ensure_ascii=True).encode())
        if stat.S_ISDIR(s.st_mode):
            with os.scandir(current) as entries:
                stack.extend(sorted((e.path for e in entries), reverse=True))
    return h.hexdigest()
