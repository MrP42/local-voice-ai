"""Generiert alle Local-Voice-AI-Icons: Wellenform + KI-Funke.

App-Icons: Signalgelb (#FFDD00) auf dunkler WAI-Kachel (#111418, runde Ecken).
Tray-Icons: freistehende Glyphe ohne Kachel — weiß (dunkles Theme) bzw. Ink
(helles Theme); Aufnahme rot, Transkription gelb.
"""

from PIL import Image, ImageDraw

YELLOW = (255, 221, 0, 255)
INK = (17, 20, 24, 255)
WHITE = (245, 246, 248, 255)
RED = (229, 72, 77, 255)

# Design im 32er-Raster (wie das SVG in LocalVoiceAiLogo.tsx).
BARS = [(7, 5), (11.5, 11), (16, 16), (20.5, 11), (25, 5)]  # (x, hoehe)
BAR_W = 2.4
CY = 17.0  # Balken-Mittellinie
DOT = (25.0, 8.2, 1.7)  # x, y, r

SS = 16  # Supersampling


def draw_glyph(d, scale, color, offset=0.0):
    for x, h in BARS:
        x0 = (x - BAR_W / 2 + offset) * scale
        x1 = (x + BAR_W / 2 + offset) * scale
        y0 = (CY - h / 2 + offset) * scale
        y1 = (CY + h / 2 + offset) * scale
        d.rounded_rectangle([x0, y0, x1, y1], radius=(BAR_W / 2) * scale, fill=color)
    dx, dy, r = DOT
    d.ellipse(
        [
            (dx - r + offset) * scale,
            (dy - r + offset) * scale,
            (dx + r + offset) * scale,
            (dy + r + offset) * scale,
        ],
        fill=color,
    )


def tile_icon(size):
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    scale = big / 32
    d.rounded_rectangle([0, 0, big - 1, big - 1], radius=7 * scale, fill=INK)
    draw_glyph(d, scale, YELLOW)
    return img.resize((size, size), Image.LANCZOS)


def tray_icon(size, color):
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Glyphe fuellt die Flaeche staerker (kein Kachelrand): 32er-Design auf
    # 26er-Innenflaeche skalieren, zentriert.
    scale = big / 32 * (32 / 27)
    off = (big - 32 * scale) / (2 * scale)
    draw_glyph(d, scale, color, offset=off)
    return img.resize((size, size), Image.LANCZOS)


def save(img, path):
    img.save(path)
    print("wrote", path, img.size)


# --- App-Icons (icons/) -----------------------------------------------------
app_sizes = {
    "icons/32x32.png": 32,
    "icons/64x64.png": 64,
    "icons/128x128.png": 128,
    "icons/128x128@2x.png": 256,
    "icons/icon.png": 512,
    "icons/logo.png": 512,
    "icons/Square30x30Logo.png": 30,
    "icons/Square44x44Logo.png": 44,
    "icons/Square71x71Logo.png": 71,
    "icons/Square89x89Logo.png": 89,
    "icons/Square107x107Logo.png": 107,
    "icons/Square142x142Logo.png": 142,
    "icons/Square150x150Logo.png": 150,
    "icons/Square284x284Logo.png": 284,
    "icons/Square310x310Logo.png": 310,
    "icons/StoreLogo.png": 50,
}
for path, size in app_sizes.items():
    save(tile_icon(size), path)

# Multi-Size-ICO fuer Explorer/Taskleiste.
ico_sizes = [16, 24, 32, 48, 64, 128, 256]
base = tile_icon(256)
base.save("icons/icon.ico", sizes=[(s, s) for s in ico_sizes])
print("wrote icons/icon.ico", ico_sizes)

# --- Tray-Icons (resources/) ------------------------------------------------
save(tray_icon(64, WHITE), "resources/tray_idle.png")
save(tray_icon(64, INK), "resources/tray_idle_dark.png")
save(tray_icon(64, RED), "resources/tray_recording.png")
save(tray_icon(64, RED), "resources/tray_recording_dark.png")
save(tray_icon(64, YELLOW), "resources/tray_transcribing.png")
save(tray_icon(64, YELLOW), "resources/tray_transcribing_dark.png")

# Farbige Linux-Varianten: Glyphe auf Kachel.
save(tile_icon(64), "resources/local-voice.png")


def tile_icon_colored(size, color):
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    scale = big / 32
    d.rounded_rectangle([0, 0, big - 1, big - 1], radius=7 * scale, fill=INK)
    draw_glyph(d, scale, color)
    return img.resize((size, size), Image.LANCZOS)


save(tile_icon_colored(64, RED), "resources/recording.png")
save(tile_icon_colored(64, YELLOW), "resources/transcribing.png")
print("done")
