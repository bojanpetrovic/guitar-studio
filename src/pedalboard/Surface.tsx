// A board's background layer (under the controls). CSS surfaces style it in
// styles.css; particle surfaces also draw on a canvas here. Used by the board and by
// the Inspector's previews.
import { useEffect, useRef } from "react";
import { BOARD_H, BOARD_W, BoardSurface, SURFACES } from "./types";

type ParticleKind = "plexus" | "bokeh" | keyof typeof SPARKS;

export default function Surface({ surface }: { surface: BoardSurface }) {
  const def = SURFACES.find((s) => s.id === surface);
  const particles = def && "particles" in def ? def.particles : undefined;
  return <div className="pb-surface">{particles && <ParticleField kind={particles} />}</div>;
}

/** One particle system, in board units (BOARD_W × BOARD_H). */
interface Sim {
  step(dt: number, t: number): void;
  draw(ctx: CanvasRenderingContext2D, t: number): void;
}

const rand = (lo: number, hi: number) => lo + Math.random() * (hi - lo);

function ParticleField({ kind }: { kind: ParticleKind }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const sim = SIMS[kind]();
    // Start mid-flow rather than from an empty board.
    for (let i = 0; i < 180; i++) sim.step(1 / 30, i / 30);
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    // Match the canvas to its size on screen (the board is CSS-scaled to fit), so
    // it's sharp on the stage and cheap in a small preview.
    const fit = () => {
      const box = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round(box.width * dpr));
      const h = Math.max(1, Math.round(box.height * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
    };

    let raf = 0;
    let frame = 0;
    let last = performance.now();
    const render = (now: number) => {
      // A frame's timestamp can be a little before the \`performance.now()\` we started
      // from: never step backwards.
      const dt = Math.max(0, Math.min(0.05, (now - last) / 1000));
      last = now;
      if (frame++ % 60 === 0) fit();
      const t = now / 1000;
      sim.step(dt, t);
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.globalCompositeOperation = "source-over";
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.setTransform(canvas.width / BOARD_W, 0, 0, canvas.height / BOARD_H, 0, 0);
      sim.draw(ctx, t);
      if (!still) raf = requestAnimationFrame(render);
    };
    raf = requestAnimationFrame(render);
    return () => cancelAnimationFrame(raf);
  }, [kind]);

  return <canvas ref={ref} className="pb-particles" />;
}

/** Colour ramps for the rising-spark surfaces: each spark runs from the first colour
 *  (fresh) to the last (about to fade out). */
const SPARKS = {
  // White-gold, through orange, to a dull red.
  embers: [[255, 245, 215], [255, 200, 90], [255, 140, 40], [235, 70, 25], [150, 30, 12]],
  // Ice white, through cyan, to deep blue.
  frost: [[240, 252, 255], [170, 235, 255], [90, 190, 255], [50, 120, 240], [25, 50, 150]],
  // Pale lime, through acid green, to teal.
  toxic: [[245, 255, 210], [200, 255, 90], [110, 240, 60], [30, 190, 110], [15, 90, 70]],
  // Pink-white, through magenta, to violet.
  ultraviolet: [[255, 235, 255], [255, 140, 240], [220, 70, 255], [140, 50, 255], [60, 20, 150]],
  // Blush white, through pink, to rose.
  sakura: [[255, 245, 248], [255, 200, 220], [255, 140, 185], [235, 80, 140], [140, 30, 80]],
} satisfies Record<string, number[][]>;

const SIMS: Record<ParticleKind, () => Sim> = {
  plexus,
  bokeh,
  ...(Object.fromEntries(Object.entries(SPARKS).map(([k, stops]) => [k, () => sparks(stops)])) as Record<
    keyof typeof SPARKS,
    () => Sim
  >),
};

/** Drifting points, linked by lines that brighten as they come together. */
function plexus(): Sim {
  const LINK = 150;
  const pts = Array.from({ length: 70 }, () => ({
    x: rand(0, BOARD_W),
    y: rand(0, BOARD_H),
    vx: rand(-22, 22),
    vy: rand(-14, 14),
    r: rand(1.2, 2.4),
  }));
  // Cyan on the left to violet on the right.
  const tint = (x: number) => {
    const k = x / BOARD_W;
    return `${Math.round(90 + 80 * k)}, ${Math.round(200 - 80 * k)}, 255`;
  };
  return {
    step(dt) {
      for (const p of pts) {
        p.x += p.vx * dt;
        p.y += p.vy * dt;
        if (p.x < 0 || p.x > BOARD_W) p.vx = -p.vx;
        if (p.y < 0 || p.y > BOARD_H) p.vy = -p.vy;
      }
    },
    draw(ctx) {
      ctx.lineWidth = 1;
      for (let i = 0; i < pts.length; i++) {
        for (let j = i + 1; j < pts.length; j++) {
          const dx = pts[i].x - pts[j].x;
          const dy = pts[i].y - pts[j].y;
          const d2 = dx * dx + dy * dy;
          if (d2 > LINK * LINK) continue;
          const a = (1 - Math.sqrt(d2) / LINK) * 0.45;
          ctx.strokeStyle = `rgba(${tint((pts[i].x + pts[j].x) / 2)}, ${a})`;
          ctx.beginPath();
          ctx.moveTo(pts[i].x, pts[i].y);
          ctx.lineTo(pts[j].x, pts[j].y);
          ctx.stroke();
        }
      }
      for (const p of pts) {
        const c = tint(p.x);
        ctx.fillStyle = `rgba(${c}, 0.14)`;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r * 4, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = `rgba(${c}, 0.95)`;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
        ctx.fill();
      }
    },
  };
}

/** Sparks rising from the bottom, swaying, flickering and shifting through `STOPS` as
 *  they fade. */
function sparks(STOPS: number[][]): Sim {
  const color = (p: number) => {
    const x = Math.min(0.999, Math.max(0, p || 0)) * (STOPS.length - 1);
    const i = Math.floor(x);
    const f = x - i;
    return STOPS[i].map((v, k) => Math.round(v + (STOPS[i + 1][k] - v) * f)).join(", ");
  };
  type Ember = { x: number; y: number; vx: number; vy: number; age: number; life: number; size: number; phase: number; sway: number };
  const spawn = (e: Partial<Ember> = {}): Ember =>
    Object.assign(e, {
      x: rand(0, BOARD_W),
      y: BOARD_H + rand(0, 30),
      vx: rand(-8, 8),
      vy: -rand(45, 115),
      age: 0,
      life: rand(5, 10),
      size: rand(1, 3),
      phase: rand(0, Math.PI * 2),
      sway: rand(6, 20),
    }) as Ember;
  const all = Array.from({ length: 140 }, () => {
    const e = spawn();
    e.age = rand(0, e.life); // spread out from the start
    e.y += e.vy * e.age;
    return e;
  });
  return {
    step(dt, t) {
      for (const e of all) {
        e.age += dt;
        if (e.age > e.life || e.y < -20) {
          spawn(e);
          continue;
        }
        e.x += (e.vx + Math.sin(t * 1.3 + e.phase) * e.sway) * dt;
        e.y += e.vy * dt;
      }
    },
    draw(ctx, t) {
      ctx.globalCompositeOperation = "lighter";
      for (const e of all) {
        const p = e.age / e.life;
        const fade = Math.min(1, p / 0.06) * Math.pow(1 - p, 0.55);
        const a = fade * (0.75 + 0.25 * Math.sin(t * 14 + e.phase));
        const c = color(p);
        ctx.fillStyle = `rgba(${c}, ${a * 0.16})`;
        ctx.beginPath();
        ctx.arc(e.x, e.y, e.size * 5, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = `rgba(${c}, ${a})`;
        ctx.beginPath();
        ctx.arc(e.x, e.y, e.size, 0, Math.PI * 2);
        ctx.fill();
      }
    },
  };
}

/** Big soft out-of-focus lights, floating and gently breathing. */
function bokeh(): Sim {
  const COLORS = ["255, 90, 170", "70, 210, 255", "255, 190, 80", "160, 110, 255"];
  // One soft disc per colour, drawn once and stamped (much cheaper than a gradient each).
  const sprites = COLORS.map((c) => {
    const s = document.createElement("canvas");
    s.width = s.height = 128;
    const g = s.getContext("2d")!;
    const grad = g.createRadialGradient(64, 64, 0, 64, 64, 64);
    grad.addColorStop(0, `rgba(${c}, 1)`);
    grad.addColorStop(0.55, `rgba(${c}, 0.55)`);
    grad.addColorStop(0.85, `rgba(${c}, 0.2)`);
    grad.addColorStop(1, `rgba(${c}, 0)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, 128, 128);
    return s;
  });
  const lights = Array.from({ length: 34 }, () => ({
    x: rand(0, BOARD_W),
    y: rand(0, BOARD_H),
    vx: rand(-10, 10),
    vy: rand(-7, 7),
    r: rand(22, 85),
    sprite: sprites[Math.floor(rand(0, sprites.length))],
    alpha: rand(0.1, 0.3),
    phase: rand(0, Math.PI * 2),
  }));
  return {
    step(dt) {
      for (const l of lights) {
        l.x += l.vx * dt;
        l.y += l.vy * dt;
        // Wrap around, off-board, so lights drift in rather than pop in.
        if (l.x < -l.r) l.x = BOARD_W + l.r;
        if (l.x > BOARD_W + l.r) l.x = -l.r;
        if (l.y < -l.r) l.y = BOARD_H + l.r;
        if (l.y > BOARD_H + l.r) l.y = -l.r;
      }
    },
    draw(ctx, t) {
      ctx.globalCompositeOperation = "lighter";
      for (const l of lights) {
        const r = l.r * (1 + 0.08 * Math.sin(t * 0.6 + l.phase));
        ctx.globalAlpha = l.alpha * (0.8 + 0.2 * Math.sin(t * 0.4 + l.phase));
        ctx.drawImage(l.sprite, l.x - r, l.y - r, r * 2, r * 2);
      }
      ctx.globalAlpha = 1;
    },
  };
}
