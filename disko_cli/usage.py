"""Conservative inactivity review: usage evidence is not a deletion policy."""
from datetime import datetime, timezone
import os
from pathlib import Path
import stat
import subprocess
import sys
import time

from .core import absolute, connect, inside, metadata


def last_used(path):
    if sys.platform != 'darwin':
        return None, 'Spotlight is only available on macOS.'
    try:
        p = subprocess.run(['/usr/bin/mdls', '-raw', '-name', 'kMDItemLastUsedDate', path],
                           capture_output=True, text=True, timeout=2)
        raw = p.stdout.strip()
        if p.returncode or raw in ('', '(null)'):
            return None, 'No indexed last-used date available.'
        return datetime.strptime(raw, '%Y-%m-%d %H:%M:%S %z').timestamp(), None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None, 'Spotlight lookup unavailable or timed out.'


def iso(timestamp):
    return datetime.fromtimestamp(timestamp, timezone.utc).isoformat() if timestamp else None


def unused(scan, days=90, min_size=100*1024**2, limit=50, under=None, kind='files',
           spotlight=True, include_unknown=False, max_check=10000):
    if days < 1 or min_size < 0 or limit < 1 or max_check < 1:
        raise ValueError('Days, limit, and max-check must be positive; min-size cannot be negative.')
    now = time.time()
    cutoff = now-days*86400
    items, issues = [], []
    considered = 0
    truncated = False
    lookup_deadline = time.monotonic()+20
    counts = {'recent_activity': 0, 'unknown_app_usage': 0, 'changed_or_unavailable': 0}
    with connect(scan) as db:
        meta = metadata(db)
        query = "SELECT * FROM entries WHERE total>=? AND kind=? ORDER BY total DESC"
        for row in db.execute(query, (min_size, 'directory' if kind == 'apps' else 'file')):
            path = row['path']
            if under and not inside(path, absolute(under)):
                continue
            parts = Path(path).parts
            # App bundles are reviewed as units; avoid offering their internal files.
            if any(p.endswith('.app') for p in parts[:-1]):
                continue
            if kind == 'apps' and not path.endswith('.app'):
                continue
            if row['error']:
                continue
            if considered >= max_check:
                truncated = True
                break
            considered += 1
            try:
                if os.path.realpath(str(Path(path).parent)) != str(Path(path).parent):
                    raise ValueError('Parent became a symlink; rescan.')
                s = os.lstat(path)
                if (s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns) != tuple(row[k] for k in ('device','inode','mode','size','mtime_ns')):
                    raise ValueError('Changed since scan; rescan before review.')
                if stat.S_ISLNK(s.st_mode):
                    raise ValueError('Symlink; not a usage candidate.')
            except (OSError, ValueError) as exc:
                counts['changed_or_unavailable'] += 1
                if len(issues) < 100:
                    issues.append({'path': path, 'error': str(exc)})
                continue
            created = getattr(s, 'st_birthtime', None)
            # Scan traversal can update directory atime; never interpret that as app use.
            access = s.st_atime if kind == 'files' else None
            file_activity = max(x for x in (s.st_mtime, created, access) if x is not None)
            if file_activity > cutoff:
                counts['recent_activity'] += 1
                continue
            used, issue = None, 'Spotlight lookup disabled.'
            if spotlight:
                if time.monotonic() < lookup_deadline:
                    used, issue = last_used(path)
                else:
                    issue = 'Spotlight time budget reached; no usage lookup for this item.'
            if kind == 'apps' and used is None and not include_unknown:
                counts['unknown_app_usage'] += 1
                continue
            activity = max(file_activity, used or 0)
            if activity > cutoff:
                counts['recent_activity'] += 1
                continue
            evidence = 'spotlight_and_filesystem_dates' if used else 'filesystem_dates_only' if kind == 'files' else 'unknown_app_usage'
            items.append({'path': path, 'allocated_bytes_estimate': row['total'], 'logical_bytes': row['logical'],
                          'category': row['category'], 'policy_reason': row['reason'],
                          'last_used': iso(used), 'last_accessed': iso(access), 'modified': iso(s.st_mtime),
                          'created': iso(created), 'latest_activity_evidence': iso(activity),
                          'days_since_activity_evidence': int((now-activity)/86400), 'evidence': evidence,
                          'usage_note': issue, 'decision': 'review',
                          'next_step': 'Review the full path; reveal in Finder for manual removal, or create an explicit cleanup plan if eligible.'})
            if len(items) >= limit:
                # Results are deliberately largest-first, not a complete inventory.
                truncated = True
                break
    return {'schema': 1, 'scan': absolute(scan), 'root': meta['root'], 'days': days, 'min_size_bytes': min_size,
            'kind': kind, 'items': items, 'considered': considered, 'limited': truncated, 'skipped': counts,
            'issues': issues, 'notice': 'Review candidates only. Last access may be stale, disabled, or updated by background tools; modification/creation are not usage. Spotlight can be absent or incomplete. No deletion is authorized by age.'}
