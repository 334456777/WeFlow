//! Messages: the rows of each message database, their XML and files, the WAL and the hardlinks.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use md5::{Digest, Md5};
use rusqlite::{params, Connection};

use super::files::{encrypt_dat, jpeg, month, mp4, png, silk, wxgf};
use super::profile::Profile;
use super::rng::{Rng, Weighted};
use super::words::{esc, sized, FILE_EXTENSIONS, PHRASES, TOPICS};
use super::world::{Kind, World};
use super::DAY;
use crate::db::hex;

/// A file written for the account, and its `hardlink.db` row.
pub(super) struct Hardlink {
    pub(super) md5: String,
    pub(super) file_name: String,
    pub(super) kind: i64,
    pub(super) dir1: String,
    pub(super) dir2: String,
    pub(super) size: i64,
    pub(super) modify_time: i64,
}

#[derive(Default)]
pub(super) struct Written {
    pub(super) hardlinks: Vec<Hardlink>,
    /// (conversation, create time, local id, server id, data)
    pub(super) voices: Vec<(usize, i64, i64, i64, Vec<u8>)>,
    pub(super) files: BTreeMap<&'static str, (u64, u64)>,
}

impl Written {
    fn file(&mut self, what: &'static str, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
        let entry = self.files.entry(what).or_default();
        entry.0 += 1;
        entry.1 += data.len() as u64;
        Ok(())
    }
}

/// An earlier message a reply can quote.
pub(super) struct Recent {
    pub(super) server_id: i64,
    pub(super) sender: usize,
    pub(super) local_type: i64,
    pub(super) time: i64,
    pub(super) content: String,
}

/// One message as it is stored.
pub(super) struct Row {
    pub(super) local_id: i64,
    pub(super) server_id: i64,
    pub(super) local_type: i64,
    pub(super) sort_seq: Option<i64>,
    pub(super) sender: Option<usize>,
    pub(super) time: i64,
    pub(super) status: i64,
    pub(super) source: Option<Vec<u8>>,
    pub(super) source_compressed: bool,
    pub(super) content: Vec<u8>,
    pub(super) compressed: bool,
}

pub(super) struct Writer<'a> {
    pub(super) world: &'a World,
    pub(super) profile: &'a Profile,
    pub(super) account_dir: &'a Path,
    pub(super) media: bool,
    pub(super) written: Written,
}

/// The parts of a message the generator makes.
pub(super) struct Made {
    pub(super) content: String,
    /// What WeChat's search index holds of it.
    pub(super) indexed: Option<String>,
    pub(super) summary: String,
    /// The member a group text mentions.
    pub(super) mention: Option<usize>,
}

impl World {
    fn nick(&self, who: usize) -> &str {
        &self.people[who].nick
    }

    pub(super) fn user(&self, who: usize) -> &str {
        &self.people[who].username
    }
}

impl<'a> Writer<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "one message's identity: conversation, sender, time and ids"
    )]
    pub(super) fn make(
        &mut self,
        rng: &mut Rng,
        conv: usize,
        local_type: i64,
        sender: usize,
        time: i64,
        ids: (i64, i64),
        recent: &VecDeque<Recent>,
    ) -> Result<Made> {
        let world: &'a World = self.world;
        let c = &world.conversations[conv];
        let words = &world.words;
        let target = rng.spread(self.profile.content_bytes(local_type));
        let from = world.user(sender);
        let to = &c.username;
        let made = |content: String, summary: &str| Made {
            content,
            indexed: None,
            summary: summary.to_string(),
            mention: None,
        };
        Ok(match local_type {
            1 => {
                let mut text = words.message(rng);
                let mut mention = None;
                if c.kind == Kind::Group && rng.chance(self.profile.messages.at_in_group) {
                    let who = c.members[rng.below(c.members.len())];
                    text = format!("@{}\u{2005}{text}", world.nick(who));
                    mention = Some(who);
                }
                Made {
                    indexed: Some(text.clone()),
                    summary: text.clone(),
                    content: text,
                    mention,
                }
            }
            3 => made(self.image(rng, &c.username, time, target)?, "[图片]"),
            34 => {
                let ms = rng.spread((4000.0, 12_000.0, 60_000.0)).max(1000.0) as i64;
                let data = silk(rng, ms);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<msg><voicemsg endflag=\"1\" cancelflag=\"0\" forwardflag=\"0\" voiceformat=\"4\" voicelength=\"{ms}\" \
                         length=\"{}\" bufid=\"0\" aeskey=\"{}\" voiceurl=\"{fill}\" voicemd5=\"\" clientmsgid=\"{}\" \
                         fromusername=\"{from}\" /></msg>",
                        data.len(),
                        "0".repeat(32),
                        ids.1
                    )
                });
                self.written.voices.push((conv, time, ids.0, ids.1, data));
                made(xml, "[语音]")
            }
            43 => made(self.video(rng, from, time, target)?, "[视频]"),
            47 => {
                let md5 = rng.hex(32);
                let len = rng.between(20_000, 900_000);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<msg><emoji fromusername=\"{from}\" tousername=\"{to}\" type=\"2\" idbuffer=\"media:0_0\" md5=\"{md5}\" \
                         len=\"{len}\" productid=\"\" androidmd5=\"{md5}\" androidlen=\"{len}\" s60v3md5=\"{md5}\" s60v3len=\"{len}\" \
                         s60v5md5=\"{md5}\" s60v5len=\"{len}\" cdnurl=\"http://emoji.invalid/{fill}\" designerid=\"\" thumburl=\"\" \
                         encrypturl=\"http://emoji.invalid/e/{md5}\" aeskey=\"{md5}\" externurl=\"http://emoji.invalid/x/{md5}\" \
                         externmd5=\"{md5}\" width=\"240\" height=\"240\" tpurl=\"\" tpauthkey=\"\" attachedtext=\"\" \
                         attachedtextcolor=\"\" lensid=\"\" emojiattr=\"\" linkid=\"\" desc=\"\"></emoji>\
                         <gameext type=\"0\" content=\"0\"></gameext></msg>"
                    )
                });
                made(xml, "[动画表情]")
            }
            50 => {
                let secs = rng.between(3, 3600);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<voipmsg type=\"VoIPBubbleMsg\"><VoIPBubbleMsg><msg><![CDATA[通话时长 {:02}:{:02}]]></msg>\n\
                         <room_type>{}</room_type>\n<red_dot>false</red_dot>\n<roomid>{}</roomid>\n<roomkey>{fill}</roomkey>\n\
                         <inviteid>{}</inviteid>\n<msg_type>100</msg_type>\n<timestamp>{}</timestamp>\n<duration>0</duration>\n\
                         </VoIPBubbleMsg></voipmsg>",
                        secs / 60,
                        secs % 60,
                        ids.0 % 2,
                        ids.1 % 1_000_000,
                        ids.1 % 999_983,
                        time * 1000
                    )
                });
                made(xml, "[通话]")
            }
            10000 => {
                let other = c.members[rng.below(c.members.len())];
                let text = match rng.below(10) {
                    0..=3 => format!("\"{}\" 撤回了一条消息", world.nick(other)),
                    4 => "你撤回了一条消息".to_string(),
                    5 | 6 if c.kind == Kind::Group => {
                        let invited = c.members[rng.below(c.members.len())];
                        format!(
                            "\"{}\"邀请\"{}\"加入了群聊",
                            world.nick(other),
                            world.nick(invited)
                        )
                    }
                    7 | 8 => format!(
                        "<sysmsg type=\"revokemsg\"><revokemsg><session>{to}</session><msgid>{}</msgid><newmsgid>{}</newmsgid>\
                         <replacemsg><![CDATA[\"{}\" 撤回了一条消息]]></replacemsg></revokemsg></sysmsg>",
                        rng.between(1, 100_000),
                        ids.1,
                        esc(world.nick(other))
                    ),
                    _ => format!(
                        "你已添加了{}，现在可以开始聊天了。",
                        world.nick(other)
                    ),
                };
                made(text.clone(), &text)
            }
            42 => {
                let who = rng.below(world.people.len());
                let (user, nick) = (world.user(who), esc(world.nick(who)));
                made(
                    format!(
                        "<?xml version=\"1.0\"?>\n<msg bigheadimgurl=\"\" smallheadimgurl=\"\" username=\"{user}\" nickname=\"{nick}\" \
                         fullpy=\"\" shortpy=\"\" alias=\"\" imagestatus=\"3\" scene=\"17\" province=\"\" city=\"\" sign=\"\" sex=\"1\" \
                         certflag=\"0\" certinfo=\"\" brandIconUrl=\"\" brandHomeUrl=\"\" brandSubscriptConfigUrl=\"\" brandFlags=\"0\" \
                         regionCode=\"CN\" />\n"
                    ),
                    "[名片]",
                )
            }
            48 => {
                let place = esc(&words.text_between(rng, 4, 12));
                made(
                    format!(
                        "<?xml version=\"1.0\"?>\n<msg>\n\t<location x=\"{:.6}\" y=\"{:.6}\" scale=\"15\" label=\"{place}\" \
                         maptype=\"roadmap\" poiname=\"{place}\" poiid=\"qqmap_{}\" fromusername=\"{from}\" />\n</msg>\n",
                        22.0 + rng.unit() * 18.0,
                        100.0 + rng.unit() * 20.0,
                        rng.between(1, 1 << 40)
                    ),
                    "[位置]",
                )
            }
            t if t & 0xFFFF_FFFF == 49 => {
                self.app_message(rng, conv, t >> 32, sender, time, target, recent)?
            }
            other => made(String::new(), &other.to_string()),
        })
    }

    /// An app message (`local_type & 0xFFFFFFFF == 49`) of type `sub`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one message's identity: conversation, sender, time and ids"
    )]
    fn app_message(
        &mut self,
        rng: &mut Rng,
        conv: usize,
        sub: i64,
        sender: usize,
        time: i64,
        target: f64,
        recent: &VecDeque<Recent>,
    ) -> Result<Made> {
        let world: &'a World = self.world;
        let c = &world.conversations[conv];
        let words = &world.words;
        let from = world.user(sender);
        let mut title = words.message(rng);
        let mut indexed = None;
        let mut summary = String::new();
        let inner = match sub {
            57 => {
                indexed = Some(title.clone());
                summary.clone_from(&title);
                let quoted = recent
                    .get(rng.below(recent.len()))
                    .map(|r| {
                        (
                            r.local_type,
                            r.server_id,
                            world.user(r.sender).to_string(),
                            world.nick(r.sender).to_string(),
                            r.content.clone(),
                            r.time,
                        )
                    })
                    .unwrap_or_else(|| {
                        (1, 1, from.to_string(), String::new(), String::new(), time)
                    });
                format!(
                    "\t\t<refermsg>\n\t\t\t<type>{}</type>\n\t\t\t<svrid>{}</svrid>\n\t\t\t<fromusr>{}</fromusr>\n\t\t\t<chatusr>{}</chatusr>\n\
                     \t\t\t<displayname>{}</displayname>\n\t\t\t<content>{}</content>\n\t\t\t<createtime>{}</createtime>\n\t\t</refermsg>\n",
                    quoted.0 & 0xFFFF_FFFF,
                    quoted.1,
                    c.username,
                    quoted.2,
                    esc(&quoted.3),
                    esc(&quoted.4),
                    quoted.5
                )
            }
            5 | 4 => {
                summary = format!("[链接] {title}");
                format!(
                    "\t\t<url>https://example.com/a/{}</url>\n\t\t<sourceusername>gh_{}</sourceusername>\n\t\t<sourcedisplayname>{}</sourcedisplayname>\n",
                    rng.hex(16),
                    rng.hex(12),
                    esc(rng.pick(TOPICS))
                )
            }
            6 => {
                let ext = *rng.pick(FILE_EXTENSIONS);
                let stem: String = words.text_between(rng, 2, 14).replace('，', "_");
                title = format!("{stem}.{ext}");
                summary = format!("[文件] {title}");
                let size = rng.spread(self.profile.media.file_bytes).max(100.0) as usize;
                if self.media && rng.chance(self.profile.media.file_here) {
                    let dir = month(time);
                    let path = self.account_dir.join("msg/file").join(&dir).join(&title);
                    let data = rng.bytes(size);
                    self.written.file("file", &path, &data)?;
                    self.written.hardlinks.push(Hardlink {
                        md5: hex(&Md5::digest(&data)),
                        file_name: title.clone(),
                        kind: 1,
                        dir1: dir,
                        dir2: String::new(),
                        size: size as i64,
                        modify_time: time,
                    });
                }
                format!(
                    "\t\t<appattach>\n\t\t\t<totallen>{size}</totallen>\n\t\t\t<attachid>@cdn_{}</attachid>\n\t\t\t<fileext>{ext}</fileext>\n\t\t</appattach>\n",
                    rng.hex(40)
                )
            }
            8 => {
                summary = "[动画表情]".into();
                format!(
                    "\t\t<appattach>\n\t\t\t<emoticonmd5>{}</emoticonmd5>\n\t\t</appattach>\n",
                    rng.hex(32).to_uppercase()
                )
            }
            19 | 24 => {
                title = if sub == 19 {
                    "群聊的聊天记录".into()
                } else {
                    words.text_plain(rng, 6)
                };
                summary = format!("[聊天记录] {title}");
                let n = rng.spread((10.0, 40.0, 200.0)).max(2.0) as usize;
                let items: String = (0..n)
                    .map(|_| {
                        let who = c.members[rng.below(c.members.len())];
                        format!(
                            "<dataitem datatype=\"1\" dataid=\"{}\"><datadesc>{}</datadesc><sourcename>{}</sourcename>\
                             <sourcetime>{}</sourcetime><sourceheadurl>https://wx.qlogo.invalid/{}</sourceheadurl></dataitem>",
                            rng.hex(32),
                            esc(&words.message(rng)),
                            esc(world.nick(who)),
                            time - rng.between(60, 30 * DAY),
                            rng.hex(40)
                        )
                    })
                    .collect();
                format!(
                    "\t\t<recorditem><![CDATA[<recordinfo><title>{}</title><desc>{}</desc><datalist count=\"{n}\">{items}</datalist></recordinfo>]]></recorditem>\n",
                    esc(&title),
                    esc(&words.text_plain(rng, 20))
                )
            }
            51 => {
                summary = format!("[视频号] {title}");
                format!(
                    "\t\t<finderFeed>\n\t\t\t<objectId>{}</objectId>\n\t\t\t<nickname>{}</nickname>\n\t\t\t<desc>{}</desc>\n\t\t\t<mediaCount>1</mediaCount>\n\t\t</finderFeed>\n",
                    rng.between(1, i64::MAX),
                    esc(&words.person_name(rng)),
                    esc(&words.message(rng))
                )
            }
            62 => {
                let patted = c.members[rng.below(c.members.len())];
                title = format!(
                    "\"{}\" 拍了拍 \"{}\"",
                    world.nick(sender),
                    world.nick(patted)
                );
                summary.clone_from(&title);
                format!(
                    "\t\t<patMsg>\n\t\t\t<chatUser>{}</chatUser>\n\t\t\t<records>\n\t\t\t\t<recordNum>1</recordNum>\n\t\t\t\t<record>\n\
                     \t\t\t\t\t<fromUser>{from}</fromUser>\n\t\t\t\t\t<pattedUser>{}</pattedUser>\n\t\t\t\t\t<templete><![CDATA[\"${{{from}}}\" 拍了拍 \"${{{}}}\"]]></templete>\n\
                     \t\t\t\t\t<createTime>{time}</createTime>\n\t\t\t\t</record>\n\t\t\t</records>\n\t\t</patMsg>\n",
                    c.username,
                    world.user(patted),
                    world.user(patted)
                )
            }
            87 => {
                summary = "[群公告]".into();
                format!(
                    "\t\t<textannouncement><![CDATA[{}]]></textannouncement>\n",
                    words.text_between(rng, 50, 450)
                )
            }
            2000 => {
                title = "转账".into();
                let other = c.members[rng.below(c.members.len())];
                let amount = rng.between(1, 200_000);
                summary = format!("[转账] ￥{}.{:02}", amount / 100, amount % 100);
                format!(
                    "\t\t<wcpayinfo>\n\t\t\t<paysubtype>{}</paysubtype>\n\t\t\t<feedesc>￥{}.{:02}</feedesc>\n\t\t\t<transcationid>{}</transcationid>\n\
                     \t\t\t<transferid>{}</transferid>\n\t\t\t<invalidtime>{}</invalidtime>\n\t\t\t<pay_memo>{}</pay_memo>\n\
                     \t\t\t<payer_username>{from}</payer_username>\n\t\t\t<receiver_username>{}</receiver_username>\n\t\t</wcpayinfo>\n",
                    1 + 2 * rng.below(2),
                    amount / 100,
                    amount % 100,
                    rng.hex(28),
                    rng.hex(28),
                    time + DAY,
                    esc(rng.pick(PHRASES)),
                    world.user(other)
                )
            }
            2001 => {
                title = "微信红包".into();
                summary = "[红包] 恭喜发财，大吉大利".into();
                format!(
                    "\t\t<wcpayinfo>\n\t\t\t<templateid>{}</templateid>\n\t\t\t<receivertitle><![CDATA[恭喜发财，大吉大利]]></receivertitle>\n\
                     \t\t\t<sendertitle><![CDATA[恭喜发财，大吉大利]]></sendertitle>\n\t\t\t<scenetext>微信红包</scenetext>\n\
                     \t\t\t<nativeurl>wxpay://c2cbizmessagehandler/hongbao/receivehongbao?msgtype=1&amp;channelid=1&amp;sendid={}</nativeurl>\n\
                     \t\t\t<sceneid>1002</sceneid>\n\t\t\t<invalidtime>{}</invalidtime>\n\t\t</wcpayinfo>\n",
                    rng.hex(32),
                    rng.between(1, i64::MAX),
                    time + DAY
                )
            }
            3 | 76 => {
                summary = format!("[音乐] {title}");
                format!(
                    "\t\t<url>https://example.com/m/{}</url>\n\t\t<songname>{}</songname>\n\t\t<singername>{}</singername>\n\t\t<musicurl>https://example.com/m/{}.mp3</musicurl>\n",
                    rng.hex(12),
                    esc(&title),
                    esc(&words.person_name(rng)),
                    rng.hex(12)
                )
            }
            33 | 36 => {
                summary = format!("[小程序] {title}");
                format!(
                    "\t\t<sourcedisplayname>{}</sourcedisplayname>\n\t\t<weappinfo>\n\t\t\t<username>gh_{}@app</username>\n\t\t\t<appid>wx{}</appid>\n\
                     \t\t\t<type>2</type>\n\t\t\t<pagepath>pages/index/index.html</pagepath>\n\t\t</weappinfo>\n",
                    esc(rng.pick(TOPICS)),
                    rng.hex(12),
                    rng.hex(16)
                )
            }
            _ => {
                summary = format!("[链接] {title}");
                String::new()
            }
        };
        let des = esc(&words.text_between(rng, 0, 40));
        let title = esc(&title);
        let content = sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<appmsg appid=\"\" sdkver=\"0\">\n\t\t<title>{title}</title>\n\t\t<des>{des}</des>\n\
                 \t\t<type>{sub}</type>\n\t\t<showtype>0</showtype>\n{inner}\t\t<thumburl>{fill}</thumburl>\n\t</appmsg>\n\
                 \t<fromusername>{from}</fromusername>\n\t<scene>0</scene>\n\t<appinfo>\n\t\t<version>1</version>\n\t\t<appname></appname>\n\
                 \t</appinfo>\n\t<commenturl></commenturl>\n</msg>\n"
            )
        });
        Ok(Made {
            content,
            indexed,
            summary,
            mention: None,
        })
    }

    /// A picture message: which of its files are here is drawn like the real ones.
    fn image(&mut self, rng: &mut Rng, talker: &str, time: i64, target: f64) -> Result<String> {
        let md5 = rng.hex(32);
        let p = &self.profile.media;
        let (thumb_here, image_here, hd_here, _) =
            p.image_sets[Weighted::new(p.image_sets.iter().map(|s| s.3)).draw(rng)];
        let thumb = rng.spread(p.thumb_bytes).max(800.0) as usize;
        let image = rng.spread(p.image_bytes).max(800.0) as usize;
        let hd = rng.spread(p.hd_bytes).max(4000.0) as usize;
        let (image_formats, hd_formats) = (p.image_formats, p.hd_formats);
        let has_original = hd_here || rng.chance(0.3);
        if self.media {
            let talker_dir = hex(&Md5::digest(talker.to_lowercase().as_bytes()));
            let dir = month(time);
            let folder = self
                .account_dir
                .join("msg/attach")
                .join(&talker_dir)
                .join(&dir)
                .join("Img");
            let formats = |rng: &mut Rng, (png_share, jpeg_share, _): (f64, f64, f64), len| {
                let x = rng.unit();
                if x < png_share {
                    png(rng, len)
                } else if x < png_share + jpeg_share {
                    jpeg(rng, len)
                } else {
                    wxgf(len)
                }
            };
            for (here, suffix, plain) in [
                (thumb_here, "_t", jpeg(rng, thumb)),
                (image_here, "", formats(rng, image_formats, image)),
                (hd_here, "_h", formats(rng, hd_formats, hd)),
            ] {
                if !here {
                    continue;
                }
                let file_name = format!("{md5}{suffix}.dat");
                let data = encrypt_dat(&plain);
                self.written
                    .file("image", &folder.join(&file_name), &data)?;
                self.written.hardlinks.push(Hardlink {
                    md5: md5.clone(),
                    file_name,
                    kind: 2,
                    dir1: talker_dir.clone(),
                    dir2: dir.clone(),
                    size: data.len() as i64,
                    modify_time: time,
                });
            }
        }
        let hd_attr = if has_original {
            format!(" hdlength=\"{hd}\"")
        } else {
            String::new()
        };
        let (w, h) = (120 + rng.below(200), 120 + rng.below(200));
        let (aes, origin, mid) = (rng.hex(32), rng.hex(32), rng.hex(220));
        Ok(sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<img aeskey=\"{aes}\" encryver=\"1\" cdnthumbaeskey=\"{aes}\" cdnthumburl=\"{fill}\" \
                 cdnthumblength=\"{thumb}\" cdnthumbheight=\"{h}\" cdnthumbwidth=\"{w}\" cdnmidheight=\"0\" cdnmidwidth=\"0\" \
                 cdnhdheight=\"0\" cdnhdwidth=\"0\" cdnmidimgurl=\"{mid}\" length=\"{image}\"{hd_attr} md5=\"{md5}\" \
                 hevc_mid_size=\"{image}\" originsourcemd5=\"{origin}\" />\n\t<platform_signature></platform_signature>\n\
                 \t<imgdatahash></imgdatahash>\n</msg>\n"
            )
        }))
    }

    fn video(&mut self, rng: &mut Rng, from: &str, time: i64, target: f64) -> Result<String> {
        let md5 = rng.hex(32);
        let p = &self.profile.media;
        let (thumb_here, video_here) = (p.video_thumb_here, p.video_here);
        let len = rng.spread(p.video_bytes).max(100_000.0) as usize;
        let thumb = rng.spread(p.video_thumb_bytes).max(1000.0) as usize;
        if self.media {
            let dir = month(time);
            let folder = self.account_dir.join("msg/video").join(&dir);
            if rng.chance(thumb_here) {
                let data = jpeg(rng, thumb);
                self.written.file(
                    "video thumbnail",
                    &folder.join(format!("{md5}_thumb.jpg")),
                    &data,
                )?;
            }
            if rng.chance(video_here) {
                let file_name = format!("{md5}.mp4");
                let data = mp4(rng, len);
                self.written
                    .file("video", &folder.join(&file_name), &data)?;
                self.written.hardlinks.push(Hardlink {
                    md5: md5.clone(),
                    file_name,
                    kind: 3,
                    dir1: dir,
                    dir2: String::new(),
                    size: len as i64,
                    modify_time: time,
                });
            }
        }
        let secs = rng.between(2, 300);
        let (aes, newmd5) = (rng.hex(32), rng.hex(32));
        Ok(sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<videomsg aeskey=\"{aes}\" cdnvideourl=\"{fill}\" cdnthumbaeskey=\"{aes}\" \
                 cdnthumburl=\"{aes}\" length=\"{len}\" playlength=\"{secs}\" cdnthumblength=\"{thumb}\" cdnthumbwidth=\"288\" \
                 cdnthumbheight=\"512\" fromusername=\"{from}\" md5=\"{md5}\" newmd5=\"{newmd5}\" isplaceholder=\"0\" rawmd5=\"\" \
                 rawlength=\"0\" cdnrawvideourl=\"\" cdnrawvideoaeskey=\"\" overwritenewmsgid=\"0\" originsourcemd5=\"\" isad=\"0\" />\n</msg>\n"
            )
        }))
    }
}

pub(super) fn source(rng: &mut Rng, kind: Kind, members: usize, at: Option<&str>) -> String {
    let signature = format!("V1_{}|v1_{}", rng.hex(8), rng.hex(8));
    match kind {
        Kind::Group => format!(
            "<msgsource>{}<sec_msg_node><uuid>{}_</uuid><risk-file-flag /><risk-file-md5-list /></sec_msg_node>\
             <alnode><fr>1</fr></alnode><pua>1</pua><eggIncluded>1</eggIncluded><silence>{}</silence><membercount>{}</membercount><signature>{signature}</signature>\
             <tmp_node><publisher-id></publisher-id></tmp_node></msgsource>",
            at.map(|a| format!("<atuserlist><![CDATA[,{a}]]></atuserlist>"))
                .unwrap_or_default(),
            rng.hex(32),
            rng.below(2),
            members + 1
        ),
        _ => format!(
            "<msgsource><alnode><fr>1</fr></alnode><bizflag>0</bizflag><pua>1</pua><eggIncluded>1</eggIncluded><signature>{signature}</signature><tmp_node><publisher-id></publisher-id></tmp_node></msgsource>"
        ),
    }
}

pub(super) fn zstd_or_plain(data: String, compress: bool) -> Result<(Vec<u8>, bool)> {
    Ok(if compress {
        (zstd::encode_all(data.as_bytes(), 1)?, true)
    } else {
        (data.into_bytes(), false)
    })
}

/// One message database being written: its tables, its `Name2Id` rows and the rows left for its WAL.
pub(super) struct Shard {
    pub(super) conn: Connection,
    pub(super) ids: HashMap<String, i64>,
    pub(super) tables: HashSet<String>,
    /// Messages from this one on go to the WAL.
    pub(super) wal_from: (i64, usize),
    pub(super) late: Vec<(String, Row)>,
}

impl Shard {
    pub(super) fn id(&mut self, username: &str, session: bool) -> Result<i64> {
        if let Some(id) = self.ids.get(username) {
            return Ok(*id);
        }
        let id = self.ids.len() as i64 + 1;
        self.conn.execute(
            "insert into Name2Id(rowid, user_name, is_session) values (?1, ?2, ?3)",
            params![id, username, i64::from(session)],
        )?;
        self.ids.insert(username.to_string(), id);
        Ok(id)
    }

    pub(super) fn insert(&mut self, world: &World, table: &str, row: &Row) -> Result<()> {
        let sender = match row.sender {
            Some(p) => self.id(world.user(p), false)?,
            None => 0,
        };
        // compressed content is a blob, the rest text
        let text = std::str::from_utf8(&row.content).unwrap_or_default();
        let content: &dyn rusqlite::ToSql = if row.compressed { &row.content } else { &text };
        let source_text;
        let source: &dyn rusqlite::ToSql = match &row.source {
            Some(s) if row.source_compressed => s,
            Some(s) => {
                source_text = String::from_utf8_lossy(s).into_owned();
                &source_text
            }
            None => &rusqlite::types::Null,
        };
        self.conn
            .prepare_cached(&format!(
                "insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, status, \
                 message_content, source) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ))?
            .execute(params![
                row.local_id,
                row.server_id,
                row.local_type,
                row.sort_seq,
                sender,
                row.time,
                row.status,
                content,
                source
            ])?;
        Ok(())
    }
}
