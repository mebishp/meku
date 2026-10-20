import { Composition } from 'remotion';
import { Demo, FPS, TOTAL_FRAMES } from './Demo';

export const Root = () => (
  <Composition id="MekuDemo" component={Demo} durationInFrames={TOTAL_FRAMES} fps={FPS} width={1920} height={1080} />
);
