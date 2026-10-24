"""2.5 s of speech-like audio: harmonics with a syllable-rate envelope. 48 kHz stereo s16le."""
import math, struct, sys
rate, secs = 48000, 2.5
with open(sys.argv[1], "wb") as f:
    for i in range(int(rate * secs)):
        t = i / rate
        env = 0.5 + 0.5 * math.sin(2 * math.pi * 4 * t)
        v = env * 0.25 * sum(math.sin(2 * math.pi * f0 * t) / k for k, f0 in enumerate([140, 280, 420, 700], 1))
        s = int(max(-1, min(1, v)) * 32767)
        f.write(struct.pack("<hh", s, s))
