//! Background worker: owns the tokio runtime and the API client.
//! The UI sends [`Cmd`]s and receives [`Event`]s; it never blocks on the network.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use tokio::sync::{Mutex, RwLock};

use crate::api::auth::{self, AuthConfig};
use crate::api::{
    AccessType, Album, ApiError, Artist, Client, ItemRef, Playlist, PlaylistItem, SearchResults, Track, User,
};
use crate::ops::analysis;
use crate::ops::files::{ExportFormat, ImportPlaylist};
use crate::ops::matcher::{self, MatchRow};
use crate::player::{PlayerCmd, PlayerHandle, dash};

pub type JobId = u64;

#[derive(Clone)]
pub struct Job {
    pub id: JobId,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self { id: NEXT.fetch_add(1, Ordering::Relaxed), cancel: Arc::new(AtomicBool::new(false)) }
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
pub enum Reason {
    /// TIDAL did not return the item at all.
    Missing,
    /// Returned, but not streamable (availability lacks STREAM).
    NotStreamable,
    /// Strict check: no usage rule allows streaming in the user's country.
    NoUsageRule,
}

#[derive(Clone, Debug)]
pub struct UnavailableHit {
    pub pid: String,
    pub playlist: String,
    pub item: PlaylistItem,
    pub reason: Reason,
    /// Best known metadata (from the response or the local cache).
    pub known: Option<Track>,
}

pub enum Cmd {
    RestoreSession(AuthConfig),
    Login(AuthConfig, Job),
    Logout,
    LoadPlaylists,
    LoadItems(String),
    CreatePlaylist { name: String, description: String, access: AccessType, add: Vec<ItemRef>, job: Job },
    UpdatePlaylist { pid: String, name: String, description: String, access: AccessType },
    DeletePlaylist(String),
    AddItems { pid: String, refs: Vec<ItemRef>, before: Option<String>, skip_dups: bool, job: Job },
    RemoveItems { pid: String, items: Vec<(ItemRef, String)>, job: Job },
    MoveItems { pid: String, items: Vec<(ItemRef, String)>, before: Option<String> },
    ApplyOrder { pid: String, current: Vec<PlaylistItem>, order: Vec<PlaylistItem>, job: Job },
    Restore { pid: String, target: Vec<ItemRef>, job: Job },
    Duplicate { source: Playlist, name: String, job: Job },
    Merge { sources: Vec<Playlist>, name: String, dedupe: bool, job: Job },
    Search(String),
    SearchMore(String),
    AlbumTracks(Album),
    ArtistDetails(Artist),
    ScanUnavailable { playlists: Vec<Playlist>, strict: bool, job: Job },
    FindReplacements { hits: Vec<UnavailableHit>, job: Job },
    Replace { pid: String, swaps: Vec<(PlaylistItem, Track)>, job: Job },
    Match { playlists: Vec<ImportPlaylist>, job: Job },
    FetchForExport { playlists: Vec<Playlist>, format: ExportFormat, dir: PathBuf, backup: bool, job: Job },
    Favourite { ids: Vec<String>, add: bool },
    Preview(Track),
}

#[derive(Clone, Debug)]
pub enum Note {
    Added { n: usize, skipped: usize },
    Removed(usize),
    Reordered,
    Created(String),
    Duplicated(String),
    Merged { name: String, n: usize },
    Updated,
    Deleted,
    Restored,
    Replaced(usize),
    Favourited { n: usize, add: bool },
}

#[derive(Clone, Debug)]
pub enum PreviewFail {
    Drm,
    Video,
    Other(String),
}

pub enum Event {
    LoginStarted(String),
    LoggedIn(User),
    LoginFailed(String),
    LoggedOut,
    Playlists(Vec<Playlist>),
    Items { pid: String, items: Vec<PlaylistItem> },
    ItemsFailed { pid: String, error: String },
    PlaylistCreated(Playlist),
    PlaylistUpdated(Playlist),
    PlaylistDeleted(String),
    Search(SearchResults),
    SearchMore { tracks: Vec<Track>, next: Option<String> },
    Album { album: Album, tracks: Vec<Track> },
    Artist { artist: Artist, albums: Vec<Album>, tracks: Vec<Track> },
    Progress { job: JobId, done: usize, total: usize },
    JobDone { job: JobId, note: Option<Note> },
    JobFailed { job: JobId, error: String },
    Unavailable(Vec<UnavailableHit>),
    Replacements(Vec<(UnavailableHit, Option<Track>)>),
    Matches(Vec<(ImportPlaylist, Vec<MatchRow>)>),
    ExportData { data: Vec<(Playlist, Vec<PlaylistItem>)>, format: ExportFormat, dir: PathBuf, backup: bool },
    Note(Note),
    Error(String),
    PreviewLoading(String),
    PreviewFailed(PreviewFail),
}

#[derive(Clone)]
struct Ctx {
    tx: mpsc::Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    client: Arc<RwLock<Option<Arc<Client>>>>,
    /// Serialises playlist mutations so batches never interleave.
    write_lock: Arc<Mutex<()>>,
    token_path: PathBuf,
    player: PlayerHandle,
}

impl Ctx {
    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
        (self.repaint)();
    }

    async fn client(&self) -> Result<Arc<Client>, ApiError> {
        self.client.read().await.clone().ok_or(ApiError::NotLoggedIn)
    }

    fn progress(&self, job: &Job, total: usize) -> impl Fn(usize) + Sync + '_ {
        let id = job.id;
        move |done| self.emit(Event::Progress { job: id, done, total })
    }
}

pub struct WorkerHandle {
    tx: tokio::sync::mpsc::UnboundedSender<Cmd>,
    pub events: mpsc::Receiver<Event>,
}

impl WorkerHandle {
    pub fn spawn(token_path: PathBuf, player: PlayerHandle, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
        let (etx, events) = mpsc::channel();
        let ctx = Ctx {
            tx: etx,
            repaint,
            client: Arc::new(RwLock::new(None)),
            write_lock: Arc::new(Mutex::new(())),
            token_path,
            player,
        };
        std::thread::Builder::new()
            .name("worker".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(async move {
                    while let Some(cmd) = rx.recv().await {
                        let ctx = ctx.clone();
                        tokio::spawn(async move { handle(ctx, cmd).await });
                    }
                });
            })
            .expect("worker thread");
        Self { tx, events }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

fn err_text(e: &ApiError) -> String {
    e.to_string()
}

async fn handle(ctx: Ctx, cmd: Cmd) {
    match cmd {
        Cmd::RestoreSession(cfg) => {
            let Some(tokens) = auth::load_tokens(&ctx.token_path) else {
                return ctx.emit(Event::LoggedOut);
            };
            let client = Arc::new(Client::new(cfg, tokens, Some(ctx.token_path.clone())));
            match client.me().await {
                Ok(user) => {
                    *ctx.client.write().await = Some(client);
                    ctx.emit(Event::LoggedIn(user));
                    load_playlists(&ctx).await;
                }
                Err(e) => {
                    tracing::info!("stored session not usable: {e}");
                    if matches!(e, ApiError::Network(_)) {
                        ctx.emit(Event::Error(err_text(&e)));
                    }
                    ctx.emit(Event::LoggedOut);
                }
            }
        }
        Cmd::Login(cfg, job) => {
            if let Err(e) = login(&ctx, cfg, &job).await {
                if !matches!(e, ApiError::Cancelled) {
                    ctx.emit(Event::LoginFailed(err_text(&e)));
                } else {
                    ctx.emit(Event::LoggedOut);
                }
            }
        }
        Cmd::Logout => {
            *ctx.client.write().await = None;
            let _ = std::fs::remove_file(&ctx.token_path);
            ctx.emit(Event::LoggedOut);
        }
        Cmd::LoadPlaylists => load_playlists(&ctx).await,
        Cmd::LoadItems(pid) => {
            reload(&ctx, &pid).await;
        }
        Cmd::Search(query) => match ctx.client().await {
            Ok(c) => match c.search(&query).await {
                Ok(r) => ctx.emit(Event::Search(r)),
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            },
            Err(e) => ctx.emit(Event::Error(err_text(&e))),
        },
        Cmd::SearchMore(next) => match ctx.client().await {
            Ok(c) => match c.search_more_tracks(&next).await {
                Ok((tracks, next)) => ctx.emit(Event::SearchMore { tracks, next }),
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            },
            Err(e) => ctx.emit(Event::Error(err_text(&e))),
        },
        Cmd::AlbumTracks(album) => {
            let r = async { ctx.client().await?.album_tracks(&album.id).await }.await;
            match r {
                Ok(tracks) => ctx.emit(Event::Album { album, tracks }),
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            }
        }
        Cmd::ArtistDetails(artist) => {
            let r = async {
                let c = ctx.client().await?;
                let tracks = c.artist_tracks(&artist.id).await.unwrap_or_default();
                let albums = c.artist_albums(&artist.id).await?;
                Ok::<_, ApiError>((albums, tracks))
            }
            .await;
            match r {
                Ok((albums, tracks)) => ctx.emit(Event::Artist { artist, albums, tracks }),
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            }
        }
        Cmd::UpdatePlaylist { pid, name, description, access } => {
            let r = async {
                let c = ctx.client().await?;
                c.update_playlist(&pid, Some(&name), Some(&description), Some(access)).await?;
                c.playlist(&pid).await
            }
            .await;
            match r {
                Ok(p) => {
                    ctx.emit(Event::PlaylistUpdated(p));
                    ctx.emit(Event::Note(Note::Updated));
                }
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            }
        }
        Cmd::DeletePlaylist(pid) => {
            let r = async { ctx.client().await?.delete_playlist(&pid).await }.await;
            match r {
                Ok(()) => {
                    ctx.emit(Event::PlaylistDeleted(pid));
                    ctx.emit(Event::Note(Note::Deleted));
                }
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            }
        }
        Cmd::MoveItems { pid, items, before } => {
            let _g = ctx.write_lock.lock().await;
            let r = async { ctx.client().await?.move_items(&pid, &items, before.as_deref(), &|_| {}, &|| false).await }
                .await;
            if let Err(e) = r {
                ctx.emit(Event::Error(err_text(&e)));
            }
            reload(&ctx, &pid).await;
        }
        Cmd::Favourite { ids, add } => {
            let r = async { ctx.client().await?.set_favourite_tracks(&ids, add).await }.await;
            match r {
                Ok(()) => ctx.emit(Event::Note(Note::Favourited { n: ids.len(), add })),
                Err(e) => ctx.emit(Event::Error(err_text(&e))),
            }
        }
        Cmd::Preview(track) => preview(&ctx, track).await,
        job_cmd => run_job(&ctx, job_cmd).await,
    }
}

/// Commands that report through `JobDone` / `JobFailed`.
async fn run_job(ctx: &Ctx, cmd: Cmd) {
    let job = match &cmd {
        Cmd::CreatePlaylist { job, .. }
        | Cmd::AddItems { job, .. }
        | Cmd::RemoveItems { job, .. }
        | Cmd::ApplyOrder { job, .. }
        | Cmd::Restore { job, .. }
        | Cmd::Duplicate { job, .. }
        | Cmd::Merge { job, .. }
        | Cmd::ScanUnavailable { job, .. }
        | Cmd::FindReplacements { job, .. }
        | Cmd::Replace { job, .. }
        | Cmd::Match { job, .. }
        | Cmd::FetchForExport { job, .. } => job.clone(),
        _ => return,
    };
    let result = job_body(ctx, cmd, &job).await;
    match result {
        Ok(note) => ctx.emit(Event::JobDone { job: job.id, note }),
        Err(e) => ctx.emit(Event::JobFailed { job: job.id, error: err_text(&e) }),
    }
}

async fn job_body(ctx: &Ctx, cmd: Cmd, job: &Job) -> Result<Option<Note>, ApiError> {
    let c = ctx.client().await?;
    let cancel = || job.cancelled();
    match cmd {
        Cmd::CreatePlaylist { name, description, access, add, .. } => {
            let _g = ctx.write_lock.lock().await;
            let p = c.create_playlist(&name, &description, access).await?;
            ctx.emit(Event::PlaylistCreated(p.clone()));
            if !add.is_empty() {
                c.add_items(&p.id, &add, None, false, &ctx.progress(job, add.len()), &cancel).await?;
            }
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            Ok(Some(Note::Created(name)))
        }
        Cmd::AddItems { pid, refs, before, skip_dups, .. } => {
            let _g = ctx.write_lock.lock().await;
            let r =
                c.add_items(&pid, &refs, before.as_deref(), skip_dups, &ctx.progress(job, refs.len()), &cancel).await;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            let skipped = r?;
            Ok(Some(Note::Added { n: refs.len() - skipped, skipped }))
        }
        Cmd::RemoveItems { pid, items, .. } => {
            let _g = ctx.write_lock.lock().await;
            let r = c.remove_items(&pid, &items, &ctx.progress(job, items.len()), &cancel).await;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            r?;
            Ok(Some(Note::Removed(items.len())))
        }
        Cmd::ApplyOrder { pid, current, order, .. } => {
            let _g = ctx.write_lock.lock().await;
            let r = c.apply_order(&pid, &current, &order, &ctx.progress(job, order.len()), &cancel).await;
            reload(ctx, &pid).await;
            r?;
            Ok(Some(Note::Reordered))
        }
        Cmd::Restore { pid, target, .. } => {
            let _g = ctx.write_lock.lock().await;
            restore(ctx, &c, &pid, &target, job).await?;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            Ok(Some(Note::Restored))
        }
        Cmd::Duplicate { source, name, .. } => {
            let items = c.playlist_items(&source.id, &cancel).await?;
            ctx.emit(Event::Items { pid: source.id.clone(), items: items.clone() });
            let _g = ctx.write_lock.lock().await;
            let p = c.create_playlist(&name, &source.description, source.access_type).await?;
            ctx.emit(Event::PlaylistCreated(p.clone()));
            let refs: Vec<ItemRef> = items.iter().map(|i| i.r.clone()).collect();
            c.add_items(&p.id, &refs, None, false, &ctx.progress(job, refs.len()), &cancel).await?;
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            Ok(Some(Note::Duplicated(name)))
        }
        Cmd::Merge { sources, name, dedupe, .. } => {
            let mut refs = Vec::new();
            for s in &sources {
                let items = c.playlist_items(&s.id, &cancel).await?;
                refs.extend(items.iter().map(|i| i.r.clone()));
                ctx.emit(Event::Items { pid: s.id.clone(), items });
            }
            if dedupe {
                let mut seen = std::collections::HashSet::new();
                refs.retain(|r| seen.insert(r.clone()));
            }
            let _g = ctx.write_lock.lock().await;
            let desc = sources.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" + ");
            let p = c.create_playlist(&name, &desc, AccessType::Unlisted).await?;
            ctx.emit(Event::PlaylistCreated(p.clone()));
            c.add_items(&p.id, &refs, None, false, &ctx.progress(job, refs.len()), &cancel).await?;
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            Ok(Some(Note::Merged { name, n: refs.len() }))
        }
        Cmd::ScanUnavailable { playlists, strict, .. } => {
            let mut hits = Vec::new();
            let total = playlists.len();
            for (n, p) in playlists.iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let items = c.playlist_items(&p.id, &cancel).await?;
                ctx.emit(Event::Items { pid: p.id.clone(), items: items.clone() });
                let mut bad_rules = std::collections::HashSet::new();
                if strict {
                    let ids: Vec<String> = items
                        .iter()
                        .filter(|i| i.r.kind == crate::api::ItemKind::Track && i.available())
                        .map(|i| i.r.id.clone())
                        .collect();
                    bad_rules = c.not_streamable(&ids, &cancel).await?;
                }
                for it in items {
                    let reason = if it.track.is_none() {
                        Some(Reason::Missing)
                    } else if !it.available() {
                        Some(Reason::NotStreamable)
                    } else if bad_rules.contains(&it.r.id) {
                        Some(Reason::NoUsageRule)
                    } else {
                        None
                    };
                    if let Some(reason) = reason {
                        hits.push(UnavailableHit {
                            pid: p.id.clone(),
                            playlist: p.name.clone(),
                            known: it.track.clone(),
                            item: it,
                            reason,
                        });
                    }
                }
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::Unavailable(hits));
            Ok(None)
        }
        Cmd::FindReplacements { hits, .. } => {
            let mut list = Vec::new();
            let total = hits.len();
            for (n, h) in hits.into_iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let found = match &h.known {
                    Some(k) => find_replacement(&c, k).await,
                    None => None,
                };
                list.push((h, found));
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::Replacements(list));
            Ok(None)
        }
        Cmd::Replace { pid, swaps, .. } => {
            let _g = ctx.write_lock.lock().await;
            let total = swaps.len();
            for (n, (old, new)) in swaps.iter().enumerate() {
                if cancel() {
                    break;
                }
                c.add_items(&pid, &[new.item_ref()], Some(&old.item_id), false, &|_| {}, &cancel).await?;
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total: total * 2 });
            }
            let old: Vec<(ItemRef, String)> = swaps.iter().map(|(o, _)| (o.r.clone(), o.item_id.clone())).collect();
            c.remove_items(
                &pid,
                &old,
                &|d| ctx.emit(Event::Progress { job: job.id, done: total + d, total: total * 2 }),
                &|| false,
            )
            .await?;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            Ok(Some(Note::Replaced(total)))
        }
        Cmd::Match { playlists, .. } => {
            let total: usize = playlists.iter().map(|p| p.rows.len()).sum();
            let mut out = Vec::new();
            let mut base = 0;
            for p in playlists {
                let n = p.rows.len();
                let b = base;
                let rows = matcher::match_rows(
                    &c,
                    p.rows.clone(),
                    &|d| ctx.emit(Event::Progress { job: job.id, done: b + d, total }),
                    &cancel,
                )
                .await?;
                base += n;
                out.push((p, rows));
            }
            ctx.emit(Event::Matches(out));
            Ok(None)
        }
        Cmd::FetchForExport { playlists, format, dir, backup, .. } => {
            let mut data = Vec::new();
            let total = playlists.len();
            for (n, p) in playlists.into_iter().enumerate() {
                let items = c.playlist_items(&p.id, &cancel).await?;
                ctx.emit(Event::Items { pid: p.id.clone(), items: items.clone() });
                data.push((p, items));
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::ExportData { data, format, dir, backup });
            Ok(None)
        }
        _ => Ok(None),
    }
}

async fn find_replacement(c: &Client, known: &Track) -> Option<Track> {
    let row = crate::ops::files::ImportRow {
        kind: known.kind,
        id: None,
        isrc: known.isrc.clone(),
        title: known.title.clone(),
        artists: known.artists.clone(),
        album: known.album.clone(),
        duration_secs: (known.duration_secs > 0).then_some(known.duration_secs),
    };
    let usable = |t: &Track| t.id != known.id && t.streamable();
    if let Some(isrc) = &known.isrc
        && let Ok(cands) = c.tracks_by_isrc(isrc).await
    {
        let mut cands: Vec<Track> = cands.into_iter().filter(usable).collect();
        cands.sort_by(|a, b| matcher::score(&row, b).total_cmp(&matcher::score(&row, a)));
        if let Some(t) = cands.into_iter().next() {
            return Some(t);
        }
    }
    let res = c.search(&matcher::search_text(&row)).await.ok()?;
    res.tracks
        .into_iter()
        .filter(usable)
        .map(|t| (matcher::score(&row, &t), t))
        .filter(|(s, _)| *s >= 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, t)| t)
}

async fn restore(ctx: &Ctx, c: &Client, pid: &str, target: &[ItemRef], job: &Job) -> Result<(), ApiError> {
    let cancel = || job.cancelled();
    let current = c.playlist_items(pid, &cancel).await?;
    let remove: Vec<(ItemRef, String)> =
        analysis::removals(&current, target).into_iter().map(|i| (i.r, i.item_id)).collect();
    let refs: Vec<ItemRef> = current.iter().map(|i| i.r.clone()).collect();
    let d = analysis::diff(&refs, target);
    let steps = remove.len() + d.missing.len() + target.len();
    let p = ctx.progress(job, steps.max(1));
    if !remove.is_empty() {
        c.remove_items(pid, &remove, &|n| p(n), &cancel).await?;
    }
    if !d.missing.is_empty() {
        c.add_items(pid, &d.missing, None, false, &|n| p(remove.len() + n), &cancel).await?;
    }
    let now = c.playlist_items(pid, &cancel).await?;
    let order = analysis::order_for(&now, target);
    let base = remove.len() + d.missing.len();
    c.apply_order(pid, &now, &order, &|n| p(base + n), &cancel).await?;
    Ok(())
}

async fn login(ctx: &Ctx, cfg: AuthConfig, job: &Job) -> Result<(), ApiError> {
    let pkce = auth::Pkce::new();
    let url = auth::authorize_url(&cfg, &pkce);
    ctx.emit(Event::LoginStarted(url.clone()));
    if let Err(e) = open::that_detached(&url) {
        tracing::warn!("could not open browser: {e}");
    }
    let (cfg2, state, cancel) = (cfg.clone(), pkce.state.clone(), job.cancel.clone());
    let code = tokio::task::spawn_blocking(move || auth::wait_for_code(&cfg2, &state, cancel))
        .await
        .map_err(|e| ApiError::Other(e.to_string()))??;
    let http = Client::http();
    let tokens = auth::exchange_code(&http, &cfg, &code, &pkce.verifier).await?;
    if let Err(e) = auth::save_tokens(&ctx.token_path, &tokens) {
        tracing::warn!("could not save tokens: {e}");
    }
    let client = Arc::new(Client::new(cfg, tokens, Some(ctx.token_path.clone())));
    let user = client.me().await?;
    *ctx.client.write().await = Some(client);
    ctx.emit(Event::LoggedIn(user));
    load_playlists(ctx).await;
    Ok(())
}

async fn load_playlists(ctx: &Ctx) {
    let r = async { ctx.client().await?.my_playlists().await }.await;
    match r {
        Ok(p) => ctx.emit(Event::Playlists(p)),
        Err(e) => ctx.emit(Event::Error(err_text(&e))),
    }
}

async fn reload(ctx: &Ctx, pid: &str) {
    let r = async { ctx.client().await?.playlist_items(pid, &|| false).await }.await;
    match r {
        Ok(items) => ctx.emit(Event::Items { pid: pid.to_owned(), items }),
        Err(e) => ctx.emit(Event::ItemsFailed { pid: pid.to_owned(), error: err_text(&e) }),
    }
}

async fn refresh_meta(ctx: &Ctx, c: &Client, pid: &str) {
    if let Ok(p) = c.playlist(pid).await {
        ctx.emit(Event::PlaylistUpdated(p));
    }
}

async fn preview(ctx: &Ctx, track: Track) {
    if track.kind == crate::api::ItemKind::Video {
        return ctx.emit(Event::PreviewFailed(PreviewFail::Video));
    }
    ctx.emit(Event::PreviewLoading(track.full_title()));
    let r = async {
        let c = ctx.client().await?;
        let m = c.track_manifest(&track.id).await?;
        if m.drm {
            return Ok(Err(PreviewFail::Drm));
        }
        let xml = match dash::decode_data_uri(&m.uri) {
            Some(r) => r.map_err(ApiError::Other)?,
            None => String::from_utf8_lossy(&c.download(&m.uri).await?).into_owned(),
        };
        let reps =
            dash::parse_mpd(&xml, if m.uri.starts_with("data:") { "" } else { &m.uri }).map_err(ApiError::Other)?;
        let rep = dash::pick(&reps).ok_or_else(|| ApiError::Other("no playable stream in manifest".into()))?;
        let mut bytes = Vec::new();
        for u in &rep.urls {
            bytes.extend(c.download(u).await?);
        }
        Ok::<_, ApiError>(Ok((bytes, m.full, rep.codecs.clone())))
    }
    .await;
    match r {
        Ok(Ok((bytes, full, codecs))) => {
            let hint = if codecs.contains("flac") && !bytes.starts_with(b"\0\0") { "flac" } else { "mp4" };
            ctx.player.send(PlayerCmd::Play {
                bytes,
                hint: hint.into(),
                title: format!("{} — {}", track.full_title(), track.artist_line()),
                track_id: track.id.clone(),
                full,
            });
        }
        Ok(Err(f)) => ctx.emit(Event::PreviewFailed(f)),
        Err(e) => ctx.emit(Event::PreviewFailed(PreviewFail::Other(err_text(&e)))),
    }
}
