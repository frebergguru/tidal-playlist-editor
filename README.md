# TIDAL Playlist Editor

*Norsk tekst følger etter den engelske.*

A desktop editor (Linux and Windows) for your own TIDAL playlists, written in Rust with
an egui interface. It talks only to the official TIDAL API (`openapi.tidal.com/v2`) and logs in
with your own TIDAL account through your own developer app.

## Features

- **Playlists:** create, rename, edit the description and visibility, duplicate, merge, and delete them.
- **Songs:**
  - find, add and remove songs, and move them by drag-and-drop or Alt+↑/↓;
  - copy or move songs to other playlists;
  - sort (also by TIDAL's popularity) or shuffle a playlist permanently, or trim it to about a given length;
  - undo and redo.
- **Find new songs:** search the TIDAL catalogue for tracks, albums and artists, and add one song, a selection or a whole album to one or more playlists.
- **Recommendations:** Tools → "Recommend songs…" (or "Similar songs…" on selected rows) asks TIDAL for songs like the ones in the playlist, ranks them, leaves out songs you already have, and lets you preview and add the ones you pick.
- **Generate playlists:** Tools → "Generate playlists…": tick genres from the list or type your own (any genre, mood or style), choose how many songs, and get one mixed playlist or one per genre. The songs come from TIDAL's own playlists for each genre, leaving out songs already in your playlists. (TIDAL's genre data is not available to third-party apps, so the app searches by genre name.)
- **Unavailable songs:**
  - scan one or all playlists for songs TIDAL no longer offers in your country, with an optional thorough check of streaming rights;
  - remove them, or replace them with the same recording (ISRC) or the best search match.
  - The app remembers song names locally, so songs that later disappear can still be identified.
- **Videos:** videos are marked "🎬 Video" in the song list, and Tools → "Find videos…" lists every video in one or all playlists, with "Open in TIDAL". "Find music versions" looks up each video's song on TIDAL; you check the suggestions (with a preview), and the ticked videos are replaced by their song in the same place, or removed.
- **Sound quality:** Tools → "Upgrade sound quality…" finds other releases of the same recordings (same ISRC) in better quality — Hi-Res or lossless, optionally Dolby Atmos — and swaps the ticked songs in place. The quality is written next to each song.
- **Clean versions:** Tools → "Clean versions…" lists the explicit songs and looks for a clean version of each; ticked songs are swapped in place, or removed when there is none.
- **Radio:** right-click a song → "Make a playlist from this song's radio", or open an artist in the search panel → "Make a playlist from the artist's radio", saves TIDAL's radio mix as a new playlist that keeps the songs after the mix changes.
- **Duplicates:** find songs with the same TIDAL id, the same recording (ISRC), or the same artist and title. Tools → "Duplicates across playlists…" lists songs that are in more than one playlist, and right-click → "Find in all playlists…" shows every playlist that has a song; tick the copies to remove.
- **Export:** JSON (full detail), CSV, M3U8 and XSPF, for one, several or all playlists.
- **Backup and restore:** "Back up all playlists" writes a timestamped folder. A restore compares a backup with the current playlist, shows what differs, and puts the songs and their order back.
- **Import:**
  - reads our JSON, CSV (including exports from other services, for example Exportify), M3U/M3U8 and XSPF files;
  - songs are matched by TIDAL id, then ISRC, then artist and title;
  - you check every match before anything is written.
- **Playback:** previews of about 30 seconds inside the app; "Open in TIDAL" for full tracks.
- **Other:** add songs to My Collection, and statistics per playlist.
- **Emoji and other scripts:** emoji in playlist and song names are shown in colour with the system's emoji font (Noto Color Emoji, Segoe UI Emoji); characters the built-in fonts lack, such as Japanese or Arabic, come from the system's fonts.
- **Spotify ⇄ TIDAL:** copy playlists, including Liked Songs, from Spotify to TIDAL, or from TIDAL to Spotify.
  - Songs are matched by ISRC first, then by artist and title.
  - You review every match before anything is written.
  - Local files, podcast episodes and videos cannot be copied, and the app tells you how many were skipped.

### Why previews and not full tracks?

TIDAL's API decides what a third-party app may play: `trackManifests` returns `PREVIEW`
with the reason `FULL_REQUIRES_HIGHER_ACCESS_TIER` for normal developer apps. If TIDAL ever gives your app a higher
tier, the same code plays the full track (the player shows "full track" instead of "preview clip").
Tracks with DRM cannot be played here and open in the TIDAL app instead.

## Setting up (once)

1. Go to the [TIDAL developer portal](https://developer.tidal.com/dashboard) and log in.
2. Create an app.
3. Add the redirect URI `http://localhost:8765/callback` (it must match the setting in the app exactly).
4. Give the app these scopes:
   `user.read collection.read collection.write playlists.read playlists.write search.read playback recommendations.read`.
5. Start TIDAL Playlist Editor, open **API settings…**, paste the **Client ID** and save.
   A client secret is not needed (the app uses PKCE).
6. Press **Log in with TIDAL** and log in in the browser. The app continues by itself; there is no time limit.

## Spotify (optional)

1. Go to the [Spotify developer dashboard](https://developer.spotify.com/dashboard) and create an app. Choose **Web API**.
2. Add the redirect URI `http://127.0.0.1:8766/callback`. Spotify does not allow `localhost`; it must be `127.0.0.1`.
3. In TIDAL Playlist Editor, open **Settings**, paste the **Spotify Client ID** and save. No client secret is needed (PKCE).
4. Open the **Spotify** menu, choose **Connect Spotify account…** and log in in the browser.

The app asks Spotify only for these scopes: `playlist-read-private`, `playlist-read-collaborative`,
`playlist-modify-private`, `playlist-modify-public` and `user-library-read` (for Liked Songs).
Spotify data is used only for the copy in progress and is not stored. The Spotify login is kept in
`spotify-tokens.json` next to the TIDAL login, readable only by you.
This project is not made by or affiliated with Spotify.

## Where your data is stored

| | Linux | Windows |
|---|---|---|
| Settings (`config.toml`) and logins (`tokens.json`, Spotify tokens), all readable only by you | `~/.config/tidal-playlist-editor/` | `%APPDATA%\freberg\tidal-playlist-editor\config\` |
| Backups and song-name cache | `~/.local/share/tidal-playlist-editor/` | `%APPDATA%\freberg\tidal-playlist-editor\data\` |

Set `TIDAL_EDITOR_HOME` to keep everything in one folder instead (portable use).
The app sends nothing anywhere except to TIDAL, and to Spotify only when you connect a Spotify
account, always with a generic User-Agent.

## Accessibility

- Screen reader support through AccessKit (AT-SPI on Linux, UI Automation on Windows).
- Everything works with the keyboard. Help → Keyboard shortcuts, or F1, lists the keys.
- A strong focus ring shows where you are.
- Reduced motion follows the system setting.
- Status is never shown by colour or sound alone.
- Text colours meet WCAG AA contrast.
- There are no time limits.
- Text size runs from 50 % to 300 % and is remembered.
- The full statement is in the app under Help → Accessibility statement.

## Building

```sh
tools/fetch-assets.sh                      # fonts (not kept in the repo; from the system or upstream)
rustup toolchain install 1.99.1            # pinned in rust-toolchain.toml
cargo run --release                        # Linux
cargo test                                 # unit, API mock and headless UI tests
cargo test screenshots -- --ignored        # renders target/screens/*.png without a window
./release.sh                               # Linux + Windows packages in dist/<version>/
```

`release.sh` runs the tests and clippy, and builds both targets. It then checks with `check-stripped.sh` that every shipped binary is stripped, and stops the release if one is not.
Windows is cross-compiled with MinGW (`x86_64-pc-windows-gnu`).

## Reporting problems

Please report problems in writing, as an issue in this project's repository. Replies are given in writing.

## Licence

MIT, see `LICENSE`. Third-party components and their licences are listed in `NOTICE`, which
`release.sh` generates and ships with every release (run `python3 -I tools/gen-notice.py` to see it).
This project is not made by or affiliated with TIDAL.

---

# TIDAL spillelisteredigering (norsk)

Et skrivebordsprogram (Linux og Windows) for å redigere dine egne spillelister i TIDAL. Det er skrevet i Rust
med egui-grensesnitt. Programmet snakker bare med TIDALs offisielle API og logger inn med din egen
TIDAL-konto via din egen utviklerapp.

## Funksjoner

- **Spillelister:** opprett, gi nytt navn, endre beskrivelse og synlighet, dupliser, slå sammen og slett.
- **Sanger:**
  - finn, legg til og fjern sanger, og flytt dem med dra-og-slipp eller Alt+↑/↓;
  - kopier eller flytt sanger til andre spillelister;
  - sorter (også etter popularitet i TIDAL) eller stokk en spilleliste permanent, eller kort den ned til omtrent en gitt lengde;
  - angre og gjør om.
- **Finn nye sanger:** søk i TIDAL etter spor, album og artister, og legg til én sang, et utvalg eller et helt album i én eller flere spillelister.
- **Anbefalinger:** Verktøy → «Anbefal sanger …» (eller «Lignende sanger …» på valgte rader) ber TIDAL om sanger som ligner på dem i spillelisten, rangerer dem, utelater sanger du allerede har, og lar deg høre smakebiter og legge til dem du velger.
- **Lag spillelister:** Verktøy → «Lag spillelister …»: kryss av for sjangre i listen eller skriv inn din egen (sjanger, stemning eller stil), velg hvor mange sanger, og få én blandet spilleliste eller én per sjanger. Sangene hentes fra TIDALs egne spillelister for hver sjanger, uten sanger som allerede er i spillelistene dine. (TIDALs sjangerdata er ikke tilgjengelig for tredjepartsapper, så programmet søker på sjangernavnet.)
- **Utilgjengelige sanger:**
  - søk gjennom én eller alle spillelister etter sanger TIDAL ikke lenger tilbyr i landet ditt, med valgfri grundig sjekk av strømmerettigheter;
  - fjern dem, eller erstatt dem med samme innspilling (ISRC) eller beste søketreff.
  - Programmet husker sangnavn lokalt, slik at sanger som forsvinner senere, fortsatt kan kjennes igjen.
- **Videoer:** videoer er merket «🎬 Video» i sanglisten, og Verktøy → «Finn videoer …» viser alle videoene i én eller alle spillelister, med «Åpne i TIDAL». «Finn musikkversjoner» slår opp sangen til hver video i TIDAL; du sjekker forslagene (med smakebit), og de avkryssede videoene byttes ut med sangen på samme plass, eller fjernes.
- **Lydkvalitet:** Verktøy → «Oppgrader lydkvalitet …» finner andre utgivelser av de samme innspillingene (samme ISRC) i bedre kvalitet – Hi-Res eller tapsfri, eventuelt Dolby Atmos – og bytter ut de avkryssede sangene på samme plass. Kvaliteten står skrevet ved hver sang.
- **Ikke-eksplisitte versjoner:** Verktøy → «Ikke-eksplisitte versjoner …» viser de eksplisitte sangene og leter etter en ikke-eksplisitt versjon av hver; avkryssede sanger byttes ut på samme plass, eller fjernes hvis det ikke finnes noen.
- **Radio:** høyreklikk en sang → «Lag spilleliste av radioen til denne sangen», eller åpne en artist i søkepanelet → «Lag spilleliste av radioen til artisten», lagrer TIDAL-radiomiksen som en ny spilleliste som beholder sangene når miksen endrer seg.
- **Duplikater:** finn sanger med samme TIDAL-id, samme innspilling (ISRC) eller samme artist og tittel. Verktøy → «Duplikater på tvers av spillelister …» viser sanger som ligger i mer enn én spilleliste, og høyreklikk → «Finn i alle spillelister …» viser alle spillelistene som har en sang; kryss av kopiene som skal fjernes.
- **Eksport:** JSON (alle detaljer), CSV, M3U8 og XSPF, for én, flere eller alle spillelister.
- **Sikkerhetskopi og gjenoppretting:** «Sikkerhetskopier alle spillelister» lager en mappe med tidsstempel. Ved gjenoppretting sammenlignes sikkerhetskopien med spillelisten slik den er nå, du ser hva som er forskjellig, og sangene og rekkefølgen settes tilbake.
- **Import:**
  - leser vår JSON, CSV (også eksport fra andre tjenester, for eksempel Exportify), M3U/M3U8 og XSPF;
  - sanger finnes via TIDAL-id, så ISRC, så artist og tittel;
  - du sjekker hvert treff før noe skrives.
- **Avspilling:** smakebiter på omtrent 30 sekunder i programmet; «Åpne i TIDAL» for hele spor.
- **Annet:** legg sanger i Min samling, og statistikk per spilleliste.
- **Emoji og andre skriftsystemer:** emoji i navn på spillelister og sanger vises i farger med systemets emoji-skrift (Noto Color Emoji, Segoe UI Emoji); tegn de innebygde skriftene mangler, for eksempel japansk eller arabisk, hentes fra systemets skrifter.
- **Spotify ⇄ TIDAL:** kopier spillelister, også Likte sanger, fra Spotify til TIDAL eller fra TIDAL til Spotify.
  - Sangene finnes via ISRC først, så artist og tittel.
  - Du sjekker hvert treff før noe lagres.
  - Lokale filer, podkastepisoder og videoer kan ikke kopieres, og programmet sier hvor mange som ble hoppet over.

### Hvorfor smakebiter og ikke hele spor?

TIDALs API bestemmer hva en tredjepartsapp får spille: `trackManifests` gir `PREVIEW` med
begrunnelsen `FULL_REQUIRES_HIGHER_ACCESS_TIER` for vanlige utviklerapper. Om TIDAL en gang gir appen din
høyere tilgangsnivå, spiller den samme koden hele sporet. Spor med DRM kan ikke spilles her og åpnes i TIDAL-appen i stedet.

## Oppsett (én gang)

1. Gå til [TIDALs utviklerportal](https://developer.tidal.com/dashboard) og logg inn.
2. Lag en app.
3. Legg til omdirigerings-URI-en `http://localhost:8765/callback` (den må være lik innstillingen i programmet).
4. Gi appen disse tilgangene:
   `user.read collection.read collection.write playlists.read playlists.write search.read playback recommendations.read`.
5. Start programmet, åpne **API-innstillinger …**, lim inn **Client ID** og lagre. Du trenger ikke klienthemmelighet.
6. Trykk **Logg inn med TIDAL** og logg inn i nettleseren. Programmet fortsetter av seg selv; det er ingen tidsgrense.

## Spotify (valgfritt)

1. Gå til [Spotifys utviklerpanel](https://developer.spotify.com/dashboard) og lag en app. Velg **Web API**.
2. Legg til omdirigerings-URI-en `http://127.0.0.1:8766/callback`. Spotify tillater ikke `localhost`; det må være `127.0.0.1`.
3. I programmet: åpne **Innstillinger**, lim inn **Spotify Client ID** og lagre. Du trenger ikke klienthemmelighet.
4. Åpne **Spotify**-menyen, velg **Koble til Spotify-konto …** og logg inn i nettleseren.

Programmet ber bare om tilgangene `playlist-read-private`, `playlist-read-collaborative`, `playlist-modify-private`,
`playlist-modify-public` og `user-library-read` (for Likte sanger). Data fra Spotify brukes bare til kopieringen som pågår,
og lagres ikke. Prosjektet er ikke laget av eller tilknyttet Spotify.

## Hvor dataene lagres

Innstillinger og innlogginger (bare lesbare for deg) ligger i `~/.config/tidal-playlist-editor/` på Linux og i
`%APPDATA%\freberg\tidal-playlist-editor\config\` på Windows. Sikkerhetskopier og lageret med sangnavn ligger i
`~/.local/share/tidal-playlist-editor/` på Linux og i `%APPDATA%\freberg\tidal-playlist-editor\data\` på Windows.
Med miljøvariabelen `TIDAL_EDITOR_HOME` holdes alt i én mappe. Programmet sender ingenting noe sted unntatt til TIDAL,
og til Spotify bare når du kobler til en Spotify-konto, alltid med en generisk User-Agent.

## Tilgjengelighet

- Skjermleserstøtte via AccessKit.
- Alt kan brukes med tastaturet (F1 viser hurtigtastene).
- En tydelig fokusring viser hvor du er.
- Redusert bevegelse følger systemet.
- Ingenting vises bare med farge eller lyd.
- Tekstfargene oppfyller kontrastkravet i WCAG AA.
- Det er ingen tidsgrenser.
- Tekststørrelsen går fra 50 % til 300 % og huskes.
- Hele erklæringen står i programmet under Hjelp → Tilgjengelighetserklæring.

## Melde fra om feil

Meld gjerne fra om problemer skriftlig, som en sak i prosjektets repo. Svar gis skriftlig.

## Lisens

MIT, se `LICENSE`. Komponenter fra tredjeparter og lisensene deres står i `NOTICE`.
Prosjektet er ikke laget av eller tilknyttet TIDAL.
