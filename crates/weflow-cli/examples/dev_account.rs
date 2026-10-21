//! The synthetic account `bun run dev` in `web/` serves (#333): made-up people, chats and pictures in real encrypted
//! databases, so the page can be worked on and tried by hand without a WeChat account.
//!
//! usage: dev_account <work dir> --weflow <weflow binary>
//!
//! `<work dir>/data` holds the account and `<work dir>/home` the WeFlow home pointing at it; both are built again only
//! when [`VERSION`] changes. The home is printed on stdout.
//!
//! What is in it: private chats with a few friends (50 to 100 messages each), a group whose pictures are on this computer, some Moments, and
//! 「图片测试」, a chat of one picture per case of the files WeChat keeps of it (#194): the thumbnail a chat shows
//! (`<md5>_t.dat`), the image opened (`<md5>.dat`) and the original downloaded on View original (`<md5>_h.dat`), each
//! there or not. Every picture is a JPEG with its file and size written on it, so which one a page shows is plain.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, ensure, Context, Result};
use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, RoomSpec, SessionSpec, SnsPostSpec};

/// Bump on any change to what is built, so the account is built again.
const VERSION: u32 = 2;
/// The account's own wxid, and its folder as WeChat names it (the wxid and a suffix of four).
const OWNER: &str = "wxid_devme";
const ACCOUNT_DIR: &str = "wxid_devme_d3v0";
const PICTURES: &str = "wxid_devpictures";
const ROOM: &str = "dev_hiking@chatroom";
const XOR_KEY: u8 = 0x5a;
const MINUTE: i64 = 60;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

const FRIENDS: &[(&str, &str)] = &[
    ("wxid_devlin", "林沐"),
    ("wxid_devzhou", "周舟"),
    ("wxid_devchen", "陈一苇"),
    ("wxid_devxu", "许知远"),
    ("wxid_devhe", "何夕"),
    ("wxid_devsong", "宋小满"),
    ("wxid_devgu", "顾南"),
    ("wxid_devshen", "沈星河"),
];

const LINES: &[&str] = &[
    "周末有空吗？",
    "有的，去哪儿？",
    "听说山上的枫叶红了",
    "那就周六早上八点出发",
    "好，我带水和零食",
    "记得穿防滑的鞋",
    "收到 👍",
    "上次拍的照片发我一下",
    "晚点整理好发你",
    "今天下雨了，改天吧",
    "没问题，下周再约",
    "路上注意安全",
    "早上好，今天降温了，多穿点",
    "中午一起吃饭吗？楼下新开了一家面馆",
    "可以，十二点楼下见",
    "我可能晚十分钟，会议还没结束",
    "不急，我先去排队",
    "这家的牛肉面真不错，下次还来",
    "周报写完了吗？",
    "还差一点，下班前发你",
    "辛苦啦",
    "明天几点的高铁？",
    "八点四十，七点半要出门",
    "要不要我送你去车站",
    "不用啦，地铁很方便",
    "到了说一声",
    "已经到了，酒店挺干净的",
    "哈哈哈哈哈",
    "这个视频笑死我了",
    "你看那本书了吗？我觉得结尾有点仓促",
    "看了，前面很精彩，后面确实赶了点",
    "生日快乐！🎂",
    "谢谢！晚上一起吃蛋糕",
    "猫今天又把杯子推下桌了",
    "它是不是在报复你昨天没陪它玩",
    "快递到了，帮我拿一下好吗",
    "好的，放你门口了",
    "下个月的旅行计划定了吗",
    "初步定在云南，大理加丽江",
    "我也想去！能带上我吗",
    "当然，人多热闹",
    "晚安",
];

/// Which files of a picture are on this computer.
#[derive(Clone, Copy, PartialEq)]
enum File {
    Thumb,
    Normal,
    Hd,
}

struct Case {
    note: &'static str,
    /// Width over height.
    aspect: f64,
    files: &'static [File],
    /// The original's size as the message gives it when the original is not here; `None`: the picture has none.
    hd_length: Option<u64>,
    /// Longest side of the original, in pixels.
    hd_side: u32,
    hue: f64,
}

const CASES: &[Case] = &[
    Case {
        note: "① 横图，原图已下载：点开显示 .dat，标题栏「原图」可点，点了换成 _h.dat",
        aspect: 4.0 / 3.0,
        files: &[File::Thumb, File::Normal, File::Hd],
        hd_length: None,
        hd_side: 3024,
        hue: 20.0,
    },
    Case {
        note: "② 竖图，原图还没下载：「原图」不可点，鼠标放上去有提示",
        aspect: 3.0 / 4.0,
        files: &[File::Thumb, File::Normal],
        hd_length: Some(3_100_000),
        hd_side: 0,
        hue: 200.0,
    },
    Case {
        note: "③ 没有原图：「原图」不可点，提示这张图片没有高清原图",
        aspect: 1.0,
        files: &[File::Thumb, File::Normal],
        hd_length: None,
        hd_side: 0,
        hue: 120.0,
    },
    Case {
        note: "④ 原图很大（4032 像素）：点「原图」后按钮先转圈，原图到了再变",
        aspect: 16.0 / 9.0,
        files: &[File::Thumb, File::Normal, File::Hd],
        hd_length: None,
        hd_side: 4032,
        hue: 280.0,
    },
    Case {
        note: "⑤ 只有缩略图：点开的图片也没下载，预览里仍是缩略图",
        aspect: 4.0 / 3.0,
        files: &[File::Thumb],
        hd_length: Some(1_800_000),
        hd_side: 0,
        hue: 45.0,
    },
    Case {
        note: "⑥ 只有原图（没有 .dat）：点开显示的就是 _h.dat",
        aspect: 3.0 / 2.0,
        files: &[File::Thumb, File::Hd],
        hd_length: None,
        hd_side: 3024,
        hue: 330.0,
    },
    Case {
        note: "⑦ 很长的竖图：看预览框按比例定的大小",
        aspect: 1.0 / 3.0,
        files: &[File::Thumb, File::Normal, File::Hd],
        hd_length: None,
        hd_side: 3024,
        hue: 170.0,
    },
    Case {
        note: "⑧ 全景横图：同上",
        aspect: 4.0,
        files: &[File::Thumb, File::Normal],
        hd_length: None,
        hd_side: 0,
        hue: 90.0,
    },
    Case {
        note: "⑨ 图片打不开：三个文件都没有，看失败的样子",
        aspect: 4.0 / 3.0,
        files: &[],
        hd_length: None,
        hd_side: 0,
        hue: 0.0,
    },
];

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// ---- pictures ----

/// 5×7 glyphs of what the labels say, a row per byte (the low five bits, the leftmost the highest).
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        '#' => [0x0a, 0x0a, 0x1f, 0x0a, 0x1f, 0x0a, 0x0a],
        '_' => [0, 0, 0, 0, 0, 0, 0x1f],
        '.' => [0, 0, 0, 0, 0, 0x0c, 0x0c],
        't' => [0x08, 0x08, 0x1c, 0x08, 0x08, 0x09, 0x06],
        'd' => [0x01, 0x01, 0x0d, 0x13, 0x11, 0x13, 0x0d],
        'a' => [0, 0, 0x0e, 0x01, 0x0f, 0x11, 0x0f],
        'h' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11],
        'x' => [0, 0, 0x11, 0x0a, 0x04, 0x0a, 0x11],
        _ => [0; 7],
    }
}

struct Canvas {
    w: u32,
    h: u32,
    rgb: Vec<u8>,
}

impl Canvas {
    fn set(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = (y as usize * self.w as usize + x as usize) * 3;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    fn rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: [u8; 3]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    /// `text` centred on `cy`, white on a dark outline, `scale` pixels a dot.
    fn text(&mut self, text: &str, cy: i64, scale: i64) {
        let width = text.chars().count() as i64 * 6 * scale - scale;
        let x0 = (self.w as i64 - width) / 2;
        let y0 = cy - 7 * scale / 2;
        for (color, grow) in [([20, 20, 30], scale.max(2) / 2), ([255, 255, 255], 0)] {
            for (k, c) in text.chars().enumerate() {
                for (row, bits) in glyph(c).iter().enumerate() {
                    for col in 0..5 {
                        if bits & (0x10 >> col) != 0 {
                            let x = x0 + (k as i64 * 6 + col) * scale;
                            let y = y0 + row as i64 * scale;
                            self.rect(
                                x - grow,
                                y - grow,
                                scale + 2 * grow,
                                scale + 2 * grow,
                                color,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// A landscape `w`×`h` in `hue`, with `label` and its size written across it, as a JPEG.
fn picture(w: u32, h: u32, hue: f64, label: &str) -> Vec<u8> {
    let mut c = Canvas {
        w,
        h,
        rgb: vec![0; (w * h * 3) as usize],
    };
    let (top, bottom) = (hsl(hue, 0.55, 0.32), hsl(hue + 30.0, 0.8, 0.72));
    let (sun_x, sun_y, sun_r) = (w as f64 * 0.68, h as f64 * 0.4, w.min(h) as f64 * 0.09);
    // each hill's ridge per column and its colour, worked out once: the loop below runs for every pixel
    let hills: Vec<(Vec<u32>, [u8; 3])> = [(0.62, 0.40), (0.74, 0.27), (0.86, 0.15)]
        .iter()
        .enumerate()
        .map(|(k, (base, light))| {
            let ridge = (0..w)
                .map(|x| {
                    (h as f64 * base
                        + (x as f64 / w as f64 * 9.0 + k as f64 * 2.1).sin() * h as f64 * 0.035)
                        as u32
                })
                .collect();
            (ridge, hsl(hue, 0.3, *light))
        })
        .collect();
    let r2 = sun_r * sun_r;
    for y in 0..h {
        let t = y as f64 / h as f64;
        let sky: [u8; 3] =
            std::array::from_fn(|i| (top[i] as f64 + (bottom[i] as f64 - top[i] as f64) * t) as u8);
        let dy2 = (y as f64 - sun_y).powi(2);
        let row = &mut c.rgb[(y * w * 3) as usize..((y + 1) * w * 3) as usize];
        for (x, px) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let mut color = if dy2 <= r2 && (x as f64 - sun_x).powi(2) + dy2 < r2 {
                [255, 230, 160]
            } else {
                sky
            };
            for (ridge, hill) in &hills {
                if y > ridge[x] {
                    color = *hill;
                }
            }
            *px = color;
        }
    }
    let scale = (w.min(h) as i64 / 60).max(1);
    c.text(label, h as i64 / 2 - 5 * scale, scale);
    c.text(
        &format!("{w}x{h}"),
        h as i64 / 2 + 6 * scale,
        (scale * 2 / 3).max(1),
    );
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 85)
        .encode(&c.rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
        .expect("encode a JPEG");
    out
}

/// `w`×`h` with the longest side `side`.
fn size(aspect: f64, side: u32) -> (u32, u32) {
    if aspect >= 1.0 {
        (side, ((side as f64 / aspect).round() as u32).max(1))
    } else {
        (((side as f64 * aspect).round() as u32).max(1), side)
    }
}

/// An old-style `.dat`: the image XOR-ed with the image key.
fn dat(image: &[u8]) -> Vec<u8> {
    image.iter().map(|b| b ^ XOR_KEY).collect()
}

/// Writes the files of one picture where WeChat keeps them, and gives the message's XML.
#[allow(clippy::too_many_arguments)]
fn write_picture(
    account: &Path,
    session: &str,
    at: i64,
    md5: &str,
    aspect: f64,
    files: &[File],
    hd_side: u32,
    hd_length: Option<u64>,
    hue: f64,
    tag: &str,
) -> Result<String> {
    let dir = account
        .join("msg/attach")
        .join(weflow_core::image::session_dir_for_storage(
            session,
            weflow_core::services::clean_account_dir_name,
        ))
        .join(weflow_core::image::year_month_from_create_time(Some(at)))
        .join("Img");
    std::fs::create_dir_all(&dir)?;
    let file = |suffix: &str, side: u32| -> Result<u64> {
        let (w, h) = size(aspect, side);
        let image = picture(w, h, hue, &format!("{tag} {suffix}.dat"));
        let path = dir.join(format!("{md5}{suffix}.dat"));
        let here = files.contains(&match suffix {
            "_t" => File::Thumb,
            "" => File::Normal,
            _ => File::Hd,
        });
        if here {
            std::fs::write(path, dat(&image))?;
        }
        Ok(image.len() as u64)
    };
    let thumb = file("_t", 240)?;
    let normal = file("", 1280)?;
    // the original's size: of its file when it is here, else as given (none: the picture has no original)
    let hd = if files.contains(&File::Hd) {
        Some(file("_h", hd_side)?)
    } else {
        hd_length
    };
    let hd = hd.map(|n| format!(" hdlength=\"{n}\"")).unwrap_or_default();
    Ok(format!(
        "<msg><img md5=\"{md5}\" length=\"{normal}\"{hd} cdnthumblength=\"{thumb}\" /></msg>"
    ))
}

// ---- the account ----

fn build(data: &Path, now: i64) -> Result<Fixture> {
    let f = Fixture::new(data, ACCOUNT_DIR);
    let mut contacts = vec![
        ContactSpec::new(OWNER, 1, "我"),
        ContactSpec::new(PICTURES, 1, "图片测试"),
        ContactSpec::new(ROOM, 2, "周末爬山群"),
    ];
    contacts.extend(FRIENDS.iter().map(|(u, n)| ContactSpec::new(u, 1, n)));
    let members: Vec<(&'static str, &'static str)> = std::iter::once((OWNER, ""))
        .chain(FRIENDS.iter().take(6).map(|(u, _)| (*u, "")))
        .collect();
    f.contact_db(
        &contacts,
        &[RoomSpec {
            username: ROOM,
            owner: FRIENDS[0].0,
            members: Box::leak(members.into_boxed_slice()),
        }],
    );

    let mut chats: Vec<(&str, Vec<MsgSpec>)> = Vec::new();
    let mut sessions = Vec::new();

    // the picture cases, a note before each, the newest conversation
    let mut msgs = Vec::new();
    let start = now - CASES.len() as i64 * 2 * MINUTE;
    for (k, case) in CASES.iter().enumerate() {
        let at = start + k as i64 * 2 * MINUTE;
        msgs.push(MsgSpec::text(2 * k as i64 + 1, PICTURES, at, case.note));
        let md5 = format!("{:032x}", 0xd0_0000 + k);
        let xml = write_picture(
            &f.account_dir,
            PICTURES,
            at + MINUTE,
            &md5,
            case.aspect,
            case.files,
            case.hd_side,
            case.hd_length,
            case.hue,
            &format!("#{}", k + 1),
        )?;
        let sender = if k % 2 == 0 { PICTURES } else { OWNER };
        msgs.push(MsgSpec::text(2 * k as i64 + 2, sender, at + MINUTE, &xml).of_type(3));
    }
    let last = msgs.last().map(|m| m.create_time).unwrap_or(now);
    sessions.push(SessionSpec {
        username: PICTURES,
        summary: "[图片]",
        last_timestamp: last,
        unread: 1,
        last_msg_type: 3,
    });
    chats.push((PICTURES, msgs));

    // the group: a day of talk every few days over two months, a picture now and then
    let mut msgs = Vec::new();
    let mut id = 1;
    for day in (0..20i64).rev().map(|k| k * 3) {
        let base = now - day * DAY - 5 * HOUR;
        for j in 0..12i64 {
            let at = base + j * 7 * MINUTE;
            let sender = members_of(id);
            if id % 9 == 0 {
                let md5 = format!("{:032x}", 0xe0_0000 + id);
                let aspect = [4.0 / 3.0, 3.0 / 4.0, 1.0][id as usize % 3];
                let hd: &[File] = if id % 36 == 0 {
                    &[File::Thumb, File::Normal, File::Hd]
                } else {
                    &[File::Thumb, File::Normal]
                };
                let xml = write_picture(
                    &f.account_dir,
                    ROOM,
                    at,
                    &md5,
                    aspect,
                    hd,
                    2048,
                    Some(2_000_000),
                    (id * 37 % 360) as f64,
                    &format!("#{id}"),
                )?;
                msgs.push(MsgSpec::text(id, sender, at, &xml).of_type(3));
            } else {
                msgs.push(MsgSpec::text(
                    id,
                    sender,
                    at,
                    LINES[(id as usize * 5) % LINES.len()],
                ));
            }
            id += 1;
        }
    }
    let last = msgs.last().map(|m| m.create_time).unwrap_or(now);
    sessions.push(SessionSpec {
        username: ROOM,
        summary: "路上注意安全",
        last_timestamp: last,
        unread: 4,
        last_msg_type: 1,
    });
    chats.push((ROOM, msgs));

    // the friends: 50 to 100 messages each, a few talks over the weeks before the latest, a day further back each
    for (k, (user, _)) in FRIENDS.iter().enumerate() {
        let newest = now - (k as i64 + 1) * DAY;
        let count = 50 + (k as i64 * 37) % 51;
        let msgs: Vec<MsgSpec> = (0..count)
            .map(|j| {
                let back = count - 1 - j;
                // a talk of up to 12 messages a few minutes apart, then three days back to the one before
                let at = newest - (back / 12) * 3 * DAY - (back % 12) * 4 * MINUTE;
                let sender = if (j + k as i64) % 3 == 0 {
                    OWNER
                } else {
                    *user
                };
                MsgSpec::text(
                    j + 1,
                    sender,
                    at,
                    LINES[(j as usize * 7 + k * 11) % LINES.len()],
                )
            })
            .collect();
        let summary = leak(msgs.last().map(|m| m.content.clone()).unwrap_or_default());
        sessions.push(SessionSpec {
            username: user,
            summary,
            last_timestamp: newest,
            unread: 0,
            last_msg_type: 1,
        });
        chats.push((user, msgs));
    }

    f.session_db(&sessions);
    f.message_shard(0, &chats);
    f.sns_db(&[
        SnsPostSpec {
            tid: 3001,
            user: FRIENDS[0].0,
            create_time: now - 2 * DAY,
            desc: "山顶的风很大，但是值得",
            kind: 2,
            media: 0,
            likes: &[(OWNER, "我"), ("wxid_devzhou", "周舟")],
            extra: "",
        },
        SnsPostSpec {
            tid: 3002,
            user: OWNER,
            create_time: now - 5 * DAY,
            desc: "周末的书店",
            kind: 2,
            media: 0,
            likes: &[("wxid_devlin", "林沐")],
            extra: "",
        },
        SnsPostSpec {
            tid: 3003,
            user: FRIENDS[2].0,
            create_time: now - 9 * DAY,
            desc: "新学的菜，第一次就成功了",
            kind: 2,
            media: 0,
            likes: &[],
            extra: "",
        },
    ]);
    Ok(f)
}

fn members_of(id: i64) -> &'static str {
    if id % 5 == 0 {
        OWNER
    } else {
        FRIENDS[(id as usize * 3) % 6].0
    }
}

fn weflow(weflow: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(weflow);
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

fn main() -> Result<()> {
    const USAGE: &str = "usage: dev_account <work dir> --weflow <weflow binary>";
    let mut args = std::env::args().skip(1);
    let work = PathBuf::from(args.next().ok_or_else(|| anyhow!(USAGE))?);
    std::fs::create_dir_all(&work)?;
    // the home keeps the account's path: from anywhere
    let work = std::path::absolute(&work)?;
    let mut exe = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--weflow" => exe = args.next().map(PathBuf::from),
            other => return Err(anyhow!("unknown argument {other}\n{USAGE}")),
        }
    }
    let exe = exe.ok_or_else(|| anyhow!(USAGE))?;
    let (data, home, stamp) = (work.join("data"), work.join("home"), work.join("version"));
    let built = std::fs::read_to_string(&stamp).ok();
    if built.as_deref() != Some(&VERSION.to_string()) || !data.exists() || !home.exists() {
        let started = std::time::Instant::now();
        let _ = std::fs::remove_file(&stamp);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let f = build(&data, now).context("build the account")?;
        let _ = std::fs::remove_dir_all(&home);
        for (key, value) in [
            ("db_path", f.account_dir.to_string_lossy().to_string()),
            ("wxid", ACCOUNT_DIR.to_string()),
            ("decrypt_key", f.key_hex()),
            ("image_xor_key", XOR_KEY.to_string()),
        ] {
            let out = weflow(&exe, &home)
                .args(["config", "set", key, &value])
                .output()?;
            ensure!(
                out.status.success(),
                "config set {key} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::write(&stamp, VERSION.to_string())?;
        eprintln!(
            "built the synthetic account in {:.1} s",
            started.elapsed().as_secs_f64()
        );
    }
    println!("{}", home.display());
    Ok(())
}
