import { loadFont as loadInter } from '@remotion/google-fonts/Inter';
import { loadFont as loadMono } from '@remotion/google-fonts/JetBrainsMono';

export const { fontFamily: sans } = loadInter('normal', { weights: ['400', '600', '800', '900'], subsets: ['latin'] });
export const { fontFamily: mono } = loadMono('normal', { weights: ['500', '700'], subsets: ['latin'] });

export const C = {
  bg: '#08090b',
  bg2: '#111317',
  text: '#f3f3f5',
  muted: '#8d9099',
  green: '#4ef08a',
  greenDeep: '#0f7a3e',
  red: '#ff5a5f',
  line: '#24262c',
};

/** Numbers shown in the video. Edit these to match your own run. */
export const COPY = {
  repo: 'github.com/Lxvi101/Disko',
  freedGB: 100,
  xcode: [
    { label: 'Simulators', gb: 55.7 },
    { label: 'Derived Data', gb: 19.1 },
    { label: 'Device Support', gb: 16.6 },
    { label: 'Caches', gb: 6.7 },
    { label: 'Archives', gb: 4.2 },
    { label: 'XcodeBuildMCP', gb: 4.1 },
  ],
};
