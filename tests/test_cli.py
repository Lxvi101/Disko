import contextlib
import io
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from disko_cli import core, cleanup
from disko_cli.cli import main


class CliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name).resolve()
        self.home = self.base/'home'
        self.home.mkdir()
        self.mock_home = patch('pathlib.Path.home', return_value=self.home)
        self.mock_home.start()
        self.cache = self.home/'Library/Caches/example'
        self.cache.mkdir(parents=True)
        (self.cache/'payload').write_bytes(b'x'*8192)
        self.db = self.base/'scan.sqlite'

    def tearDown(self):
        self.mock_home.stop()
        self.tmp.cleanup()

    def scan(self):
        core.scan(self.home, self.db)

    def plan(self):
        self.scan()
        target = self.base/'plan.json'
        p = cleanup.plan(self.db, [str(self.cache)], target)
        return target, p

    def test_allocated_hardlinks_symlinks_and_sparse(self):
        os.link(self.cache/'payload', self.cache/'hardlink')
        (self.cache/'loop').symlink_to(self.home, target_is_directory=True)
        sparse = self.home/'sparse'
        with sparse.open('wb') as f:
            f.truncate(64*1024*1024)
        self.scan()
        with core.connect(self.db) as db:
            files = db.execute("SELECT * FROM entries WHERE kind='file'").fetchall()
            self.assertEqual(sum(r['allocated'] for r in files), (self.cache/'payload').stat().st_blocks*512 + sparse.stat().st_blocks*512)
            self.assertEqual(db.execute("SELECT count(*) FROM entries WHERE kind='symlink'").fetchone()[0], 1)
            root = db.execute('SELECT total FROM entries WHERE parent IS NULL').fetchone()[0]
            self.assertEqual(root, db.execute('SELECT sum(allocated) FROM entries').fetchone()[0])

    def test_exclusion_and_incomplete_scan(self):
        core.scan(self.home, self.db, [str(self.cache)])
        r = core.report(self.db)
        self.assertEqual(r['coverage_issue_count'], 1)
        with self.assertRaises(ValueError):
            cleanup.plan(self.db, [str(self.cache)], self.base/'plan')
        with core.connect(self.db, True) as db:
            db.execute("UPDATE meta SET value='false' WHERE key='complete'")
        with self.assertRaises(ValueError):
            core.report(self.db)

    def test_plan_and_apply_dry_run_restore_and_purge(self):
        path, p = self.plan()
        self.assertTrue(cleanup.apply(path)['dry_run'])
        self.assertTrue(self.cache.exists())
        with self.assertRaises(ValueError):
            cleanup.apply(path, True, 'wrong')
        applied = cleanup.apply(path, True, p['confirmation'])
        self.assertTrue(applied['ok'])
        self.assertFalse(self.cache.exists())
        journal = applied['journal']
        self.assertTrue(cleanup.restore(journal)['dry_run'])
        cleanup.restore(journal, True)
        self.assertEqual((self.cache/'payload').read_bytes(), b'x'*8192)
        # A restored item's original inode/mtime persist; a fresh scan and plan are normal practice.
        self.db = self.base/'rescan.sqlite'
        self.scan()
        path = self.base/'second-plan'
        p = cleanup.plan(self.db, [str(self.cache)], path)
        applied = cleanup.apply(path, True, p['confirmation'])
        dry = cleanup.purge(applied['journal'])
        with self.assertRaises(ValueError):
            cleanup.purge(applied['journal'], True)
        cleanup.purge(applied['journal'], True, dry['confirmation'])
        self.assertFalse(Path(dry['items'][0]).exists())

    def test_stale_scan_new_nested_file_and_modified_plan(self):
        path, p = self.plan()
        (self.cache/'new').write_text('new data')
        with self.assertRaises(ValueError):
            cleanup.apply(path)
        p['items'][0]['path'] = str(self.home)
        path.write_text(json.dumps(p))
        with self.assertRaises(ValueError):
            cleanup.apply(path)

    def test_protected_symlink_parent_and_overlap(self):
        self.scan()
        for path in (self.home, self.home/'Library', self.home/'Library/CloudStorage', self.home/'.ssh'):
            with self.assertRaises(ValueError):
                cleanup.safe_selection(path)
        alias = self.home/'alias'
        alias.symlink_to(self.cache.parent)
        with self.assertRaises(ValueError):
            cleanup.safe_selection(alias/'example')
        with self.assertRaises(ValueError):
            cleanup.plan(self.db, [str(self.cache), str(self.cache/'payload')], self.base/'plan')

    def test_restore_collision_and_journal_escape(self):
        path, p = self.plan()
        applied = cleanup.apply(path, True, p['confirmation'])
        self.cache.mkdir()
        with self.assertRaises(ValueError):
            cleanup.restore(applied['journal'], True)
        self.cache.rmdir()
        j = cleanup.load(applied['journal'])
        j['items'][0]['target'] = str(self.home/'precious')
        Path(applied['journal']).write_text(json.dumps(j))
        with self.assertRaises(ValueError):
            cleanup.purge(applied['journal'])

    def test_partial_apply_is_recoverable(self):
        second = self.home/'Downloads/file'
        second.parent.mkdir()
        second.write_text('keep me')
        self.scan()
        path = self.base/'plan'
        p = cleanup.plan(self.db, [str(self.cache), str(second)], path)
        real_move = cleanup.move
        def fail_second(src, dst):
            if src == str(second):
                raise OSError('simulated move failure')
            real_move(src, dst)
        with patch('disko_cli.cleanup.move', side_effect=fail_second):
            result = cleanup.apply(path, True, p['confirmation'])
        self.assertFalse(result['ok'])
        cleanup.restore(result['journal'], True)
        self.assertTrue(self.cache.exists())
        self.assertEqual(second.read_text(), 'keep me')

    def test_cli_json_error(self):
        output = io.StringIO()
        with contextlib.redirect_stderr(output), self.assertRaises(SystemExit):
            main(['report', str(self.base/'missing')])
        self.assertFalse(json.loads(output.getvalue())['ok'])

    def test_protected_descendants(self):
        project = self.home/'project'
        (project/'.git').mkdir(parents=True)
        self.scan()
        with self.assertRaises(ValueError):
            cleanup.plan(self.db, [str(project)], self.base/'plan')

    def test_symlink_inside_cache_does_not_delete_target(self):
        precious = self.home/'precious'
        precious.write_text('important')
        (self.cache/'link').symlink_to(precious)
        path, p = self.plan()
        result = cleanup.apply(path, True, p['confirmation'])
        dry = cleanup.purge(result['journal'])
        cleanup.purge(result['journal'], True, dry['confirmation'])
        self.assertEqual(precious.read_text(), 'important')


    def test_cloud_roots_excluded_by_default(self):
        cloud = self.home/'Library/Mobile Documents'
        cloud.mkdir()
        (cloud/'original').write_text('cloud original')
        self.scan()
        with core.connect(self.db) as db:
            row = db.execute('SELECT error FROM entries WHERE path=?', (str(cloud),)).fetchone()
            self.assertEqual(row['error'], 'Excluded from scan.')
            self.assertIsNone(db.execute('SELECT id FROM entries WHERE path=?', (str(cloud/'original'),)).fetchone())

    def test_changed_scan_rejected_before_plan(self):
        self.scan()
        (self.cache/'payload').write_text('different size')
        with self.assertRaises(ValueError):
            cleanup.plan(self.db, [str(self.cache)], self.base/'plan')

    def test_permission_error_visible_and_blocks_cleanup(self):
        real = os.scandir
        def restricted(path):
            if str(path) == str(self.cache):
                raise PermissionError('test: permission denied')
            return real(path)
        with patch('disko_cli.core.os.scandir', side_effect=restricted):
            self.scan()
        self.assertEqual(core.report(self.db)['coverage_issue_count'], 1)
        with self.assertRaises(ValueError):
            cleanup.plan(self.db, [str(self.cache)], self.base/'plan')

if __name__ == '__main__':
    unittest.main()
