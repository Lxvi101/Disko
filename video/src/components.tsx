import React from 'react';
import { AbsoluteFill, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig, Easing } from 'remotion';
import { arc as d3arc } from 'd3-shape';
import { C, sans } from './theme';

/** Size unit that works for both 16:9 and square renders. */
export const useU = () => {
  const { width, height } = useVideoConfig();
  return Math.min(width, height) / 100;
};

export const pop = (frame: number, fps: number, delay = 0, stiffness = 260) =>
  spring({ frame: frame - delay, fps, config: { damping: 14, stiffness, mass: 0.6 } });

/** Big text that slams in from oversized and blurred. */
export const Slam: React.FC<{ children: React.ReactNode; delay?: number; size?: number; color?: string; weight?: number; style?: React.CSSProperties }> = ({ children, delay = 0, size = 11, color = C.text, weight = 900, style }) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const u = useU();
  const p = pop(f, fps, delay, 320);
  const scale = interpolate(p, [0, 1], [1.9, 1]);
  const blur = interpolate(p, [0, 1], [18, 0], { extrapolateRight: 'clamp' });
  return (
    <div style={{ fontFamily: sans, fontWeight: weight, fontSize: size * u, color, letterSpacing: '-0.045em', lineHeight: 0.95, transform: `scale(${scale})`, filter: `blur(${blur}px)`, opacity: f < delay ? 0 : Math.min(1, p * 2), textAlign: 'center', ...style }}>
      {children}
    </div>
  );
};

/** Quick white/green flash at the start of a scene. */
export const CutFlash: React.FC<{ color?: string }> = ({ color = '#fff' }) => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 5], [0.55, 0], { extrapolateRight: 'clamp' });
  return <AbsoluteFill style={{ background: color, opacity: o, pointerEvents: 'none' }} />;
};

export const Shake: React.FC<{ children: React.ReactNode; from: number; to: number; amount?: number }> = ({ children, from, to, amount = 1.2 }) => {
  const f = useCurrentFrame();
  const u = useU();
  const on = f >= from && f < to;
  const decay = on ? 1 - (f - from) / (to - from) : 0;
  const x = on ? Math.sin(f * 2.7) * amount * u * decay : 0;
  const y = on ? Math.cos(f * 3.3) * amount * 0.6 * u * decay : 0;
  return <AbsoluteFill style={{ transform: `translate(${x}px, ${y}px)` }}>{children}</AbsoluteFill>;
};

/** Mirror-ball light rays and floating specks, like the app icon. */
export const DiscoLights: React.FC<{ intensity?: number; speed?: number }> = ({ intensity = 1, speed = 1 }) => {
  const f = useCurrentFrame();
  const { width, height } = useVideoConfig();
  const rot = f * 0.35 * speed;
  const dots = Array.from({ length: 70 }, (_, i) => {
    const seed = Math.sin(i * 91.7) * 10000;
    const r = seed - Math.floor(seed);
    const seed2 = Math.sin(i * 12.9) * 10000;
    const r2 = seed2 - Math.floor(seed2);
    const a = r * Math.PI * 2 + f * 0.012 * speed * (i % 2 ? 1 : -1);
    const d = 0.25 + r2 * 0.75;
    const tw = 0.4 + 0.6 * Math.abs(Math.sin(f * 0.15 + i));
    return { x: width / 2 + Math.cos(a) * d * width * 0.6, y: height * 0.35 + Math.sin(a) * d * height * 0.7, s: 3 + r2 * 7, o: tw };
  });
  return (
    <AbsoluteFill style={{ opacity: intensity, overflow: 'hidden' }}>
      <AbsoluteFill style={{ background: `conic-gradient(from ${rot}deg at 50% 0%, ${Array.from({ length: 18 }, (_, i) => `${i % 2 ? 'transparent' : 'rgba(78,240,138,0.13)'} ${(i * 100) / 18}%`).join(', ')})`, maskImage: 'radial-gradient(ellipse at 50% 0%, black 10%, transparent 75%)', WebkitMaskImage: 'radial-gradient(ellipse at 50% 0%, black 10%, transparent 75%)' }} />
      {dots.map((d, i) => (
        <div key={i} style={{ position: 'absolute', left: d.x, top: d.y, width: d.s, height: d.s, borderRadius: 2, background: C.green, opacity: d.o * 0.55, boxShadow: `0 0 ${d.s * 3}px ${C.green}` }} />
      ))}
      <AbsoluteFill style={{ background: 'radial-gradient(ellipse at 50% 110%, rgba(78,240,138,0.18), transparent 60%)' }} />
    </AbsoluteFill>
  );
};

/** Colors copied from the app's src/lib/colors.ts so the wheel matches the real one. */
const hueForAngle = (t: number) => (265 + (t / (2 * Math.PI)) * 320) % 360;
const folderColor = (hue: number, depth: number) => `hsl(${Math.round(hue)} 98% ${Math.round(Math.min(0.6 + (depth - 1) * 0.045, 0.8) * 100)}%)`;

type Seg = { a0: number; a1: number; depth: number; delay: number };
const makeSegments = (): Seg[] => {
  const segs: Seg[] = [];
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const split = (a0: number, a1: number, depth: number) => {
    segs.push({ a0, a1, depth, delay: depth * 7 + (a0 / (Math.PI * 2)) * 14 });
    if (depth >= 4 || a1 - a0 < 0.08) return;
    const n = 2 + Math.floor(rnd() * 4);
    const weights = Array.from({ length: n }, () => 0.2 + rnd());
    const sum = weights.reduce((x, y) => x + y, 0) / (0.55 + rnd() * 0.4);
    let a = a0;
    for (const w of weights) {
      const next = Math.min(a1, a + ((a1 - a0) * w) / sum);
      split(a, next, depth + 1);
      a = next;
    }
  };
  const tops = [0.3, 0.2, 0.14, 0.1, 0.08, 0.06, 0.05, 0.04];
  let a = 0;
  for (const t of tops) { const b = a + t * Math.PI * 2 * 0.97; split(a, b, 1); a = b; }
  return segs;
};
const SEGMENTS = makeSegments();

export const Sunburst: React.FC<{ size: number; start?: number; spin?: number; highlight?: number }> = ({ size, start = 0, spin = 0, highlight }) => {
  const f = useCurrentFrame() - start;
  const { fps } = useVideoConfig();
  const r = size / 2;
  const ring = r / 5.2;
  const gen = d3arc<Seg>()
    .innerRadius((d) => ring * (d.depth === 1 ? 0.95 : d.depth) + 1)
    .outerRadius((d) => ring * (d.depth + 1))
    .startAngle((d) => d.a0)
    .endAngle((d) => d.a0 + (d.a1 - d.a0) * Math.min(1, spring({ frame: f - d.delay, fps, config: { damping: 18, stiffness: 140 } })))
    .padAngle(0.006)
    .cornerRadius(2);
  return (
    <svg width={size} height={size} viewBox={`${-r} ${-r} ${size} ${size}`} style={{ overflow: 'visible', transform: `rotate(${spin}deg)` }}>
      <circle r={ring * 0.9} fill={C.bg2} stroke={C.line} />
      {SEGMENTS.map((s, i) => {
        const dim = highlight !== undefined && !(s.a0 >= SEGMENTS[highlight].a0 - 1e-6 && s.a1 <= SEGMENTS[highlight].a1 + 1e-6);
        return <path key={i} d={gen(s) ?? ''} fill={folderColor(hueForAngle((s.a0 + s.a1) / 2), s.depth)} opacity={dim ? 0.3 : 1} />;
      })}
    </svg>
  );
};

/** macOS-style window around a real screenshot. */
export const AppWindow: React.FC<{ src: string; width: number; style?: React.CSSProperties }> = ({ src, width, style }) => {
  const h = width * 0.625;
  const bar = width * 0.028;
  return (
    <div style={{ width, borderRadius: width * 0.012, overflow: 'hidden', background: C.bg2, boxShadow: '0 40px 120px rgba(0,0,0,0.65), 0 0 0 1px rgba(255,255,255,0.08), 0 0 80px rgba(78,240,138,0.12)', ...style }}>
      <div style={{ height: bar, display: 'flex', alignItems: 'center', gap: bar * 0.3, paddingLeft: bar * 0.5, background: '#1c1d22' }}>
        {['#ff5f57', '#febc2e', '#28c840'].map((c) => <div key={c} style={{ width: bar * 0.38, height: bar * 0.38, borderRadius: '50%', background: c }} />)}
      </div>
      <Img src={staticFile(src)} style={{ width, height: h, objectFit: 'cover', objectPosition: 'top left', display: 'block' }} />
    </div>
  );
};

export const Bg: React.FC<{ children?: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill style={{ background: C.bg, fontFamily: sans, color: C.text }}>{children}</AbsoluteFill>
);

export const Center: React.FC<{ children: React.ReactNode; gap?: number; style?: React.CSSProperties }> = ({ children, gap = 0, style }) => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap, ...style }}>{children}</AbsoluteFill>
);

export const ease = Easing.bezier(0.2, 0.8, 0.2, 1);
