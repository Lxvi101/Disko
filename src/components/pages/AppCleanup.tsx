import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowUpRight, Check, ChevronLeft, FolderOpen, LockKeyhole, RefreshCw, Search, Trash2 } from 'lucide-react';
import { useStore } from '../../store';
import { APP_CLEANERS, appCacheItems } from '../../lib/appCleanup';
import { LIVE_APPS, type AppProfile } from '../../lib/appProfiles';
import { api, errorText } from '../../lib/api';
import { formatBytes, shortPath } from '../../lib/format';
import { XcodeCleanupSection } from './Xcode';
import { Spinner } from '../ui';
import type { AppStorageInventory, CandidateItem, CleanupMode, StagedItem, XcodeInventory } from '../../types';

type Card = { id: string; name: string; category: string; color: string; mark: string; description: string; tagline: string; total?: number; potential?: number; count?: number; partial?: boolean; error?: string; scope: string };
const sum = (items: { total: number }[]) => items.reduce((n, i) => n + i.total, 0);
function clean(items: StagedItem[], mode: CleanupMode) {
  if (!items.length || useStore.getState().cleanupBusy) return;
  useStore.setState({ cleanupSelection: items, cleanupMode: mode, cleanModalOpen: true });
}

function AppIcon({ app, size = 56 }: { app: Pick<Card, 'id' | 'color' | 'mark'>; size?: number }) {
  const image = ['helium', 'xcode'].includes(app.id);
  return (
    <span className={`app-icon ${image ? 'image' : ''}`} style={{ width: size, height: size, fontSize: size * 0.5, '--app-color': app.color } as React.CSSProperties}>
      {image ? <img src={`/app-icons/${app.id}.png`} alt="" /> : app.id === 'cursor' ? <ArrowUpRight strokeWidth={2.6} style={{ width: size * 0.55, height: size * 0.55 }} /> : <span>{app.mark}</span>}
    </span>
  );
}

/** The App Store "GET" button, reused as the amount of space an app can give back. */
function SizePill({ app, onClick }: { app: Card; onClick?: () => void }) {
  const label = app.error ? 'Retry' : app.potential === undefined ? null : app.potential > 0 ? formatBytes(app.potential) : 'Clean';
  return (
    <span className="app-pill" role={onClick ? 'button' : undefined} onClick={onClick}>
      {label ?? <Spinner size={12} />}
    </span>
  );
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
  const [query, setQuery] = useState('');
  const scroller = useRef<HTMLDivElement>(null);
  const homeScroll = useRef(0);

  useEffect(() => {
    let active = true;
    setLoading(true); setErrors({}); setInventories({}); setXcode(undefined);
    const requests = LIVE_APPS.map((app) => api.appStorage(app.id).then((data) => { if (active) setInventories((old) => ({ ...old, [app.id]: data })); }).catch((e) => { if (active) setErrors((old) => ({ ...old, [app.id]: errorText(e) })); }));
    requests.push(api.xcodeStorage().then((data) => { if (active) setXcode(data); }).catch((e) => { if (active) setErrors((old) => ({ ...old, xcode: errorText(e) })); }));
    Promise.allSettled(requests).then(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [version, refresh]);

  // Product pages open at the top; going back returns to the same place in the list.
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = selected ? 0 : homeScroll.current;
  }, [selected]);
  const open = (id: string) => { homeScroll.current = scroller.current?.scrollTop ?? 0; setSelected(id); };

  const scanned = APP_CLEANERS.map((app) => ({ ...app, items: appCacheItems(candidates, home, app.cacheRoots) })).filter((app) => app.items.length);
  const cards: Card[] = [
    ...LIVE_APPS.map((app) => ({ ...app, total: inventories[app.id]?.total, potential: inventories[app.id]?.potential, count: inventories[app.id]?.items.length, partial: inventories[app.id]?.partial, error: errors[app.id], scope: 'Known app storage' })),
    { id: 'xcode', name: 'Xcode', category: 'Developer tools', color: '#3b8ef0', mark: 'X', description: 'Simulators, builds & device support', tagline: 'Choose what stays. Reclaim the rest.', total: xcode ? sum(xcode.items) : undefined, potential: xcode ? sum(xcode.items.filter((i) => !i.blocked && i.group !== 'archives' && i.group !== 'mcp')) : undefined, count: xcode?.items.length, partial: xcode?.items.some((i) => i.partial), error: errors.xcode, scope: 'Developer data' },
    ...scanned.map((app) => ({ ...app, category: ['chrome', 'safari'].includes(app.id) ? 'Browsers' : 'Everyday apps', mark: app.name[0], description: app.what, tagline: app.what, total: sum(app.items), potential: sum(app.items), count: app.items.length, scope: 'Scanned caches' })),
  ];
  const ranked = [...cards].sort((a, b) => (b.potential ?? -1) - (a.potential ?? -1));
  const visible = ranked.filter((app) => `${app.name} ${app.description} ${app.category}`.toLowerCase().includes(query.toLowerCase()));
  const potential = cards.reduce((n, app) => n + (app.potential ?? 0), 0);
  const featured = loading ? null : ranked.find((app) => (app.potential ?? 0) > 0) ?? null;
  const activeCard = cards.find((app) => app.id === selected);

  return (
    <div ref={scroller} className="app-store-page h-full overflow-y-auto">
      <div className="app-store-content">
        {activeCard ? (
          <ProductPage
            key={activeCard.id}
            card={activeCard}
            loading={loading}
            busy={busy}
            onBack={() => setSelected(null)}
            onRefresh={() => setRefresh((n) => n + 1)}
          >
            {activeCard.error ? (
              <div role="alert" className="app-store-section text-[12.5px]" style={{ color: 'var(--danger)' }}>
                {activeCard.error}
                <button className="btn ml-3" disabled={loading} onClick={() => setRefresh((n) => n + 1)}>Retry</button>
              </div>
            ) : activeCard.id === 'xcode' ? (
              xcode ? <XcodeCleanupSection snapshot={xcode} onRefresh={() => setRefresh((n) => n + 1)} /> : <Measuring />
            ) : LIVE_APPS.some((a) => a.id === activeCard.id) ? (
              inventories[activeCard.id]
                ? <AppDetail key={`${version}-${refresh}`} app={LIVE_APPS.find((a) => a.id === activeCard.id)!} inventory={inventories[activeCard.id]} home={home} busy={busy} />
                : <Measuring />
            ) : (
              <ScannedDetail app={scanned.find((a) => a.id === activeCard.id)!} home={home} busy={busy} />
            )}
          </ProductPage>
        ) : (
          <>
            <header className="flex items-end justify-between gap-4 mb-6">
              <div>
                <h1 className="text-[30px] font-bold tracking-tight leading-none">Apps</h1>
                <p className="text-[13px] mt-2" style={{ color: 'var(--muted)' }}>Free up space hiding inside the apps you use.</p>
              </div>
              <div className="flex items-center gap-2">
                <label className="relative">
                  <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5" style={{ color: 'var(--dim)' }} />
                  <input className="input !pl-8 !w-52 !rounded-full" aria-label="Search apps" placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
                </label>
                <button className="btn btn-ghost btn-icon" title="Measure again" disabled={loading || busy} onClick={() => setRefresh((n) => n + 1)}>
                  {loading ? <Spinner /> : <RefreshCw className="w-4 h-4" />}
                </button>
              </div>
            </header>

            {!query && (
              <div className="app-store-hero-row">
                {featured ? (
                  <button className="app-feature" style={{ '--app-color': featured.color } as React.CSSProperties} onClick={() => open(featured.id)}>
                    <span className="app-store-eyebrow" style={{ color: 'inherit', opacity: 0.75 }}>BIGGEST SPACE SAVER</span>
                    <span className="block text-[28px] font-bold tracking-tight leading-tight mt-1.5 max-w-[420px]">{featured.tagline}</span>
                    <span className="flex items-center gap-3.5 mt-auto pt-8">
                      <AppIcon app={featured} size={52} />
                      <span className="flex-1 min-w-0">
                        <span className="block text-[14px] font-semibold">{featured.name}</span>
                        <span className="block text-[12px] opacity-75 truncate">{featured.description}</span>
                      </span>
                      <span className="app-pill on-color">{formatBytes(featured.potential ?? 0)}</span>
                    </span>
                  </button>
                ) : (
                  <div className="app-feature placeholder">
                    <span className="app-store-eyebrow">MEASURING YOUR APPS</span>
                    <span className="block text-[28px] font-bold tracking-tight leading-tight mt-1.5">Finding space inside your apps…</span>
                    <Spinner className="mt-auto" />
                  </div>
                )}
                <div className="app-summary">
                  <span className="app-store-eyebrow">YOU COULD FREE UP</span>
                  <span className="block tnum text-[40px] font-bold tracking-tight leading-none mt-3">{loading && potential === 0 ? '—' : formatBytes(potential)}</span>
                  <span className="block text-[12px] mt-2" style={{ color: 'var(--muted)' }}>across {cards.filter((c) => (c.potential ?? 0) > 0).length} apps · nothing is removed until you choose</span>
                  <div className="app-summary-bar" aria-hidden="true">
                    {ranked.filter((c) => (c.potential ?? 0) > 0).map((c) => <span key={c.id} style={{ flexGrow: c.potential, background: c.color }} title={`${c.name} · ${formatBytes(c.potential ?? 0)}`} />)}
                  </div>
                  <div className="mt-3 space-y-1.5">
                    {ranked.filter((c) => (c.potential ?? 0) > 0).slice(0, 3).map((c) => (
                      <button key={c.id} className="w-full flex items-center gap-2 text-[12px] text-left" onClick={() => open(c.id)}>
                        <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ background: c.color }} />
                        <span className="flex-1 truncate" style={{ color: 'var(--muted)' }}>{c.name}</span>
                        <span className="tnum font-medium">{formatBytes(c.potential ?? 0)}</span>
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            )}

            <div className="flex items-baseline justify-between mt-9 mb-1">
              <h2 className="text-[20px] font-bold tracking-tight">{query ? 'Results' : 'Top space savers'}</h2>
              {loading && <span className="text-[12px] flex items-center gap-1.5" style={{ color: 'var(--muted)' }}><Spinner size={11} /> Measuring</span>}
            </div>
            <div className="app-chart">
              {visible.map((app, i) => (
                <button key={app.id} data-app={app.id} className="app-chart-row" onClick={() => open(app.id)}>
                  {!query && <span className="app-chart-rank tnum">{i + 1}</span>}
                  <AppIcon app={app} />
                  <span className="app-chart-body">
                    <span className="app-chart-text">
                      <span className="block text-[13.5px] font-semibold truncate">{app.name}</span>
                      <span className="block text-[12px] truncate mt-0.5" style={{ color: 'var(--muted)' }}>{app.description}</span>
                      <span className="block text-[11px] truncate mt-0.5" style={{ color: 'var(--dim)' }}>{app.category}</span>
                    </span>
                    <SizePill app={app} />
                  </span>
                </button>
              ))}
            </div>
            {!visible.length && <p className="text-center py-12 text-[13px]" style={{ color: 'var(--muted)' }}>No apps match “{query}”.</p>}
            <p className="mt-8 text-[11px] leading-relaxed max-w-[720px]" style={{ color: 'var(--dim)' }}>
              Sizes cover known locations only; shared APFS blocks may change the space you get back. Quit an app before cleaning it. More apps appear here after a disk scan.
            </p>
          </>
        )}
      </div>
    </div>
  );
};

function Measuring() {
  return <div className="app-store-section flex items-center gap-2 text-[12.5px]" style={{ color: 'var(--muted)' }}><Spinner /> Measuring storage…</div>;
}

function ProductPage({ card, loading, busy, onBack, onRefresh, children }: { card: Card; loading: boolean; busy: boolean; onBack: () => void; onRefresh: () => void; children: React.ReactNode }) {
  const profile = LIVE_APPS.find((a) => a.id === card.id);
  return (
    <div>
      <div className="flex items-center justify-between mb-5">
        <button className="btn btn-ghost !pl-1.5" onClick={onBack}><ChevronLeft className="w-4 h-4" /> Apps</button>
        <button className="btn btn-ghost btn-icon" title="Measure again" disabled={loading || busy} onClick={onRefresh}>{loading ? <Spinner /> : <RefreshCw className="w-4 h-4" />}</button>
      </div>
      <header className="flex items-center gap-6">
        <AppIcon app={card} size={112} />
        <div className="min-w-0 flex-1">
          <h1 className="text-[28px] font-bold tracking-tight leading-tight">{card.name}</h1>
          <p className="text-[14px] mt-0.5" style={{ color: 'var(--muted)' }}>{card.tagline}</p>
          <p className="text-[12px] mt-3" style={{ color: 'var(--dim)' }}>{card.description}</p>
        </div>
      </header>
      <div className="app-info-strip">
        <Info label="Can free up" value={card.error ? '—' : card.potential === undefined ? '…' : formatBytes(card.potential)} sub="after review" />
        <Info label="App storage" value={card.total === undefined ? '…' : `${card.partial ? '≥ ' : ''}${formatBytes(card.total)}`} sub={card.scope.toLowerCase()} />
        <Info label="Category" value={card.category} sub={profile ? 'measured live' : card.id === 'xcode' ? 'measured live' : 'from your last scan'} />
        <Info label="Items" value={card.count === undefined ? '…' : card.count} sub="you choose what goes" />
      </div>
      {profile && <div className="app-store-kept"><Check className="w-4 h-4 shrink-0 mt-px" /><p><strong>Always kept:</strong> {profile.kept}</p></div>}
      {children}
      {profile && (
        <section className="mt-10">
          <h2 className="text-[20px] font-bold tracking-tight mb-3">Good to know</h2>
          <div className="app-store-explainer">
            <div><h3>What’s taking up space</h3><p>{profile.what}</p></div>
            <div><h3>After cleanup</h3><p>{profile.consequence} Quarantine keeps a copy you can restore; permanent deletion skips the Trash.</p></div>
            <div><h3>Keep it under control</h3><p>{profile.prevention}</p></div>
          </div>
        </section>
      )}
    </div>
  );
}

function Info({ label, value, sub }: { label: string; value: React.ReactNode; sub: string }) {
  return (
    <div className="app-info">
      <span className="app-store-eyebrow">{label.toUpperCase()}</span>
      <span className="block text-[20px] font-bold tracking-tight tnum mt-1.5 truncate">{value}</span>
      <span className="block text-[11px] mt-1 truncate" style={{ color: 'var(--dim)' }}>{sub}</span>
    </div>
  );
}

type ListItem = { path: string; name: string; total: number; detail: string; blocked?: string | null; partial?: boolean };

/** A checklist of removable entries with one clean-up action at the bottom. */
function ItemList({ items, home, busy, color, onClean }: { items: ListItem[]; home: string; busy: boolean; color: string; onClean: (paths: ListItem[], mode: CleanupMode) => void }) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const available = items.filter((item) => !item.blocked);
  const chosen = available.filter((item) => selected.has(item.path));
  const toggle = (path: string) => setSelected((old) => { const next = new Set(old); next.has(path) ? next.delete(path) : next.add(path); return next; });
  return (
    <section className="mt-8">
      <div className="flex items-baseline justify-between mb-3">
        <h2 className="text-[20px] font-bold tracking-tight">What you can clean</h2>
        {available.length > 1 && (
          <button className="btn btn-ghost btn-sm" disabled={busy} onClick={() => setSelected(new Set(chosen.length === available.length ? [] : available.map((i) => i.path)))}>
            {chosen.length === available.length ? 'Select none' : 'Select all'}
          </button>
        )}
      </div>
      <div className="app-store-items">
        {items.map((item) => (
          <label key={item.path} className="app-store-item" style={{ opacity: item.blocked ? 0.65 : 1 }}>
            <input type="checkbox" aria-label={`Select ${item.name}`} disabled={busy || !!item.blocked} checked={selected.has(item.path)} onChange={() => toggle(item.path)} />
            {item.blocked && <LockKeyhole className="w-4 h-4 shrink-0" style={{ color: 'var(--dim)' }} />}
            <div className="flex-1 min-w-0">
              <div className="text-[13px] font-medium truncate">{item.name}</div>
              <p className="text-[11.5px] mt-0.5 truncate" style={{ color: item.blocked ? 'var(--warn)' : 'var(--muted)' }}>{item.blocked ?? item.detail}</p>
              <p className="text-[10.5px] mt-0.5 truncate selectable" title={item.path} style={{ color: 'var(--dim)' }}>{shortPath(item.path, home)}</p>
            </div>
            <span className="tnum text-[14px] font-semibold whitespace-nowrap">{item.partial ? '≥ ' : ''}{formatBytes(item.total)}</span>
            <button className="btn btn-ghost btn-icon" title="Show in Finder" aria-label={`Show ${item.name} in Finder`} onClick={(e) => { e.preventDefault(); api.reveal(item.path).catch((err) => useStore.getState().toast({ title: errorText(err), kind: 'error' })); }}><FolderOpen className="w-4 h-4" /></button>
          </label>
        ))}
        {!items.length && <p className="text-center p-10 text-[12.5px]" style={{ color: 'var(--muted)' }}>Nothing to clean here right now.</p>}
      </div>
      {items.length > 0 && (
        <div className="app-store-actions">
          <span className="text-[12.5px] flex-1" style={{ color: 'var(--muted)' }}>
            {chosen.length ? <><strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(sum(chosen))}</strong> selected from {chosen.length} item{chosen.length === 1 ? '' : 's'}</> : 'Select the items you want to remove.'}
          </span>
          <button className="btn btn-ghost" disabled={busy || !chosen.length} title="Delete permanently…" onClick={() => onClean(chosen, 'delete')}><Trash2 className="w-3.5 h-3.5" /></button>
          <button className="app-pill solid" style={{ '--app-color': color } as React.CSSProperties} disabled={busy || !chosen.length} onClick={() => onClean(chosen, 'quarantine')}>Clean up…</button>
        </div>
      )}
    </section>
  );
}

function AppDetail({ app, inventory, home, busy }: { app: AppProfile; inventory: AppStorageInventory; home: string; busy: boolean }) {
  const staged = (items: ListItem[]): StagedItem[] => items.map((item) => {
    const group = inventory.items.find((i) => i.path === item.path)?.group;
    return { path: item.path, name: item.name, total: item.total, kind: group === 'version' && app.id === 'claude' ? 'file' : 'directory', category: group === 'version' ? 'review' : 'rebuildable', reason: app.consequence, source: 'suggestion' };
  });
  return (
    <>
      {inventory.warnings.map((warning) => <p key={warning} role="status" className="text-[12px] mt-4" style={{ color: 'var(--warn)' }}>{warning}</p>)}
      <ItemList
        items={inventory.items.map((i) => ({ path: i.path, name: i.name, total: i.total, detail: i.subtitle, blocked: i.blocked, partial: i.partial }))}
        home={home}
        busy={busy}
        color={app.color}
        onClean={(items, mode) => clean(staged(items), mode)}
      />
    </>
  );
}

function ScannedDetail({ app, home, busy }: { app: (typeof APP_CLEANERS)[number] & { items: CandidateItem[] }; home: string; busy: boolean }) {
  return (
    <>
      <div className="app-store-section mt-6 text-[12.5px] leading-relaxed" style={{ color: 'var(--muted)' }}>{app.consequence} {app.before}</div>
      <ItemList
        items={app.items.map((i) => ({ path: i.path, name: i.name, total: i.total, detail: 'Cache · rebuilt when needed' }))}
        home={home}
        busy={busy}
        color={app.color}
        onClean={(items, mode) => clean(items.map((item) => ({ ...app.items.find((c) => c.path === item.path)!, source: 'suggestion' })), mode)}
      />
    </>
  );
}
