//! `weflow export`: messages in each format, media, and the session and contact lists.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::files::{
    csv_escape, export_path, make_parent, write_csv_file, write_json_file, write_text_file,
};

use super::analytics::day_span;
use serde_json::{json, Value};
use weflow_core::contract::{ExportWritten, MessagesBundle, MessagesFolder, MessagesZip};
use weflow_core::dates::date_range_args;
use weflow_core::error::{AppError, AppResult};
use weflow_core::export_msg::{ListKind, ListPage};
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct ExportCommand {
    #[command(subcommand)]
    command: ExportSubcommand,
}

#[derive(Subcommand, Debug)]
enum ExportSubcommand {
    /// Export the session list
    Sessions {
        /// Only these conversations (repeatable; default: all)
        #[arg(long = "session")]
        sessions: Vec<String>,
        /// Export format: json (default), csv, txt, html, excel or sql
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// avatar: put the avatars on the HTML page, in `media/<output name>/avatars/` beside the output file
        #[arg(long, value_delimiter = ',')]
        media: Vec<String>,
        /// With --media avatar: only those WeChat keeps on this computer, none downloaded
        #[arg(long)]
        no_avatar_download: bool,
        /// Pack the export and its media folder into one zip (`--out` a folder: <list>.zip in it)
        #[expect(
            rustdoc::invalid_html_tags,
            reason = "the comment is the --help text, where <list> stands for the list's name"
        )]
        #[arg(long)]
        zip: bool,
    },
    /// Export the contact list
    Contacts {
        /// Export format: json (default), csv, txt, html, excel or sql
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// The contacts as `chat contacts --parsed` lists them instead of the database rows
        #[arg(long)]
        parsed: bool,
        /// Only these kinds of contact (repeat for several; with --parsed)
        #[arg(long, requires = "parsed", value_parser = ["friend", "group", "official", "former_friend"], action = clap::ArgAction::Append)]
        kind: Vec<String>,
        /// avatar: put the avatars on the HTML page, in `media/<output name>/avatars/` beside the output file
        #[arg(long, value_delimiter = ',')]
        media: Vec<String>,
        /// With --media avatar: only those WeChat keeps on this computer, none downloaded
        #[arg(long)]
        no_avatar_download: bool,
        /// Pack the export and its media folder into one zip (`--out` a folder: <list>.zip in it)
        #[expect(
            rustdoc::invalid_html_tags,
            reason = "the comment is the --help text, where <list> stands for the list's name"
        )]
        #[arg(long)]
        zip: bool,
    },
    /// Export the footprint statistics
    Footprint {
        /// Export format: json (default), or csv or excel for the timeline (a row for each private chat and each @ of you)
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the images, voice (WAV), videos, stickers and files of messages
    Media {
        /// Output directory
        #[arg(long)]
        out: PathBuf,
        /// Only media of this conversation id (default: all conversations)
        #[arg(long)]
        session: Option<String>,
        /// What to export: image, voice, video, emoji, file or all
        #[arg(long, default_value = "all")]
        r#type: String,
        /// Only messages from this date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// Only messages up to this date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the messages of one conversation
    Messages {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Output file; with several formats a folder, or with --zip the zip (a folder: <conversation>.zip in it)
        #[expect(
            rustdoc::invalid_html_tags,
            reason = "the comment is the --help text, where <conversation> stands for the conversation's name"
        )]
        #[arg(long)]
        out: PathBuf,
        /// txt (default), json, arkme-json, chatlab, chatlab-jsonl, excel, weclone, html, sql; several comma separated
        #[arg(long, value_delimiter = ',', default_value = "txt")]
        format: Vec<String>,
        /// Pack the export (every format, and the media folder) into one zip
        #[arg(long)]
        zip: bool,
        /// Only export messages sent by this person: their bare wxid, as `chat contacts` lists it (not an account folder name with a `_xxxx` suffix)
        #[arg(long)]
        sender: Option<String>,
        /// How senders are named: group-nickname, remark or nickname (default: group-nickname, i.e. group nickname, then remark, nickname, wxid)
        #[arg(long)]
        display_name: Option<String>,
        /// Excel: compact columns (time, sender, type, content)
        #[arg(long)]
        excel_compact: bool,
        /// Copy media next to the export and point the messages at the copies: any of image, voice, video, emoji,
        /// file, avatar (comma separated) or all. Files go to `media/<output name>/` beside the output file.
        #[arg(long, value_delimiter = ',')]
        media: Vec<String>,
        /// With avatars in --media: only those WeChat keeps on this computer, none downloaded
        #[arg(long)]
        no_avatar_download: bool,
    },
}

pub(crate) async fn handle_export(command: ExportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match command.command {
        ExportSubcommand::Sessions {
            sessions,
            format,
            out,
            media,
            no_avatar_download,
            zip,
        } => {
            // an unknown format or media kind is refused before anything is read
            let format = ListFormat::named(format.as_deref())?;
            let avatars = list_avatars(&media, no_avatar_download, format.0)?;
            let mut data = hub.sessions_with_names()?;
            if !sessions.is_empty() {
                data = sessions_named(data, &sessions)?;
            }
            let written =
                export_list(hub, ListKind::Sessions, format, &out, &data, avatars, zip).await?;
            Ok(json!(written))
        }
        ExportSubcommand::Contacts {
            format,
            out,
            parsed,
            kind,
            media,
            no_avatar_download,
            zip,
        } => {
            let format = ListFormat::named(format.as_deref())?;
            let avatars = list_avatars(&media, no_avatar_download, format.0)?;
            let data = if parsed {
                let mut contacts = hub.chat_contacts_parsed()?;
                if !kind.is_empty() {
                    contacts.retain(|c| kind.iter().any(|k| k == c.kind.name()));
                }
                if contacts.is_empty() {
                    return Err(AppError::runtime("no contacts of the kinds asked for"));
                }
                json!(contacts)
            } else {
                hub.contacts()?
            };
            let written =
                export_list(hub, ListKind::Contacts, format, &out, &data, avatars, zip).await?;
            Ok(json!(written))
        }
        ExportSubcommand::Footprint {
            format,
            out,
            start,
            end,
        } => {
            // the footprint is a summary with lists in it: JSON as it is, CSV and Excel as its timeline; the other
            // formats are for messages, and are refused before anything is read (#315)
            let format = format.as_deref().unwrap_or("json").to_ascii_lowercase();
            let extension = match format.as_str() {
                "json" => "json",
                "csv" => "csv",
                "excel" | "xlsx" => "xlsx",
                other => {
                    return Err(AppError::usage(format!(
                        "export footprint writes json, csv or excel, not {other}"
                    )))
                }
            };
            let (b, e) = day_span(&start, &end)?;
            let data = hub.footprint(b, e)?;
            if extension == "json" {
                let path = export_path(&out, "footprint.json");
                write_json_file(&path, &data)?;
                return Ok(json!(ExportWritten { out: path, format }));
            }
            let (headers, rows) = hub.footprint_timeline(&data)?;
            let path = export_path(&out, &format!("footprint.{extension}"));
            make_parent(&path)?;
            if extension == "csv" {
                let cell = |v: &Value| {
                    csv_escape(&v.as_str().map_or_else(|| v.to_string(), str::to_string))
                };
                let mut csv = headers
                    .iter()
                    .map(|h| csv_escape(h))
                    .collect::<Vec<_>>()
                    .join(",")
                    + "\n";
                for row in &rows {
                    csv += &(row.iter().map(cell).collect::<Vec<_>>().join(",") + "\n");
                }
                write_text_file(&path, &csv)?;
            } else {
                weflow_core::export::export_table_excel(&headers, &rows, &path)
                    .map_err(|err| AppError::runtime(err.to_string()))?;
            }
            Ok(json!(ExportWritten { out: path, format }))
        }
        ExportSubcommand::Media {
            out,
            session,
            r#type,
            start,
            end,
        } => {
            let (start_ts, end_ts) = date_range_args(start.as_deref(), end.as_deref())?;
            Ok(json!(
                hub.export_media(session.as_deref(), &out, &r#type, start_ts, end_ts)
                    .await?
            ))
        }
        ExportSubcommand::Messages {
            session_id,
            start,
            end,
            out,
            format,
            sender,
            display_name,
            excel_compact,
            media,
            no_avatar_download,
            zip,
        } => {
            let (start_ts, end_ts) = date_range_args(start.as_deref(), end.as_deref())?;
            let mut formats: Vec<String> = Vec::new();
            for f in format
                .iter()
                .map(|f| f.trim().to_ascii_lowercase())
                .filter(|f| !f.is_empty())
            {
                if !formats.contains(&f) {
                    formats.push(f);
                }
            }
            let fmt = formats.first().cloned().unwrap_or_else(|| "txt".into());
            let avatars = if no_avatar_download {
                weflow_core::api::AvatarSource::Local
            } else {
                weflow_core::api::AvatarSource::Download
            };
            let media_opts = parse_media_selection(&media, avatars)?;
            let parse_display = |s: &str| {
                weflow_core::export_msg::names::DisplayPref::parse(s).ok_or_else(|| {
                    AppError::usage("--display-name must be group-nickname, remark or nickname")
                })
            };
            let ext_of = |fmt: &str| -> Result<&'static str, AppError> {
                Ok(match fmt {
                    "txt" => "txt",
                    "json" | "arkme-json" => "json",
                    "chatlab" => "chatlab.json",
                    "chatlab-jsonl" => "jsonl",
                    "excel" | "xlsx" => "xlsx",
                    "weclone" => "csv",
                    "html" => "html",
                    "sql" => "sql",
                    other => {
                        return Err(AppError::usage(format!(
                            "unsupported message export format: {other}; supported: {}",
                            weflow_core::services::MESSAGE_EXPORT_FORMATS
                        )))
                    }
                })
            };
            // json and arkme-json write the same extension: two of them would take one file
            let mut exts: Vec<&str> = Vec::new();
            for f in &formats {
                let e = ext_of(f)?;
                if exts.contains(&e) {
                    return Err(AppError::usage(format!(
                        "--format {f} writes .{e} as another format given does; give one of them"
                    )));
                }
                exts.push(e);
            }
            let ext = exts[0];
            let display_pref = parse_display(display_name.as_deref().unwrap_or("group-nickname"))?;
            let safe_name: String = session_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '@' || c == '.' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let target = export_path(&out, &format!("{safe_name}.{ext}"));
            let request = weflow_core::services::MessageExportRequest {
                session_id,
                format: fmt,
                // the end is inclusive seconds
                start: start_ts,
                end: end_ts.map(|e| e - 1),
                sender,
                display_pref,
                excel_compact,
            };
            if formats.len() == 1 && !zip {
                let exported = if media_opts.enabled {
                    hub.export_messages_with_media(&request, &target, &media_opts)
                        .await?
                } else {
                    hub.export_messages(&request, &target)?
                };
                return Ok(json!(exported));
            }
            let bundle = export_messages_bundle(
                hub,
                &request,
                &formats,
                &exts,
                &safe_name,
                &out,
                zip,
                &media_opts,
            )
            .await?;
            Ok(json!(bundle))
        }
    }
}

/// `export messages` in several formats or packed (#342): every format into one folder, the media in its `media/`
/// folder; with `zip`, the folder (beside the zip, taken away after) packed into the zip, whose path is the result's
/// `out`, the formats' files named inside it.
#[allow(clippy::too_many_arguments)]
async fn export_messages_bundle(
    hub: &ServiceHub,
    request: &weflow_core::services::MessageExportRequest,
    formats: &[String],
    exts: &[&str],
    safe_name: &str,
    out: &Path,
    zip: bool,
    media: &weflow_core::api::ApiMediaOptions,
) -> AppResult<MessagesBundle> {
    let zip_path = zip.then(|| export_path(out, &format!("{safe_name}.zip")));
    let folder = match &zip_path {
        Some(z) => z.with_file_name(format!(
            ".{}.parts",
            z.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        )),
        None if out.extension().is_some() => return Err(AppError::usage(
            "several formats are written to a folder: give --out a folder, or pack them with --zip",
        )),
        None => out.to_path_buf(),
    };
    if zip_path.is_some() {
        let _ = std::fs::remove_dir_all(&folder);
    }
    std::fs::create_dir_all(&folder).map_err(|err| {
        AppError::runtime(format!("failed to create {}: {err}", folder.display()))
    })?;
    let outs: Vec<(String, PathBuf)> = formats
        .iter()
        .zip(exts)
        .map(|(f, e)| (f.clone(), folder.join(format!("{safe_name}.{e}"))))
        .collect();
    let written = hub.export_messages_formats(request, &outs, media).await;
    let Some(zip_path) = zip_path else {
        return Ok(MessagesBundle::Folder(MessagesFolder {
            out: folder,
            formats: formats.to_vec(),
            files: written?,
        }));
    };
    let packed = written.and_then(|mut files| {
        weflow_core::export::zip_folder(&folder, &zip_path).map_err(|err| {
            AppError::runtime(format!("failed to write {}: {err:#}", zip_path.display()))
        })?;
        // the files are inside the zip now: their names there
        for (file, (_, path)) in files.iter_mut().zip(&outs) {
            file.out = path.file_name().map(PathBuf::from).unwrap_or_default();
        }
        Ok(files)
    });
    let _ = std::fs::remove_dir_all(&folder);
    let files = packed?;
    let media = files.first().and_then(|f| f.media.clone());
    Ok(MessagesBundle::Zip(MessagesZip {
        out: zip_path,
        formats: formats.to_vec(),
        files,
        media,
    }))
}

/// `--media image,voice` / `--media all` → the media kinds to copy next to a message export.
/// The media `--media` names; the avatars, when named, come from `avatars`.
fn parse_media_selection(
    values: &[String],
    avatars: weflow_core::api::AvatarSource,
) -> Result<weflow_core::api::ApiMediaOptions, AppError> {
    let mut opts = weflow_core::api::ApiMediaOptions::default();
    for v in values
        .iter()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
    {
        match v.as_str() {
            "all" => {
                (
                    opts.images,
                    opts.voices,
                    opts.videos,
                    opts.emojis,
                    opts.files,
                ) = (true, true, true, true, true);
                opts.avatars = Some(avatars);
            }
            "image" | "images" => opts.images = true,
            "voice" | "voices" => opts.voices = true,
            "video" | "videos" => opts.videos = true,
            "emoji" | "emojis" | "sticker" | "stickers" => opts.emojis = true,
            "file" | "files" => opts.files = true,
            "avatar" | "avatars" => opts.avatars = Some(avatars),
            other => {
                let kinds = "image, voice, video, emoji, file, avatar or all";
                return Err(AppError::usage(format!(
                    "unknown media kind: {other}; use {kinds}"
                )));
            }
        }
    }
    opts.enabled = opts.images
        || opts.voices
        || opts.videos
        || opts.emojis
        || opts.files
        || opts.avatars.is_some();
    Ok(opts)
}

/// A format `export sessions` and `export contacts` write a list in.
#[derive(Clone, Copy)]
enum ListFormat {
    Json,
    Csv,
    Txt,
    Html,
    Excel,
    Sql,
}

impl ListFormat {
    /// The format `--format` names (JSON without one), and the name as it was given, in lower case. The formats
    /// for messages (ChatLab, WeClone) are refused: a list has no senders or texts to put in them (#659).
    fn named(format: Option<&str>) -> AppResult<(Self, String)> {
        let name = format.unwrap_or("json").to_ascii_lowercase();
        let format = match name.as_str() {
            "json" => Self::Json,
            "csv" => Self::Csv,
            "txt" => Self::Txt,
            "html" => Self::Html,
            "excel" | "xlsx" => Self::Excel,
            "sql" => Self::Sql,
            other => {
                return Err(AppError::usage(format!(
                    "unsupported export format: {other}; supported: json, csv, txt, html, excel, sql"
                )));
            }
        };
        Ok((format, name))
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Txt => "txt",
            Self::Html => "html",
            Self::Excel => "xlsx",
            Self::Sql => "sql",
        }
    }
}

/// Where the avatars of a list's HTML page come from, as `--media` and `--no-avatar-download` ask (`None`: no
/// avatars). Only avatars go with a list, and only on its HTML page.
fn list_avatars(
    media: &[String],
    no_download: bool,
    format: ListFormat,
) -> AppResult<Option<weflow_core::api::AvatarSource>> {
    let mut wanted = false;
    for kind in media.iter().map(|m| m.trim().to_ascii_lowercase()) {
        match kind.as_str() {
            "" => {}
            "avatar" | "avatars" => wanted = true,
            other => {
                return Err(AppError::usage(format!(
                    "a list takes no {other} media; use --media avatar"
                )));
            }
        }
    }
    if !wanted {
        return Ok(None);
    }
    if !matches!(format, ListFormat::Html) {
        return Err(AppError::usage(
            "--media avatar puts the avatars on the HTML page; use --format html",
        ));
    }
    Ok(Some(if no_download {
        weflow_core::api::AvatarSource::Local
    } else {
        weflow_core::api::AvatarSource::Download
    }))
}

/// Writes the list as [`write_list`] does; with `zip`, into a folder beside the zip (taken away after) that is packed
/// into the zip (`out`, or `<list>.zip` in the folder `out`): the list's file and its `media/` folder. The result's
/// `out` is then the zip.
async fn export_list(
    hub: &ServiceHub,
    kind: ListKind,
    format: (ListFormat, String),
    out: &Path,
    value: &Value,
    avatars: Option<weflow_core::api::AvatarSource>,
    zip: bool,
) -> AppResult<ExportWritten> {
    if !zip {
        return write_list(hub, kind, format, out, value, avatars).await;
    }
    if out
        .extension()
        .is_some_and(|ext| !ext.eq_ignore_ascii_case("zip"))
    {
        return Err(AppError::usage(
            "--zip writes a zip: give --out a folder or a .zip file",
        ));
    }
    let zip_path = export_path(out, &format!("{}.zip", kind.name()));
    let folder = zip_path.with_file_name(format!(
        ".{}.parts",
        zip_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    // the file itself: the folder's name has a dot, which would make it a file's
    let file = folder.join(format!("{}.{}", kind.name(), format.0.extension()));
    let packed = async {
        let written = write_list(hub, kind, format, &file, value, avatars).await?;
        weflow_core::export::zip_folder(&folder, &zip_path).map_err(|err| {
            AppError::runtime(format!("failed to write {}: {err:#}", zip_path.display()))
        })?;
        Ok::<_, AppError>(written.format)
    }
    .await;
    let _ = std::fs::remove_dir_all(&folder);
    Ok(ExportWritten {
        out: zip_path,
        format: packed?,
    })
}

/// Writes the list `value` to `out` (the file named after the list, with the format's extension, when `out` is a
/// folder): as it is in JSON and text, and as a table whose columns are the rows' keys in the other formats; on the
/// HTML page, the avatars of its rows when `avatars` says where from.
async fn write_list(
    hub: &ServiceHub,
    kind: ListKind,
    (format, format_name): (ListFormat, String),
    out: &Path,
    value: &Value,
    avatars: Option<weflow_core::api::AvatarSource>,
) -> AppResult<ExportWritten> {
    use weflow_core::export;
    let name = kind.name();
    let path = export_path(out, &format!("{name}.{}", format.extension()));
    make_parent(&path)?;
    match format {
        ListFormat::Json => write_json_file(&path, value)?,
        ListFormat::Csv => write_csv_file(&path, value)?,
        ListFormat::Txt => write_text_file(&path, &format!("{value:#}"))?,
        ListFormat::Html => {
            let (columns, rows) = export::list_table(value);
            let pictures = match avatars {
                Some(source) => {
                    hub.export_list_avatars(list_users(value), &path, source)
                        .await?
                }
                None => Default::default(),
            };
            let page = ListPage {
                kind,
                columns: &columns,
                rows: &rows,
                avatars: &pictures,
                exported_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| {
                        i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
                    }),
            };
            export::export_list_html(&page, &path)
                .map_err(|err| AppError::runtime(err.to_string()))?;
        }
        ListFormat::Excel | ListFormat::Sql => {
            let (headers, rows) = export::list_table(value);
            match format {
                ListFormat::Excel => export::export_table_excel(&headers, &rows, &path),
                _ => export::export_table_sql(name, &headers, &rows, &path),
            }
            .map_err(|err| AppError::runtime(err.to_string()))?;
        }
    }
    Ok(ExportWritten {
        out: path,
        format: format_name,
    })
}

/// The usernames of a list's rows, each once, in the list's order.
fn list_users(list: &Value) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("username")?.as_str())
        .filter(|user| !user.is_empty() && seen.insert(*user))
        .map(str::to_string)
        .collect()
}

/// The rows of the session list whose `username` is one of `ids`, compared exactly (#664).
///
/// # Errors
///
/// When no row is one of `ids`, rather than writing an empty list.
fn sessions_named(list: Value, ids: &[String]) -> AppResult<Value> {
    let Value::Array(rows) = list else {
        return Ok(list);
    };
    let rows: Vec<Value> = rows
        .into_iter()
        .filter(|row| {
            row.get("username")
                .and_then(Value::as_str)
                .is_some_and(|username| ids.iter().any(|id| id == username))
        })
        .collect();
    if rows.is_empty() {
        return Err(AppError::usage(
            "none of the sessions asked for is in the session list",
        ));
    }
    Ok(Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_takes_only_avatars_and_only_on_its_html_page() {
        use weflow_core::api::AvatarSource;
        let media = |kinds: &[&str]| kinds.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        let html = ListFormat::Html;
        assert_eq!(list_avatars(&[], false, html).unwrap(), None);
        assert_eq!(
            list_avatars(&media(&["avatar"]), false, html).unwrap(),
            Some(AvatarSource::Download)
        );
        assert_eq!(
            list_avatars(&media(&["Avatars"]), true, html).unwrap(),
            Some(AvatarSource::Local)
        );
        assert!(list_avatars(&media(&["image"]), false, html).is_err());
        assert!(list_avatars(&media(&["avatar"]), false, ListFormat::Json).is_err());
        // nothing asked for: any format
        assert_eq!(list_avatars(&[], true, ListFormat::Csv).unwrap(), None);
    }

    #[test]
    fn a_list_names_each_user_once() {
        let list = json!([
            { "username": "wxid_a" },
            { "username": "room@chatroom" },
            { "username": "wxid_a" },
            { "username": "" },
            { "name": "no username" },
        ]);
        assert_eq!(list_users(&list), ["wxid_a", "room@chatroom"]);
    }
}
