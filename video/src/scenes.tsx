import React from 'react';
import { AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame, useVideoConfig } from 'remotion';
import { AppWindow, Bg, Center, CutFlash, DiscoLights, Shake, Slam, Sunburst, ease, pop, useU } from './components';
import { C, COPY, mono, sans } from './theme';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/** 1. "Your Mac is full." with a storage bar that fills to red. */
export const Full: React.FC = () => {
  const f = useCurrentFrame();
  const u = useU();
  const fill = interpolate(f, [0, 30], [0.62, 0.997], { ...clamp, easing: ease });
  const free = interpolate(f, [0, 30], [310, 2.1], clamp);
  const barColor = fill > 0.95 ? C.red : '#f5a524';
  return (
    <Bg>
      <Shake from={28} to={44} amount={1.4}>
        <Center gap={5 * u}>
          <Slam size={12}>Your Mac is full.</Slam>
          <div style={{ width: 62 * u, opacity: interpolate(f, [4, 10], [0, 1], clamp) }}>
            <div style={{ height: 3.2 * u, borderRadius: 2 * u, background: C.line, overflow: 'hidden' }}>
              <div style={{ width: `${fill * 100}%`, height: '100%', background: barColor, boxShadow: `0 0 ${3 * u}px ${barColor}` }} />
            </div>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 1.6 * u, fontFamily: mono, fontSize: 2.4 * u, color: C.muted }}>
              <span>Macintosh HD</span>
              <span style={{ color: fill > 0.95 ? C.red : C.muted }}>{free.toFixed(1)} GB free</span>
            </div>
          </div>
        </Center>
      </Shake>
    </Bg>
  );
};

/** 2. "Where did it all go?" one word at a time. */
export const Where: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const u = useU();
  const words = ['Where', 'did', 'it', 'all', 'go?'];
  return (
    <Bg>
      <CutFlash />
      <Center>
        <div style={{ display: 'flex', flexWrap: 'wrap', justifyContent: 'center', gap: '0 2.4vmin', maxWidth: '90%' }}>
          {words.map((w, i) => {
            const p = pop(f, fps, i * 5, 400);
            return <span key={w} style={{ fontFamily: sans, fontWeight: 900, fontSize: 11 * u, letterSpacing: '-0.045em', color: i === 4 ? C.green : C.text, opacity: f < i * 5 ? 0 : 1, transform: `translateY(${(1 - p) * 6 * u}px) scale(${0.6 + 0.4 * p})`, display: 'inline-block' }}>{w}</span>;
          })}
        </div>
      </Center>
    </Bg>
  );
};

/** 3. Icon drops in under the mirror-ball lights, wordmark types on. */
export const Logo: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const u = useU();
  const p = pop(f, fps, 2, 180);
  const letters = 'Disko'.split('');
  return (
    <Bg>
      <DiscoLights intensity={interpolate(f, [0, 12], [0, 1], clamp)} speed={1.6} />
      <CutFlash color={C.green} />
      <Center gap={3 * u}>
        <Img src={staticFile('icon.png')} style={{ width: 34 * u, height: 34 * u, borderRadius: '22.5%', transform: `translateY(${(1 - p) * -40 * u}px) scale(${0.5 + p * 0.5}) rotate(${(1 - p) * -12}deg)`, filter: `drop-shadow(0 0 ${4 * u}px rgba(78,240,138,0.5))` }} />
        <div style={{ display: 'flex' }}>
          {letters.map((l, i) => {
            const q = pop(f, fps, 14 + i * 3, 360);
            return <span key={i} style={{ fontFamily: sans, fontWeight: 900, fontSize: 13 * u, letterSpacing: '-0.05em', opacity: f < 14 + i * 3 ? 0 : 1, transform: `translateY(${(1 - q) * 4 * u}px)`, display: 'inline-block' }}>{l}</span>;
          })}
        </div>
      </Center>
    </Bg>
  );
};

/** 4. The sunburst builds itself. */
export const Wheel: React.FC = () => {
  const f = useCurrentFrame();
  const { width, height } = useVideoConfig();
  const u = useU();
  const wide = width > height;
  const size = wide ? 78 * u : 56 * u;
  const zoom = interpolate(f, [0, 100], [0.9, 1.08], clamp);
  return (
    <Bg>
      <CutFlash />
      <AbsoluteFill style={{ flexDirection: wide ? 'row' : 'column', alignItems: 'center', justifyContent: 'center', gap: wide ? 8 * u : 4 * u }}>
        <div style={{ transform: `scale(${zoom})` }}>
          <Sunburst size={size} spin={interpolate(f, [0, 100], [-20, 0], { ...clamp, easing: ease })} highlight={f > 62 ? 0 : undefined} />
        </div>
        <div style={{ width: wide ? 70 * u : 'auto', textAlign: wide ? 'left' : 'center' }}>
          <Slam size={wide ? 9 : 8} delay={18} style={{ textAlign: 'inherit' }}>See where</Slam>
          <Slam size={wide ? 9 : 8} delay={26} style={{ textAlign: 'inherit' }}>every gigabyte</Slam>
          <Slam size={wide ? 9 : 8} delay={34} color={C.green} style={{ textAlign: 'inherit' }}>went.</Slam>
        </div>
      </AbsoluteFill>
    </Bg>
  );
};

/** A real screenshot swinging in from a 3D tilt. */
export const Shot: React.FC<{ src: string; kicker: string; title: React.ReactNode; flash?: boolean }> = ({ src, kicker, title, flash = true }) => {
  const f = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const u = useU();
  const wide = width > height;
  const p = pop(f, fps, 0, 120);
  const w = wide ? width * 0.62 : width * 0.88;
  const drift = interpolate(f, [0, 90], [0, -2 * u], clamp);
  return (
    <Bg>
      <DiscoLights intensity={0.35} />
      {flash && <CutFlash />}
      <AbsoluteFill style={{ flexDirection: wide ? 'row' : 'column-reverse', alignItems: 'center', justifyContent: 'center', gap: wide ? 5 * u : 4 * u, padding: 4 * u }}>
        <div style={{ perspective: 2400 }}>
          <AppWindow src={src} width={w} style={{ transform: `translateY(${drift}px) rotateY(${(1 - p) * (wide ? -28 : 0)}deg) rotateX(${(1 - p) * 18}deg) scale(${0.8 + p * 0.2})`, opacity: Math.min(1, p * 1.6) }} />
        </div>
        <div style={{ width: wide ? width * 0.26 : 'auto', textAlign: wide ? 'left' : 'center' }}>
          <div style={{ fontFamily: mono, fontWeight: 700, fontSize: 2.4 * u, color: C.green, letterSpacing: '0.08em', textTransform: 'uppercase', marginBottom: 1.6 * u, opacity: interpolate(f, [3, 8], [0, 1], clamp) }}>{kicker}</div>
          <Slam size={wide ? 6.4 : 7} delay={4} style={{ textAlign: 'inherit', lineHeight: 1.02 }}>{title}</Slam>
        </div>
      </AbsoluteFill>
    </Bg>
  );
};

/** 6. Xcode gets called out. */
export const Xcode: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const u = useU();
  const total = COPY.xcode.reduce((s, x) => s + x.gb, 0);
  const max = Math.max(...COPY.xcode.map((x) => x.gb));
  const phase2 = 30;
  const phase3 = phase2 + 60;
  const counted = interpolate(f, [phase2, phase2 + COPY.xcode.length * 7 + 6], [0, total], clamp);
  return (
    <Bg>
      <AbsoluteFill style={{ opacity: 0.16 }}>
        <Img src={staticFile('shots/xcode.png')} style={{ width: '100%', height: '100%', objectFit: 'cover', filter: 'blur(6px)' }} />
      </AbsoluteFill>
      <CutFlash />
      {f < phase2 ? (
        <Center><Slam size={11}>And then there's <span style={{ color: '#5fa8ff' }}>Xcode.</span></Slam></Center>
      ) : (
        <Shake from={phase3} to={phase3 + 14} amount={1}>
          <Center gap={1.4 * u}>
            {COPY.xcode.map((x, i) => {
              const p = pop(f, fps, phase2 + i * 7, 300);
              return (
                <div key={x.label} style={{ display: 'flex', alignItems: 'center', gap: 2 * u, width: 84 * u, opacity: f < phase2 + i * 7 ? 0 : 1, transform: `translateX(${(1 - p) * -20 * u}px)` }}>
                  <div style={{ width: 26 * u, fontSize: 3 * u, fontWeight: 600, textAlign: 'right', whiteSpace: 'nowrap', color: C.muted }}>{x.label}</div>
                  <div style={{ flex: 1, height: 4.2 * u }}>
                    <div style={{ width: `${(x.gb / max) * 100 * p}%`, height: '100%', borderRadius: 0.8 * u, background: `hsl(${212 - i * 14} 95% ${62 + i * 3}%)` }} />
                  </div>
                  <div style={{ width: 14 * u, fontFamily: mono, fontWeight: 700, fontSize: 3 * u }}>{x.gb.toFixed(1)} GB</div>
                </div>
              );
            })}
            <div style={{ marginTop: 3 * u, opacity: f < phase3 ? 0.9 : 1 }}>
              {f < phase3 ? (
                <div style={{ fontFamily: mono, fontWeight: 700, fontSize: 7 * u, color: C.red }}>{counted.toFixed(1)} GB</div>
              ) : (
                <Slam size={6.6} delay={phase3}><span style={{ color: C.red }}>{total.toFixed(0)} GB.</span> Xcode, you absolute hoarder.</Slam>
              )}
            </div>
          </Center>
        </Shake>
      )}
    </Bg>
  );
};

/** 7. The payoff: 100 GB counter, bar drains to green. */
export const Freed: React.FC = () => {
  const f = useCurrentFrame();
  const u = useU();
  const n = interpolate(f, [4, 45], [0, COPY.freedGB], { ...clamp, easing: ease });
  const fill = interpolate(f, [4, 45], [0.997, 0.9], { ...clamp, easing: ease });
  const landed = f >= 45;
  return (
    <Bg>
      <DiscoLights intensity={landed ? 1 : 0.3} speed={2} />
      <CutFlash color={C.green} />
      <Shake from={45} to={57} amount={1.2}>
        <Center gap={2 * u}>
          <div style={{ fontFamily: sans, fontWeight: 900, fontSize: 28 * u, letterSpacing: '-0.06em', lineHeight: 0.9, color: landed ? C.green : C.text, textShadow: landed ? `0 0 ${6 * u}px rgba(78,240,138,0.55)` : 'none', transform: `scale(${landed ? 1 + Math.max(0, 0.12 - (f - 45) * 0.012) : 1})` }}>
            {Math.round(n)}<span style={{ fontSize: 12 * u, marginLeft: 1.5 * u }}>GB</span>
          </div>
          <div style={{ fontSize: 5 * u, fontWeight: 800, letterSpacing: '-0.03em' }}>freed on my Mac.</div>
          <div style={{ width: 56 * u, marginTop: 2 * u }}>
            <div style={{ height: 2.4 * u, borderRadius: 2 * u, background: C.line, overflow: 'hidden' }}>
              <div style={{ width: `${fill * 100}%`, height: '100%', background: fill < 0.95 ? C.green : C.red }} />
            </div>
          </div>
        </Center>
      </Shake>
    </Bg>
  );
};

/** 8. Safety: quarantine first. */
export const Safe: React.FC = () => (
  <Shot src="shots/quarantine.png" kicker="Quarantine" title={<>Nothing gets deleted till <span style={{ color: C.green }}>you</span> say so.</>} />
);

/** 9. End card. */
export const End: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const u = useU();
  const p = pop(f, fps, 0, 160);
  return (
    <Bg>
      <DiscoLights speed={0.8} />
      <CutFlash color={C.green} />
      <Center gap={2.4 * u}>
        <Img src={staticFile('icon.png')} style={{ width: 22 * u, height: 22 * u, borderRadius: '22.5%', transform: `scale(${0.6 + 0.4 * p})`, filter: `drop-shadow(0 0 ${3 * u}px rgba(78,240,138,0.45))` }} />
        <Slam size={12} delay={4}>Disko</Slam>
        <div style={{ fontSize: 3.6 * u, fontWeight: 600, color: C.muted, opacity: interpolate(f, [14, 22], [0, 1], clamp) }}>Free & open source · for macOS</div>
        <div style={{ marginTop: 1.5 * u, padding: `${1.2 * u}px ${2.6 * u}px`, borderRadius: 1.4 * u, border: `1px solid rgba(78,240,138,0.4)`, background: 'rgba(78,240,138,0.08)', fontFamily: mono, fontWeight: 700, fontSize: 3.2 * u, color: C.green, opacity: interpolate(f, [22, 30], [0, 1], clamp), transform: `translateY(${interpolate(f, [22, 30], [2 * u, 0], clamp)}px)` }}>{COPY.repo}</div>
      </Center>
    </Bg>
  );
};
