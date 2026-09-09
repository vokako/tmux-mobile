import { backendIcon, paneAgent } from '../core/agents.ts';
import type { TmuxPane } from '../core/ws.ts';
import type { ProjectRow } from '../projects/projects.ts';

export type SidebarPane = Pick<TmuxPane,
  'session' | 'window' | 'window_name' | 'active' | 'current_command' | 'pane_title' | 'child_cmd'>;

interface RowAgent {
  icon: string | null;
  name: string;
  state: string;
}

/** Live rows show active agent windows; closed rows show declared agent slots.
 * The four-chip cap is presentation, not the full roster count. */
export function rowAgents(row: ProjectRow, panes: readonly SidebarPane[], agentStates: Readonly<Record<string, string>>): RowAgent[] {
  if (row.live) {
    const out: RowAgent[] = [];
    const seen = new Set<number>();
    for (const p of panes) {
      if (p.session !== row.project.session || !p.active || seen.has(p.window)) continue;
      seen.add(p.window);
      const agent = paneAgent(p);
      if (!agent) continue;
      out.push({
        icon: agent.icon, name: p.window_name,
        state: agentStates[`${row.project.session}:${p.window_name}`] ?? 'idle',
      });
    }
    return out.slice(0, 4);
  }
  return (row.slots ?? [])
    .filter((s) => s.kind === 'agent')
    .slice(0, 4)
    .map((s) => ({ icon: backendIcon(s.command), name: s.window_name, state: '' }));
}

/** Hover counts the whole roster, not the four visible chips. */
export function rowAgentCounts(row: ProjectRow, panes: readonly SidebarPane[]) {
  const declared = (row.slots ?? []).filter((x) => String(x.kind ?? '').toLowerCase() === 'agent').map((x) => x.window_name);
  if (!row.live) return { live: 0, stopped: declared.length };
  const running = new Set<string>();
  for (const p of panes) {
    if (p.session === row.project.session && p.active && paneAgent(p)) running.add(p.window_name);
  }
  return { live: running.size, stopped: declared.filter((n) => !running.has(n)).length };
}
