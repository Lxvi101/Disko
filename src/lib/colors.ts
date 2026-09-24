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
  if (light) {
    const l = Math.max(0.52 - (depth - 1) * 0.015, 0.44) + (depth - 1) * 0.05;
    const s = Math.max(0.72 - (depth - 1) * 0.05, 0.5);
    return hsl(hue, s, Math.min(l, 0.75));
  }
  const l = Math.min(0.60 + (depth - 1) * 0.045, 0.80);
  const s = 0.98;
  return hsl(hue, s, l);
}

export function fileColor(depth: number, light = false): string {
  if (light) return hsl(225, 0.04, Math.max(0.6 - (depth - 1) * 0.05, 0.4));
  return hsl(225, 0.04, Math.min(0.4 + (depth - 1) * 0.07, 0.7));
}

export function restColor(light = false): string {
  return light ? 'hsl(225 6% 80%)' : '#393c40';
}

export const GREY = 'hsl(225 4% 55%)';
