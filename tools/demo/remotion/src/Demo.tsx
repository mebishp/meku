import React from 'react';
import { AbsoluteFill, Easing, Freeze, OffthreadVideo, Sequence, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig } from 'remotion';
import { CameraMotionBlur } from '@remotion/motion-blur';
import data from './events.json';

export const FPS = 30;
const VIDEO_SECONDS = 31.1;
const VIDEO_FRAMES = Math.floor(VIDEO_SECONDS * FPS);
const OUTRO = 78;
export const TOTAL_FRAMES = VIDEO_FRAMES + OUTRO;

// The recorded window (screen px) and where it sits on the canvas.
const [AX, AY, AW, AH] = data.area;
const CANVAS = { w: 1600, h: 900 };
const WIN = { x: (CANVAS.w - AW) / 2, y: (CANVAS.h - AH) / 2 - 10 };
const toComp = (x: number, y: number) => ({ x: x - AX + WIN.x, y: y - AY + WIN.y });
// Window-local point -> canvas point.
const W = (x: number, y: number) => ({ x: WIN.x + x, y: WIN.y + y });

// ---------- camera: eased zooms between keyframes ----------
type Key = { t: number; s: number; c: { x: number; y: number } };
const mid = { x: CANVAS.w / 2, y: CANVAS.h / 2 };
const KEYS: Key[] = [
  { t: 0.0, s: 1.0, c: mid },
  { t: 1.9, s: 1.0, c: mid },
  { t: 3.3, s: 1.35, c: W(555, 300) },
  { t: 12.0, s: 1.42, c: W(560, 330) },
  { t: 17.2, s: 1.38, c: W(560, 330) },
  { t: 18.3, s: 1.65, c: W(470, 290) },
  { t: 19.9, s: 1.65, c: W(470, 290) },
  { t: 21.3, s: 1.0, c: mid },
  { t: 29.6, s: 1.04, c: mid },
  { t: 31.1, s: 1.0, c: mid },
];
const ease = Easing.inOut(Easing.cubic);
const camera = (t: number) => {
  if (t <= KEYS[0].t) return KEYS[0];
  for (let i = 0; i < KEYS.length - 1; i++) {
    const a = KEYS[i], b = KEYS[i + 1];
    if (t <= b.t) {
      const p = ease((t - a.t) / (b.t - a.t));
      return { t, s: a.s + (b.s - a.s) * p, c: { x: a.c.x + (b.c.x - a.c.x) * p, y: a.c.y + (b.c.y - a.c.y) * p } };
    }
  }
  return KEYS[KEYS.length - 1];
};

// ---------- cursor path from the logged real pointer ----------
type Ev = { t: number; x?: number; y?: number; click?: boolean; theme?: boolean };
const events = data.events as Ev[];
const moves = events.filter((e) => e.x !== undefined && e.y !== undefined && !e.click) as Required<Pick<Ev, 't' | 'x' | 'y'>>[];
const clicks = events.filter((e) => e.click) as Required<Pick<Ev, 't' | 'x' | 'y'>>[];
const themeTimes = events.filter((e) => e.theme).map((e) => e.t);
const THEMES = ['Meku Dark', 'Solarized Light', 'Solarized Dark', 'Nord', 'Gruvbox Dark', 'Dracula'];

const park = toComp(1700, 950);
// Before the first logged move the cursor flies in from beyond the bottom-right corner.
const FLY_END = moves[0].t;
const cursorAt = (t: number) => {
  if (t < FLY_END) {
    const p = Easing.out(Easing.cubic)(Math.max(0, t) / FLY_END);
    const from = { x: CANVAS.w + 140, y: CANVAS.h + 120 };
    const bend = Math.sin(p * Math.PI) * -90; // a slight arc
    return { x: from.x + (park.x - from.x) * p + bend, y: from.y + (park.y - from.y) * p + bend * 0.4 };
  }
  const last = moves[moves.length - 1];
  if (t >= last.t) {
    const p = Easing.in(Easing.cubic)(Math.min(1, (t - last.t) / 0.9));
    const c = toComp(last.x, last.y);
    return { x: c.x + (CANVAS.w + 160 - c.x) * p, y: c.y + (CANVAS.h + 140 - c.y) * p };
  }
  let i = moves.findIndex((m) => m.t > t);
  if (i <= 0) i = 1;
  const a = moves[i - 1], b = moves[i];
  const p = b.t === a.t ? 1 : (t - a.t) / (b.t - a.t);
  return toComp(a.x + (b.x - a.x) * p, a.y + (b.y - a.y) * p);
};

const Cursor: React.FC<{ t: number }> = ({ t }) => {
  const { x, y } = cursorAt(t);
  const ripple = clicks.map((c) => ({ c, d: t - c.t })).find(({ d }) => d >= 0 && d < 0.55);
  const rp = ripple ? ripple.d / 0.55 : 0;
  const pos = ripple ? toComp(ripple.c.x, ripple.c.y) : { x: 0, y: 0 };
  const pressed = clicks.some((c) => t - c.t >= 0 && t - c.t < 0.12);
  return (
    <>
      {ripple && (
        <div style={{ position: 'absolute', left: pos.x - 30, top: pos.y - 30, width: 60, height: 60, borderRadius: 30, border: '3px solid rgba(255,255,255,0.9)', background: 'rgba(120,150,255,0.25)', opacity: 1 - rp, transform: `scale(${0.3 + rp * 1.1})` }} />
      )}
      <svg width="34" height="34" viewBox="0 0 24 24" style={{ position: 'absolute', left: x - 4, top: y - 3, transform: `scale(${pressed ? 0.88 : 1})`, filter: 'drop-shadow(0 3px 5px rgba(0,0,0,0.45))' }}>
        <path d="M4 2.5 L4 18.5 L8.4 14.8 L11.2 21.2 L13.9 20 L11.1 13.7 L16.8 13.4 Z" fill="#fff" stroke="#111" strokeWidth="1.4" strokeLinejoin="round" />
      </svg>
    </>
  );
};

// ---------- background ----------
const Background: React.FC<{ t: number }> = ({ t }) => {
  const a = Math.sin(t * 0.35), b = Math.cos(t * 0.28);
  return (
    <AbsoluteFill
      style={{
        background: [
          `radial-gradient(900px 700px at ${22 + a * 10}% ${25 + b * 8}%, rgba(56,189,248,0.55), transparent 60%)`,
          `radial-gradient(800px 650px at ${80 - b * 9}% ${75 + a * 8}%, rgba(244,114,182,0.6), transparent 60%)`,
          `radial-gradient(700px 600px at ${60 + a * 8}% ${15 - b * 6}%, rgba(251,191,36,0.32), transparent 60%)`,
          'linear-gradient(135deg, #1e1b4b 0%, #4c1d95 48%, #9d174d 100%)',
        ].join(','),
      }}
    />
  );
};

// ---------- captions ----------
const CAPTIONS: [number, number, string][] = [
  [0.9, 3.1, 'A fast, simple markdown editor'],
  [3.3, 17.0, 'Type markdown and see it come alive'],
  [17.4, 20.2, 'Tick a checkbox with a click'],
  [20.6, 30.4, 'Change the whole look with one shortcut'],
];
const Caption: React.FC<{ t: number }> = ({ t }) => {
  const cur = CAPTIONS.find(([a, b]) => t >= a && t < b);
  if (!cur) return null;
  const [a, b, text] = cur;
  const op = interpolate(t, [a, a + 0.35, b - 0.3, b], [0, 1, 1, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const rise = interpolate(t, [a, a + 0.35], [14, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const themeIdx = themeTimes.filter((tt) => t >= tt).length - 1;
  const showTheme = text.startsWith('Change') && themeIdx >= 0;
  return (
    <div style={{ position: 'absolute', left: 0, right: 0, bottom: 34, display: 'flex', justifyContent: 'center', gap: 12, opacity: op, transform: `translateY(${rise}px)` }}>
      <div style={{ padding: '12px 26px', borderRadius: 999, background: 'rgba(18,14,48,0.88)', border: '1px solid rgba(255,255,255,0.18)', color: '#fff', fontSize: 28, fontWeight: 600, letterSpacing: -0.2, backdropFilter: 'blur(14px)' }}>{text}</div>
      {showTheme && (
        <div style={{ padding: '12px 22px', borderRadius: 999, background: 'rgba(255,255,255,0.92)', color: '#1e1b4b', fontSize: 26, fontWeight: 700 }}>{THEMES[Math.min(themeIdx, THEMES.length - 1)]}</div>
      )}
    </div>
  );
};

// ---------- scene ----------
const Stage: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const t = frame / fps;
  const cam = camera(Math.min(t, VIDEO_SECONDS));
  // The window springs in at the start.
  const enter = spring({ frame, fps, config: { damping: 18, stiffness: 90 }, durationInFrames: 36 });
  const outroP = interpolate(frame, [VIDEO_FRAMES, VIDEO_FRAMES + 30], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const winScale = (0.9 + 0.1 * enter) * (1 - 0.1 * outroP);
  const winOpacity = Math.min(1, enter * 1.4) * (1 - outroP);
  const tx = CANVAS.w / 2 - cam.c.x * cam.s, ty = CANVAS.h / 2 - cam.c.y * cam.s;
  const vFrame = Math.min(frame, VIDEO_FRAMES - 1);
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{ transform: `translate(${tx}px, ${ty}px) scale(${cam.s})`, transformOrigin: '0 0' }}>
        <div style={{ position: 'absolute', left: WIN.x, top: WIN.y, width: AW, height: AH, borderRadius: 13, overflow: 'hidden', opacity: winOpacity, transform: `scale(${winScale})`, transformOrigin: '50% 50%', boxShadow: '0 50px 110px rgba(8,6,30,0.55), 0 12px 30px rgba(8,6,30,0.35), 0 0 0 1px rgba(255,255,255,0.12)' }}>
          {frame < VIDEO_FRAMES ? (
            <OffthreadVideo src={staticFile('raw.mp4')} muted style={{ width: AW, height: AH }} />
          ) : (
            <Freeze frame={VIDEO_FRAMES - 1}>
              <OffthreadVideo src={staticFile('raw.mp4')} muted style={{ width: AW, height: AH }} />
            </Freeze>
          )}
        </div>
        {frame < VIDEO_FRAMES && <Cursor t={t} />}
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const p = spring({ frame: frame - 8, fps, config: { damping: 16, stiffness: 100 } });
  const fade = interpolate(frame, [0, 14], [0, 1], { extrapolateRight: 'clamp' });
  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', opacity: fade }}>
      <div style={{ transform: `scale(${0.85 + 0.15 * p}) translateY(${(1 - p) * 20}px)`, opacity: p, textAlign: 'center', color: '#fff', fontFamily: 'Inter, sans-serif' }}>
        <img src={staticFile('icon.svg')} width={120} height={120} style={{ filter: 'drop-shadow(0 14px 30px rgba(0,0,0,0.4))' }} />
        <div style={{ fontSize: 96, fontWeight: 800, letterSpacing: -2, marginTop: 14 }}>Meku</div>
        <div style={{ fontSize: 32, fontWeight: 500, opacity: 0.92, marginTop: 6 }}>Fast, simple markdown for your notes</div>
      </div>
    </AbsoluteFill>
  );
};

export const Demo: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = frame / fps;
  return (
    <AbsoluteFill style={{ fontFamily: 'Inter, "Segoe UI", Helvetica, sans-serif', background: '#1e1b4b' }}>
      <Background t={t} />
      <CameraMotionBlur shutterAngle={140} samples={5}>
        <Stage frame={frame} />
      </CameraMotionBlur>
      <Caption t={t} />
      <Sequence from={VIDEO_FRAMES}>
        <Outro />
      </Sequence>
    </AbsoluteFill>
  );
};
