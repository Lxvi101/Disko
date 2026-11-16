// Manual browser fixture: /tests/fixtures/disk-access.html?mode=denied|granted|error
import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { ScanScreen } from '../../src/components/ScanScreen';
import { useStore } from '../../src/store';
import '../../src/index.css';

const mode = new URLSearchParams(location.search).get('mode') ?? 'denied';
let requests = 0;
mockIPC(async command => {
  if (command === 'check_full_disk_access') return false;
  if (command === 'request_full_disk_access') {
    requests += 1;
    document.title = `Access requests: ${requests}`;
    if (mode === 'error') throw new Error('Could not open Full Disk Access settings');
    return mode === 'granted';
  }
  if (command === 'reveal_running_app') throw new Error('This is a development executable. Open the built Disko.app to grant access to Disko.');
  if (command === 'list_scans') return [];
}, { shouldMockEvents: true });
// Old permanent dismissal must not prevent a new request.
localStorage.setItem('disko.fdaDismissed', '1');
useStore.setState({ scans: [], scan: null, autoscan: null });
document.documentElement.dataset.theme = 'light';
createRoot(document.getElementById('root')!).render(<ScanScreen />);
