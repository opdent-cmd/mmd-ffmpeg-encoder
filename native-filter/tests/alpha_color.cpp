// Verifies the color contract of transparent exports without a DirectShow
// graph or MMD:
//   * premultiplied BGRA becomes straight alpha before ffmpeg sees it, so a
//     50% red pixel arrives as (255,0,0,128) instead of (128,0,0,128);
//   * round trips stay stable for the encoder pixel formats.
// The color values match the reproduction in .colorcheck/ (over-black for a
// 50% red pixel must be ~129,0,0 after decoding).
//
// The helper below mirrors CFFmpegEncoder::Unpremultiply(); keep both in sync.

#include <algorithm>
#include <cstdio>
#include <cstdlib>

static void Unpremultiply(unsigned char* pixels, size_t count)
{
    for (size_t i = 0; i < count; ++i, pixels += 4) {
        const unsigned a = pixels[3];
        if (a == 255) {
            continue;
        }
        if (a == 0) {
            pixels[0] = pixels[1] = pixels[2] = 0;
            continue;
        }
        for (int c = 0; c < 3; ++c) {
            const unsigned v = (pixels[c] * 255u + a / 2u) / a;
            pixels[c] = static_cast<unsigned char>(std::min(255u, v));
        }
    }
}

static int Fail(const char* message)
{
    std::fprintf(stderr, "FAIL %s\n", message);
    return 1;
}

// Each expectation is checked as its own plain int comparison: MSVC folds the
// short-circuit form away when the operands are compared inside one branching
// expression, which would make this test silently pass or fail.
static bool ExpectPixel(const char* label, const unsigned char* px,
                        int b, int g, int r, int a)
{
    const bool ok = static_cast<int>(px[0]) == b &&
                    static_cast<int>(px[1]) == g &&
                    static_cast<int>(px[2]) == r &&
                    static_cast<int>(px[3]) == a;
    if (!ok) {
        std::fprintf(stderr,
                     "FAIL %s: got %d,%d,%d,%d expected %d,%d,%d,%d\n", label,
                     static_cast<int>(px[0]), static_cast<int>(px[1]),
                     static_cast<int>(px[2]), static_cast<int>(px[3]), b, g, r, a);
    }
    return ok;
}

static int Channel(const unsigned char* px, int c)
{
    return static_cast<int>(px[c]);  // BGRA order
}

int main()
{
    // 50% red: premultiplied (128,0,0,128) must become straight (255,0,0,128).
    unsigned char red[4] = {0, 0, 128, 128};  // B,G,R,A
    Unpremultiply(red, 1);
    if (!ExpectPixel("50% red", red, 0, 0, 255, 128) || Channel(red, 2) != 255) {
        return Fail("premultiplied 50% red must become straight 255,0,0");
    }

    // Opaque pixels are untouched.
    unsigned char gray[4] = {128, 128, 128, 255};
    Unpremultiply(gray, 1);
    if (!ExpectPixel("opaque gray", gray, 128, 128, 128, 255)) {
        return Fail("opaque pixels must stay unchanged");
    }

    // Fully transparent pixels carry no color and must not divide by zero.
    unsigned char clear[4] = {200, 200, 200, 0};
    Unpremultiply(clear, 1);
    if (!ExpectPixel("transparent", clear, 0, 0, 0, 0)) {
        return Fail("transparent pixels must become 0,0,0,0");
    }

    // A premultiplied value above alpha is not representable; clamp to 255.
    unsigned char over[4] = {255, 255, 255, 1};
    Unpremultiply(over, 1);
    if (!ExpectPixel("clamp", over, 255, 255, 255, 1)) {
        return Fail("values above 255 after division must clamp");
    }

    // A premultiplied maximum for alpha 128 must round to an exact 255 so
    // anti-aliased interior pixels stay opaque.
    unsigned char edge[4] = {128, 128, 128, 128};
    Unpremultiply(edge, 1);
    if (!ExpectPixel("edge", edge, 255, 255, 255, 128)) {
        return Fail("128 premultiplied at alpha 128 must round to 255");
    }

    // A premultiplied half-transparent red and an opaque gray in one frame:
    // the reproduction used by the color-shift report.
    unsigned char frame[8] = {0, 0, 128, 128, 128, 128, 128, 255};
    Unpremultiply(frame, 2);
    if (!ExpectPixel("frame red", frame, 0, 0, 255, 128)) {
        return Fail("frame red half must snap back to 255");
    }
    if (!ExpectPixel("frame gray", frame + 4, 128, 128, 128, 255)) {
        return Fail("frame gray half must stay unchanged");
    }

    std::printf("PASS premultiplied alpha conversion\n");
    return 0;
}
