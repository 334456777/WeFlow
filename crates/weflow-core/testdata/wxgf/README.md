# Synthetic HEVC streams

Test input for `src/hevc.rs` and `tests/image_e2e.rs`: the first frame of ffmpeg's `testsrc2` pattern (70x46, a size that
needs a conformance window), encoded with libx265, and ffmpeg's decode of it. No real WeChat data.

```sh
ffmpeg -f lavfi -i testsrc2=size=70x46:rate=1 -frames:v 1 -pix_fmt yuvj420p -color_range pc \
  -c:v libx265 -x265-params log-level=none:info=0 -f hevc full_range.hevc
ffmpeg -f lavfi -i testsrc2=size=70x46:rate=1 -frames:v 1 -pix_fmt yuv420p -color_range tv \
  -c:v libx265 -x265-params log-level=none:info=0 -f hevc limited_range.hevc
ffmpeg -f hevc -i full_range.hevc -frames:v 1 -f rawvideo -pix_fmt yuvj420p full_range.yuv
ffmpeg -f hevc -i limited_range.hevc -frames:v 1 -f rawvideo -pix_fmt yuv420p limited_range.yuv
```

The other layouts and bit depths (Main 10 and range extensions), each named `<pix_fmt>_<full|limited>`:

```sh
for spec in "yuv420p10le pc yuv420p10_full" "yuv420p10le tv yuv420p10_limited" "yuv422p pc yuv422p_full" \
            "yuv444p tv yuv444p_limited" "gray10le pc gray10_full"; do
  set -- $spec
  ffmpeg -f lavfi -i testsrc2=size=70x46:rate=1 -frames:v 1 -pix_fmt $1 -color_range $2 \
    -c:v libx265 -x265-params log-level=none:info=0 -f hevc $3.hevc
  ffmpeg -f hevc -i $3.hevc -frames:v 1 -f rawvideo -pix_fmt $1 $3.yuv
done
```

The `.yuv` files are planar; 10-bit samples take two bytes, little-endian.
