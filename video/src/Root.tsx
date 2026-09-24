import React from 'react';
import { Composition } from 'remotion';
import { DURATION, Launch, LaunchShort, SHORT_DURATION } from './Launch';

export const Root: React.FC = () => (
  <>
    <Composition id="Launch" component={Launch} durationInFrames={DURATION} fps={30} width={1920} height={1080} />
    <Composition id="LaunchSquare" component={Launch} durationInFrames={DURATION} fps={30} width={1080} height={1080} />
    <Composition id="LaunchShort" component={LaunchShort} durationInFrames={SHORT_DURATION} fps={30} width={1920} height={1080} />
  </>
);
