# TODO

Ideas and open work for TIDAL Playlist Editor, roughly in order of value.
API paths are from the TIDAL OpenAPI spec; the spec lists fields that turn out empty
for third-party apps (genres did), so check each one against the live API before building on it.

## Finish what exists

- [ ] **New release.** The packages in `dist/0.1.0/` predate recommendations, the genre
      generator, colour emoji and Find videos. Run `release.sh`.
- [ ] **Windows test.** Colour emoji (Segoe UI Emoji, COLR) and the system font lookup
      (`egui_system_font_provider`, DirectWrite) compile for Windows but have not been seen running there.
- [ ] **Screen reader check of the running app** (AT-SPI on Linux). The headless tests
      cover names through AccessKit, but the live window has not been checked.
- [ ] **Live test of writing features:** generate a playlist, swap videos for songs,
      add recommendations, on a real account.

## New features

- [ ] **Upgrade sound quality:** find songs that have a better version (HiRes, lossless,
      Dolby Atmos; `mediaTags` on tracks) and swap them in place. Same flow as
      Find videos / Find replacements: find, review with preview, replace in place.
- [ ] **Clean versions:** list explicit songs (`explicit`), and swap them for the clean
      version or remove them.
- [ ] **Save a mix or radio as a playlist:** copy Daily / Discovery / New release mixes
      (`/userDailyMixes/{id}/relationships/items` and friends) before they change, and make
      a playlist from a song's or artist's radio (`/tracks/{id}/relationships/radio`,
      `/artists/{id}/relationships/radio`).
- [ ] **Playlist folders:** show and organise playlists in folders
      (`/userCollectionFolders`, `/folders`, `/folderItems`).
- [ ] **Playlist cover image:** set a cover from the app (`/playlists/{id}/relationships/coverArt`,
      `/artworks`).
- [ ] **Where is this song?** Show every playlist that contains a song, and find duplicates
      across playlists, not only within one.

## Sorting and filtering (check the data first)

- [ ] Sort or build playlists by **tempo** (`bpm`), **musical key** (`key`, `keyScale`) or
      **popularity** (`popularity`). Check with a real login that these are filled in for
      third-party apps before building on them.

## Smaller things

- [ ] **Trim to a length:** for example "make this playlist about 1 hour".
- [ ] **Song and video counts in the sidebar** (`numberOfTrackItems`, `numberOfVideoItems`).

## Known limitations

- A live or unplugged video gets the studio recording as its music version: TIDAL keeps
  "MTV Unplugged" and the like in the video's `version`, which the search does not use.
- Emoji written in text style (a plain ❤ without U+FE0F) stay black and white, as Unicode intends.
