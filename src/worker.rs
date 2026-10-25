//! Background worker: owns the tokio runtime and the API client.
//! The UI sends [`Cmd`]s and receives [`Event`]s; it never blocks on the network.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use tokio::sync::{Mutex, RwLock};

use crate::api::auth::{self, AuthConfig};
use crate::api::{
    AccessType, Album, ApiError, Artist, Client, Genre, ItemRef, Playlist, PlaylistItem, SearchResults, Track, User,
};
use crate::ops::analysis;
use crate::ops::files::{ExportFormat, ImportPlaylist};
use crate::ops::generate;
use crate::ops::matcher::{self, MatchRow};
use crate::ops::recommend::{self, Recommendation};
use crate::ops::upgrade;
use crate::player::{PlayerCmd, PlayerHandle, dash};
use crate::spotify::{SpPlaylist, SpUser, SpotifyClient};

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
    /// Playback check: TIDAL refuses playback here (`GEO_RESTRICTED`).
    GeoRestricted,
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

/// What «better» means for [`Cmd::FindBetter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetterMode {
    /// Higher sound quality of the same recording; `atmos` counts Dolby Atmos as best.
    Quality { atmos: bool },
    /// The clean version of an explicit song.
    Clean,
}

#[derive(Clone, Debug)]
pub struct BetterHit {
    pub playlist: Playlist,
    pub item: PlaylistItem,
    pub track: Track,
    pub better: Option<Track>,
}

pub enum Cmd {
    RestoreSession(AuthConfig),
    Login(AuthConfig, Job),
    Logout,
    LoadPlaylists,
    LoadItems(String),
    CreatePlaylist {
        name: String,
        description: String,
        access: AccessType,
        add: Vec<ItemRef>,
        job: Job,
    },
    UpdatePlaylist {
        pid: String,
        name: String,
        description: String,
        access: AccessType,
    },
    DeletePlaylist(String),
    AddItems {
        pid: String,
        refs: Vec<ItemRef>,
        before: Option<String>,
        skip_dups: bool,
        job: Job,
    },
    RemoveItems {
        pid: String,
        items: Vec<(ItemRef, String)>,
        job: Job,
    },
    MoveItems {
        pid: String,
        items: Vec<(ItemRef, String)>,
        before: Option<String>,
    },
    /// Adds items to other playlists (and/or a new one), then removes them
    /// from `from` only when every add succeeded.
    MoveTo {
        from: String,
        items: Vec<(ItemRef, String)>,
        targets: Vec<String>,
        new_name: Option<String>,
        skip_dups: bool,
        job: Job,
    },
    ApplyOrder {
        pid: String,
        current: Vec<PlaylistItem>,
        order: Vec<PlaylistItem>,
        job: Job,
    },
    Restore {
        pid: String,
        target: Vec<ItemRef>,
        job: Job,
    },
    Duplicate {
        source: Playlist,
        name: String,
        job: Job,
    },
    Merge {
        sources: Vec<Playlist>,
        name: String,
        dedupe: bool,
        job: Job,
    },
    Search(String),
    SearchMore(String),
    AlbumTracks(Album),
    ArtistDetails(Artist),
    ScanUnavailable {
        playlists: Vec<Playlist>,
        strict: bool,
        geo: bool,
        job: Job,
    },
    FindReplacements {
        hits: Vec<UnavailableHit>,
        job: Job,
    },
    Replace {
        pid: String,
        swaps: Vec<(PlaylistItem, Track)>,
        job: Job,
    },
    Match {
        playlists: Vec<ImportPlaylist>,
        job: Job,
    },
    FetchForExport {
        playlists: Vec<Playlist>,
        format: ExportFormat,
        dir: PathBuf,
        backup: bool,
        job: Job,
    },
    Favourite {
        ids: Vec<String>,
        add: bool,
    },
    /// The audio track of each video (item id, video metadata), reported as
    /// [`Event::MusicVersions`].
    FindMusic {
        videos: Vec<(String, Track)>,
        job: Job,
    },
    /// Songs in `playlists` with a better version, reported as [`Event::Better`].
    /// For [`BetterMode::Clean`] every explicit song is listed, found or not.
    FindBetter {
        playlists: Vec<Playlist>,
        mode: BetterMode,
        job: Job,
    },
    /// Every video in `playlists`, reported as [`Event::Videos`].
    FindVideos {
        playlists: Vec<Playlist>,
        job: Job,
    },
    /// Similar songs for `pid`, from TIDAL's similar tracks of each seed.
    /// `have`: songs to leave out (already in the playlist or already shown).
    Recommend {
        pid: String,
        seeds: Vec<String>,
        have: Vec<Track>,
        job: Job,
    },
    /// New playlists of songs from `genres`: one per genre when `names` has
    /// one name per genre, else one mixed playlist named `names[0]`. Songs in
    /// the `known` playlists are left out.
    Generate {
        genres: Vec<Genre>,
        count: usize,
        names: Vec<String>,
        description: String,
        known: Vec<Playlist>,
        job: Job,
    },
    Preview(Track),
    SpotifyRestore(AuthConfig),
    SpotifyLogin(AuthConfig, Job),
    SpotifyLogout,
    SpotifyLoadPlaylists,
    /// Read Spotify playlists and match their songs on TIDAL (→ review dialog).
    FromSpotify {
        playlists: Vec<SpPlaylist>,
        job: Job,
    },
    /// Read TIDAL playlists and match their songs on Spotify (→ review dialog).
    /// `known` fills in names of TIDAL items that are no longer returned.
    ToSpotify {
        playlists: Vec<Playlist>,
        known: std::collections::HashMap<String, Track>,
        job: Job,
    },
    /// Create a Spotify playlist (or use `target`) and add the matched tracks.
    SpotifyWrite {
        name: String,
        description: String,
        public: bool,
        target: Option<String>,
        ids: Vec<String>,
        job: Job,
    },
}

/// Where matched songs will be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Tidal,
    Spotify,
}

#[derive(Clone, Debug)]
pub enum Note {
    Added {
        n: usize,
        skipped: usize,
    },
    Removed(usize),
    Reordered,
    Created(String),
    Duplicated(String),
    Merged {
        name: String,
        n: usize,
    },
    Updated,
    Deleted,
    Restored,
    Replaced(usize),
    Generated {
        playlists: usize,
        songs: usize,
        wanted: usize,
    },
    Favourited {
        n: usize,
        add: bool,
    },
    /// Spotify items that cannot be moved (local files, podcast episodes, videos).
    Skipped(usize),
    SpotifyWritten {
        name: String,
        n: usize,
    },
}

#[derive(Clone, Debug)]
pub enum PreviewFail {
    Drm,
    GeoRestricted,
    Video,
    Other(String),
}

pub enum Event {
    LoginStarted(String),
    LoggedIn(User),
    LoginFailed(String),
    LoggedOut,
    Playlists(Vec<Playlist>),
    Items {
        pid: String,
        items: Vec<PlaylistItem>,
    },
    ItemsFailed {
        pid: String,
        error: String,
    },
    PlaylistCreated(Playlist),
    PlaylistUpdated(Playlist),
    PlaylistDeleted(String),
    Search(SearchResults),
    SearchMore {
        tracks: Vec<Track>,
        next: Option<String>,
    },
    Album {
        album: Album,
        tracks: Vec<Track>,
    },
    Artist {
        artist: Artist,
        albums: Vec<Album>,
        tracks: Vec<Track>,
    },
    Progress {
        job: JobId,
        done: usize,
        total: usize,
    },
    /// Progress as a share (0–1000) when items and playlists are mixed.
    Permille {
        job: JobId,
        done: usize,
    },
    JobDone {
        job: JobId,
        note: Option<Note>,
    },
    JobFailed {
        job: JobId,
        error: String,
    },
    Unavailable(Vec<UnavailableHit>),
    /// (video item id, its audio track if found).
    MusicVersions(Vec<(String, Option<Track>)>),
    Better {
        mode: BetterMode,
        hits: Vec<BetterHit>,
    },
    /// (playlist, video) pairs, in playlist order.
    Videos(Vec<(Playlist, PlaylistItem)>),
    Recommendations {
        pid: String,
        recs: Vec<Recommendation>,
    },
    Replacements(Vec<(UnavailableHit, Option<Track>)>),
    Matches {
        dest: Dest,
        playlists: Vec<(ImportPlaylist, Vec<MatchRow>)>,
    },
    SpotifyLoginStarted(String),
    SpotifyLoggedIn(SpUser),
    /// `expired`: the refresh token stopped working; the user must connect again.
    SpotifyLoggedOut {
        expired: bool,
    },
    SpotifyLoginFailed(String),
    SpotifyPlaylists(Vec<SpPlaylist>),
    ExportData {
        data: Vec<(Playlist, Vec<PlaylistItem>)>,
        format: ExportFormat,
        dir: PathBuf,
        backup: bool,
    },
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
    spotify: Arc<RwLock<Option<Arc<SpotifyClient>>>>,
    sp_token_path: PathBuf,
}

impl Ctx {
    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
        (self.repaint)();
    }

    async fn client(&self) -> Result<Arc<Client>, ApiError> {
        self.client.read().await.clone().ok_or(ApiError::NotLoggedIn)
    }

    async fn spotify(&self) -> Result<Arc<SpotifyClient>, ApiError> {
        self.spotify
            .read()
            .await
            .clone()
            .ok_or_else(|| ApiError::Auth(format!("{}: not connected", crate::spotify::LOGIN_ERROR)))
    }

    /// After a Spotify call failed: if the Spotify login is gone, forget it and tell the UI.
    /// (TIDAL errors in the same job must not disconnect Spotify.)
    async fn spotify_failed(&self, e: &ApiError) {
        if crate::spotify::is_login_error(e) {
            *self.spotify.write().await = None;
            let _ = std::fs::remove_file(&self.sp_token_path);
            self.emit(Event::SpotifyLoggedOut { expired: true });
        }
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
    pub fn spawn(
        token_path: PathBuf,
        sp_token_path: PathBuf,
        player: PlayerHandle,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
        let (etx, events) = mpsc::channel();
        let ctx = Ctx {
            tx: etx,
            repaint,
            client: Arc::new(RwLock::new(None)),
            write_lock: Arc::new(Mutex::new(())),
            token_path,
            player,
            spotify: Arc::new(RwLock::new(None)),
            sp_token_path,
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

/// Error text in the UI's language. `Cancelled` stays the fixed word
/// "cancelled", which the UI recognises and shows as a note.
fn err_text(e: &ApiError) -> String {
    let tr = crate::i18n::current();
    match e {
        ApiError::Cancelled => "cancelled".into(),
        ApiError::NotLoggedIn => tr.err_not_logged_in.into(),
        ApiError::Network(n) => format!("{}: {n}", tr.err_network),
        ApiError::Auth(m) => format!("{}: {m}", tr.err_login),
        ApiError::Http { .. } | ApiError::Other(_) => e.to_string(),
    }
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
        Cmd::SpotifyRestore(cfg) => {
            let Some(tokens) = auth::load_tokens(&ctx.sp_token_path) else { return };
            let client = Arc::new(SpotifyClient::new(cfg, tokens, Some(ctx.sp_token_path.clone())));
            match client.me().await {
                Ok(user) => {
                    *ctx.spotify.write().await = Some(client);
                    ctx.emit(Event::SpotifyLoggedIn(user));
                }
                Err(e) => {
                    tracing::info!("stored Spotify session not usable: {e}");
                    ctx.spotify_failed(&e).await;
                }
            }
        }
        Cmd::SpotifyLogin(cfg, job) => {
            let r = async {
                let tokens = browser_login(&ctx, &cfg, &job, Event::SpotifyLoginStarted).await?;
                if let Err(e) = auth::save_tokens(&ctx.sp_token_path, &tokens) {
                    tracing::warn!("could not save Spotify tokens: {e}");
                }
                let client = Arc::new(SpotifyClient::new(cfg, tokens, Some(ctx.sp_token_path.clone())));
                let user = client.me().await?;
                *ctx.spotify.write().await = Some(client);
                Ok::<_, ApiError>(user)
            }
            .await;
            ctx.emit(Event::JobDone { job: job.id, note: None });
            match r {
                Ok(user) => {
                    ctx.emit(Event::SpotifyLoggedIn(user));
                    sp_load_playlists(&ctx).await;
                }
                Err(ApiError::Cancelled) => ctx.emit(Event::SpotifyLoggedOut { expired: false }),
                Err(e) => ctx.emit(Event::SpotifyLoginFailed(err_text(&e))),
            }
        }
        Cmd::SpotifyLogout => {
            *ctx.spotify.write().await = None;
            let _ = std::fs::remove_file(&ctx.sp_token_path);
            ctx.emit(Event::SpotifyLoggedOut { expired: false });
        }
        Cmd::SpotifyLoadPlaylists => sp_load_playlists(&ctx).await,
        job_cmd => run_job(&ctx, job_cmd).await,
    }
}

/// Commands that report through `JobDone` / `JobFailed`.
async fn run_job(ctx: &Ctx, cmd: Cmd) {
    let job = match &cmd {
        Cmd::CreatePlaylist { job, .. }
        | Cmd::AddItems { job, .. }
        | Cmd::RemoveItems { job, .. }
        | Cmd::MoveTo { job, .. }
        | Cmd::ApplyOrder { job, .. }
        | Cmd::Restore { job, .. }
        | Cmd::Duplicate { job, .. }
        | Cmd::Merge { job, .. }
        | Cmd::ScanUnavailable { job, .. }
        | Cmd::FindReplacements { job, .. }
        | Cmd::Recommend { job, .. }
        | Cmd::FindVideos { job, .. }
        | Cmd::FindBetter { job, .. }
        | Cmd::FindMusic { job, .. }
        | Cmd::Generate { job, .. }
        | Cmd::Replace { job, .. }
        | Cmd::Match { job, .. }
        | Cmd::FetchForExport { job, .. }
        | Cmd::FromSpotify { job, .. }
        | Cmd::ToSpotify { job, .. }
        | Cmd::SpotifyWrite { job, .. } => job.clone(),
        _ => return,
    };
    let spotify_job = matches!(cmd, Cmd::FromSpotify { .. } | Cmd::ToSpotify { .. } | Cmd::SpotifyWrite { .. });
    let result = job_body(ctx, cmd, &job).await;
    if spotify_job && let Err(e) = &result {
        ctx.spotify_failed(e).await;
    }
    match result {
        Ok(note) => ctx.emit(Event::JobDone { job: job.id, note }),
        Err(e) => ctx.emit(Event::JobFailed { job: job.id, error: err_text(&e) }),
    }
}

async fn job_body(ctx: &Ctx, cmd: Cmd, job: &Job) -> Result<Option<Note>, ApiError> {
    let cancel = || job.cancelled();
    // Writing to Spotify does not need the TIDAL session.
    let cmd = match cmd {
        Cmd::SpotifyWrite { name, description, public, target, ids, .. } => {
            let sp = ctx.spotify().await?;
            let pid = match target {
                Some(t) => t,
                None => sp.create_playlist(&name, &description, public).await?.id,
            };
            sp.add_tracks(&pid, &ids, &ctx.progress(job, ids.len()), &cancel).await?;
            sp_load_playlists(ctx).await;
            return Ok(Some(Note::SpotifyWritten { name, n: ids.len() }));
        }
        other => other,
    };
    let c = ctx.client().await?;
    match cmd {
        Cmd::CreatePlaylist { name, description, access, add, .. } => {
            let _g = ctx.write_lock.lock().await;
            let p = c.create_playlist(&name, &description, access).await?;
            ctx.emit(Event::PlaylistCreated(p.clone()));
            let r = if add.is_empty() {
                Ok(0)
            } else {
                c.add_items(&p.id, &add, None, false, &ctx.progress(job, add.len()), &cancel).await
            };
            // Reload even after a failed add, so the UI shows what was written.
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            r?;
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
        Cmd::MoveTo { from, items, targets, new_name, skip_dups, .. } => {
            let _g = ctx.write_lock.lock().await;
            let refs: Vec<ItemRef> = items.iter().map(|(r, _)| r.clone()).collect();
            let steps = refs.len() * (targets.len() + usize::from(new_name.is_some()) + 1);
            let p = ctx.progress(job, steps.max(1));
            let mut base = 0;
            let mut r: Result<(), ApiError> = Ok(());
            for pid in &targets {
                r = c.add_items(pid, &refs, None, skip_dups, &|n| p(base + n), &cancel).await.map(|_| ());
                refresh_meta(ctx, &c, pid).await;
                reload(ctx, pid).await;
                base += refs.len();
                if r.is_err() {
                    break;
                }
            }
            if r.is_ok()
                && let Some(name) = &new_name
            {
                r = match c.create_playlist(name, "", AccessType::Unlisted).await {
                    Ok(np) => {
                        ctx.emit(Event::PlaylistCreated(np.clone()));
                        let a = c.add_items(&np.id, &refs, None, false, &|n| p(base + n), &cancel).await;
                        refresh_meta(ctx, &c, &np.id).await;
                        reload(ctx, &np.id).await;
                        base += refs.len();
                        a.map(|_| ())
                    }
                    Err(e) => Err(e),
                };
            }
            if r.is_ok() {
                r = c.remove_items(&from, &items, &|n| p(base + n), &cancel).await;
            }
            // The source is reloaded in every case: the UI removed the rows early.
            refresh_meta(ctx, &c, &from).await;
            reload(ctx, &from).await;
            r?;
            Ok(Some(Note::Removed(items.len())))
        }
        Cmd::ApplyOrder { pid, current, order, .. } => {
            let _g = ctx.write_lock.lock().await;
            // The UI's copy may be out of date (an earlier write failed or was
            // cancelled), so order against what the server holds now.
            let r = match c.playlist_items(&pid, &cancel).await {
                Ok(now) => {
                    let mut a: Vec<&str> = now.iter().map(|i| i.item_id.as_str()).collect();
                    let mut b: Vec<&str> = order.iter().map(|i| i.item_id.as_str()).collect();
                    a.sort_unstable();
                    b.sort_unstable();
                    if a == b {
                        c.apply_order(&pid, &now, &order, &ctx.progress(job, order.len()), &cancel).await
                    } else {
                        tracing::info!(
                            "order for {pid} skipped: playlist changed ({} of {current:?} items)",
                            now.len(),
                            current = current.len()
                        );
                        Err(ApiError::Other(crate::i18n::current().err_playlist_changed.into()))
                    }
                }
                Err(e) => Err(e),
            };
            reload(ctx, &pid).await;
            r?;
            Ok(Some(Note::Reordered))
        }
        Cmd::Restore { pid, target, .. } => {
            let _g = ctx.write_lock.lock().await;
            let r = restore(ctx, &c, &pid, &target, job).await;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            r?;
            Ok(Some(Note::Restored))
        }
        Cmd::Duplicate { source, name, .. } => {
            let items = c.playlist_items(&source.id, &cancel).await?;
            ctx.emit(Event::Items { pid: source.id.clone(), items: items.clone() });
            let _g = ctx.write_lock.lock().await;
            let p = c.create_playlist(&name, &source.description, source.access_type).await?;
            ctx.emit(Event::PlaylistCreated(p.clone()));
            let refs: Vec<ItemRef> = items.iter().map(|i| i.r.clone()).collect();
            let r = c.add_items(&p.id, &refs, None, false, &ctx.progress(job, refs.len()), &cancel).await;
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            r?;
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
            let r = c.add_items(&p.id, &refs, None, false, &ctx.progress(job, refs.len()), &cancel).await;
            refresh_meta(ctx, &c, &p.id).await;
            reload(ctx, &p.id).await;
            r?;
            Ok(Some(Note::Merged { name, n: refs.len() }))
        }
        Cmd::ScanUnavailable { playlists, strict, geo, .. } => {
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
                let mut geo_blocked = std::collections::HashSet::new();
                if geo {
                    let ids: Vec<String> = items
                        .iter()
                        .filter(|i| {
                            i.r.kind == crate::api::ItemKind::Track && i.available() && !bad_rules.contains(&i.r.id)
                        })
                        .map(|i| i.r.id.clone())
                        .collect();
                    // Per-track progress inside this playlist's share of the bar.
                    let geo_progress = |d: usize, t: usize| {
                        let done = (n * 1000 + d * 1000 / t.max(1)) / total.max(1);
                        ctx.emit(Event::Permille { job: job.id, done });
                    };
                    geo_blocked = geo_check(&c, ids, job, &geo_progress).await?;
                }
                for it in items {
                    let reason = if it.track.is_none() {
                        Some(Reason::Missing)
                    } else if !it.available() {
                        Some(Reason::NotStreamable)
                    } else if bad_rules.contains(&it.r.id) {
                        Some(Reason::NoUsageRule)
                    } else if geo_blocked.contains(&it.r.id) {
                        Some(Reason::GeoRestricted)
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
                if geo {
                    ctx.emit(Event::Permille { job: job.id, done: (n + 1) * 1000 / total.max(1) });
                } else {
                    ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
                }
            }
            ctx.emit(Event::Unavailable(hits));
            Ok(None)
        }
        Cmd::FindMusic { videos, .. } => {
            let total = videos.len();
            let mut out = Vec::with_capacity(total);
            for (n, (id, v)) in videos.into_iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                out.push((id, find_replacement(&c, &v, true).await));
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::MusicVersions(out));
            Ok(None)
        }
        Cmd::FindVideos { playlists, .. } => {
            let mut found = Vec::new();
            let total = playlists.len();
            for (n, p) in playlists.into_iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let items = c.playlist_items(&p.id, &cancel).await?;
                let videos = items.iter().filter(|i| i.r.kind == crate::api::ItemKind::Video).cloned();
                found.extend(videos.map(|v| (p.clone(), v)));
                ctx.emit(Event::Items { pid: p.id.clone(), items });
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::Videos(found));
            Ok(None)
        }
        Cmd::FindBetter { playlists, mode, .. } => {
            let total = playlists.len();
            let mut hits = Vec::new();
            // A song in several playlists is looked up once.
            let mut known: std::collections::HashMap<String, Option<Track>> = std::collections::HashMap::new();
            for (n, p) in playlists.into_iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let items = c.playlist_items(&p.id, &cancel).await?;
                ctx.emit(Event::Items { pid: p.id.clone(), items: items.clone() });
                let todo: Vec<(PlaylistItem, Track)> = items
                    .into_iter()
                    .filter_map(|i| {
                        let t = i.track.clone()?;
                        let wanted = match mode {
                            BetterMode::Quality { atmos } => t.isrc.is_some() && !upgrade::is_best(&t, atmos),
                            BetterMode::Clean => t.explicit,
                        };
                        (wanted && t.kind == crate::api::ItemKind::Track && i.available()).then_some((i, t))
                    })
                    .collect();
                let mut ask: Vec<Track> = todo.iter().map(|(_, t)| t.clone()).collect();
                ask.retain(|t| !known.contains_key(&t.id));
                ask.dedup_by(|a, b| a.id == b.id);
                let progress = |d: usize, t: usize| {
                    let done = (n * 1000 + d * 1000 / t.max(1)) / total.max(1);
                    ctx.emit(Event::Permille { job: job.id, done });
                };
                let found = better_versions(&c, &ask, mode, job, &progress).await?;
                known.extend(ask.into_iter().map(|t| t.id).zip(found));
                for (item, track) in todo {
                    let better = known.get(&track.id).cloned().flatten();
                    if better.is_some() || mode == BetterMode::Clean {
                        hits.push(BetterHit { playlist: p.clone(), item, track, better });
                    }
                }
                ctx.emit(Event::Permille { job: job.id, done: (n + 1) * 1000 / total.max(1) });
            }
            ctx.emit(Event::Better { mode, hits });
            Ok(None)
        }
        Cmd::Recommend { pid, seeds, have, .. } => {
            let total = seeds.len();
            let mut lists = Vec::with_capacity(total);
            for (n, id) in seeds.iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                lists.push(c.similar_tracks(id).await?);
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total });
            }
            ctx.emit(Event::Recommendations { pid, recs: recommend::rank(lists, &have) });
            Ok(None)
        }
        Cmd::Generate { genres, count, names, description, known, .. } => {
            let per_genre = names.len() == genres.len() && genres.len() > 1;
            let total = known.len() + genres.len() + if per_genre { genres.len() } else { 1 };
            let mut step = 0;
            let mut tick = || {
                step += 1;
                ctx.emit(Event::Progress { job: job.id, done: step, total });
            };
            let mut have = std::collections::HashSet::new();
            for p in &known {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let items = c.playlist_items(&p.id, &cancel).await?;
                have.extend(items.iter().filter_map(|i| i.track.as_ref()).flat_map(recommend::keys));
                ctx.emit(Event::Items { pid: p.id.clone(), items });
                tick();
            }
            let wants = if per_genre { vec![count; genres.len()] } else { generate::split(count, genres.len()) };
            let mut lists = Vec::new();
            for (g, want) in genres.iter().zip(wants) {
                let songs = genre_songs(&c, g, want, &have, &cancel).await?;
                // A song picked for one genre is not picked again for another.
                have.extend(songs.iter().flat_map(recommend::keys));
                lists.push(songs);
                tick();
            }
            let outputs: Vec<(String, Vec<Track>)> = if per_genre {
                names.into_iter().zip(lists).collect()
            } else {
                vec![(names.into_iter().next().unwrap_or_default(), generate::interleave(lists))]
            };
            let _g = ctx.write_lock.lock().await;
            let (mut made, mut songs) = (0, 0);
            for (name, tracks) in outputs {
                if tracks.is_empty() || cancel() {
                    continue;
                }
                let p = c.create_playlist(&name, &description, AccessType::Unlisted).await?;
                ctx.emit(Event::PlaylistCreated(p.clone()));
                let refs: Vec<ItemRef> = tracks.iter().map(Track::item_ref).collect();
                let r = c.add_items(&p.id, &refs, None, false, &|_| {}, &cancel).await;
                refresh_meta(ctx, &c, &p.id).await;
                reload(ctx, &p.id).await;
                r?;
                made += 1;
                songs += refs.len();
                tick();
            }
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let wanted = if per_genre { count * genres.len() } else { count };
            Ok(Some(Note::Generated { playlists: made, songs, wanted }))
        }
        Cmd::FindReplacements { hits, .. } => {
            let mut list = Vec::new();
            let total = hits.len();
            for (n, h) in hits.into_iter().enumerate() {
                if cancel() {
                    return Err(ApiError::Cancelled);
                }
                let found = match &h.known {
                    Some(k) => find_replacement(&c, k, false).await,
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
            // Only originals whose replacement was really added get removed, so
            // a cancel or an error never loses a song without a substitute.
            let mut added = 0;
            let mut failed = None;
            for (n, (old, new)) in swaps.iter().enumerate() {
                if cancel() {
                    break;
                }
                if let Err(e) = c.add_items(&pid, &[new.item_ref()], Some(&old.item_id), false, &|_| {}, &cancel).await
                {
                    failed = Some(e);
                    break;
                }
                added += 1;
                ctx.emit(Event::Progress { job: job.id, done: n + 1, total: total * 2 });
            }
            let old: Vec<(ItemRef, String)> =
                swaps[..added].iter().map(|(o, _)| (o.r.clone(), o.item_id.clone())).collect();
            let removed = c
                .remove_items(
                    &pid,
                    &old,
                    &|d| ctx.emit(Event::Progress { job: job.id, done: total + d, total: total * 2 }),
                    &|| false,
                )
                .await;
            refresh_meta(ctx, &c, &pid).await;
            reload(ctx, &pid).await;
            removed?;
            if let Some(e) = failed {
                return Err(e);
            }
            if added < total {
                return Err(ApiError::Cancelled);
            }
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
                    &*c,
                    p.rows.clone(),
                    &|d| ctx.emit(Event::Progress { job: job.id, done: b + d, total }),
                    &cancel,
                )
                .await?;
                base += n;
                out.push((p, rows));
            }
            ctx.emit(Event::Matches { dest: Dest::Tidal, playlists: out });
            Ok(None)
        }
        Cmd::FromSpotify { playlists, .. } => {
            let sp = ctx.spotify().await?;
            let mut sources = Vec::new();
            let mut skipped = 0;
            for p in &playlists {
                let items = sp.playlist_tracks(p, &cancel).await?;
                skipped += items.skipped;
                sources.push(ImportPlaylist {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    access_type: if p.public { AccessType::Public } else { AccessType::Unlisted },
                    // Spotify ids mean nothing to TIDAL: match by ISRC, then artist + title.
                    rows: items.tracks.iter().map(|t| row_from_track(t, false)).collect(),
                    exact: false,
                });
            }
            if skipped > 0 {
                ctx.emit(Event::Note(Note::Skipped(skipped)));
            }
            let out = match_all(ctx, &*c, sources, job).await?;
            ctx.emit(Event::Matches { dest: Dest::Tidal, playlists: out });
            Ok(None)
        }
        Cmd::ToSpotify { playlists, known, .. } => {
            let sp = ctx.spotify().await?;
            let mut sources = Vec::new();
            let mut skipped = 0;
            for p in &playlists {
                let items = c.playlist_items(&p.id, &cancel).await?;
                ctx.emit(Event::Items { pid: p.id.clone(), items: items.clone() });
                let mut rows = Vec::new();
                for it in &items {
                    match it.track.as_ref().or_else(|| known.get(&it.r.id)) {
                        Some(t) if t.kind == crate::api::ItemKind::Track && !t.title.is_empty() => {
                            rows.push(row_from_track(t, false));
                        }
                        _ => skipped += 1,
                    }
                }
                sources.push(ImportPlaylist {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    access_type: p.access_type,
                    rows,
                    exact: false,
                });
            }
            if skipped > 0 {
                ctx.emit(Event::Note(Note::Skipped(skipped)));
            }
            let out = match_all(ctx, &*sp, sources, job).await?;
            ctx.emit(Event::Matches { dest: Dest::Spotify, playlists: out });
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

/// Codes that describe this app's access, not the track. If every answer is one
/// of these, the check cannot tell anything and is stopped with an explanation.
const APP_LEVEL_CODES: [&str; 4] = ["CLIENT_NOT_ENTITLED", "PREREQUISITE_MISSING", "CONCURRENT_PLAYBACK", "FORBIDDEN"];

/// Playback check: asks TIDAL for each track's playback manifest (4 at a time)
/// and returns the ids refused with `GEO_RESTRICTED`.
async fn geo_check(
    c: &Arc<Client>,
    ids: Vec<String>,
    job: &Job,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<std::collections::HashSet<String>, ApiError> {
    let total = ids.len();
    let mut blocked = std::collections::HashSet::new();
    let (mut done, mut app_level) = (0usize, 0usize);
    let mut last_app_code = String::new();
    let mut pending = ids.into_iter();
    let mut set = tokio::task::JoinSet::new();
    loop {
        while set.len() < 4 && !job.cancelled() {
            let Some(id) = pending.next() else { break };
            let c = c.clone();
            set.spawn(async move { (c.playback_restriction(&id).await, id) });
        }
        let Some(res) = set.join_next().await else { break };
        let (r, id) = res.map_err(|e| ApiError::Other(e.to_string()))?;
        match r? {
            Some(code) if code == "GEO_RESTRICTED" => {
                blocked.insert(id);
            }
            Some(code) if APP_LEVEL_CODES.contains(&code.as_str()) => {
                app_level += 1;
                last_app_code = code;
            }
            _ => {}
        }
        done += 1;
        progress(done, total);
        // Small playlists are judged once every track has answered.
        if (done >= 5 || done == total) && app_level == done {
            set.abort_all();
            let tr = crate::i18n::current();
            return Err(ApiError::Other(crate::i18n::fmt(tr.err_playback_check_refused, &[&last_app_code])));
        }
    }
    if job.cancelled() {
        return Err(ApiError::Cancelled);
    }
    Ok(blocked)
}

/// [`better_version`] for each of `tracks` (4 at a time), in the same order.
async fn better_versions(
    c: &Arc<Client>,
    tracks: &[Track],
    mode: BetterMode,
    job: &Job,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Option<Track>>, ApiError> {
    let total = tracks.len();
    let mut out = vec![None; total];
    let mut pending = tracks.iter().cloned().enumerate();
    let mut set = tokio::task::JoinSet::new();
    let mut done = 0;
    loop {
        while set.len() < 4 && !job.cancelled() {
            let Some((n, t)) = pending.next() else { break };
            let c = c.clone();
            set.spawn(async move { (n, better_version(&c, &t, mode).await) });
        }
        let Some(res) = set.join_next().await else { break };
        let (n, r) = res.map_err(|e| ApiError::Other(e.to_string()))?;
        out[n] = r?;
        done += 1;
        progress(done, total);
    }
    if job.cancelled() {
        return Err(ApiError::Cancelled);
    }
    Ok(out)
}

/// A better version of `t`: for quality, another release of the same
/// recording (same ISRC); for a clean version, by ISRC and then by search.
/// A lookup TIDAL refuses (an HTTP error) just finds nothing.
async fn better_version(c: &Client, t: &Track, mode: BetterMode) -> Result<Option<Track>, ApiError> {
    let soft = |r: Result<Vec<Track>, ApiError>| match r {
        Err(e) if e.status().is_some() => {
            tracing::debug!("better version of {}: {e}", t.id);
            Ok(Vec::new())
        }
        other => other,
    };
    let by_isrc = match &t.isrc {
        Some(isrc) => soft(c.tracks_by_isrc(isrc).await)?,
        None => Vec::new(),
    };
    match mode {
        BetterMode::Quality { atmos } => Ok(upgrade::better_quality(t, by_isrc, atmos)),
        BetterMode::Clean => {
            if let Some(b) = upgrade::clean_version(t, by_isrc) {
                return Ok(Some(b));
            }
            // TIDAL's search is sensitive to word order: try artist first, then title first.
            let text = matcher::search_text(&upgrade::row_of(t));
            let title_first = format!("{} {}", t.title, t.artists.first().map(String::as_str).unwrap_or(""));
            for q in [text.clone(), title_first.trim().to_owned(), format!("{text} clean")] {
                let found = soft(c.search(&q).await.map(|r| r.tracks))?;
                if let Some(b) = upgrade::clean_version(t, found) {
                    return Ok(Some(b));
                }
            }
            Ok(None)
        }
    }
}

/// The same song as `known`, playable: by ISRC first, then by search.
/// With `from_video`, `known` is a music video and the answer is its audio track:
/// the title is cleaned of video words, and ISRC, album and length are ignored
/// (a video has its own ISRC, and is often longer than the song).
async fn find_replacement(c: &Client, known: &Track, from_video: bool) -> Option<Track> {
    let row = crate::ops::files::ImportRow {
        kind: if from_video { crate::api::ItemKind::Track } else { known.kind },
        id: None,
        isrc: if from_video { None } else { known.isrc.clone() },
        title: if from_video { matcher::music_title(&known.title) } else { known.title.clone() },
        artists: known.artists.clone(),
        album: if from_video { None } else { known.album.clone() },
        duration_secs: (known.duration_secs > 0 && !from_video).then_some(known.duration_secs),
    };
    let usable =
        |t: &Track| t.id != known.id && t.streamable() && (!from_video || t.kind == crate::api::ItemKind::Track);
    if let Some(isrc) = &row.isrc
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
        .map(|(s, t)| (s + matcher::tie_break(&row, &t), t))
        // `min_by` with the order reversed keeps the first of equals: TIDAL's search order.
        .min_by(|a, b| b.0.total_cmp(&a.0))
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

/// Builds a match row from a known track (Spotify or TIDAL metadata).
fn row_from_track(t: &Track, keep_id: bool) -> crate::ops::files::ImportRow {
    crate::ops::files::ImportRow {
        kind: crate::api::ItemKind::Track,
        id: keep_id.then(|| t.id.clone()),
        isrc: t.isrc.clone(),
        title: t.full_title(),
        artists: t.artists.clone(),
        album: t.album.clone(),
        duration_secs: (t.duration_secs > 0).then_some(t.duration_secs),
    }
}

async fn match_all<C: matcher::Catalog>(
    ctx: &Ctx,
    catalog: &C,
    sources: Vec<ImportPlaylist>,
    job: &Job,
) -> Result<Vec<(ImportPlaylist, Vec<MatchRow>)>, ApiError> {
    let total: usize = sources.iter().map(|p| p.rows.len()).sum();
    let mut out = Vec::new();
    let mut base = 0;
    for p in sources {
        let b = base;
        let rows = matcher::match_rows(
            catalog,
            p.rows.clone(),
            &|d| ctx.emit(Event::Progress { job: job.id, done: b + d, total }),
            &|| job.cancelled(),
        )
        .await?;
        base += p.rows.len();
        out.push((p, rows));
    }
    Ok(out)
}

async fn sp_load_playlists(ctx: &Ctx) {
    let r = async { ctx.spotify().await?.playlists().await }.await;
    match r {
        Ok(p) => ctx.emit(Event::SpotifyPlaylists(p)),
        Err(e) => {
            ctx.spotify_failed(&e).await;
            ctx.emit(Event::Error(err_text(&e)));
        }
    }
}

/// PKCE in the system browser with a loopback redirect; returns fresh tokens.
async fn browser_login(
    ctx: &Ctx,
    cfg: &AuthConfig,
    job: &Job,
    started: fn(String) -> Event,
) -> Result<auth::TokenSet, ApiError> {
    let pkce = auth::Pkce::new();
    let url = auth::authorize_url(cfg, &pkce);
    ctx.emit(started(url.clone()));
    if let Err(e) = open::that_detached(&url) {
        tracing::warn!("could not open browser: {e}");
    }
    let (cfg2, state, cancel) = (cfg.clone(), pkce.state.clone(), job.cancel.clone());
    let code = tokio::task::spawn_blocking(move || auth::wait_for_code(&cfg2, &state, cancel))
        .await
        .map_err(|e| ApiError::Other(e.to_string()))??;
    auth::exchange_code(&Client::http(), cfg, &code, &pkce.verifier).await
}

async fn login(ctx: &Ctx, cfg: AuthConfig, job: &Job) -> Result<(), ApiError> {
    let tokens = browser_login(ctx, &cfg, job, Event::LoginStarted).await?;
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

/// TIDAL playlists searched per genre for songs.
const GENRE_SOURCES: usize = 8;

/// Up to `want` playable songs for a genre that are not in `have`: tracks
/// found by searching TIDAL for the genre (`g.id`), mostly from its own
/// editorial playlists such as «Jazz Classics».
async fn genre_songs(
    c: &Client,
    g: &Genre,
    want: usize,
    have: &std::collections::HashSet<String>,
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<Track>, ApiError> {
    if want == 0 {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    match c.search(&g.id).await {
        Ok(s) => found.extend(s.tracks),
        Err(e) if e.status().is_some() => tracing::debug!("track search for {}: {e}", g.name),
        Err(e) => return Err(e),
    }
    let sources = match c.search_playlists(&g.id).await {
        Ok(p) => p,
        Err(e) if e.status().is_some() => {
            tracing::debug!("playlist search for {}: {e}", g.name);
            Vec::new()
        }
        Err(e) => return Err(e),
    };
    // Gather about three times what is needed, so leaving out the songs
    // already owned still leaves enough to pick from at random.
    for p in sources.iter().take(GENRE_SOURCES) {
        if found.len() >= want * 3 {
            break;
        }
        match c.playlist_items(&p.id, cancel).await {
            Ok(items) => found.extend(items.into_iter().filter_map(|i| i.track)),
            Err(ApiError::Cancelled) => return Err(ApiError::Cancelled),
            Err(e) => tracing::debug!("genre source {}: {e}", p.id),
        }
    }
    let mut picked = generate::fresh(found, have);
    picked.truncate(want);
    Ok(picked)
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
        let rep = dash::pick(&reps).ok_or_else(|| ApiError::Other(crate::i18n::current().err_no_stream.into()))?;
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
        Err(e) if e.code() == Some("GEO_RESTRICTED") => ctx.emit(Event::PreviewFailed(PreviewFail::GeoRestricted)),
        Err(e) => ctx.emit(Event::PreviewFailed(PreviewFail::Other(err_text(&e)))),
    }
}
