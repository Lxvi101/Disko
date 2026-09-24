"""Explicit selections, fresh metadata validation, quarantine, and journaled recovery."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import time
import uuid

from .core import absolute, classify, connect, fingerprint, inside, metadata


def digest(obj):
    return hashlib.sha256(json.dumps(obj, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def save_new(path, obj):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump(obj, f, indent=2)
        f.write('\n')
        f.flush()
        os.fsync(f.fileno())


def load(path):
    with open(path) as f:
        return json.load(f)


def safe_selection(path):
    path = absolute(path)
    home = absolute(Path.home())
    category, _ = classify(path)
    containers = ['Desktop', 'Documents', 'Downloads', 'Pictures', 'Movies', 'Music', 'Library',
                  'Library/Caches', 'Library/Logs', 'Library/Developer', 'Library/Developer/Xcode',
                  'Library/Developer/Xcode/DerivedData', '.cache', '.npm', '.cargo']
    if category in ('keep', 'protected') or path in [str(Path(home)/p) for p in containers]:
        raise ValueError(f'Protected selection: {path}; select individual disposable items.')
    if os.path.realpath(str(Path(path).parent)) != str(Path(path).parent):
        raise ValueError(f'Symlinked parent is not eligible: {path}')
    if not inside(path, home) or os.path.islink(path):
        raise ValueError('Only non-symlink items in your home can be selected.')
    return path


@contextlib.contextmanager
def directory_fd(path):
    """Anchor every ancestor without following symlinks during a move."""
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in Path(absolute(path)).parts[1:]:
            new = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = new
        yield fd
    finally:
        os.close(fd)


def move(source, target):
    with directory_fd(Path(source).parent) as src, directory_fd(Path(target).parent) as dst:
        if os.stat('.', dir_fd=src).st_dev != os.stat('.', dir_fd=dst).st_dev:
            raise ValueError('Cross-volume move refused; no copy/delete fallback.')
        try:
            os.stat(Path(target).name, dir_fd=dst, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError(f'Refusing to overwrite {target}')
        os.rename(Path(source).name, Path(target).name, src_dir_fd=src, dst_dir_fd=dst)


def plan(scan_path, paths, output):
    items = []
    with connect(scan_path) as db:
        meta = metadata(db)
        for value in paths:
            path = safe_selection(value)
            if any(inside(path, i['path']) or inside(i['path'], path) for i in items):
                raise ValueError('Selections overlap.')
            row = db.execute('SELECT * FROM entries WHERE path=?', (path,)).fetchone()
            if not row:
                raise ValueError(f'Path was not scanned: {path}')
            # Descendant query uses parent IDs, not LIKE patterns in user filenames.
            tree = db.execute('''WITH RECURSIVE tree AS (
                SELECT * FROM entries WHERE id=? UNION ALL
                SELECT e.* FROM entries e JOIN tree t ON e.parent=t.id)
                SELECT * FROM tree''', (row['id'],))
            for child in tree:
                if child['error'] or classify(child['path'])[0] in ('keep', 'protected'):
                    raise ValueError(f'Incomplete or protected subtree: {child["path"]}')
                s = os.lstat(child['path'])
                if (s.st_dev,s.st_ino,s.st_mode,s.st_size,s.st_mtime_ns,s.st_ctime_ns) != tuple(child[k] for k in ('device','inode','mode','size','mtime_ns','ctime_ns')):
                    raise ValueError(f'Scan is stale: {child["path"]}; rescan.')
            items.append({'path': path, 'allocated_bytes_estimate': row['total'],
                          'category': row['category'], 'reason': row['reason'], 'fingerprint': fingerprint(path)})
    body = {'schema': 1, 'created': time.time(), 'scan': absolute(scan_path), 'scan_started': meta['started'],
            'home': absolute(Path.home()), 'action': 'quarantine', 'items': items,
            'notice': 'Quit owning apps before execution. Quarantine does not free space; purge is a separate irreversible action.'}
    body['confirmation'] = digest(body)
    save_new(output, body)
    return body


def checked_plan(path, confirmation=None):
    p = load(path)
    token = p.pop('confirmation')
    if digest(p) != token or (confirmation is not None and token != confirmation):
        raise ValueError('Plan confirmation mismatch or modified plan.')
    if p['schema'] != 1 or p['home'] != absolute(Path.home()) or p['action'] != 'quarantine' or not p['items']:
        raise ValueError('Invalid plan or wrong user.')
    p['confirmation'] = token
    selected = []
    for item in p['items']:
        source = safe_selection(item['path'])
        if any(inside(source, x) or inside(x, source) for x in selected):
            raise ValueError('Selections overlap.')
        selected.append(source)
        if fingerprint(source) != item['fingerprint']:
            raise ValueError(f'Files changed after planning: {source}; rescan and replan.')
    return p


def apply(path, execute=False, confirmation=None):
    p = checked_plan(path, confirmation)
    if not execute:
        return {'dry_run': True, 'plan': p}
    if confirmation is None:
        raise ValueError('Execution requires --confirm with the full plan confirmation.')
    trash = Path.home()/'.Trash'
    if not trash.exists():
        trash.mkdir(mode=0o700)
    with directory_fd(trash) as fd:
        if os.fstat(fd).st_uid != os.getuid():
            raise ValueError('Trash must be owned by the current user.')
    destination = trash / ('disko-' + uuid.uuid4().hex)
    destination.mkdir(mode=0o700)
    journal = {'schema': 1, 'home': absolute(Path.home()), 'quarantine': str(destination), 'plan_confirmation': p['confirmation'],
               'items': [dict(item, target=str(destination/f'item-{n}')) for n,item in enumerate(p['items'])]}
    journal_path = destination/'journal.json'
    save_new(journal_path, journal)  # Durable intent before any move; state inferred from both locations.
    moved = []
    try:
        for item in journal['items']:
            safe_selection(item['path'])
            if fingerprint(item['path']) != item['fingerprint']:
                raise ValueError(f'Files changed before move: {item["path"]}')
            move(item['path'], item['target'])
            moved.append(item['path'])
    except (OSError, ValueError) as exc:
        return {'ok': False, 'error': str(exc), 'moved': moved, 'journal': str(journal_path), 'recovery': 'Use restore with this journal; partial operations are recoverable.'}
    return {'ok': True, 'moved': moved, 'journal': str(journal_path), 'freed_bytes': 0, 'purge_confirmation': digest(journal)}


def checked_journal(path):
    j = load(path)
    q = Path(j['quarantine'])
    if j['schema'] != 1 or j['home'] != absolute(Path.home()) or q.parent != Path.home()/'.Trash' or not q.name.startswith('disko-'):
        raise ValueError('Invalid quarantine journal.')
    if absolute(path) != str(q/'journal.json') or os.path.realpath(q) != str(q):
        raise ValueError('Journal must remain in its original non-symlink quarantine.')
    for n, item in enumerate(j['items']):
        safe_selection(item['path'])
        if item['target'] != str(q/f'item-{n}'):
            raise ValueError('Invalid quarantine item path.')
    return j


def restore(path, execute=False):
    j = checked_journal(path)
    results = []
    for item in j['items']:
        source, target = item['target'], item['path']
        if not os.path.lexists(source):
            results.append({'path': target, 'status': 'not_in_quarantine'})
            continue
        if os.path.lexists(target):
            raise ValueError(f'Original path occupied; restore refuses overwrite: {target}')
        if fingerprint(source) != item['fingerprint']:
            raise ValueError(f'Quarantined data changed: {source}')
        if execute:
            move(source, target)
        results.append({'path': target, 'status': 'restored' if execute else 'would_restore'})
    return {'dry_run': not execute, 'items': results}


def purge(path, execute=False, confirmation=None):
    j = checked_journal(path)
    token = digest(j)
    if execute and confirmation != token:
        raise ValueError('Permanent deletion requires --confirm with the journal confirmation.')
    results = []
    for item in j['items']:
        target = item['target']
        if not os.path.lexists(target):
            continue
        if fingerprint(target) != item['fingerprint']:
            raise ValueError(f'Quarantined data changed: {target}')
        results.append(target)
    if execute:
        if not shutil.rmtree.avoids_symlink_attacks:
            raise ValueError('This Python platform lacks safe recursive deletion.')
        with directory_fd(j['quarantine']) as fd:
            for target in results:
                s = os.stat(Path(target).name, dir_fd=fd, follow_symlinks=False)
                if stat.S_ISDIR(s.st_mode):
                    shutil.rmtree(Path(target).name, dir_fd=fd)
                else:
                    os.unlink(Path(target).name, dir_fd=fd)
    return {'dry_run': not execute, 'irreversible': True, 'confirmation': token, 'items': results,
            'notice': 'Only recorded quarantine items are removed. APFS snapshots/clones may retain blocks; measure volume free space again.'}
