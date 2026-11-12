"""Print a sixel and a kitty-protocol image, for checking inline images.

    python tools/imgtest.py            # both
    python tools/imgtest.py sixel      # just one
    python tools/imgtest.py kitty
"""

import base64
import struct
import sys
import zlib

W, H = 160, 96


def pixel(x, y):
    # a diagonal colour gradient with a white border
    if x < 2 or y < 2 or x >= W - 2 or y >= H - 2:
        return (255, 255, 255)
    return (x * 255 // W, y * 255 // H, 255 - x * 255 // W)


def sixel():
    # 64-colour palette: 4 levels per channel
    def q(c):
        return min(3, c * 4 // 256)

    out = ['\x1bPq', '"1;1;%d;%d' % (W, H)]
    for i in range(64):
        r, g, b = (i >> 4) & 3, (i >> 2) & 3, i & 3
        out.append('#%d;2;%d;%d;%d' % (i, r * 100 // 3, g * 100 // 3, b * 100 // 3))
    for band in range(0, H, 6):
        for colour in range(64):
            row = []
            used = False
            for x in range(W):
                bits = 0
                for i in range(6):
                    y = band + i
                    if y < H:
                        r, g, b = pixel(x, y)
                        if (q(r) << 4 | q(g) << 2 | q(b)) == colour:
                            bits |= 1 << i
                if bits:
                    used = True
                row.append(chr(0x3F + bits))
            if used:
                out.append('#%d' % colour + ''.join(row) + '$')
        out.append('-')
    out.append('\x1b\\')
    return ''.join(out)


def png():
    raw = b''.join(b'\x00' + b''.join(bytes(pixel(x, y)) for x in range(W)) for y in range(H))

    def chunk(kind, data):
        c = kind + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xFFFFFFFF)

    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def kitty():
    data = base64.standard_b64encode(png()).decode()
    parts = [data[i:i + 4096] for i in range(0, len(data), 4096)]
    out = []
    for i, p in enumerate(parts):
        more = 1 if i < len(parts) - 1 else 0
        keys = 'a=T,f=100,q=2,m=%d' % more if i == 0 else 'm=%d' % more
        out.append('\x1b_G%s;%s\x1b\\' % (keys, p))
    return ''.join(out)


what = sys.argv[1] if len(sys.argv) > 1 else 'both'
if what in ('both', 'sixel'):
    print('sixel:')
    sys.stdout.write(sixel())
    print()
if what in ('both', 'kitty'):
    print('kitty:')
    sys.stdout.write(kitty())
    print()
print('IMGTEST DONE')
sys.stdout.flush()
