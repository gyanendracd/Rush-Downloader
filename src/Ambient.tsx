/* The live background along the foot of the window, behind every page:
 * a few soft ribbons of the logo's blues drifting sideways at different
 * speeds, with pools of light moving slowly under them. It fades out
 * upward, over the lowest 35% of the window, so text never sits on it.
 *
 * Only transforms move (each ribbon is drawn twice as wide as the window
 * and slid by exactly one repeat), so it costs the window next to nothing.
 * Reduced motion holds it still; reduced transparency leaves it out. */
import { memo } from "react";

/** One repeat of a wave, in viewBox units; the ribbon holds two. */
const W = 1200;
const H = 300;

/** A filled wave: `waves` humps per repeat, around `y`, `amp` high. Drawn
 *  over two repeats so sliding it by one repeat is seamless. */
function ribbon(y: number, amp: number, waves: number, phase: number): string {
  const steps = 96;
  let d = "";
  for (let i = 0; i <= steps * 2; i++) {
    const x = (i / steps) * W;
    const a = (i / steps) * waves * Math.PI * 2 + phase;
    // two sines together, so the crests are not all alike
    const yy = y + Math.sin(a) * amp + Math.sin(a * 2 + phase * 1.7) * amp * 0.28;
    d += `${i === 0 ? "M" : "L"}${x.toFixed(1)},${yy.toFixed(1)}`;
  }
  return `${d} L${2 * W},${H} L0,${H} Z`;
}

/** A thin line along a wave, for a little definition between the ribbons. */
function line(y: number, amp: number, waves: number, phase: number): string {
  return ribbon(y, amp, waves, phase).replace(/ L[\d.]+,300 L0,300 Z$/, "");
}

const RIBBONS = [
  { d: ribbon(150, 26, 1, 0), cls: "ambient-r1" },
  { d: ribbon(180, 34, 2, 1.3), cls: "ambient-r2" },
  { d: ribbon(215, 22, 1, 2.4), cls: "ambient-r3" },
];
const LINES = [
  { d: line(140, 30, 2, 0.6), cls: "ambient-l1" },
  { d: line(200, 24, 3, 2.1), cls: "ambient-l2" },
];

export const Ambient = memo(function Ambient() {
  return (
    <div className="ambient" aria-hidden="true">
      <span className="ambient-orb ambient-o1" />
      <span className="ambient-orb ambient-o2" />
      <span className="ambient-orb ambient-o3" />
      {RIBBONS.map((r) => (
        <svg key={r.cls} className={`ambient-wave ${r.cls}`} viewBox={`0 0 ${2 * W} ${H}`} preserveAspectRatio="none">
          <path d={r.d} />
        </svg>
      ))}
      {LINES.map((l) => (
        <svg key={l.cls} className={`ambient-wave ambient-line ${l.cls}`} viewBox={`0 0 ${2 * W} ${H}`} preserveAspectRatio="none">
          <path d={l.d} vectorEffect="non-scaling-stroke" />
        </svg>
      ))}
    </div>
  );
});
