"""Renders Nimbra's night (src/night.rs): a dusk horizon in OKLab with domain-warped
stratus, worked out per pixel as a fragment shader would, and the film-grain tile the app
lays over it at one texel per device pixel. Deterministic: the same files every run.

Usage: python3 -I scripts/night.py [out_dir]   (default: assets/night; needs NumPy, Pillow)
"""

import sys
import numpy as np
from PIL import Image

W, H = 1280, 800
HORIZON = 0.72  # night::HORIZON

# ---------------------------------------------------------------- colour


def oklch(l, c, h):
    a = c * np.cos(np.radians(h))
    b = c * np.sin(np.radians(h))
    return np.array([l, a, b], dtype=np.float64)


def oklab_to_linear_srgb(lab):
    l, a, b = lab[..., 0], lab[..., 1], lab[..., 2]
    l_ = l + 0.3963377774 * a + 0.2158037573 * b
    m_ = l - 0.1055613458 * a - 0.0638541728 * b
    s_ = l - 0.0894841775 * a - 1.2914855480 * b
    l3, m3, s3 = l_**3, m_**3, s_**3
    r = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3
    g = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3
    bl = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3
    return np.stack([r, g, bl], axis=-1)


def encode_srgb(lin):
    lin = np.clip(lin, 0.0, 1.0)
    return np.where(lin <= 0.0031308, 12.92 * lin, 1.055 * np.power(lin, 1 / 2.4) - 0.055)


# ---------------------------------------------------------------- noise


class Noise:
    def __init__(self, seed, size=256):
        rng = np.random.default_rng(seed)
        self.table = rng.random((size, size))
        self.size = size

    def __call__(self, x, y):
        n = self.size
        xi, yi = np.floor(x).astype(np.int64), np.floor(y).astype(np.int64)
        fx, fy = x - xi, y - yi
        ux = fx * fx * fx * (fx * (fx * 6 - 15) + 10)
        uy = fy * fy * fy * (fy * (fy * 6 - 15) + 10)
        t = self.table
        x0, x1, y0, y1 = xi % n, (xi + 1) % n, yi % n, (yi + 1) % n
        a, b = t[y0, x0], t[y0, x1]
        c, d = t[y1, x0], t[y1, x1]
        return (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uy

    def fbm(self, x, y, octaves=5):
        total, amp, freq, norm = 0.0, 0.5, 1.0, 0.0
        for k in range(octaves):
            # Rotate each octave so lattice axes never line up into visible streaks.
            ang = 0.5 * k
            ca, sa = np.cos(ang), np.sin(ang)
            xr, yr = ca * x - sa * y, sa * x + ca * y
            total = total + amp * self(xr * freq + 17.3 * k, yr * freq + 9.1 * k)
            norm += amp
            amp *= 0.5
            freq *= 2.03
        return total / norm


# ---------------------------------------------------------------- field


def ramp(stops, v, soften):
    """Stops [(position, oklab)] interpolated in OKLab, then softened so no stop leaves a
    Mach band (a kink in a ramp reads as a line)."""
    lut_n = 4096
    t = np.linspace(0, 1, lut_n)
    pos = np.array([p for p, _ in stops])
    labs = np.array([c for _, c in stops])
    lut = np.stack([np.interp(t, pos, labs[:, i]) for i in range(3)], axis=-1)
    sigma = soften * lut_n
    k = np.arange(-int(3 * sigma), int(3 * sigma) + 1)
    kernel = np.exp(-0.5 * (k / sigma) ** 2)
    kernel /= kernel.sum()
    pad = len(k) // 2
    padded = np.pad(lut, ((pad, pad), (0, 0)), mode="edge")
    smooth = np.stack([np.convolve(padded[:, i], kernel, mode="valid") for i in range(3)], -1)
    idx = np.clip((v * (lut_n - 1)).astype(np.int64), 0, lut_n - 1)
    return smooth[idx]


def mix(a, b, t):
    return a + (b - a) * t[..., None]


def field(theme):
    v, u = np.mgrid[0:H, 0:W].astype(np.float64)
    v = (v + 0.5) / H
    u = (u + 0.5) / W
    aspect = W / H

    if theme == "dark":
        stops = [
            (0.00, oklch(0.135, 0.010, 278)),  # ink
            (0.34, oklch(0.165, 0.030, 280)),  # deep indigo
            (0.58, oklch(0.215, 0.052, 292)),  # indigo into violet
            (0.68, oklch(0.265, 0.050, 318)),  # dusty violet
            (HORIZON, oklch(0.300, 0.046, 352)),  # mauve at the line
            (0.745, oklch(0.180, 0.020, 300)),  # the land takes over at once
            (1.00, oklch(0.125, 0.010, 280)),
        ]
        ember = oklch(0.400, 0.075, 48)  # muted ember, low chroma
        ember_peak = 0.62
        cloud_amp = 0.032
        vignette = 0.10
    else:
        stops = [
            (0.00, oklch(0.982, 0.004, 285)),  # paper
            (0.40, oklch(0.968, 0.012, 292)),  # lilac haze
            (0.62, oklch(0.952, 0.020, 305)),
            (HORIZON, oklch(0.940, 0.022, 62)),  # warm sand at the line
            (0.745, oklch(0.958, 0.012, 70)),
            (1.00, oklch(0.975, 0.005, 285)),
        ]
        ember = oklch(0.915, 0.042, 68)
        ember_peak = 0.55
        cloud_amp = 0.010
        vignette = 0.025

    lab = ramp(stops, v, 0.018)

    # Stratus near the horizon: domain-warped fbm, stretched along the horizon.
    noise = Noise(7)
    px, py = u * aspect * 2.0, v * 14.0
    qx = noise.fbm(px, py, 4)
    qy = noise.fbm(px + 5.2, py + 1.3, 4)
    rx = noise.fbm(px + 3.0 * qx + 1.7, py + 3.0 * qy + 9.2, 4)
    ry = noise.fbm(px + 3.0 * qx + 8.3, py + 3.0 * qy + 2.8, 4)
    cloud = noise.fbm(px + 2.5 * rx, py + 2.5 * ry, 5)
    cloud = (cloud - 0.5) * 2.0
    band = np.exp(-(((v - (HORIZON - 0.14)) / 0.13) ** 2)) * (v < HORIZON)
    band = band * np.clip((HORIZON - v) / 0.015, 0, 1)
    lab[..., 0] += cloud_amp * cloud * band
    # Clouds catch the warm light from below: a touch of the ember's hue in their bright parts.
    lab[..., 1:] += 0.25 * (ember[1:] - lab[..., 1:]) * (np.clip(cloud, 0, 1) * band)[..., None] * 0.35

    # Last light, off to the right of centre. It spreads up into the sky and stops at the line.
    dx = (u - 0.64) * aspect
    dv = v - HORIZON
    up = np.exp(-((dx / 0.62) ** 2)) * np.exp(-((np.maximum(-dv, 0) / 0.11) ** 1.6))
    down = np.exp(-((dx / 0.5) ** 2)) * np.exp(-((np.maximum(dv, 0) / 0.012) ** 2))
    glow = np.where(dv < 0, up, down)
    # The light varies along the horizon with the same clouds, so it is never a smooth blob.
    glow = glow * (1.0 + 0.35 * cloud * band)
    lab = mix(lab, np.broadcast_to(ember, lab.shape), np.clip(glow * ember_peak, 0, 1))

    # A faint reflection on the land, broken up like a wet field.
    refl = np.exp(-((dx / 0.6) ** 2)) * np.exp(-(((v - HORIZON) / 0.05) ** 2)) * (v > HORIZON + 0.004)
    ripple = noise.fbm(u * aspect * 1.4 + 40, v * 60.0, 3)
    lab = mix(lab, np.broadcast_to(ember, lab.shape), np.clip(refl * ripple * 0.12 * ember_peak, 0, 1))

    # Vignette, in lightness only.
    cx, cy = (u - 0.5) * aspect / 1.2, (v - 0.55)
    r2 = cx * cx + cy * cy
    lab[..., 0] *= 1.0 - vignette * np.clip(r2, 0, 1)

    rgb = encode_srgb(oklab_to_linear_srgb(lab))
    return (np.round(rgb * 255)).astype(np.uint8)


def grain(size=256, seed=3):
    """Luminance grain: black or white texels at a few percent, slightly clumped so it reads
    as film, not as digital static. Alpha carries the amount."""
    rng = np.random.default_rng(seed)
    n = rng.standard_normal((size, size))
    # Clump: a little of each neighbour, wrapping so the tile repeats seamlessly.
    n = 0.7 * n + 0.15 * (np.roll(n, 1, 0) + np.roll(n, 1, 1))
    n /= n.std()
    alpha = np.clip(np.abs(n) * 9.0, 0, 40)  # 0..~16% at the extremes, ~7 levels mean
    alpha = np.round(alpha).astype(np.uint8)
    colour = np.where(n > 0, 255, 0).astype(np.uint8)
    rgba = np.stack([colour, colour, colour, alpha], -1)
    return rgba


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "assets/night"
    for theme in ("dark", "light"):
        Image.fromarray(field(theme), "RGB").save(f"{out}/night-{theme}.png", optimize=True)
    Image.fromarray(grain(), "RGBA").save(f"{out}/grain.png", optimize=True)


if __name__ == "__main__":
    main()
