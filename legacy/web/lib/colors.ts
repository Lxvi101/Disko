/** Sunburst colouring: hue follows the angle on the wheel, depth lightens. */

export function hsl(h: number, s: number, l: number): string {
  return `hsl(${Math.round(h)} ${Math.round(s * 100)}% ${Math.round(l * 100)}%)`;
}

/** Hue for an arc whose mid angle is `angle` radians (0 = top, clockwise). */
export function hueForAngle(angle: number): number {
  const t = ((angle % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
  // Start at violet at 12 o'clock, run through magenta, red, orange, yellow, green, cyan, blue.
  return (265 + (t / (2 * Math.PI)) * 320) % 360;
}

export function folderColor(hue: number, depth: number, light = false): string {
  // Soft, glassy tones that sit with the rest of the app; deeper rings get lighter and calmer.
  if (light) {
    const l = Math.min(0.6 + (depth - 1) * 0.05, 0.82);
    const s = Math.max(0.68 - (depth - 1) * 0.06, 0.4);
    return hsl(hue, s, l);
  }
  const l = Math.min(0.6 + (depth - 1) * 0.04, 0.76);
  const s = Math.max(0.72 - (depth - 1) * 0.05, 0.45);
  return hsl(hue, s, l);
}

export function fileColor(depth: number, light = false): string {
  if (light) return hsl(225, 0.12, Math.min(0.78 + (depth - 1) * 0.03, 0.88));
  return hsl(225, 0.08, Math.min(0.34 + (depth - 1) * 0.05, 0.56));
}

/** A file in the block view: a quiet shade of its folder's hue rather than flat grey. */
export function fileTint(hue: number, depth: number, light = false): string {
  if (light) return hsl(hue, 0.18, Math.max(0.66 - (depth - 1) * 0.04, 0.5));
  return hsl(hue, 0.14, Math.min(0.36 + (depth - 1) * 0.05, 0.56));
}

export function restColor(light = false): string {
  return light ? 'hsl(225 6% 80%)' : '#393c40';
}

export const GREY = 'hsl(225 4% 55%)';

/** Legible label colour on top of a fill produced by this module (hsl() or #rrggbb). */
export function inkFor(fill: string): string {
  let lightness = 50;
  const h = /hsl\([\d.]+\s+[\d.]+%\s+([\d.]+)%\)/.exec(fill);
  if (h) lightness = Number(h[1]);
  else if (/^#[0-9a-f]{6}$/i.test(fill)) {
    const n = parseInt(fill.slice(1), 16);
    lightness = ((0.2126 * (n >> 16) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255)) / 255) * 100;
  }
  return lightness >= 56 ? 'rgba(0,0,0,0.78)' : 'rgba(255,255,255,0.92)';
}
