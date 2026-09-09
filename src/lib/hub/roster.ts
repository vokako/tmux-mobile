interface TeamMember {
  team?: string | null;
}

interface TeamNode<T> {
  team: string | null;
  path: string;
  agents: T[];
  children: TeamNode<T>[];
  order?: TeamNode<T>[];
}

export interface RosterGroup<T> extends TeamNode<T> {
  items: Array<T | RosterGroup<T>>;
}

/** Preserve first appearance across solo agents, teams and nested team paths. */
export function groupRoster<T extends TeamMember>(managedAgents: readonly T[]): Array<T | RosterGroup<T>> {
  const root: TeamNode<T> = { team: null, path: '', agents: [], children: [] };
  const nodeFor = (path: string) => {
    let node = root;
    let acc = '';
    for (const seg of path.split('/')) {
      acc = acc ? `${acc}/${seg}` : seg;
      let child = node.children.find((c) => c.path === acc);
      if (!child) { child = { team: seg, path: acc, agents: [], children: [] }; node.children.push(child); node.order = node.order ?? []; node.order.push(child); }
      node = child;
    }
    return node;
  };
  const items = new Map<TeamNode<T>, Array<T | TeamNode<T>>>([[root, []]]);
  for (const a of managedAgents) {
    const node = a.team ? nodeFor(a.team) : root;
    if (!items.has(node)) items.set(node, []);
    if (node !== root) {
      let parent = root;
      let acc = '';
      const segs = node.path.split('/');
      for (let i = 0; i < segs.length; i++) {
        acc = acc ? `${acc}/${segs[i]}` : segs[i]!;
        const child = parent.children.find((c) => c.path === acc)!;
        if (!items.has(parent)) items.set(parent, []);
        if (!items.get(parent)!.includes(child)) items.get(parent)!.push(child);
        parent = child;
      }
    }
    items.get(node)!.push(a);
  }
  const build = (node: TeamNode<T>): Array<T | RosterGroup<T>> =>
    (items.get(node) ?? []).map((x) => ((x as TeamNode<T>).children ? { ...(x as TeamNode<T>), items: build(x as TeamNode<T>) } : x as T));
  return build(root);
}
