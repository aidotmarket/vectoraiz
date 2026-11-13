"""Bounded 8-bit RGB/RGBA PNG decoding for private pixel comparisons."""
import struct
import zlib


def decode(data):
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError('PNG signature')
    offset, compressed = 8, bytearray()
    width = height = channels = None
    while offset < len(data):
        size, kind = struct.unpack('>I4s', data[offset:offset+8])
        payload = data[offset+8:offset+8+size]
        crc = struct.unpack('>I', data[offset+8+size:offset+12+size])[0]
        if zlib.crc32(kind + payload) != crc:
            raise ValueError('PNG CRC')
        if kind == b'IHDR':
            width, height, depth, color, compression, filtering, interlace = struct.unpack('>IIBBBBB', payload)
            if depth != 8 or color not in (2, 6) or compression or filtering or interlace:
                raise ValueError('unsupported PNG layout')
            if not 0 < width <= 4096 or not 0 < height <= 4096:
                raise ValueError('image bound')
            channels = 3 if color == 2 else 4
        elif kind == b'IDAT':
            compressed.extend(payload)
        elif kind == b'IEND':
            break
        offset += size + 12
    stride = width * channels
    limit = (stride+1)*height
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, limit+1)
    if len(raw) != limit or not decoder.eof:
        raise ValueError('decoded image bound')
    previous = bytearray(stride)
    pixels = bytearray()
    for y in range(height):
        start = y * (stride+1)
        filter_kind = raw[start]
        row = bytearray(raw[start+1:start+1+stride])
        if filter_kind > 4:
            raise ValueError('PNG filter')
        for x in range(stride):
            left = row[x-channels] if x >= channels else 0
            up = previous[x]
            corner = previous[x-channels] if x >= channels else 0
            if filter_kind == 1: predictor = left
            elif filter_kind == 2: predictor = up
            elif filter_kind == 3: predictor = (left+up)//2
            elif filter_kind == 4:
                p = left + up - corner
                distances = (abs(p-left), abs(p-up), abs(p-corner))
                predictor = (left, up, corner)[distances.index(min(distances))]
            else: predictor = 0
            row[x] = (row[x]+predictor) & 255
        pixels.extend(row)
        previous = row
    return width, height, channels, bytes(pixels)


def rgb(decoded):
    width, height, channels, pixels = decoded
    if channels == 3:
        return pixels
    # PDFium's raster should be opaque; never discard meaningful alpha.
    if any(a != 255 for a in pixels[3::4]):
        raise ValueError('nonopaque raster')
    result = bytearray(width*height*3)
    for channel in range(3):
        result[channel::3] = pixels[channel::4]
    return bytes(result)
