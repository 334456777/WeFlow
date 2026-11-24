# Synthetic HEVC frames of the benchmark account

The WXGF pictures of the benchmark account (`src/fixture/bench/files.rs`, #1221): one intra picture each, of ffmpeg's
generated patterns, encoded with libx265 (ffmpeg 7.1.1). No real WeChat data.

Their sizes copy the aggregates `wxgf_coverage_probe` gave on a real account: a WXGF picture there has a median of
1.2 million luma samples (from 0.34 to 3.1 million between the 10th and the 90th percentile), 4:2:0 at 8 bits, about
0.3 bit per luma sample, and its stream is nearly all of its `.dat`. Each frame stands for the pictures of the file
sizes it is the largest one that fits:

| Frame | Luma samples | Bytes | Bits per luma sample |
|---|---:|---:|---:|
| `wxgf_800x480.hevc` | 384,000 | 11,279 | 0.23 |
| `wxgf_672x1200.hevc` | 806,400 | 30,603 | 0.30 |
| `wxgf_1600x900.hevc` | 1,440,000 | 49,699 | 0.28 |
| `wxgf_1920x1080.hevc` | 2,073,600 | 75,049 | 0.29 |
| `wxgf_2048x1152.hevc` | 2,359,296 | 145,781 | 0.49 |
| `wxgf_1152x2048.hevc` | 2,359,296 | 248,444 | 0.84 |

All are Main profile: libx265 writes its still-picture profile as a range extension, which a real WXGF picture almost
never is, and Main decodes a single intra picture the same way. Each was encoded with

```sh
ffmpeg -f lavfi -i "<pattern>" -frames:v 1 -pix_fmt yuvj420p -color_range pc -c:v libx265 -profile:v main \
  -x265-params crf=<crf>:log-level=none:info=0 -f hevc <frame>.hevc
```

| Frame | CRF | Pattern |
|---|---:|---|
| `wxgf_800x480` | 47 | `mandelbrot=size=800x480:rate=1:start_scale=1.2,noise=alls=10:allf=t:all_seed=1` |
| `wxgf_672x1200` | 39 | `mandelbrot=size=672x1200:rate=1:start_scale=0.3:start_x=-0.16:start_y=1.04:inner=convergence` |
| `wxgf_1600x900` | 42 | `mandelbrot=size=1600x900:rate=1:start_scale=0.9:start_x=-0.55:start_y=0.0,noise=alls=4:allf=t:all_seed=1` |
| `wxgf_1920x1080` | 39 | `mandelbrot=size=1920x1080:rate=1:start_scale=0.4:start_x=-0.74:start_y=0.15,noise=alls=8:allf=t:all_seed=1` |
| `wxgf_2048x1152` | 42 | `mandelbrot=size=2048x1152:rate=1:start_scale=0.05:start_x=-0.7436:start_y=0.1318` |
| `wxgf_1152x2048` | 42 | `life=size=1152x2048:rate=1:ratio=0.4:seed=5:mold=10:life_color=#c0a080:death_color=#203040:mold_color=#406030,gblur=sigma=0.8,noise=alls=14:allf=t:all_seed=1` |
