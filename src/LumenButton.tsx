// A button drawn as a pill of glass by a small WebGL2 shader: light flows
// along its inside edge, a soft glow follows the pointer, and a press sends
// a ripple out from where it landed. While it is `ready` it fills with blue
// water from the start side, behind a wavy front; when it is not, the water
// drains away and the glass is a deep, quiet blue. Adapted from the "Lumen" button
// to the app's colours (blue and cyan) and to a plain button: its text, its
// click and everything else a button does stay in the DOM, and only the
// glass is drawn. Without WebGL2 it falls back to a flat blue pill.

import { useCallback, useEffect, useRef, useState, type ButtonHTMLAttributes, type Ref } from "react";
import { ArrowRight } from "lucide-react";

const VERT_SHADER = `#version 300 es
precision highp float;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
`;

const FRAG_SHADER = `#version 300 es
precision highp float;

out vec4 outColor;

uniform vec2 uRes;
uniform float uDpr;
uniform float uTime;
uniform vec2 uMouse;
uniform float uHover;
uniform float uPress;
uniform float uFill;  // 0 empty .. 1 full of water
uniform float uRtl;   // 1 when the water comes in from the right
uniform vec3 uClick; // xy: where it was pressed, device px from bottom left; z: seconds since (<0: none)

float sdRoundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float hash(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float vnoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  return mix(
    mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
    mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
    u.y
  );
}

const mat2 mRot = mat2(1.6, 1.2, -1.2, 1.6);

float fbm(vec2 p) {
  float v = 0.0;
  float a = 0.5;
  for (int i = 0; i < 4; i++) {
    v += a * vnoise(p);
    p = mRot * p;
    a *= 0.5;
  }
  return v;
}

// The light along the rim: the app's blues and cyan, round and back.
vec3 ramp(float t) {
  t = fract(t);
  float x = t * 4.0;
  float idx = floor(x);
  float f = fract(x);
  vec3 c0 = vec3(0.133, 0.827, 0.933); // cyan   #22d3ee
  vec3 c1 = vec3(0.220, 0.741, 0.973); // sky    #38bdf8
  vec3 c2 = vec3(0.376, 0.647, 0.980); // blue   #60a5fa
  vec3 c3 = vec3(0.576, 0.773, 0.992); // pale   #93c5fd
  if (idx < 1.0) return mix(c0, c1, f);
  if (idx < 2.0) return mix(c1, c2, f);
  if (idx < 3.0) return mix(c2, c3, f);
  return mix(c3, c0, f);
}

float hash21(vec2 p) {
  return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
  vec2 px = gl_FragCoord.xy;
  vec2 p = px - uRes * 0.5;
  float H = uRes.y;

  float d = sdRoundBox(p, uRes * 0.5, H * 0.5);
  float edge = -d;
  float cover = clamp(edge + 0.5, 0.0, 1.0);
  if (cover <= 0.0) discard;

  vec2 uv = p / H;

  // Both colours are given before the tone curve at the end, which brings
  // them back to what is named here. Empty: deep glass (#16295a to #1d3570).
  // Water: the app's blue (#2b74f0), a little lighter at the top.
  float up = smoothstep(-0.5, 0.5, p.y / H);
  vec3 glass = mix(vec3(0.056, 0.110, 0.272), vec3(0.076, 0.146, 0.361), up);
  vec3 waterCol = mix(vec3(0.081, 0.292, 1.141), vec3(0.161, 0.446, 2.0), up);

  // The water's front: across the pill from the start side, wavy along its
  // height, and calm again once the pill is full.
  float along = mix(px.x, uRes.x - px.x, uRtl);
  float calm = smoothstep(0.92, 1.0, uFill);
  float wave = (sin(uv.y * 9.0 + uTime * 3.2) * 0.55 + sin(uv.y * 17.0 - uTime * 4.6) * 0.3
              + sin(uv.y * 31.0 + uTime * 6.1) * 0.15) * H * 0.09 * (1.0 - calm);
  float front = uFill * (uRes.x + H * 0.5) - H * 0.25 + wave;
  float water = 1.0 - smoothstep(front - 1.2 * uDpr, front + 1.2 * uDpr, along);
  if (uFill <= 0.0005) water = 0.0;

  vec3 base = mix(glass, waterCol, water);
  base *= (1.0 - 0.12 * uPress);
  vec3 col = base;

  // A ripple out from where it was pressed, bending the flow as it passes.
  float age = uClick.z;
  float ring = 0.0;
  vec2 wuv = uv;
  if (age >= 0.0) {
    float R = age * H * 2.6;
    float w = H * 0.22;
    float dist = length(px - uClick.xy);
    ring = exp(-pow((dist - R) / w, 2.0)) * exp(-age * 2.4);
    vec2 dir = (dist > 0.001) ? (px - uClick.xy) / dist : vec2(0.0);
    wuv += dir * ring * 0.10;
  }

  vec2 q = vec2(fbm(wuv + 0.05 * uTime), fbm(wuv + vec2(5.2, 1.3) + 0.06 * uTime));
  vec2 r = vec2(
    fbm(wuv + 4.0 * q + vec2(1.7, 9.2) + 0.07 * uTime),
    fbm(wuv + 4.0 * q + vec2(8.3, 2.8) + 0.04 * uTime)
  );
  float f = fbm(wuv + 4.0 * r + 0.08 * uTime);
  vec3 iri = ramp(f * 1.15 + r.x * 0.6 + uTime * 0.03);

  // Light flowing along the inside of the rim; the middle stays calm.
  float rimW = H * mix(0.30, 0.50, uHover) * (1.0 - 0.22 * uPress);
  float rim = pow(1.0 - smoothstep(0.0, rimW, edge), 1.7);
  float bias = mix(0.55, 1.0, smoothstep(0.45, -0.55, p.y / H));
  float breathe = 0.5 + 0.5 * sin(uTime * 1.5708);
  float k = rim * bias * (0.55 + 0.45 * smoothstep(0.25, 0.75, f)) * mix(0.55, 1.0, uHover) * (0.92 + 0.08 * breathe);
  col += iri * k * 0.9;

  // A fine line along the edge, brighter at the top.
  float line = 1.0 - smoothstep(0.0, 1.25 * uDpr, abs(edge - 0.75 * uDpr));
  col += mix(vec3(1.0), iri, 0.5) * line * mix(0.25, 0.7, uHover);
  col += vec3(1.0) * line * smoothstep(0.0, 0.6, p.y / H) * 0.2;

  // A soft light where the pointer is.
  vec2 m = uMouse - px;
  float sp = exp(-dot(m, m) / pow(H * 0.9, 2.0)) * uHover;
  col += mix(vec3(1.0), iri, 0.5) * sp * 0.12;

  // The ripple's own light, and a flash when it reaches the rim.
  if (age >= 0.0) {
    col += iri * ring * 0.35;
    col += iri * rim * exp(-pow((age - 0.42) / 0.22, 2.0)) * 0.8;
  }

  // The water: slow light moving under its surface, and white foam on its
  // front while it is coming in.
  col += iri * water * 0.10 * smoothstep(0.3, 0.8, f);
  float foam = exp(-pow((along - front) / (2.2 * uDpr), 2.0)) * (1.0 - calm) * step(0.0005, uFill);
  col += vec3(0.75, 0.95, 1.0) * foam * 1.2;
  col += vec3(0.6, 0.9, 1.0) * exp(-pow((along - front + 6.0 * uDpr) / (6.0 * uDpr), 2.0)) * 0.25 * (1.0 - calm) * step(0.0005, uFill);

  col = 1.0 - exp(-col * 1.6);
  col += (hash21(px + fract(uTime) * 91.0) - 0.5) / 255.0;
  outColor = vec4(col * cover, cover);
}
`;

type Uniforms = Record<
  "uRes" | "uDpr" | "uTime" | "uMouse" | "uHover" | "uPress" | "uFill" | "uRtl" | "uClick",
  WebGLUniformLocation | null
>;

export function LumenButton({
  children,
  className,
  ref,
  ready = true,
  onPointerEnter,
  onPointerMove,
  onPointerLeave,
  onPointerDown,
  onPointerUp,
  onFocus,
  onBlur,
  onKeyDown,
  onKeyUp,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  ref?: Ref<HTMLButtonElement>;
  /** Filled with water while true: what it does is ready to be done. */
  ready?: boolean;
}) {
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [noGl, setNoGl] = useState(false);

  // What the pointer and keyboard ask for, and where the drawing is now.
  const anim = useRef({
    time: 0,
    hover: 0,
    hoverTarget: 0,
    press: 0,
    pressTarget: 0,
    fill: ready ? 1 : 0,
    fillTarget: ready ? 1 : 0,
    mouseX: 0,
    mouseY: 0,
    targetMouseX: 0,
    targetMouseY: 0,
    clickX: 0,
    clickY: 0,
    clickAge: -1,
    scale: 1,
    scaleV: 0,
    scaleTarget: 1,
    width: 1,
    height: 1,
    dpr: 1,
    hovered: false,
    visible: true,
  });

  anim.current.fillTarget = ready ? 1 : 0;

  // The button's ref is handed on to whoever asked for it as well.
  const setRefs = useCallback(
    (el: HTMLButtonElement | null) => {
      btnRef.current = el;
      if (typeof ref === "function") ref(el);
      else if (ref) (ref as { current: HTMLButtonElement | null }).current = el;
    },
    [ref],
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    const btn = btnRef.current;
    if (!canvas || !btn) return;

    const options = { alpha: true, premultipliedAlpha: true, antialias: false, powerPreference: "low-power" } as const;
    let gl: WebGL2RenderingContext | null = null;
    let program: WebGLProgram | null = null;
    let uniforms: Uniforms | null = null;

    const build = (ctx: WebGL2RenderingContext): boolean => {
      const shader = (type: number, src: string) => {
        const s = ctx.createShader(type);
        if (!s) return null;
        ctx.shaderSource(s, src);
        ctx.compileShader(s);
        if (!ctx.getShaderParameter(s, ctx.COMPILE_STATUS)) {
          console.error("[LumenButton]", ctx.getShaderInfoLog(s));
          ctx.deleteShader(s);
          return null;
        }
        return s;
      };
      const vs = shader(ctx.VERTEX_SHADER, VERT_SHADER);
      const fs = shader(ctx.FRAGMENT_SHADER, FRAG_SHADER);
      const prog = ctx.createProgram();
      if (!vs || !fs || !prog) return false;
      ctx.attachShader(prog, vs);
      ctx.attachShader(prog, fs);
      ctx.linkProgram(prog);
      ctx.deleteShader(vs);
      ctx.deleteShader(fs);
      if (!ctx.getProgramParameter(prog, ctx.LINK_STATUS)) {
        console.error("[LumenButton]", ctx.getProgramInfoLog(prog));
        ctx.deleteProgram(prog);
        return false;
      }
      const at = (name: string) => ctx.getUniformLocation(prog, name);
      gl = ctx;
      program = prog;
      uniforms = {
        uRes: at("uRes"),
        uDpr: at("uDpr"),
        uTime: at("uTime"),
        uMouse: at("uMouse"),
        uHover: at("uHover"),
        uPress: at("uPress"),
        uFill: at("uFill"),
        uRtl: at("uRtl"),
        uClick: at("uClick"),
      };
      return true;
    };

    let ctx: WebGL2RenderingContext | null = null;
    try {
      ctx = (canvas.getContext("webgl2", options) as WebGL2RenderingContext | null);
    } catch {
      ctx = null;
    }
    if (!ctx || !build(ctx)) {
      setNoGl(true);
      return;
    }

    // Drawn at the screen's own sharpness, at most twice the CSS size.
    const resize = () => {
      const rect = btn.getBoundingClientRect();
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.max(1, Math.round(rect.width * dpr));
      const h = Math.max(1, Math.round(rect.height * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      const a = anim.current;
      a.width = w;
      a.height = h;
      a.dpr = dpr;
      if (!a.hovered) {
        a.mouseX = a.targetMouseX = w / 2;
        a.mouseY = a.targetMouseY = h / 2;
      }
      gl?.viewport(0, 0, w, h);
    };
    resize();
    const sized = new ResizeObserver(resize);
    sized.observe(btn);

    // Nothing is drawn while the button is off screen.
    const seen = new IntersectionObserver(([entry]) => {
      anim.current.visible = entry.isIntersecting;
    });
    seen.observe(btn);

    const lost = (e: Event) => {
      e.preventDefault();
      gl = null;
      program = null;
    };
    const restored = () => {
      const again = (canvas.getContext("webgl2", options) as WebGL2RenderingContext | null);
      if (again && build(again)) resize();
      else setNoGl(true);
    };
    canvas.addEventListener("webglcontextlost", lost);
    canvas.addEventListener("webglcontextrestored", restored);

    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const ease = (v: number, target: number, tau: number, dt: number) => v + (target - v) * (1 - Math.exp(-dt / tau));
    let frame = 0;
    let last = performance.now();

    const tick = (now: number) => {
      frame = requestAnimationFrame(tick);
      const a = anim.current;
      if (document.hidden || !a.visible) {
        last = now;
        return;
      }
      const dt = Math.min((now - last) / 1000, 0.05);
      last = now;

      a.hover = ease(a.hover, a.hoverTarget, 0.18, dt);
      a.press = ease(a.press, a.pressTarget, a.pressTarget > a.press ? 0.06 : 0.22, dt);
      // the water comes in steadily (about a second and a half) and drains faster
      if (still) a.fill = a.fillTarget;
      else if (a.fill < a.fillTarget) a.fill = Math.min(a.fillTarget, a.fill + dt / 1.5);
      else a.fill = Math.max(a.fillTarget, a.fill - dt / 0.6);
      const mouseTau = a.hovered ? 0.08 : 0.6;
      a.mouseX = ease(a.mouseX, a.targetMouseX, mouseTau, dt);
      a.mouseY = ease(a.mouseY, a.targetMouseY, mouseTau, dt);

      // the squeeze of a press, on a spring; never larger than 1.02
      if (still) {
        a.scale = 1;
      } else {
        a.scaleV += ((a.scaleTarget - a.scale) * 420 - a.scaleV * 16) * dt;
        a.scale = Math.min(a.scale + a.scaleV * dt, 1.02);
      }
      btn.style.setProperty("--lumen-scale", a.scale.toFixed(4));

      if (a.clickAge >= 0) {
        a.clickAge += dt;
        if (a.clickAge > 1.2) a.clickAge = -1;
      }
      a.time = still ? 2.3 : a.time + dt * (1 + 0.9 * a.hover);

      if (gl && program && uniforms) {
        gl.useProgram(program);
        gl.uniform2f(uniforms.uRes, a.width, a.height);
        gl.uniform1f(uniforms.uDpr, a.dpr);
        gl.uniform1f(uniforms.uTime, a.time);
        gl.uniform2f(uniforms.uMouse, a.mouseX, a.mouseY);
        gl.uniform1f(uniforms.uHover, a.hover);
        gl.uniform1f(uniforms.uPress, a.press);
        gl.uniform1f(uniforms.uFill, a.fill);
        gl.uniform1f(uniforms.uRtl, document.documentElement.dir === "rtl" ? 1 : 0);
        gl.uniform3f(uniforms.uClick, a.clickX, a.clickY, a.clickAge);
        gl.disable(gl.BLEND);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
    };
    frame = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(frame);
      sized.disconnect();
      seen.disconnect();
      canvas.removeEventListener("webglcontextlost", lost);
      canvas.removeEventListener("webglcontextrestored", restored);
      // The context is left alive: a canvas gives the same one back if this
      // runs again (React mounts twice in development), and a lost one could
      // not draw. It goes with the canvas.
      const g = gl as WebGL2RenderingContext | null;
      if (g && program) g.deleteProgram(program);
    };
  }, []);

  /** Where a pointer event is, in device px from the bottom left, as the shader counts. */
  const point = (e: React.PointerEvent<HTMLButtonElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const dpr = anim.current.dpr;
    return { x: (e.clientX - rect.left) * dpr, y: (rect.bottom - e.clientY) * dpr };
  };

  const press = (x: number, y: number) => {
    const a = anim.current;
    a.pressTarget = 1;
    a.scaleTarget = 0.965;
    a.clickX = x;
    a.clickY = y;
    a.clickAge = 0;
  };

  const release = () => {
    anim.current.pressTarget = 0;
    anim.current.scaleTarget = 1;
  };

  return (
    <button
      {...rest}
      ref={setRefs}
      className={`lumen-btn${ready ? " is-ready" : ""}${noGl ? " lumen-btn--flat" : ""}${className ? ` ${className}` : ""}`}
      onPointerEnter={(e) => {
        anim.current.hovered = true;
        anim.current.hoverTarget = 1;
        onPointerEnter?.(e);
      }}
      onPointerMove={(e) => {
        const at = point(e);
        anim.current.targetMouseX = at.x;
        anim.current.targetMouseY = at.y;
        onPointerMove?.(e);
      }}
      onPointerLeave={(e) => {
        const a = anim.current;
        a.hovered = false;
        a.hoverTarget = 0;
        a.targetMouseX = a.width / 2;
        a.targetMouseY = a.height / 2;
        release();
        onPointerLeave?.(e);
      }}
      onPointerDown={(e) => {
        const at = point(e);
        press(at.x, at.y);
        onPointerDown?.(e);
      }}
      onPointerUp={(e) => {
        release();
        onPointerUp?.(e);
      }}
      onFocus={(e) => {
        if (!anim.current.hovered) anim.current.hoverTarget = 0.6;
        onFocus?.(e);
      }}
      onBlur={(e) => {
        if (!anim.current.hovered) anim.current.hoverTarget = 0;
        release();
        onBlur?.(e);
      }}
      onKeyDown={(e) => {
        if (!e.repeat && (e.key === "Enter" || e.key === " ")) press(anim.current.width / 2, anim.current.height / 2);
        onKeyDown?.(e);
      }}
      onKeyUp={(e) => {
        if (e.key === "Enter" || e.key === " ") release();
        onKeyUp?.(e);
      }}
    >
      <canvas ref={canvasRef} className="lumen-btn-gl" aria-hidden="true" />
      <span className="lumen-btn-content">
        <span>{children}</span>
        <ArrowRight className="lumen-btn-arrow" size={16} strokeWidth={2.2} aria-hidden="true" />
      </span>
    </button>
  );
}
