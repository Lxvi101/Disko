import React from 'react';
import { Audio, Series, staticFile } from 'remotion';
import { End, Freed, Full, Logo, Safe, Shot, Where, Wheel, Xcode } from './scenes';
import { C } from './theme';

const G = ({ children }: { children: React.ReactNode }) => <span style={{ color: C.green }}>{children}</span>;

/** Scene lengths in frames at 30 fps. Multiples of 15 so cuts land on the 120 BPM beat in public/beat.wav. */
export const TIMELINE = [
  { d: 45, el: <Full /> },
  { d: 45, el: <Where /> },
  { d: 60, el: <Logo /> },
  { d: 90, el: <Wheel /> },
  { d: 60, el: <Shot src="shots/map.png" kicker="Map" title={<>Click through your <G>whole disk.</G></>} /> },
  { d: 30, el: <Shot src="shots/suggestions.png" kicker="Suggestions" title={<>Junk that <G>rebuilds itself.</G></>} /> },
  { d: 30, el: <Shot src="shots/inactive.png" kicker="Inactive" title={<>Stuff you <G>forgot about.</G></>} /> },
  { d: 30, el: <Shot src="shots/apps.png" kicker="App cleanup" title={<>Leftovers from <G>dead apps.</G></>} /> },
  { d: 30, el: <Shot src="shots/assistant.png" kicker="Assistant" title={<>Ask what's <G>safe to delete.</G></>} /> },
  { d: 120, el: <Xcode /> },
  { d: 90, el: <Freed /> },
  { d: 60, el: <Safe /> },
  { d: 90, el: <End /> },
];

/** Silent ~15 s cut for the README GIF. */
export const SHORT = [
  { d: 40, el: <Logo /> },
  { d: 55, el: <Wheel /> },
  { d: 40, el: TIMELINE[4].el },
  { d: 25, el: TIMELINE[5].el },
  { d: 30, el: TIMELINE[8].el },
  { d: 120, el: <Xcode /> },
  { d: 70, el: <Freed /> },
  { d: 60, el: <End /> },
];

const length = (t: typeof TIMELINE) => t.reduce((s, x) => s + x.d, 0);
export const DURATION = length(TIMELINE);
export const SHORT_DURATION = length(SHORT);

const Scenes: React.FC<{ items: typeof TIMELINE }> = ({ items }) => (
  <Series>
    {items.map((s, i) => (
      <Series.Sequence key={i} durationInFrames={s.d}>{s.el}</Series.Sequence>
    ))}
  </Series>
);

export const Launch: React.FC = () => (
  <>
    <Audio src={staticFile('beat.wav')} />
    <Scenes items={TIMELINE} />
  </>
);

export const LaunchShort: React.FC = () => <Scenes items={SHORT} />;
