"""Synthesizes the launch video soundtrack (120 BPM, 26 s) so there's no licensed music involved."""
import math, random, struct, wave

SR, LEN = 44100, 26.0
N = int(SR * LEN)
buf = [0.0] * N
random.seed(4)

def add(t0, samples, gain=1.0):
    i0 = int(t0 * SR)
    for i, s in enumerate(samples):
        if 0 <= i0 + i < N:
            buf[i0 + i] += s * gain

def kick(d=0.42):
    out, ph = [], 0.0
    for i in range(int(d * SR)):
        t = i / SR
        ph += 2 * math.pi * (45 + 120 * math.exp(-t * 28)) / SR
        out.append(math.sin(ph) * math.exp(-t * 7) + (random.uniform(-1, 1) * math.exp(-t * 300) * 0.3))
    return out

def noise(d, decay, hp=True):
    out, prev = [], 0.0
    for i in range(int(d * SR)):
        n = random.uniform(-1, 1)
        s = n - prev if hp else n
        prev = n
        out.append(s * math.exp(-(i / SR) * decay))
    return out

def clap():
    return [s * 0.8 for s in noise(0.22, 18, hp=False)]

def impact():
    boom = kick(1.4)
    crash = noise(1.6, 2.6)
    return [boom[i] * 1.2 + (crash[i] * 0.35 if i < len(crash) else 0) for i in range(len(boom))] + crash[len(boom):]

def riser(d):
    out = []
    for i in range(int(d * SR)):
        x = i / (d * SR)
        out.append(random.uniform(-1, 1) * x ** 2 * 0.35 + math.sin(2 * math.pi * (200 + 1400 * x * x) * i / SR) * x * 0.12)
    return out

def tick():
    return [math.sin(2 * math.pi * 1800 * i / SR) * math.exp(-i / SR * 60) * 0.5 for i in range(int(0.08 * SR))]

beat = 0.5
# 0-3 s: tense thumps while the Mac fills up, riser into the logo.
for t in (0, 1.0, 1.5, 2.0, 2.5):
    add(t, kick(), 0.7)
add(2.0, riser(1.0))
add(3.0, impact(), 0.9)
# 3-14 s: groove.
def groove(a, b, clap_on=True):
    t = a
    while t < b - 1e-6:
        add(t, kick(), 0.85)
        add(t + beat / 2, noise(0.05, 90), 0.22)
        if clap_on and round((t - a) / beat) % 2 == 1:
            add(t, clap(), 0.35)
        t += beat
groove(3.5, 14.0)
for t in (8.0, 10.0, 11.0, 12.0, 13.0):
    add(t, noise(0.35, 9), 0.18)  # swish on feature cuts
# 14-18 s: Xcode. Drop out, then ticks as the bars land, hit on the punchline.
add(14.0, impact(), 0.6)
for i in range(5):
    add(15.0 + i * 7 / 30, tick(), 1.0)
add(15.0, kick(), 0.6); add(16.0, kick(), 0.6)
add(16.0, riser(1.0), 0.8)
add(17.0, impact(), 1.0)
add(17.5, kick(), 0.7)
# 18-21 s: counter rises, lands at 19.5 s.
add(18.0, riser(1.5), 1.1)
add(19.5, impact(), 1.1)
groove(20.0, 23.0)
# 23-26 s: end card.
add(23.0, impact(), 0.9)
groove(23.5, 25.0, clap_on=False)

peak = max(abs(s) for s in buf) or 1
fade = int(1.5 * SR)
with wave.open('public/beat.wav', 'wb') as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
    frames = bytearray()
    for i, s in enumerate(buf):
        g = min(1.0, (N - i) / fade)
        frames += struct.pack('<h', int(max(-1, min(1, s / peak * 0.9 * g)) * 32767))
    w.writeframes(bytes(frames))
print('ok')
