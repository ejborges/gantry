// The dashboard grid: twelve columns, rows of a fixed height, tiles that never overlap.

export interface Box {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}
export const COLUMNS = 12;
export const ROW_PX = 64;
export const GAP_PX = 12;

export const overlaps = (a: Box, b: Box) => a.id !== b.id && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;

/** Keep a tile inside the grid: at least one cell, never past the twelfth column. */
export function clamp<T extends Box>(t: T): T {
  const w = Math.min(COLUMNS, Math.max(1, Math.round(t.w)));
  const h = Math.min(12, Math.max(1, Math.round(t.h)));
  const x = Math.min(COLUMNS - w, Math.max(0, Math.round(t.x)));
  const y = Math.min(500, Math.max(0, Math.round(t.y)));
  return { ...t, x, y, w, h };
}

/**
 * Put `moved` where it was dropped and push whatever it now covers down, and whatever
 * those cover in turn. Returns every tile; the ones that changed are new objects.
 */
export function settle<T extends Box>(tiles: T[], moved: T): T[] {
  const placed = clamp(moved);
  const out = tiles.map((t) => (t.id === placed.id ? placed : t));
  const queue = [placed.id];
  let guard = 0;
  while (queue.length && guard++ < 2000) {
    const next = queue.shift();
    const pusher = out.find((t) => t.id === next);
    if (!pusher) continue;
    for (let i = 0; i < out.length; i++) {
      const t = out[i];
      if (t.id === placed.id || !overlaps(pusher, t)) continue;
      out[i] = { ...t, y: Math.min(500, pusher.y + pusher.h) };
      queue.push(t.id);
    }
  }
  return out;
}

/** Slide every tile up as far as it goes without passing another: closes the gaps. */
export function compact<T extends Box>(tiles: T[]): T[] {
  const done: T[] = [];
  for (const t of [...tiles].sort((a, b) => a.y - b.y || a.x - b.x)) {
    let y = 0;
    const blocked = (at: number) => done.some((d) => overlaps({ ...t, y: at }, d));
    while (blocked(y)) y++;
    done.push(y === t.y ? t : { ...t, y });
  }
  return tiles.map((t) => done.find((d) => d.id === t.id)!);
}

/** The first free place for a new tile of this size: beneath everything else. */
export function placeNew(tiles: Box[], w: number, h: number): { x: number; y: number; w: number; h: number } {
  const bottom = Math.max(0, ...tiles.map((t) => t.y + t.h));
  // Beside the last row if there is room, otherwise on a new one.
  const last = tiles.filter((t) => t.y + t.h === bottom);
  const right = Math.max(0, ...last.map((t) => t.x + t.w));
  const top = last.length ? Math.min(...last.map((t) => t.y)) : bottom;
  const beside = { id: "", x: right, y: top, w, h };
  if (last.length && right + w <= COLUMNS && !tiles.some((t) => overlaps(beside, { ...t, id: t.id || "x" }))) return { x: right, y: top, w, h };
  return { x: 0, y: bottom, w: Math.min(COLUMNS, w), h };
}

/** Which tiles differ in place or size between two layouts. */
export function changed<T extends Box>(before: T[], after: T[]): T[] {
  return after.filter((a) => {
    const b = before.find((t) => t.id === a.id);
    return !b || b.x !== a.x || b.y !== a.y || b.w !== a.w || b.h !== a.h;
  });
}
