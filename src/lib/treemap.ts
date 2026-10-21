import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy';
import { BRANCH_HUES, compareArcs, ownBytes, type Datum } from './sunburst';

/** Height of the name band along the top of a folder block. */
export const HEADER = 17;

export type Block = HierarchyRectangularNode<Datum> & { hue: number; header: number };
export interface BlockLayout {
  nodes: Block[];
  byPath: Map<string, Block>;
  root: Block;
  width: number;
  height: number;
}

/** Nested block view: every folder is a frame holding its contents, sized by bytes. */
export function layoutBlocks(datum: Datum, width: number, height: number): BlockLayout {
  const root = hierarchy(datum).sum(ownBytes).sort(compareArcs) as unknown as Block;
  // d3 positions a node before asking for its padding, so the band is only reserved
  // where it can hold a readable label; smaller folders keep a thin frame.
  const top = (n: HierarchyRectangularNode<Datum>) => {
    const b = n as Block;
    b.header = n.depth === 0 ? 0 : n.x1 - n.x0 > 44 && n.y1 - n.y0 > HEADER * 2.4 ? HEADER : 2;
    return b.header;
  };
  const side = (n: HierarchyRectangularNode<Datum>) => (n.depth === 0 ? 0 : 2);
  treemap<Datum>()
    .tile(treemapSquarify.ratio(1.2))
    .size([Math.max(1, width), Math.max(1, height)])
    .paddingInner(1)
    .paddingTop(top)
    .paddingLeft(side)
    .paddingRight(side)
    .paddingBottom(side)(root);

  const nodes: Block[] = [];
  const byPath = new Map<string, Block>();
  root.each((n) => {
    const b = n as Block;
    b.header ??= 0;
    if (b.depth === 0 || b.x1 - b.x0 < 1 || b.y1 - b.y0 < 1) return;
    let branch: Block = b;
    while (branch.parent && branch.depth > 1) branch = branch.parent as Block;
    const rank = root.children?.indexOf(branch) ?? 0;
    const sibling = b.parent?.children?.indexOf(b) ?? 0;
    b.hue = (BRANCH_HUES[rank % BRANCH_HUES.length] + (b.depth > 1 ? ((sibling % 5) - 2) * 6 : 0) + 360) % 360;
    nodes.push(b);
    byPath.set(b.data.path, b);
  });
  // Parents first, so every folder is painted beneath its contents.
  nodes.sort((a, b) => a.depth - b.depth);
  return { nodes, byPath, root, width, height };
}

/** Deepest drawn block under a point. */
export function blockAt(layout: BlockLayout, x: number, y: number): Block | null {
  let found: Block | null = null;
  let n: Block | undefined = layout.root;
  while (n) {
    const next: Block | undefined = (n.children as Block[] | undefined)?.find((c) => x >= c.x0 && x < c.x1 && y >= c.y0 && y < c.y1 && layout.byPath.get(c.data.path) === c);
    if (next) found = next;
    n = next;
  }
  return found;
}
