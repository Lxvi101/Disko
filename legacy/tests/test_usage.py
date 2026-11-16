import os
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from disko_cli import core, usage


class UsageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.file = self.root/'old.bin'
        self.file.write_bytes(b'x'*8192)
        self.old = time.time()-200*86400
        os.utime(self.file, (self.old, self.old))
        self.db = self.root.parent/(self.root.name+'.sqlite')
        core.scan(self.root, self.db)
        real_stat = os.lstat
        def old_birth(path, *args, **kwargs):
            s = real_stat(path, *args, **kwargs)
            if str(path) == str(self.file):
                attrs = {name: getattr(s, name) for name in dir(s) if name.startswith('st_')}
                attrs['st_birthtime'] = self.old
                return SimpleNamespace(**attrs)
            return s
        self.stat_patch = patch('disko_cli.usage.os.lstat', side_effect=old_birth)
        self.stat_patch.start()

    def tearDown(self):
        self.stat_patch.stop()
        self.db.unlink()
        self.temp.cleanup()

    def test_old_metadata_is_review_only(self):
        r = usage.unused(self.db, min_size=1, spotlight=False)
        self.assertEqual(len(r['items']), 1)
        self.assertEqual(r['items'][0]['evidence'], 'filesystem_dates_only')
        self.assertEqual(r['items'][0]['decision'], 'review')
        self.assertIsNone(r['items'][0]['last_used'])
        self.assertGreaterEqual(r['items'][0]['days_since_activity_evidence'], 199)

    def test_recent_access_excludes_old_modification(self):
        os.utime(self.file, (time.time(), self.old))
        self.assertEqual(usage.unused(self.db, min_size=1, spotlight=False)['items'], [])

    def test_recent_spotlight_use_excludes_candidate(self):
        with patch('disko_cli.usage.last_used', return_value=(time.time(), None)):
            self.assertEqual(usage.unused(self.db, min_size=1)['items'], [])

    def test_unavailable_spotlight_is_explicit(self):
        with patch('disko_cli.usage.last_used', return_value=(None, 'Unavailable')):
            r = usage.unused(self.db, min_size=1)
        self.assertEqual(r['items'][0]['usage_note'], 'Unavailable')

    def test_changed_file_skipped(self):
        self.file.write_text('replacement data')
        r = usage.unused(self.db, min_size=1, spotlight=False)
        self.assertEqual(r['items'], [])
        self.assertEqual(r['skipped']['changed_or_unavailable'], 1)

    def test_app_unknown_usage_requires_opt_in_and_ignores_directory_atime(self):
        self.file.unlink()
        self.file = self.root/'Old.app'
        self.file.mkdir()
        (self.file/'program').write_bytes(b'x'*8192)
        os.utime(self.file, (time.time(), self.old))
        self.db.unlink()
        core.scan(self.root, self.db)
        with patch('disko_cli.usage.last_used', return_value=(None, 'Unavailable')):
            self.assertEqual(usage.unused(self.db, kind='apps', min_size=1)['items'], [])
            r = usage.unused(self.db, kind='apps', min_size=1, include_unknown=True)
        self.assertEqual(len(r['items']), 1)
        self.assertEqual(r['items'][0]['evidence'], 'unknown_app_usage')
        self.assertIsNone(r['items'][0]['last_accessed'])

if __name__ == '__main__':
    unittest.main()
