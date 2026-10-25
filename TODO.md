# TODO

Ideas and open work for TIDAL Playlist Editor, roughly in order of value.
API paths are from the TIDAL OpenAPI spec; the spec lists fields that turn out empty
for third-party apps (genres did), so check each one against the live API before building on it.

## Finish what exists

- [ ] **Windows test.** Colour emoji (Segoe UI Emoji, COLR) and the system font lookup
      (`egui_system_font_provider`, DirectWrite) compile for Windows but have not been seen running there.
- [ ] **Screen reader check of the running app** (AT-SPI on Linux). The headless tests
      cover names through AccessKit, but the live window has not been checked.

## Done

- Live test of every writing feature on a real account (`cargo test live_ -- --ignored`, see
  `src/worker.rs`): all passed, and the test playlist was restored exactly. It found that the
  genre generator picked up artists named after the genre (fixed).
- Release 0.2.0 built (`dist/0.2.0/`, stripped, tests and clippy clean).
- Upgrade sound quality (same ISRC; Hi-Res, lossless, optionally Dolby Atmos).
  Note: `/tracks?filter[isrc]=` with several ISRCs returns only one track per ISRC, so the
  lookup is one ISRC per request.
- Clean versions of explicit songs (swap or remove).
- Song and artist radio, and the user's own mixes (`/userDailyMixes/me`, `/userDiscoveryMixes/me`,
  `/userNewReleaseMixes/me` → `relationships/items`; needs `recommendations.read`), saved as playlists.
- Where is this song, and duplicates across playlists.
- Trim to a length; popularity sort; song and video counts in the sidebar.
- Playlist cover image: not possible. `POST /artworks` needs the `w_usr` scope, which the
  spec marks INTERNAL (not available to third-party apps).
- Checked live: `bpm`, `key`, `keyScale` and `toneTags` are not returned at all for
  third-party apps, so sorting by tempo or key is not possible. `popularity` is filled in.

## Known limitations

- A live or unplugged video gets the studio recording as its music version: TIDAL keeps
  "MTV Unplugged" and the like in the video's `version`, which the search does not use.
- Emoji written in text style (a plain ❤ without U+FE0F) stay black and white, as Unicode intends.
