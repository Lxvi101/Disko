const REPO = "Lxvi101/Disko";
const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
const root = document.documentElement;
root.classList.add("js");

let party = false;
let pulse = 0; // 0..1, peaks on every beat while partying

/* ---------- mirror ball reflections ----------
   Specks sit on an invisible cylinder around the viewer. As it turns they sweep
   across the page, fastest in the middle and fading at the edges, the way real
   disco ball reflections cross a wall. */

const refl = document.getElementById("reflections");
const rctx = refl.getContext("2d");
const CALM_COLORS = ["#4dffa0", "#4dffa0", "#8dffc4", "#c9ffe0", "#ffffff"];
const PARTY_COLORS = ["#4dffa0", "#ff3db4", "#35e0ff", "#ffd23d", "#ffffff", "#a57bff"];
const sprites = new Map();

function sprite(color) {
  if (sprites.has(color)) return sprites.get(color);
  const s = 48;
  const c = document.createElement("canvas");
  c.width = c.height = s;
  const g = c.getContext("2d");
  const glow = g.createRadialGradient(s / 2, s / 2, 0, s / 2, s / 2, s / 2);
  glow.addColorStop(0, color + "ff");
  glow.addColorStop(0.3, color + "66");
  glow.addColorStop(1, color + "00");
  g.fillStyle = glow;
  g.fillRect(0, 0, s, s);
  g.fillStyle = color;
  g.beginPath();
  g.roundRect(s / 2 - 7, s / 2 - 7, 14, 14, 3);
  g.fill();
  g.fillStyle = "#ffffff";
  g.globalAlpha = 0.7;
  g.beginPath();
  g.roundRect(s / 2 - 4, s / 2 - 4, 8, 8, 2);
  g.fill();
  sprites.set(color, c);
  return c;
}

const specks = Array.from({ length: 180 }, (_, i) => ({
  theta: Math.random() * Math.PI * 2,
  y: Math.pow(Math.random(), 1.25), // a little denser near the top, under the ball
  size: 10 + Math.random() * 22,
  calm: CALM_COLORS[i % CALM_COLORS.length],
  party: PARTY_COLORS[i % PARTY_COLORS.length],
  twinkle: Math.random() * Math.PI * 2,
}));

let W = 0, H = 0, dpr = 1;
function resize() {
  dpr = Math.min(devicePixelRatio || 1, 2);
  W = innerWidth;
  H = innerHeight;
  refl.width = W * dpr;
  refl.height = H * dpr;
  tiles.width = W * dpr;
  tiles.height = H * dpr;
}

let rot = 0;
let speed = 0.0009;
function drawReflections(t) {
  const target = party ? 0.006 : 0.0009;
  speed += (target - speed) * 0.03;
  rot += speed;

  // fade the room lights as you scroll away from the hero
  const fade = Math.max(0.28, 1 - scrollY / (H * 1.1));
  rctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  rctx.clearRect(0, 0, W, H);
  rctx.globalCompositeOperation = "lighter";

  const cx = W / 2;
  const spread = Math.max(W, 900) * 0.62;
  for (const s of specks) {
    const a = s.theta + rot;
    const depth = Math.cos(a);
    if (depth <= 0.02) continue;
    const x = cx + Math.sin(a) * spread;
    const y = s.y * H * 1.05;
    const tw = 0.65 + 0.35 * Math.sin(t / 700 + s.twinkle);
    const beat = party ? 0.55 + pulse * 0.9 : 1;
    rctx.globalAlpha = Math.min(1, Math.pow(depth, 1.4) * tw * fade * beat * 0.85);
    const size = s.size * (0.55 + depth * 0.45);
    const stretch = 1 + (1 - depth) * 0.6 + (party ? 0.8 : 0); // motion streak toward the edges
    const img = sprite(party ? s.party : s.calm);
    rctx.drawImage(img, x - (size * stretch) / 2, y - size / 2, size * stretch, size);
  }
  rctx.globalCompositeOperation = "source-over";
}

/* ---------- mirror tile confetti ---------- */

const tiles = document.getElementById("tiles");
const tctx = tiles.getContext("2d");
let bits = [];

function shower(x, y, n = 90) {
  if (calm) return;
  for (let i = 0; i < n; i++) {
    const a = -Math.PI / 2 + (Math.random() - 0.5) * Math.PI * 1.1;
    const v = 5 + Math.random() * 10;
    bits.push({
      x, y,
      vx: Math.cos(a) * v,
      vy: Math.sin(a) * v,
      rot: Math.random() * Math.PI,
      vrot: (Math.random() - 0.5) * 0.3,
      flip: Math.random() * Math.PI,
      vflip: 0.08 + Math.random() * 0.15,
      size: 5 + Math.random() * 7,
      hue: Math.random() < 0.7 ? 150 : [320, 190, 45][i % 3],
      life: 1,
    });
  }
}

function drawTiles() {
  tctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  tctx.clearRect(0, 0, W, H);
  if (!bits.length) return;
  for (const b of bits) {
    b.vy += 0.25;
    b.vx *= 0.985;
    b.vy *= 0.985;
    b.x += b.vx;
    b.y += b.vy;
    b.rot += b.vrot;
    b.flip += b.vflip;
    b.life -= 0.005;
    const face = Math.cos(b.flip); // mirror tiles flash when they face you
    const light = 35 + Math.abs(face) * 55;
    tctx.save();
    tctx.globalAlpha = Math.max(0, Math.min(1, b.life * 1.5));
    tctx.translate(b.x, b.y);
    tctx.rotate(b.rot);
    tctx.scale(1, face);
    tctx.fillStyle = `hsl(${b.hue} 90% ${light}%)`;
    tctx.fillRect(-b.size / 2, -b.size / 2, b.size, b.size);
    tctx.restore();
  }
  bits = bits.filter((b) => b.life > 0 && b.y < H + 30);
}

/* ---------- beat ---------- */

const beat = document.getElementById("beat");
const BEAT = 0.5; // seconds, 120 BPM
const FLASHES = ["#4dffa0", "#ff3db4", "#35e0ff", "#ffd23d"];
let lastBeat = -1;

function updatePulse() {
  if (!party) {
    pulse *= 0.9;
  } else if (!beat.paused) {
    const t = beat.currentTime;
    const phase = (t % BEAT) / BEAT;
    pulse = Math.pow(1 - phase, 3);
    const n = Math.floor(t / BEAT);
    if (n !== lastBeat) {
      lastBeat = n;
      root.style.setProperty("--flash", FLASHES[n % FLASHES.length]);
      if (n % 8 === 0) shower(W * (0.2 + Math.random() * 0.6), -10, 50);
    }
  } else {
    // audio blocked or still loading: fake the tempo
    const phase = ((performance.now() / 1000) % BEAT) / BEAT;
    pulse = Math.pow(1 - phase, 3);
  }
  root.style.setProperty("--pulse", pulse.toFixed(3));
}

for (const btn of document.querySelectorAll(".party-toggle")) {
  btn.addEventListener("click", (e) => {
    party = !party;
    document.body.classList.toggle("party", party);
    for (const b of document.querySelectorAll(".party-toggle")) {
      b.setAttribute("aria-pressed", String(party));
      b.querySelector(".party-label").textContent = party ? "Stop the party" : "Party mode";
    }
    if (party) {
      beat.currentTime = 0;
      beat.volume = 0.7;
      beat.play().catch(() => {});
      const r = e.currentTarget.getBoundingClientRect();
      shower(r.left + r.width / 2, r.top, 140);
    } else {
      beat.pause();
    }
  });
}

/* ---------- scroll: nav, app window tilt ---------- */

const nav = document.querySelector(".nav");
const win = document.getElementById("window");

function onScroll() {
  nav.classList.toggle("scrolled", scrollY > 20);
  if (calm) return;
  const r = win.getBoundingClientRect();
  // 0 when the window's top enters the bottom of the screen, 1 when it's at 25% height
  const p = Math.min(1, Math.max(0, (H - r.top) / (H * 0.75)));
  const e = 1 - Math.pow(1 - p, 3);
  win.style.setProperty("--tilt", `${(1 - e) * 22}deg`);
  win.style.setProperty("--scale", (0.9 + e * 0.1).toFixed(4));
}
addEventListener("scroll", onScroll, { passive: true });

/* ---------- card spotlight ---------- */

for (const card of document.querySelectorAll(".card")) {
  card.addEventListener("pointermove", (e) => {
    const r = card.getBoundingClientRect();
    card.style.setProperty("--mx", `${e.clientX - r.left}px`);
    card.style.setProperty("--my", `${e.clientY - r.top}px`);
  });
}

/* ---------- reveal on scroll ---------- */

const io = new IntersectionObserver(
  (entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      en.target.classList.add("in");
      io.unobserve(en.target);
    }
  },
  { rootMargin: "0px 0px -8% 0px" }
);
for (const group of document.querySelectorAll(".hero-copy, .stats, .bento, .encore-copy")) {
  [...group.querySelectorAll(".reveal")].forEach((el, i) => el.style.setProperty("--delay", `${i * 0.09}s`));
}
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));

/* ---------- loop ---------- */

resize();
onScroll();
addEventListener("resize", () => { resize(); onScroll(); });

if (calm) {
  drawReflections(0);
} else {
  const frame = (t) => {
    updatePulse();
    drawReflections(t);
    drawTiles();
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

/* ---------- download: resolve the latest release asset ---------- */

const downloads = document.querySelectorAll("[data-download]");
const labels = document.querySelectorAll("[data-download-label]");
const metas = document.querySelectorAll("[data-download-meta]");

function setAll(nodes, fn) { nodes.forEach(fn); }

function pickAsset(assets) {
  const dmg = assets.filter((a) => a.name.toLowerCase().endsWith(".dmg"));
  return (
    dmg.find((a) => /universal/i.test(a.name)) ||
    dmg[0] ||
    assets.find((a) => a.name.toLowerCase().endsWith(".app.zip")) ||
    assets.find((a) => a.name.toLowerCase().endsWith(".zip"))
  );
}

fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
  headers: { Accept: "application/vnd.github+json" },
})
  .then((r) => {
    if (r.status === 404) throw new Error("no-release");
    if (!r.ok) throw new Error("api");
    return r.json();
  })
  .then((release) => {
    const asset = pickAsset(release.assets || []);
    const size = asset ? ` · ${(asset.size / 1024 / 1024).toFixed(0)} MB` : "";
    setAll(downloads, (a) => (a.href = asset ? asset.browser_download_url : release.html_url));
    setAll(labels, (l) => (l.textContent = "Download for Mac"));
    setAll(metas, (m) => (m.textContent = `${release.tag_name}${size} · Free and open source · macOS 12 or later`));
  })
  .catch((err) => {
    // No release published yet: send people to the build instructions instead of a 404.
    if (err.message !== "no-release") return; // rate limited or offline: keep /releases/latest
    setAll(downloads, (a) => (a.href = `https://github.com/${REPO}#running-it`));
    setAll(labels, (l) => (l.textContent = "Get it on GitHub"));
    setAll(metas, (m) => (m.textContent = "No build published yet, so build from source for now · macOS 12 or later"));
  });
