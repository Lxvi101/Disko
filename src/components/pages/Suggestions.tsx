import React, { useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Sparkles, Plus, Check, Eye, Zap, Info, Bot, Folder, File, ChevronRight, Terminal, RefreshCw, Ghost, ChevronDown } from 'lucide-react';
import { useStore } from '../../store';
import { api } from '../../lib/api';
import { catMeta, formatBytes, shortPath, tagLabel, COST_LABELS, formatDate } from '../../lib/format';
import { CategoryChip, Empty, Spinner } from '../ui';
import type { CandidateItem, RemnantGroup, RemnantMember, StagedItem } from '../../types';
import { openItemMenu } from '../ContextMenu';

type Group = 'rebuildable' | 'review' | 'leftovers';

const MEMBER_KIND: Record<string, string> = {
  cache: 'Cache',
  http: 'HTTP cache',
  'saved-state': 'Window state',
  logs: 'Logs',
  webkit: 'Web storage',
  state: 'Application Support',
  container: 'Container',
  group: 'Group container',
  prefs: 'Preferences',
  scripts: 'Scripts',
  cookies: 'Cookies',
};

function toStaged(c: CandidateItem): StagedItem {
  return { path: c.path, name: c.name, kind: c.kind, total: c.total, category: c.category, reason: c.reason, tag: c.tag, action: c.action, tool: c.tool, source: 'suggestion' };
}

function memberToStaged(g: RemnantGroup, m: RemnantMember): StagedItem {
  const kind = m.kind === 'prefs' || m.kind === 'cookies' ? 'file' : 'directory';
  const persistent = m.persistent ? ' May hold settings or documents of that app.' : '';
  return {
    path: m.path,
    name: m.path.split('/').filter(Boolean).pop() ?? m.path,
    kind,
    total: m.total,
    category: m.persistent ? 'review' : 'rebuildable',
    reason: `${MEMBER_KIND[m.kind] ?? m.kind} left behind by ${g.id}, which is no longer installed.${persistent}`,
    tag: 'orphaned-app',
    action: 'quarantine',
    source: 'remnant',
  };
}

export const SuggestionsPage: React.FC = () => {
  const { candidates, remnants, isStaged, toggleStage, stageMany, staged, info, navigate, setAssistantOpen } = useStore();
  const [group, setGroup] = useState<Group>('rebuildable');
  const [q, setQ] = useState('');
  const [rebuilding, setRebuilding] = useState(false);

  const grouped = useMemo(() => {
    const s = q.toLowerCase();
    const filter = (c: CandidateItem) => !s || c.path.toLowerCase().includes(s);
    return {
      rebuildable: candidates.filter((c) => c.category === 'rebuildable' && filter(c)),
      review: candidates.filter((c) => c.category === 'review' && filter(c)),
    };
  }, [candidates, q]);

  const leftovers = useMemo(() => {
    const s = q.toLowerCase();
    return (remnants?.groups ?? []).filter((g) => !s || g.id.includes(s) || g.members.some((m) => m.path.toLowerCase().includes(s)));
  }, [remnants, q]);
  const leftoverBytes = leftovers.reduce((a, g) => a + g.total, 0);

  const list = group === 'leftovers' ? [] : grouped[group];
  const collectable = list.filter((c) => c.action === 'quarantine');
  const listBytes = collectable.reduce((a, b) => a + b.total, 0);
  const allStaged = collectable.length > 0 && collectable.every((c) => isStaged(c.path));

  const stageAll = () => {
    if (group === 'leftovers') {
      const items = leftovers.flatMap((g) => g.members.filter((m) => m.preselect).map((m) => memberToStaged(g, m)));
      stageMany(items);
      return;
    }
    stageMany(collectable.map(toStaged));
  };
  const leftoverPreselect = leftovers.flatMap((g) => g.members.filter((m) => m.preselect));
  const leftoverAllStaged = leftoverPreselect.length > 0 && leftoverPreselect.every((m) => isStaged(m.path));

  const rebuildRemnants = async () => {
    setRebuilding(true);
    try {
      const r = await api.remnants(true);
      useStore.setState({ remnants: r });
    } catch {}
    setRebuilding(false);
  };

  const askPlan = () => {
    setAssistantOpen(true);
    window.dispatchEvent(
      new CustomEvent('disko:ask', {
        detail:
          'Build me a cleanup plan. Read candidates.json and remnants.json and query the scan database for the largest folders. Rank what I should remove first by size and safety using the decision model: clearly disposable, likely abandoned, needs confirmation. Give the recreate cost for each, use manager commands where they are safer than moving a folder, explain the trade-offs briefly, and list the exact paths in a disko-actions block.',
      }),
    );
  };

  if (candidates.length === 0 && !remnants) {
    return (
      <Empty
        icon={<Sparkles className="w-6 h-6" />}
        title="No suggestions yet"
        body="Run a scan from the sidebar. Disko will look for rebuildable caches, build output, large review-worthy folders, and leftovers of uninstalled apps."
      />
    );
  }

  const tabs: { id: Group; label: string; bytes: number; color: string; soft: string; ring: string; text: string }[] = [
    { id: 'rebuildable', ...catMeta('rebuildable'), bytes: grouped.rebuildable.reduce((a, b) => a + b.total, 0) },
    { id: 'review', ...catMeta('review'), bytes: grouped.review.reduce((a, b) => a + b.total, 0) },
    { id: 'leftovers', ...catMeta('other'), label: 'Leftovers', bytes: leftoverBytes, color: '#a78bfa', text: '#c4b5fd', soft: 'rgba(167, 139, 250, 0.14)', ring: 'rgba(167, 139, 250, 0.35)' },
  ];

  return (
    <div className="h-full flex flex-col min-h-0">
      <div className="px-6 pt-4 pb-3 flex items-center gap-3 flex-shrink-0">
        <div className="inline-flex items-center rounded-xl p-0.5 glass">
          {tabs.map((t) => {
            const active = group === t.id;
            return (
              <button key={t.id} onClick={() => setGroup(t.id)} className="relative px-3 py-1.5 rounded-[10px] text-[12.5px] font-medium flex items-center gap-2">
                {active && <motion.span layoutId="sugg-tab" className="absolute inset-0 rounded-[10px]" style={{ background: t.soft, border: `1px solid ${t.ring}` }} transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
                <span className="relative flex items-center gap-2" style={{ color: active ? t.text : 'var(--muted)' }}>
                  <span className="w-2 h-2 rounded-full" style={{ background: t.color }} />
                  {t.label}
                  <span className="mono text-[11px] opacity-80">{formatBytes(t.bytes)}</span>
                </span>
              </button>
            );
          })}
        </div>
        <input className="input !py-1.5 !text-[12.5px] max-w-xs" placeholder="Filter by path…" value={q} onChange={(e) => setQ(e.target.value)} />
        <div className="ml-auto flex items-center gap-2">
          <button className="btn" onClick={askPlan}>
            <Bot className="w-3.5 h-3.5" style={{ color: 'var(--text)' }} /> Ask Disko for a plan
          </button>
          {group === 'leftovers' ? (
            <>
              <button className="btn btn-ghost btn-icon" title="Look again" onClick={rebuildRemnants} disabled={rebuilding}>
                {rebuilding ? <Spinner /> : <RefreshCw className="w-3.5 h-3.5" />}
              </button>
              <button className="btn" onClick={stageAll} disabled={leftoverPreselect.length === 0 || leftoverAllStaged}>
                {leftoverAllStaged ? <><Check className="w-3.5 h-3.5" /> Caches collected</> : <><Plus className="w-3.5 h-3.5" /> Collect caches · {formatBytes(leftoverPreselect.reduce((a, m) => a + m.total, 0))}</>}
              </button>
            </>
          ) : (
            <button className="btn" onClick={stageAll} disabled={collectable.length === 0 || allStaged}>
              {allStaged ? <><Check className="w-3.5 h-3.5" /> All collected</> : <><Plus className="w-3.5 h-3.5" /> Collect all · {formatBytes(listBytes)}</>}
            </button>
          )}
        </div>
      </div>

      <div className="px-6 pb-3 flex-shrink-0">
        <div className="card px-4 py-2.5 flex items-center gap-2.5 text-[12px]" style={{ color: 'var(--muted)' }}>
          {group === 'rebuildable' ? <Zap className="w-4 h-4 flex-shrink-0" style={{ color: '#34d399' }} /> : group === 'review' ? <Info className="w-4 h-4 flex-shrink-0" style={{ color: '#fbbf24' }} /> : <Ghost className="w-4 h-4 flex-shrink-0" style={{ color: '#a78bfa' }} />}
          {group === 'rebuildable'
            ? 'Known cache and build-output locations that apps recreate on demand. Quit the owning app first. Where a tool manages the store, its command is shown; it knows what is still referenced.'
            : group === 'review'
              ? 'Large folders that need a decision: downloads, environments, runtimes, backups. Each shows what it would cost to recreate. Inspect them, or ask the assistant.'
              : 'Data left behind by apps whose bundle identifier is no longer installed anywhere on this Mac. Caches are safe to collect; Application Support and containers may hold settings, so they are never preselected.'}
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-6 pb-6">
        {group === 'leftovers' ? (
          <LeftoversList groups={leftovers} agents={remnants?.agents ?? []} home={info?.home ?? ''} isStaged={isStaged} onToggle={(g, m) => toggleStage(memberToStaged(g, m))} onToggleAgent={(a) => toggleStage({ path: a.path, name: a.path.split('/').pop() ?? a.path, kind: 'file', total: 0, category: 'review', reason: a.reason, tag: 'orphaned-service', action: 'quarantine', source: 'remnant' })} loaded={!!remnants} />
        ) : (
          <div className="grid grid-cols-1 gap-2">
            <AnimatePresence initial={false}>
              {list.map((c, i) => {
                const s = isStaged(c.path);
                const m = catMeta(c.category);
                const managed = c.action !== 'quarantine';
                return (
                  <motion.div
                    key={c.path}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, scale: 0.97 }}
                    transition={{ delay: Math.min(i * 0.02, 0.3), type: 'spring', stiffness: 300, damping: 28 }}
                    className="card card-hover p-3.5 flex items-start gap-3 group"
                    style={s ? { opacity: 0.55 } : undefined}
                    onContextMenu={(e) => openItemMenu(e, toStaged(c))}
                  >
                    <span className="w-9 h-9 rounded-xl flex items-center justify-center flex-shrink-0 mt-0.5" style={{ background: m.soft, color: m.text }}>
                      {c.kind === 'directory' ? <Folder className="w-4 h-4" /> : <File className="w-4 h-4" />}
                    </span>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 min-w-0">
                        <span className="text-[13px] font-semibold truncate min-w-0">{c.name}</span>
                        <CategoryChip category={c.category} small />
                        {c.tag && <span className="chip" style={{ fontSize: 10 }}>{tagLabel(c.tag)}</span>}
                      </div>
                      <div className="mono text-[10.5px] truncate mt-0.5 selectable" style={{ color: 'var(--dim)' }} title={c.path}>
                        {shortPath(c.path, info?.home ?? '')}
                      </div>
                      <p className="text-[11.5px] leading-snug mt-1.5" style={{ color: 'var(--muted)' }}>{c.reason}</p>
                      <div className="flex items-center gap-3 mt-1 flex-wrap">
                        {c.cost && c.cost !== 'unknown' && (
                          <span className="text-[11px] flex items-center gap-1" style={{ color: c.cost === 'irreplaceable' ? 'var(--danger)' : c.cost === 'network-large' ? 'var(--warn)' : '#6ee7b7' }}>
                            <Zap className="w-3 h-3" /> {COST_LABELS[c.cost] ?? c.cost}
                          </span>
                        )}
                        {managed && c.tool && (
                          <span className="mono text-[10.5px] px-2 py-0.5 rounded-md selectable flex items-center gap-1.5" style={{ background: 'var(--bg-3)' }} title="This tool knows what is still referenced; prefer it over moving the folder.">
                            <Terminal className="w-3 h-3" style={{ color: 'var(--muted)' }} /> {c.tool}
                          </span>
                        )}
                        {managed && !c.tool && (
                          <span className="text-[11px]" style={{ color: 'var(--muted)' }}>Remove inside the owning app.</span>
                        )}
                      </div>
                    </div>
                    <div className="flex flex-col items-end gap-2 flex-shrink-0">
                      <span className="mono text-[15px] font-semibold">{formatBytes(c.total)}</span>
                      <div className="flex items-center gap-1">
                        {c.kind === 'directory' && (
                          <button className="btn btn-ghost btn-icon opacity-0 group-hover:opacity-100 transition-opacity" title="Open in explorer" onClick={() => navigate(c.path)}>
                            <ChevronRight className="w-3.5 h-3.5" />
                          </button>
                        )}
                        <button className="btn btn-ghost btn-icon opacity-0 group-hover:opacity-100 transition-opacity" title="Reveal in Finder" onClick={() => api.reveal(c.path).catch(() => {})}>
                          <Eye className="w-3.5 h-3.5" />
                        </button>
                        <button className={managed ? 'btn btn-ghost btn-sm' : 'btn btn-sm'} title={managed ? 'Move the folder anyway instead of using the tool' : undefined} onClick={() => toggleStage(toStaged(c))}>
                          {s ? <><Check className="w-3 h-3" /> Collected</> : <><Plus className="w-3 h-3" /> {managed ? 'Collect anyway' : 'Collect'}</>}
                        </button>
                      </div>
                    </div>
                  </motion.div>
                );
              })}
            </AnimatePresence>
            {list.length === 0 && (
              <div className="py-16 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>
                Nothing in this group{q ? ' matches your filter' : ''}.
              </div>
            )}
          </div>
        )}
        {staged.length > 0 && (
          <p className="text-[11px] mt-4 text-center" style={{ color: 'var(--dim)' }}>
            {staged.length} item{staged.length === 1 ? '' : 's'} collected. Review them in the collector at the bottom.
          </p>
        )}
      </div>
    </div>
  );
};

const LeftoversList: React.FC<{
  groups: RemnantGroup[];
  agents: { path: string; label: string; program: string; reason: string }[];
  home: string;
  loaded: boolean;
  isStaged: (p: string) => boolean;
  onToggle: (g: RemnantGroup, m: RemnantMember) => void;
  onToggleAgent: (a: { path: string; label: string; program: string; reason: string }) => void;
}> = ({ groups, agents, home, loaded, isStaged, onToggle, onToggleAgent }) => {
  const [open, setOpen] = useState<Record<string, boolean>>({});
  if (!loaded) {
    return (
      <div className="py-16 flex items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--dim)' }}>
        <Spinner /> Comparing installed apps with Library…
      </div>
    );
  }
  if (groups.length === 0 && agents.length === 0) {
    return (
      <div className="py-16 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>
        No leftovers found. Every bundle identifier in Library belongs to an installed app.
      </div>
    );
  }
  return (
    <div className="grid grid-cols-1 gap-2">
      {groups.map((g) => {
        const likely = g.confidence === 'likely';
        const expanded = open[g.id] ?? false;
        const stagedCount = g.members.filter((m) => isStaged(m.path)).length;
        return (
          <div key={g.id} className="card p-3.5">
            <div className="flex items-start gap-3">
              <span className="w-9 h-9 rounded-xl flex items-center justify-center flex-shrink-0 mt-0.5" style={{ background: 'rgba(167, 139, 250, 0.14)', color: '#c4b5fd' }}>
                <Ghost className="w-4 h-4" />
              </span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-[13px] font-semibold truncate min-w-0">{g.name}</span>
                  <span className="chip" style={{ fontSize: 10, color: likely ? '#c4b5fd' : 'var(--warn)' }}>{likely ? 'Likely uninstalled' : 'Needs confirmation'}</span>
                  {stagedCount > 0 && <span className="chip" style={{ fontSize: 10 }}>{stagedCount} collected</span>}
                </div>
                <div className="mono text-[10.5px] truncate mt-0.5 selectable" style={{ color: 'var(--dim)' }}>{g.id}</div>
                <ul className="text-[11.5px] leading-snug mt-1.5 space-y-0.5" style={{ color: 'var(--muted)' }}>
                  {g.evidence.slice(0, expanded ? undefined : 2).map((e) => (
                    <li key={e}>· {e}</li>
                  ))}
                </ul>
                {g.newest_mtime ? <div className="text-[11px] mt-1" style={{ color: 'var(--dim)' }}>Last changed {formatDate(g.newest_mtime)}</div> : null}
              </div>
              <div className="flex flex-col items-end gap-2 flex-shrink-0">
                <span className="mono text-[15px] font-semibold">{formatBytes(g.total)}</span>
                <button className="btn btn-ghost btn-sm" onClick={() => setOpen({ ...open, [g.id]: !expanded })}>
                  {g.members.length} item{g.members.length === 1 ? '' : 's'} <ChevronDown className="w-3 h-3 transition-transform" style={{ transform: expanded ? 'rotate(180deg)' : undefined }} />
                </button>
              </div>
            </div>
            <AnimatePresence initial={false}>
              {expanded && (
                <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: 0.16 }} className="overflow-hidden">
                  <div className="mt-3 pt-2" style={{ borderTop: '1px solid var(--line)' }}>
                    {g.members.map((m) => {
                      const s = isStaged(m.path);
                      return (
                        <div key={m.path} className="group flex items-center gap-2.5 px-1 h-8 rounded-lg row-hover" onContextMenu={(e) => openItemMenu(e, memberToStaged(g, m))}>
                          <span className="chip flex-shrink-0" style={{ fontSize: 10, color: m.persistent ? 'var(--warn)' : '#6ee7b7' }}>{MEMBER_KIND[m.kind] ?? m.kind}</span>
                          <span className="mono text-[10.5px] truncate flex-1 min-w-0 selectable" title={m.path}>{shortPath(m.path, home)}</span>
                          <span className="tnum text-[11.5px] flex-shrink-0" style={{ color: 'var(--muted)' }}>{m.total ? formatBytes(m.total) : '—'}</span>
                          <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100" title="Reveal in Finder" onClick={() => api.reveal(m.path).catch(() => {})}><Eye className="w-3.5 h-3.5" /></button>
                          <button className="btn btn-ghost btn-icon !p-1" title={m.persistent ? 'May hold settings or documents; collect only if you are sure' : 'Collect'} style={s ? { color: 'var(--text)' } : undefined} onClick={() => onToggle(g, m)}>{s ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}</button>
                        </div>
                      );
                    })}
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        );
      })}
      {agents.length > 0 && (
        <div className="card p-3.5 mt-2">
          <div className="text-[12.5px] font-semibold mb-1">Launch agents whose program is gone</div>
          <p className="text-[11.5px] mb-2" style={{ color: 'var(--muted)' }}>launchd keeps trying to start these. They are tiny, but removing them stops the failures.</p>
          {agents.map((a) => {
            const s = isStaged(a.path);
            return (
              <div key={a.path} className="group flex items-center gap-2.5 px-1 h-8 rounded-lg row-hover" onContextMenu={(e) => openItemMenu(e, { path: a.path, name: a.path.split('/').pop() ?? a.path, kind: 'file', total: 0, category: 'review', reason: a.reason, tag: 'orphaned-service', action: 'quarantine', source: 'remnant' })}>
                <span className="text-[12px] truncate flex-1 min-w-0">{a.label}</span>
                <span className="mono text-[10.5px] truncate flex-1 min-w-0" style={{ color: 'var(--dim)' }} title={a.program}>{shortPath(a.program, home)}</span>
                <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100" title="Reveal in Finder" onClick={() => api.reveal(a.path).catch(() => {})}><Eye className="w-3.5 h-3.5" /></button>
                <button className="btn btn-ghost btn-icon !p-1" style={s ? { color: 'var(--text)' } : undefined} onClick={() => onToggleAgent(a)}>{s ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}</button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
