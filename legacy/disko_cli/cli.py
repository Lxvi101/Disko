import argparse
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys
from . import core, cleanup, usage


def doctor():
    result = {'platform': platform.platform(), 'python': platform.python_version(),
              'home': str(Path.home()), 'uid': os.getuid(),
              'notes': ['Run scans as your normal user.', 'Permission errors are listed in reports; macOS privacy permissions may limit coverage.',
                        'No snapshot removal or forced purgeable-space allocation is performed.']}
    if sys.platform == 'darwin':
        for name, cmd in [('disk', ['/usr/sbin/diskutil', 'info', '-plist', '/']),
                          ('snapshots', ['/usr/bin/tmutil', 'listlocalsnapshots', '/'])]:
            try:
                p = subprocess.run(cmd, capture_output=True, text=True, timeout=20)
                result[name] = {'exit_code': p.returncode, 'output': p.stdout, 'error': p.stderr}
            except (OSError, subprocess.TimeoutExpired) as exc:
                result[name] = {'error': str(exc)}
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description='Independent agent disk auditor. Scan → report → plan → quarantine → restore or purge.')
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('scan', help='Read filesystem metadata without following symlinks or crossing volumes')
    p.add_argument('root', nargs='?', default=str(Path.home()))
    p.add_argument('--output', required=True)
    p.add_argument('--exclude', action='append', default=[])
    for name in ('report', 'candidates'):
        p = sub.add_parser(name)
        p.add_argument('scan')
        p.add_argument('--limit', type=int, default=30)
        p.add_argument('--under', help='Show immediate children of this absolute path')
        p.add_argument('--json', action='store_true')
    p = sub.add_parser('unused', help='Find large items with old usage evidence for manual review')
    p.add_argument('scan')
    p.add_argument('--days', type=int, default=90)
    p.add_argument('--min-size-mib', type=float, default=100)
    p.add_argument('--limit', type=int, default=50)
    p.add_argument('--under', help='Limit review to this entire subtree')
    p.add_argument('--kind', choices=['files', 'apps'], default='files')
    p.add_argument('--no-spotlight', action='store_true')
    p.add_argument('--include-unknown', action='store_true', help='Include old app bundles even if actual usage is unknown')
    p.add_argument('--max-check', type=int, default=10000)
    p.add_argument('--json', action='store_true')
    p = sub.add_parser('reveal', help='Reveal one existing path in Finder for manual inspection')
    p.add_argument('path')
    p = sub.add_parser('plan', help='Record explicit selections and fresh recursive metadata fingerprints')
    p.add_argument('scan')
    p.add_argument('--path', action='append', required=True)
    p.add_argument('--output', required=True)
    for name in ('apply', 'restore', 'purge'):
        p = sub.add_parser(name)
        p.add_argument('file', help='Plan JSON for apply; quarantine journal for restore/purge')
        p.add_argument('--execute', action='store_true', help='Perform the action; otherwise dry run')
        if name != 'restore':
            p.add_argument('--confirm', help='Full confirmation hash returned by plan or purge dry run')
    sub.add_parser('doctor', help='Read-only volume and Time Machine diagnostics')
    args = parser.parse_args(argv)
    try:
        if args.command == 'scan':
            result = core.scan(args.root, args.output, args.exclude)
        elif args.command in ('report', 'candidates'):
            if args.limit < 1:
                raise ValueError('--limit must be positive.')
            result = core.report(args.scan, args.limit, args.under, args.command == 'candidates')
            if not args.json:
                print(f"{'Allocated':>12}  {'Decision':12} Path")
                for r in result['items']:
                    print(f"{r['total']/1024**3:9.2f} GiB  {r['category']:12} {json.dumps(r['path'], ensure_ascii=False)}")
                    print(f"                            {r['reason']}")
                print(f"\nCoverage issues: {result['coverage_issue_count']}. {result['accounting']}")
                return
        elif args.command == 'unused':
            result = usage.unused(args.scan, args.days, int(args.min_size_mib*1024**2), args.limit,
                                  args.under, args.kind, not args.no_spotlight, args.include_unknown, args.max_check)
            if not args.json:
                print(f"{'Allocated':>12}  {'Days':>6}  {'Evidence':32} Path")
                for item in result['items']:
                    print(f"{item['allocated_bytes_estimate']/1024**3:9.2f} GiB  {item['days_since_activity_evidence']:6}  {item['evidence']:32} {json.dumps(item['path'], ensure_ascii=False)}")
                print(f"\n{result['notice']}")
                print(f"Checked: {result['considered']}; limited: {result['limited']}; skipped: {json.dumps(result['skipped'])}")
                return
        elif args.command == 'reveal':
            path = core.absolute(args.path)
            if sys.platform != 'darwin' or not os.path.lexists(path):
                raise ValueError('Reveal requires macOS and an existing path.')
            p = subprocess.run(['/usr/bin/open', '-R', path], capture_output=True, text=True, timeout=10)
            if p.returncode:
                raise ValueError(p.stderr.strip() or 'Finder reveal failed.')
            result = {'revealed': path}
        elif args.command == 'plan':
            result = cleanup.plan(args.scan, args.path, args.output)
        elif args.command == 'apply':
            result = cleanup.apply(args.file, args.execute, args.confirm)
        elif args.command == 'restore':
            result = cleanup.restore(args.file, args.execute)
        elif args.command == 'purge':
            result = cleanup.purge(args.file, args.execute, args.confirm)
        else:
            result = doctor()
        print(json.dumps(result, indent=2))
        if result.get('ok') is False:
            sys.exit(1)
    except (OSError, ValueError, KeyError, TypeError, sqlite3.Error, subprocess.TimeoutExpired, OverflowError) as exc:
        print(json.dumps({'ok': False, 'error': str(exc)}), file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print(json.dumps({'ok': False, 'error': 'Interrupted; incomplete scans cannot be used for cleanup.'}), file=sys.stderr)
        sys.exit(130)

if __name__ == '__main__':
    main()
