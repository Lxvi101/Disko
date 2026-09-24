import React, { useEffect, useRef, useState } from 'react';
import { Archive, ArrowDownToLine, ArrowUpRight, Check, ChevronRight, FolderOpen, Info, LockKeyhole, RefreshCw, Search, Sparkles, Trash2 } from 'lucide-react';
import { useStore } from '../../store';
import { APP_CLEANERS, appCacheItems } from '../../lib/appCleanup';
import { LIVE_APPS, type AppProfile } from '../../lib/appProfiles';
import { api, errorText } from '../../lib/api';
import { formatBytes, shortPath } from '../../lib/format';
import { XcodeCleanupSection } from './Xcode';
import { Spinner } from '../ui';
import type { AppStorageInventory, CleanupMode, StagedItem, XcodeInventory } from '../../types';

type Card = { id: string; name: string; category: string; color: string; mark: string; description: string; total?: number; potential?: number; partial?: boolean; error?: string; scope: string };
const sum = (items: { total: number }[]) => items.reduce((n, i) => n + i.total, 0);
function clean(items: StagedItem[], mode: CleanupMode) {
  if (!items.length || useStore.getState().cleanupBusy) return;
  useStore.setState({ cleanupSelection: items, cleanupMode: mode, cleanModalOpen: true });
}
function AppIcon({ app, large = false }: { app: Pick<Card, 'id' | 'color' | 'mark'>; large?: boolean }) {
  return <span className={`app-store-icon ${large ? 'large' : ''}`} style={{ background: `color-mix(in srgb, ${app.color} 17%, var(--bg-2))`, color: app.color }}>
    {['helium', 'xcode'].includes(app.id) ? <img src={`/app-icons/${app.id}.png`} alt="" /> : app.id === 'cursor' ? <ArrowUpRight strokeWidth={2.8} /> : <span>{app.mark}</span>}
  </span>;
}

export const AppCleanupPage: React.FC = () => {
  const candidates = useStore((s) => s.candidates);
  const home = useStore((s) => s.info?.home ?? '');
  const version = useStore((s) => s.dataVersion);
  const busy = useStore((s) => s.cleanupBusy);
  const [inventories, setInventories] = useState<Record<string, AppStorageInventory>>({});
  const [xcode, setXcode] = useState<XcodeInventory>();
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [refresh, setRefresh] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [filter, setFilter] = useState('All apps');
  const [query, setQuery] = useState('');
  const detailRef = useRef<HTMLElement>(null);
  useEffect(() => {
    let active = true;
    setLoading(true); setErrors({}); setInventories({}); setXcode(undefined);
    const requests = LIVE_APPS.map((app) => api.appStorage(app.id).then((data) => { if (active) setInventories((old) => ({ ...old, [app.id]: data })); }).catch((e) => { if (active) setErrors((old) => ({ ...old, [app.id]: errorText(e) })); }));
    requests.push(api.xcodeStorage().then((data) => { if (active) setXcode(data); }).catch((e) => { if (active) setErrors((old) => ({ ...old, xcode: errorText(e) })); }));
    Promise.allSettled(requests).then(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [version, refresh]);
  const scanned = APP_CLEANERS.map((app) => ({ ...app, items: appCacheItems(candidates, home, app.cacheRoots) })).filter((app) => app.items.length);
  const cards: Card[] = [...LIVE_APPS.map((app) => ({ ...app, total: inventories[app.id]?.total, potential: inventories[app.id]?.potential, partial: inventories[app.id]?.partial, error: errors[app.id], scope: 'Known app storage' })),
    { id: 'xcode', name: 'Xcode', category: 'Developer tools', color: '#80bafa', mark: 'X', description: 'Simulators, builds & device support', total: xcode ? sum(xcode.items) : undefined, potential: xcode ? sum(xcode.items.filter((i) => !i.blocked && i.group !== 'archives' && i.group !== 'mcp')) : undefined, partial: xcode?.items.some((i) => i.partial), error: errors.xcode, scope: 'Developer data' },
    ...scanned.map((app) => ({ ...app, category: ['chrome', 'safari'].includes(app.id) ? 'Browsers' : 'Everyday apps', mark: app.name[0], description: app.what, total: sum(app.items), potential: sum(app.items), scope: 'Scanned caches' }))];
  const visible = cards.filter((app) => (filter === 'All apps' || filter === app.category) && `${app.name} ${app.description}`.toLowerCase().includes(query.toLowerCase()));
  const activeCard = cards.find((app) => app.id === selected);
  const activeProfile = LIVE_APPS.find((app) => app.id === selected);
  const activeScanned = scanned.find((app) => app.id === selected);
  const show = (id: string) => { setSelected(id); requestAnimationFrame(() => detailRef.current?.scrollIntoView({ behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth', block: 'start' })); };
  const potential = cards.reduce((n, app) => n + (app.potential ?? 0), 0);
  return <div className="app-store-page h-full overflow-y-auto">
    <div className="app-store-content">
      <header className="flex items-center justify-between gap-4 mb-6"><div><p className="app-store-eyebrow">MADE FOR YOUR MAC</p><h1 className="text-[30px] font-semibold tracking-tight mt-1">App cleanup</h1></div><button className="btn" disabled={loading || busy} onClick={() => setRefresh((n) => n + 1)}>{loading ? <Spinner /> : <RefreshCw className="w-3.5 h-3.5" />} {loading ? 'Measuring…' : 'Refresh'}</button></header>
      <section className="app-store-feature">
        <div className="relative z-10"><div className="flex items-center gap-2 text-[11px] font-medium mb-4"><Sparkles className="w-4 h-4" /> LESS CLUTTER. MORE POSSIBILITY.</div><h2>Great apps.<br />A little lighter.</h2><p className="mt-3 max-w-[340px] text-[13px] leading-relaxed">Old builds, oversized caches, yesterday’s agents.<br />Find the space hiding inside your favorite apps.</p><button className="app-store-feature-button mt-5" onClick={() => show('adobe')}>Explore Adobe cleanup <ChevronRight className="w-4 h-4" /></button></div>
        <div className="app-store-feature-stat"><div className="app-store-orbit" aria-hidden="true"><AppIcon app={LIVE_APPS[0]} /><AppIcon app={LIVE_APPS[3]} /><AppIcon app={LIVE_APPS[1]} /></div><div className="tnum text-[43px] font-semibold tracking-[-2px]">{loading && potential === 0 ? '—' : formatBytes(potential)}</div><div className="text-[12px] mt-1">{loading ? 'found so far' : 'available to review'}</div><div className="text-[10px] opacity-60 mt-3">You choose what goes.</div></div>
      </section>
      <div className="flex items-center justify-between flex-wrap gap-3 mt-7 mb-4"><div className="app-store-filters" aria-label="App categories">{['All apps', 'Creative', 'Browsers', 'Developer tools', ...(scanned.some((a) => !['chrome', 'safari'].includes(a.id)) ? ['Everyday apps'] : [])].map((label) => <button key={label} aria-pressed={filter === label} onClick={() => setFilter(label)}>{label}</button>)}</div><label className="relative"><Search className="absolute left-2.5 top-2.5 w-3.5 h-3.5" style={{ color: 'var(--muted)' }} /><input className="input !pl-8 !w-44" aria-label="Search apps" placeholder="Find an app" value={query} onChange={(e) => setQuery(e.target.value)} /></label></div>
      <div className="app-store-grid">{visible.map((app) => <button key={app.id} className="app-store-card" aria-pressed={selected === app.id} aria-controls="app-cleanup-detail" onClick={() => show(app.id)} style={{ '--app-color': app.color } as React.CSSProperties}>
        <div className="flex items-center gap-3"><AppIcon app={app} /><div className="min-w-0"><h3 className="font-semibold text-[17px] tracking-tight">{app.name}</h3><p className="text-[10px] mt-1" style={{ color: 'var(--muted)' }}>{app.category}</p></div><ChevronRight className="w-4 h-4 ml-auto" style={{ color: 'var(--dim)' }} /></div>
        <p className="text-[11.5px] mt-4 mb-5 truncate" style={{ color: 'var(--muted)' }}>{app.description}</p>
        <div className="flex items-end justify-between gap-2"><div><div className="text-[27px] leading-none tracking-tight tnum font-semibold">{app.error ? 'Unavailable' : app.potential === undefined ? '—' : formatBytes(app.potential)}</div><div className="text-[10px] mt-2" style={{ color: app.color }}>{app.error ? 'Open to retry' : app.potential === undefined ? 'Measuring storage…' : 'to review for cleanup'}</div></div><span className="app-store-review">Review</span></div>
        <div className="app-store-meter"><span style={{ width: `${app.total ? Math.min(100, (app.potential ?? 0) / app.total * 100) : 0}%`, background: app.color }} /></div>
        <div className="flex justify-between text-[10px] gap-2" style={{ color: 'var(--muted)' }}><span>{app.scope}</span><span className="tnum">{app.partial ? '≥ ' : ''}{app.total === undefined ? '—' : formatBytes(app.total)}</span></div>
      </button>)}</div>
      {!visible.length && <p className="text-center py-12" style={{ color: 'var(--muted)' }}>No apps match this search.</p>}
      <p className="flex gap-2 mt-4 text-[10.5px] leading-relaxed" style={{ color: 'var(--muted)' }}><Info className="w-3.5 h-3.5 shrink-0" />Live sizes cover known locations; custom folders and shared APFS blocks may change the space reclaimed. Running apps must be closed before cleanup. Other supported apps appear after a disk scan.</p>
      {activeCard && <section id="app-cleanup-detail" ref={detailRef} className="app-store-detail mt-7">
        <div className="flex items-center gap-4 p-5"><AppIcon app={activeCard} large /><div><p className="app-store-eyebrow">INSIDE {activeCard.name.toUpperCase()}</p><h2 className="text-[22px] font-semibold tracking-tight mt-1">{activeProfile?.tagline ?? `${activeCard.name} storage`}</h2></div></div>
        {activeCard.error ? <div role="alert" className="px-5 pb-5">{activeCard.error}<button className="btn ml-3" disabled={loading} onClick={() => setRefresh((n) => n + 1)}>Retry</button></div> : selected === 'xcode' ? xcode ? <XcodeCleanupSection snapshot={xcode} onRefresh={() => setRefresh((n) => n + 1)} /> : <div className="p-6 flex gap-2"><Spinner /> Measuring Xcode…</div> : activeProfile ? inventories[activeProfile.id] ? <AppDetail key={`${activeProfile.id}-${version}-${refresh}`} app={activeProfile} inventory={inventories[activeProfile.id]} home={home} busy={busy} /> : <div className="p-6 flex gap-2"><Spinner /> Measuring app storage…</div> : activeScanned && <div className="px-5 pb-5"><p className="text-[12px] mb-4 leading-relaxed" style={{ color: 'var(--muted)' }}>{activeScanned.consequence} {activeScanned.before}</p>{activeScanned.items.map((item) => <div key={item.path} className="flex items-center gap-3 py-3 border-t" style={{ borderColor: 'var(--line)' }}><span className="flex-1">{item.name}</span><span>{formatBytes(item.total)}</span><button className="btn" disabled={busy} onClick={() => clean([{ ...item, source: 'suggestion' }], 'quarantine')}>Quarantine…</button><button className="btn" disabled={busy} onClick={() => clean([{ ...item, source: 'suggestion' }], 'delete')}>Delete…</button></div>)}</div>}
      </section>}
    </div>
  </div>;
};

function AppDetail({ app, inventory, home, busy }: { app: AppProfile; inventory: AppStorageInventory; home: string; busy: boolean }) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [info, setInfo] = useState(false);
  const available = inventory.items.filter((item) => !item.blocked);
  const chosen = available.filter((item) => selected.has(item.path));
  const staged = (items: typeof chosen): StagedItem[] => items.map((item) => ({ path: item.path, name: item.name, total: item.total, kind: item.group === 'version' && app.id === 'claude' ? 'file' : 'directory', category: item.group === 'version' ? 'review' : 'rebuildable', reason: app.consequence, source: 'suggestion' }));
  return <div className="px-5 pb-5">
    <div className="app-store-kept"><Check className="w-4 h-4 shrink-0" /><p>{app.kept}</p></div>
    <button className="btn btn-ghost my-3" aria-expanded={info} onClick={() => setInfo(!info)}><Info className="w-3.5 h-3.5" /> What happens when I clean up? <ChevronRight className={`w-3.5 h-3.5 ${info ? 'rotate-90' : ''}`} /></button>
    {info && <div className="app-store-explainer"><div><h3>What’s taking up space</h3><p>{app.what}</p></div><div><h3>After cleanup</h3><p>{app.consequence} Quarantine keeps a restorable copy on this disk; purge it later to reclaim space. Permanent deletion bypasses Trash.</p></div><div><h3>Keep it under control</h3><p>{app.prevention}</p></div></div>}
    {inventory.warnings.map((warning) => <p key={warning} role="status" className="text-[12px] mb-3" style={{ color: 'var(--warn)' }}>{warning}</p>)}
    <div className="app-store-items"><div className="flex items-center gap-3 px-4 py-3 text-[11px]" style={{ background: 'var(--bg-3)', color: 'var(--muted)' }}><input type="checkbox" aria-label={`Select available ${app.name} items`} disabled={busy || !available.length} checked={!!available.length && chosen.length === available.length} onChange={(e) => setSelected(new Set(e.target.checked ? available.map((i) => i.path) : []))} /><span className="flex-1">{inventory.items.length} entries · current installations kept</span><span>Size on disk</span></div>
      {inventory.items.map((item) => <div key={item.path} className="app-store-item"><input type="checkbox" aria-label={`Select ${item.name}`} disabled={busy || !!item.blocked} checked={selected.has(item.path)} onChange={() => setSelected((old) => { const next = new Set(old); next.has(item.path) ? next.delete(item.path) : next.add(item.path); return next; })} />{item.blocked ? <LockKeyhole className="w-4 h-4 shrink-0" style={{ color: 'var(--dim)' }} /> : <ArrowDownToLine className="w-4 h-4 shrink-0" style={{ color: app.color }} />}<div className="flex-1 min-w-0"><div className="text-[12px] font-medium">{item.name}</div><p className="text-[10px] mt-1 truncate selectable" title={item.path} style={{ color: 'var(--dim)' }}>{shortPath(item.path, home)}</p><p className="text-[10.5px] mt-1" style={{ color: item.blocked ? 'var(--warn)' : 'var(--muted)' }}>{item.blocked ?? item.subtitle}</p></div><span className="tnum text-[15px] whitespace-nowrap">{item.partial ? '≥ ' : ''}{formatBytes(item.total)}</span><button className="btn btn-ghost btn-icon" aria-label={`Show ${item.name} in Finder`} onClick={() => api.reveal(item.path).catch((e) => useStore.getState().toast({ title: errorText(e), kind: 'error' }))}><FolderOpen className="w-4 h-4" /></button><button className="btn btn-ghost btn-icon" aria-label={`Delete ${item.name}`} disabled={busy || !!item.blocked} onClick={() => clean(staged([item]), 'delete')}><Trash2 className="w-3.5 h-3.5" /></button></div>)}
      {!inventory.items.length && <p className="text-center p-10 text-[12px]" style={{ color: 'var(--muted)' }}>No supported caches or agent versions found. There’s nothing to clean here.</p>}
    </div>
    <div className="flex items-center flex-wrap gap-2 mt-4"><span className="text-[12px] flex-1" style={{ color: 'var(--muted)' }}>{chosen.length ? `${chosen.length} selected · ${formatBytes(sum(chosen))}` : 'Select the entries you want to clean.'}</span><button className="btn" disabled={busy || !chosen.length} onClick={() => clean(staged(chosen), 'quarantine')}><Archive className="w-3.5 h-3.5" /> Quarantine…</button><button className="btn" disabled={busy || !chosen.length} onClick={() => clean(staged(chosen), 'delete')}><Trash2 className="w-3.5 h-3.5" /> Delete permanently…</button></div>
  </div>;
}
