import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Archive, Box, Check, ChevronRight, Code2, Cpu, Eye, Folder, Info, RefreshCw, Search, Smartphone, Trash2 } from 'lucide-react';
import { api, errorText } from '../../lib/api';
import { formatBytes, shortPath } from '../../lib/format';
import { useStore } from '../../store';
import { Modal, Spinner } from '../ui';
import type { CleanupMode, StagedItem, XcodeInventory, XcodeItem } from '../../types';

const GROUPS = [
  { id: 'simulators', label: 'Simulator devices', icon: Smartphone, color: '#8b9cf8', description: 'Every virtual device, individually.', what: 'Each device is a separate virtual iPhone, iPad, Watch or other Apple device. Agents and test runs can create many copies of the same model and OS. Each copy has its own installed apps, databases, photos and settings.', consequence: 'Permanent deletion removes the selected device through Apple’s simctl, including all its apps and test data. The shared OS runtime stays installed. You can create a fresh device later, but its previous data will be gone.', advice: 'Review same-model copies and devices with unavailable runtimes. Matching model and OS does not mean identical data. Disko cannot infer which devices are single-use. Shut down selected devices and stop agents first.', recovery: 'Quarantine preserves the device folder for restoration. Keep Xcode, Simulator and agents closed while it is quarantined. Restore before using that device again; Disko will not overwrite a recreated device.' },
  { id: 'support', label: 'Device support', icon: Cpu, color: '#65c8c0', description: 'Symbols for your physical devices.', what: 'Xcode stores OS debug symbols and support files for physical devices you connect. Folder names identify the hardware and OS build. These are separate from Simulator devices and your phone’s backups.', consequence: 'Your phone and its data are unaffected. Debugging or interpreting old crash logs may need the matching symbols again. Xcode may prepare or download support on a future connection; older symbols may be harder to obtain.', advice: '“Older version” compares versions for the same hardware identifier. “Newest stored” means newest on this Mac, not necessarily the OS currently installed on your phone. Keep versions you still test or diagnose.', recovery: 'Restore from quarantine to keep the exact symbols. After permanent deletion, reconnect the matching device and let Xcode prepare support again where available.' },
  { id: 'derived', label: 'Derived Data', icon: Code2, color: '#e4bd79', description: 'Builds, indexes and project caches.', what: 'Xcode’s generated build products, indexes, module caches and downloaded package artifacts. Project folders are listed separately, so you can keep active work warm.', consequence: 'Your source projects remain in place. The next build and indexing pass can take longer, and package dependencies may need downloading again.', advice: 'Stop builds and close Xcode and build agents first. Select old project folders, or select all here when you want a fresh build cache.', recovery: 'Xcode recreates this data as projects build. Quarantine can restore the previous cache if needed.' },
  { id: 'mcp', label: 'XcodeBuildMCP', icon: Box, color: '#c49bf0', description: 'Workspace copies made for agents.', what: 'Data under ~/Library/Developer/XcodeBuildMCP, including cached project copies and build workspaces. Contents depend on your MCP setup and may include edits made by an agent.', consequence: 'Selected workspaces are removed. A future MCP session may recreate its copy or rebuild. Any changes that exist only in that copy are lost after permanent deletion.', advice: 'End the corresponding agent sessions. Inspect copies in Finder and confirm useful edits are saved in your original repository before removing them. Disko does not assume every copy is disposable.', recovery: 'Quarantine keeps an exact copy you can restore. Recreating a workspace after permanent deletion will not recover unsaved or uncommitted changes that existed only there.' },
  { id: 'caches', label: 'Xcode caches', icon: Folder, color: '#7cbded', description: 'Generated tools and simulator caches.', what: 'Cache entries in Xcode, xcodebuild and CoreSimulator cache locations. These are separate from simulator device data.', consequence: 'Tools recreate caches on demand. First launches and builds can be slower, and some content may need downloading.', advice: 'Quit Xcode and Simulator and stop builds before cleaning these entries.', recovery: 'Restore from quarantine or allow the owning tool to rebuild the cache.' },
  { id: 'archives', label: 'Archives', icon: Archive, color: '#dc9f9c', description: 'Release builds. Review carefully.', what: 'Dated groups of Xcode archives containing release builds and debug symbols. These can be important for distribution and diagnosing crashes from previously shipped versions.', consequence: 'Deleting an archive removes that local release build and its symbols. Rebuilding source may not reproduce the exact original binary or dSYM.', advice: 'Keep archives for shipped releases unless you have a verified backup of the binaries and symbols. Inspect each dated folder before cleanup.', recovery: 'Quarantine supports restoration. After permanent deletion, recovering the exact archive requires a separate backup.' },
];
const groupFor = (id: string) => GROUPS.find((g) => g.id === id) ?? GROUPS[0];
const asStaged = (item: XcodeItem): StagedItem => ({ path: item.path, name: item.name, kind: 'directory', total: item.total, category: ['derived', 'caches'].includes(item.group) ? 'rebuildable' : 'review', reason: groupFor(item.group).consequence, source: 'suggestion' });
const date = (value: number | string | null) => value ? new Date(typeof value === 'number' ? value * 1000 : value).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) : 'Unknown';

export const XcodeCleanupSection: React.FC<{ snapshot?: XcodeInventory; onRefresh?: () => void }> = ({ snapshot, onRefresh }) => {
  const home = useStore((s) => s.info?.home ?? '');
  const version = useStore((s) => s.dataVersion);
  const busy = useStore((s) => s.cleanupBusy);
  const [inventory, setInventory] = useState<XcodeInventory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [group, setGroup] = useState('simulators');
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<XcodeItem | string | null>(null);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    if (snapshot) { setInventory(snapshot); setLoading(false); setError(null); setSelected(new Set()); return; }
    let active = true;
    setLoading(true); setError(null);
    api.xcodeStorage().then((data) => { if (active) { setInventory(data); setSelected((old) => new Set([...old].filter((p) => data.items.some((i) => i.path === p && !i.blocked)))); } })
      .catch((e) => { if (active) setError(errorText(e)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [version, refresh, snapshot]);
  const items = inventory?.items ?? [];
  const activeGroup = groupFor(group);
  const totals = useMemo(() => Object.fromEntries(GROUPS.map((g) => [g.id, items.filter((i) => i.group === g.id).reduce((n, i) => n + i.total, 0)])), [inventory]);
  const visible = items.filter((i) => i.group === group && `${i.name} ${i.path} ${i.subtitle}`.toLowerCase().includes(query.toLowerCase()) && (filter === 'all' || i.badges.includes(filter)));
  const chosen = items.filter((i) => selected.has(i.path) && !i.blocked);
  const total = items.reduce((sum, i) => sum + i.total, 0);
  const selectedBytes = chosen.reduce((sum, i) => sum + i.total, 0);
  const toggle = (path: string) => setSelected((old) => { const next = new Set(old); next.has(path) ? next.delete(path) : next.add(path); return next; });
  const clean = useCallback((targets: XcodeItem[], mode: CleanupMode) => {
    if (!targets.length || useStore.getState().cleanupBusy) return;
    setDetail(null);
    useStore.setState({ cleanupSelection: targets.map(asStaged), cleanupMode: mode, cleanModalOpen: true });
  }, []);
  const detailItem = typeof detail === 'object' ? detail : null;
  const detailGroup = groupFor(typeof detail === 'string' ? detail : detailItem?.group ?? group);
  return <div className="px-5 pb-5">
    <div className="xcode-hero mt-3 mb-4 rounded-xl p-4 flex items-center gap-6">
      <div className="flex-1"><h2 className="text-[17px] font-semibold tracking-tight">Choose what stays. Reclaim the rest.</h2>
        <p className="text-[12px] mt-1" style={{ color: 'var(--muted)' }}>Review individual devices, OS versions and project workspaces.</p>
      </div>
      <div className="text-right flex-shrink-0"><div className="text-[29px] tracking-tight tnum">{loading && !inventory ? '—' : formatBytes(total)}</div><div className="text-[11px] mt-1" style={{ color: 'var(--muted)' }}>{items.length} entries · measured on disk</div></div>
      <button aria-label="Refresh Xcode storage" title="Refresh live storage" className="btn btn-icon" disabled={loading || busy} onClick={() => onRefresh ? onRefresh() : setRefresh((v) => v + 1)}>{loading ? <Spinner /> : <RefreshCw className="w-4 h-4" />}</button>
    </div>
    <div className="grid grid-cols-3 xl:grid-cols-6 gap-2.5 mb-5" aria-label="Storage categories">
      {GROUPS.map((g) => <button key={g.id} onClick={() => { setGroup(g.id); setFilter('all'); }} aria-pressed={group === g.id} className="card text-left p-3.5 transition-colors" style={{ borderColor: group === g.id ? g.color : undefined, background: group === g.id ? `color-mix(in srgb, ${g.color} 9%, var(--bg-2))` : undefined }}>
        <g.icon className="w-4 h-4 mb-3" style={{ color: g.color }} /><div className="text-[12px] font-medium">{g.label}</div><div className="tnum text-[20px] mt-1">{formatBytes(totals[g.id] ?? 0)}</div>
      </button>)}
    </div>
    {error && <div role="alert" className="card p-3 mb-3 text-[12px]" style={{ color: 'var(--danger)' }}>{error} <button className="underline" onClick={() => onRefresh ? onRefresh() : setRefresh((v) => v + 1)}>Retry</button></div>}
    {inventory?.warnings.map((warning) => <div key={warning} className="card p-3 mb-3 text-[12px]" style={{ color: 'var(--warn)' }}>{warning}</div>)}
    <div className="flex items-center gap-3 mb-3 flex-wrap">
      <div><h2 className="text-[16px] font-semibold">{activeGroup.label}</h2><p className="text-[11.5px] mt-0.5" style={{ color: 'var(--muted)' }}>{activeGroup.description}</p></div>
      <button className="btn btn-ghost btn-sm" onClick={() => setDetail(group)}><Info className="w-3.5 h-3.5" /> What happens if I delete these?</button>
      <div className="flex-1" />
      <label className="relative"><Search className="absolute left-2.5 top-2.5 w-3.5 h-3.5" style={{ color: 'var(--dim)' }} /><input aria-label="Filter Xcode storage" className="input !pl-8 !w-48" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Find a device or project…" /></label>
      {(group === 'simulators' || group === 'support') && <select className="input !w-auto" aria-label="Review filter" value={filter} onChange={(e) => setFilter(e.target.value)}>
        <option value="all">All entries</option>{(group === 'support' ? ['Older version', 'Newest stored'] : ['Same model & OS', 'Runtime unavailable', 'Shutdown']).map((v) => <option key={v}>{v}</option>)}
      </select>}
    </div>
    <div className="card overflow-hidden">
      <div className="flex items-center gap-3 px-4 py-2.5 text-[11px]" style={{ background: 'var(--bg-3)', color: 'var(--muted)' }}>
        <input type="checkbox" aria-label="Select all visible entries" disabled={loading || busy || !visible.some((i) => !i.blocked)} checked={visible.some((i) => !i.blocked) && visible.filter((i) => !i.blocked).every((i) => selected.has(i.path))} onChange={(e) => setSelected((old) => { const next = new Set(old); visible.filter((i) => !i.blocked).forEach((i) => e.target.checked ? next.add(i.path) : next.delete(i.path)); return next; })} />
        <span className="flex-1">{visible.length} entries · largest first</span><span>Size on disk</span>
      </div>
      {loading && !inventory ? <div className="flex justify-center gap-2 p-14 text-[12px]"><Spinner /> Measuring Xcode storage…</div> : visible.map((item) => <div key={item.path} className="xcode-row flex items-center gap-3 px-4 py-3.5" style={{ borderTop: '1px solid var(--line)', opacity: item.blocked ? 0.65 : 1 }}>
        <input type="checkbox" aria-label={`Select ${item.name}`} checked={selected.has(item.path)} disabled={!!item.blocked || busy || loading} onChange={() => toggle(item.path)} />
        <div className="w-10 h-12 rounded-xl flex items-center justify-center flex-shrink-0" style={{ background: `color-mix(in srgb, ${activeGroup.color} 12%, transparent)`, color: activeGroup.color }}><activeGroup.icon className="w-5 h-5" /></div>
        <button className="flex-1 min-w-0 text-left" onClick={() => setDetail(item)}>
          <div className="text-[13px] font-semibold truncate">{item.name}</div>
          <div className="text-[11.5px] mt-0.5 truncate" style={{ color: 'var(--muted)' }}>{item.subtitle || shortPath(item.path, home)}</div>
          <div className="flex gap-1.5 flex-wrap mt-1.5">{item.badges.map((badge) => <span key={badge} className="chip !text-[9px] !py-0" style={{ color: badge === 'Older version' || badge === 'Runtime unavailable' ? 'var(--warn)' : undefined }}>{badge}</span>)}<span className="text-[10px]" style={{ color: 'var(--dim)' }}>{item.group === 'simulators' ? `Last boot: ${date(item.last_booted)}` : `Modified ${date(item.modified)}`}</span></div>
          {item.blocked && <div className="text-[10px] mt-1" style={{ color: 'var(--warn)' }}>{item.blocked}</div>}
        </button>
        <div className="text-right"><div className="tnum text-[16px] font-medium">{item.partial ? '≥ ' : ''}{formatBytes(item.total)}</div><div className="mt-1 h-1 w-20 rounded-full ml-auto" style={{ background: 'var(--bg-3)' }}><div className="h-full rounded-full" style={{ width: `${Math.max(2, item.total / Math.max(1, totals[group]) * 100)}%`, background: activeGroup.color }} /></div></div>
        <button className="btn btn-ghost btn-icon" aria-label={`Inspect ${item.name}`} onClick={() => setDetail(item)}><ChevronRight className="w-4 h-4" /></button>
        <button className="btn btn-ghost btn-icon" aria-label={`Delete ${item.name}`} title="Delete permanently…" disabled={!!item.blocked || busy || loading} onClick={() => clean([item], 'delete')}><Trash2 className="w-3.5 h-3.5" /></button>
      </div>)}
      {!loading && visible.length === 0 && <div className="p-12 text-center text-[12px]" style={{ color: 'var(--muted)' }}>{items.some((i) => i.group === group) ? 'No entries match this filter.' : 'No entries found in these locations.'}</div>}
    </div>
    <div className="mt-3 text-[11px] flex gap-2" style={{ color: 'var(--dim)' }}><Info className="w-3.5 h-3.5 flex-shrink-0" /><p>Live inventory of standard locations. Sizes are estimates; shared APFS blocks and snapshots can change the space reclaimed. Modified dates do not prove inactivity. Shared OS runtimes are managed in Xcode → Settings → Components.</p></div>
    {chosen.length > 0 && <div className="sticky bottom-0 mt-4 panel-raised px-4 py-3 flex items-center gap-3">
      <Check className="w-4 h-4" /><span className="text-[12px] flex-1">{chosen.length} selected across categories <span className="tnum font-semibold">· {formatBytes(selectedBytes)}</span></span>
      <button className="btn btn-ghost btn-sm" disabled={busy} onClick={() => setSelected(new Set())}>Clear</button>
      <button className="btn" disabled={busy || loading || !!error} onClick={() => clean(chosen, 'quarantine')}><Archive className="w-3.5 h-3.5" /> Quarantine…</button>
      <button className="btn btn-primary" disabled={busy || loading || !!error} onClick={() => clean(chosen, 'delete')}><Trash2 className="w-3.5 h-3.5" /> Delete…</button>
    </div>}
    <Modal open={detail !== null} onClose={() => setDetail(null)} title={detailItem?.name ?? detailGroup.label} subtitle={detailItem ? `${formatBytes(detailItem.total)} · ${detailItem.subtitle}` : detailGroup.description} width={620}
      footer={detailItem ? <><button className="btn btn-ghost" onClick={() => api.reveal(detailItem.path).catch((e) => useStore.getState().toast({ kind: 'error', title: 'Could not reveal item', detail: errorText(e) }))}><Eye className="w-3.5 h-3.5" /> Finder</button><button className="btn" disabled={!!detailItem.blocked || busy || loading} onClick={() => clean([detailItem], 'quarantine')}>Quarantine…</button><button className="btn btn-primary" disabled={!!detailItem.blocked || busy || loading} onClick={() => clean([detailItem], 'delete')}>Delete permanently…</button></> : <button className="btn" onClick={() => setDetail(null)}>Done</button>}>
      {detailItem && <p className="mono text-[11px] break-all selectable mb-4" style={{ color: 'var(--muted)' }}>{shortPath(detailItem.path, home)}</p>}
      {[['What it is', detailGroup.what], ['What deletion changes', detailGroup.consequence], ['What to keep', detailGroup.advice], ['Getting it back', detailGroup.recovery]].map(([title, body]) => <div key={title} className="mb-4"><h3 className="text-[12px] font-semibold mb-1">{title}</h3><p className="text-[12px] leading-relaxed" style={{ color: 'var(--muted)' }}>{body}</p></div>)}
      {detailItem?.partial && <p className="text-[12px]" style={{ color: 'var(--warn)' }}>Some files could not be read. The displayed size is a lower bound.</p>}
      {detailItem?.blocked && <p className="text-[12px]" style={{ color: 'var(--warn)' }}>{detailItem.blocked}</p>}
    </Modal>
  </div>;
};
