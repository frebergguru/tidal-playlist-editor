# TODO

Ideas and open work for TIDAL Playlist Editor, roughly in order of value.
API paths are from the TIDAL OpenAPI spec; the spec lists fields that turn out empty
for third-party apps (genres did), so check each one against the live API before building on it.

## Finish what exists

- [ ] **New release.** The packages in `dist/0.1.0/` predate recommendations, the genre
      generator, colour emoji, Find videos, sound quality, clean versions, radio, songs across
      playlists and trim. Run `release.sh`.
- [ ] **Windows test.** Colour emoji (Segoe UI Emoji, COLR) and the system font lookup
      (`egui_system_font_provider`, DirectWrite) compile for Windows but have not been seen running there.
- [ ] **Screen reader check of the running app** (AT-SPI on Linux). The headless tests
      cover names through AccessKit, but the live window has not been checked.
- [ ] **Live test of writing features:** generate a playlist, swap videos for songs,
      add recommendations, upgrade sound quality, swap clean versions, save a radio,
      remove copies across playlists, trim, on a real account.

## New features

- [ ] **Playlist folders** (`/userCollectionFolders`, `/folders`, `/folderItems`). Checked live:
      there is no way to list a user's folders (`/userCollectionFolders` needs `filter[id]`,
      `/userCollectionFolders/me` is 404, and `/userCollectionPlaylists/me` lists only playlists).
      Needs an account with a folder to see where folder ids show up.
- [ ] **Playlist cover image.** Flow in the spec: `POST /artworks` with `mediaType: IMAGE` and
      `sourceFile {md5Hash, size ≤ 20 MB}` → upload the bytes to `sourceFile.uploadLink`
      (its own method and headers; a presigned link, so no bearer token) → `PATCH
      /playlists/{id}/relationships/coverArt` with `[{type: artworks, id}]` (empty clears it).
      `POST /artworks` lists the `w_usr` scope, which the app does not ask for today. Needs an MD5
      crate and a live write test before building.
- [ ] **Save Daily / Discovery / New release mixes.** Not possible now: they need the
      `recommendations.read` scope, and there is no endpoint that lists a user's mix ids.
      (Song and artist radio are done.)

## Done

- Upgrade sound quality (same ISRC; Hi-Res, lossless, optionally Dolby Atmos).
  Note: `/tracks?filter[isrc]=` with several ISRCs returns only one track per ISRC, so the
  lookup is one ISRC per request.
- Clean versions of explicit songs (swap or remove).
- Song and artist radio saved as a playlist.
- Where is this song, and duplicates across playlists.
- Trim to a length; popularity sort; song and video counts in the sidebar.
- Checked live: `bpm`, `key`, `keyScale` and `toneTags` are not returned at all for
  third-party apps, so sorting by tempo or key is not possible. `popularity` is filled in.

## Known limitations

- A live or unplugged video gets the studio recording as its music version: TIDAL keeps
  "MTV Unplugged" and the like in the video's `version`, which the search does not use.
- Emoji written in text style (a plain ❤ without U+FE0F) stay black and white, as Unicode intends.
