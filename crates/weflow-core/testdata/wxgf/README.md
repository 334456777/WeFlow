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
