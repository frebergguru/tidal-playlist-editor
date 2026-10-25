# AGENTS.md

Notes for AI coding agents (and people) working on TIDAL Playlist Editor.
README.md describes the app for users; this file covers how the code is built and the rules it follows.

## What it is

A desktop editor for TIDAL playlists: Rust, eframe/egui 0.37, the official TIDAL API v2
(`https://openapi.tidal.com/v2`, JSON:API). Linux and Windows. The UI is in English and
Norwegian (bokmål). Users register their own TIDAL developer app and enter its Client ID
on first run; Spotify (optional) works the same way.

## Build and test

```sh
tools/fetch-assets.sh                  # fonts into assets/fonts/ (build.rs fails without them)
cargo build                            # toolchain pinned in rust-toolchain.toml (1.99.1)
cargo test                             # unit, API-mock (wiremock) and headless UI tests (egui_kittest)
cargo clippy --all-targets -- -D warnings
cargo fmt                              # rustfmt.toml: max_width 120
cargo test screenshots -- --ignored    # renders target/screens/*.png, no window
cargo test emoji_screenshot -- --ignored
./release.sh                           # Linux tar.gz + AppImage, Windows zip in dist/<version>/
```

- Every change must leave `cargo test` and `cargo clippy --all-targets -- -D warnings` clean;
  `release.sh` runs both and stops on any failure.
- Windows is cross-compiled with MinGW (`x86_64-pc-windows-gnu`). After changing
  dependencies, also run `cargo check --target x86_64-pc-windows-gnu`.
- Look at the UI through the headless screenshots (`target/screens/`), not by opening a window.
- `cargo fmt` also reformats `build.rs`; keep that file as it is (`git checkout build.rs`).

## Layout

| Path | What |
|---|---|
| `src/main.rs` | Start-up, logging, window options |
| `src/api/` | TIDAL client: auth (PKCE, loopback redirect), retries, paging, batching (`client.rs`), JSON:API parsing (`jsonapi.rs`), models |
| `src/spotify.rs` | Spotify Web API client (copying playlists both ways) |
| `src/worker.rs` | Background worker: owns the tokio runtime and the API clients |
| `src/ops/` | Pure logic with unit tests: analysis (duplicates, sort, stats), files (import/export), matcher, recommend, generate |
| `src/player/` | 30-second preview playback (DASH, rodio) |
| `src/ui/` | egui UI: `mod.rs` (App, menus, event handling), `dialogs.rs`, `tracks.rs`, `search.rs`, `style.rs` (theme, fonts, emoji) |
| `src/ui/tests.rs` | Headless UI tests through the AccessKit tree |
| `src/i18n.rs` | Every UI string, English and Norwegian |
| `src/store.rs` | Config, token files, track cache |
| `tools/` | `fetch-assets.sh` (fonts, SHA-256 pinned), `gen-notice.py` (NOTICE), `gen-icons.sh` (icon PNG/ICO), `fetch-appimage.sh` (AppImage runtime, its source and licences, SHA-256 pinned, into `target/appimage/`) |
| `packaging/` | `.desktop` file; the app icon: `icon.svg` (our own design) and the `icon.png`/`icon.ico` rendered from it (committed; window icon, Linux package, Windows `.exe` resource via `windres` in `build.rs`) |
| `release.sh`, `check-stripped.sh` | Release build and the strip check |

## How the code works

- **The UI never blocks on the network.** It sends a `worker::Cmd` and handles `worker::Event`s.
  Long jobs carry a `Job` (id plus cancel flag) and report `Progress`/`Permille`,
  then `JobDone` (with an optional `Note`) or `JobFailed`. A job that opens a result dialog
  goes through `App::open_result`, which waits if the user is busy in another dialog.
- **Playlist writes** go through the worker's `write_lock`. Batches follow the API spec:
  50 items for add and remove, 20 for reorder (PATCH), 20 ids per `filter[id]`.
  After a write the worker reloads the playlist (`reload`, `refresh_meta`).
- **Undo:** call `App::snapshot` before a change; `add_to_playlist` and
  `remove_from_playlist` already do.
- **`include` fallbacks:** some include paths are rejected with HTTP 400 on some servers,
  so the client tries a list of variants (`get_all_with_includes`). Follow the same pattern
  for new relationship calls.
- **Dialogs** are entries in `Dialog` (in `dialogs.rs`), drawn by `dialogs::show`.
  Use `frame` and `ok_cancel`. Esc and Cancel always close a dialog. Move focus to a
  sensible first control when `first` is true.

### What the API gives a third-party app (checked against the live API)

- Playback: `/trackManifests` gives previews only (`FULL_REQUIRES_HIGHER_ACCESS_TIER`).
  The app plays previews and offers "Open in TIDAL"; do not try to get full streams.
- Genres: `/genres?filter[id]=USER_SELECTABLE` and the genre relationships of tracks and
  albums come back **empty**. The playlist generator therefore uses a built-in genre list
  (`ops/generate.rs`) and searches TIDAL for the genre name, which returns TIDAL's
  editorial playlists.
- Recommendations use `/tracks/{id}/relationships/similarTracks`.
- The OpenAPI spec is used as a local reference in `refs/` (gitignored). Download it from
  the TIDAL developer portal; do not commit it.

## Rules

- **Only our own files in the repository.** Fonts (`assets/fonts/`), API specs (`refs/`)
  and `NOTICE` are third-party or generated: fetched by `tools/fetch-assets.sh`,
  generated by `release.sh`, and gitignored. Do not add third-party files or vendored code.
  `scratchpad/` is for local scratch files and is never committed.
- **Every UI string goes in `src/i18n.rs`** with both English and Norwegian text.
  Placeholders are `{0}`, `{1}`, … filled by `i18n::fmt`. Norwegian puts a space before
  the ellipsis (`Finn duplikater …`) and uses «» quotes.
- **Accessibility is a requirement, not an extra.** Every control has an accessible name
  (an icon-only button sets one with `widget_info`), everything works from the keyboard,
  status is never shown only by colour or an icon (write it in text too), reduced motion
  follows the system, WCAG AA contrast, no time limits, and text from 50 % to 300 %
  without clipping. The UI tests check names through AccessKit; add a test for each new dialog.
- **Fonts and emoji** (`ui/style.rs`): Noto Sans, then DejaVu Sans; then the system's
  fonts on demand (`egui_system_font_provider`); emoji with emoji presentation are drawn
  in colour from the system emoji font by a priority glyph rasterizer; egui's
  black-and-white Noto Emoji is the last fallback.
- **Privacy:** requests use the generic User-Agent `tidal-playlist-editor/<version>`.
  Never put a user's name or email address in a request. Tokens are stored in a 0600 file.
- **Releases:** every shipped binary must be stripped. `release.sh` enforces this with
  `check-stripped.sh` (ELF: no `.symtab` or `.debug_*`; PE: no `.debug_*` and 0 COFF
  symbols) and checks that the Windows `.exe` needs no MinGW runtime DLLs. Do not
  weaken these checks. `NOTICE` (generated) must list every crate and bundled font with
  its licence text, and the MinGW-w64 runtime notice the Windows `.exe` links in.
- **AppImage:** the type2 runtime it starts with is third-party and links in libfuse
  (LGPL-2.1). `release.sh` checks the runtime is stripped, puts the runtime's licences
  in the AppImage's `NOTICE` (`gen-notice.py --appimage`), and ships its complete
  source (`appimage-runtime-source-<tag>.tar.gz`) next to the AppImage. Publish that
  file with every AppImage. Moving to another runtime tag means new pins in
  `tools/fetch-appimage.sh` and the libfuse version its build script uses.
- **Commits:** small, with a message explaining why. The repository identity is
  "TIDAL Playlist Editor" with the GitHub no-reply address.
