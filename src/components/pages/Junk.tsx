import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { RefreshCw, Search, Sparkles, Trash2 } from 'lucide-react';
import { useStore, useCollectedSelection } from '../../store';
import { api, errorText } from '../../lib/api';
import { formatBytes } from '../../lib/format';
import { clusterGroups, junkToGroups } from '../../lib/programs';
import { ClusterBrowser } from '../ClusterBrowser';
import { Empty, Spinner } from '../ui';
import type { JunkItem, StagedItem } from '../../types';

/** "3 mo old", "2 yr old": how long since the folder last changed. */
export function age(secs: number | null): string {
  if (!secs) return '';
  const days = (Date.now() / 1000 - secs) / 86400;
  if (days < 1) return 'today';
  if (days < 30) return `${Math.floor(days)} d old`;
  if (days < 365) return `${Math.floor(days / 30)} mo old`;
  return `${Math.floor(days / 365)} yr old`;
}

const toStaged = (it: JunkItem): StagedItem => ({ path: it.path, name: it.name, kind: it.kind, total: it.total, category: it.category, reason: it.reason, tag: it.tag, action: it.action, tool: it.tool, source: 'suggestion' });

/** Everything the last disk scan found that tools recreate, clustered by the program that owns it. */
export const JunkPage: React.FC = () => {
  const home = useStore((s) => s.info?.home ?? '');
  const version = useStore((s) => s.dataVersion);
  const db = useStore((s) => s.info?.db_path);
  const busy = useStore((s) => s.cleanupBusy);
  const [items, setItems] = useState<JunkItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [q, setQ] = useState('');
  const byPath = useMemo(() => new Map((items ?? []).map((i) => [i.path, i])), [items]);
  const lookup = useCallback((p: string) => { const it = byPath.get(p); return it && toStaged(it); }, [byPath]);
  const [selected, setSelected] = useCollectedSelection(lookup);

  useEffect(() => {
    let live = true;
    setError(null);
    api.junk().then((r) => { if (live) setItems(r); }).catch((e) => live && setError(errorText(e)));
    return () => { live = false; };
  }, [version, db, refresh]);

  const clusters = useMemo(() => {
    const s = q.trim().toLowerCase();
    const filtered = (items ?? []).filter((it) => !s || it.path.toLowerCase().includes(s));
    return clusterGroups(junkToGroups(filtered, home));
  }, [items, q, home]);
  const total = (items ?? []).reduce((n, i) => n + i.total, 0);

  if (error) return <Empty icon={<Trash2 className="w-6 h-6" />} title="Could not look for junk" body={error} action={<button className="btn" onClick={() => setRefresh((n) => n + 1)}>Try again</button>} />;
  if (!items) return <div className="h-full flex items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--muted)' }}><Spinner /> Looking for junk…</div>;
  if (!items.length) return <Empty icon={<Sparkles className="w-6 h-6" />} title="No junk found" body="No dependencies, build files, caches, logs or installers over 1 MB in this scan." />;

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[1180px] mx-auto px-8 pt-7 pb-10">
        <header className="flex items-end justify-between gap-4 mb-6">
          <div>
            <h1 className="text-[28px] font-bold tracking-tight leading-none">Junk</h1>
            <p className="text-[13px] mt-2" style={{ color: 'var(--muted)' }}>
              <strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(total)}</strong> in {items.length} places from your last disk scan, sorted by program. Ticked items go to the Collector below.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <label className="relative">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--dim)' }} />
              <input className="input !pl-8 !w-56 !rounded-full !py-1.5 !text-[12.5px]" placeholder="Filter by project or path" value={q} onChange={(e) => setQ(e.target.value)} />
            </label>
            <button className="btn btn-ghost btn-icon" title="Look again" onClick={() => { setItems(null); setRefresh((n) => n + 1); }}><RefreshCw className="w-4 h-4" /></button>
          </div>
        </header>
        {clusters.length ? <ClusterBrowser clusters={clusters} selected={selected} onChange={setSelected} busy={busy} /> : <p className="py-12 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>Nothing matches “{q}”.</p>}
      </div>
    </div>
  );
};
