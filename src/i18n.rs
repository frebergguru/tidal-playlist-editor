//! User interface strings in English and Norwegian (bokmål).
//! Placeholders are `{0}`, `{1}`, … and are filled with [`fmt`].

use crate::api::AccessType;
use crate::ops::analysis::SortKey;
use crate::store::Language;

macro_rules! strings {
    ($($key:ident => $en:expr, $nb:expr;)*) => {
        pub struct Tr { $(pub $key: &'static str,)* }
        pub static EN: Tr = Tr { $($key: $en,)* };
        pub static NB: Tr = Tr { $($key: $nb,)* };
    };
}

pub fn for_language(lang: Language) -> &'static Tr {
    match lang {
        Language::English => &EN,
        Language::Norwegian => &NB,
        Language::Auto => {
            let loc = sys_locale::get_locale().unwrap_or_default().to_lowercase();
            if loc.starts_with("nb") || loc.starts_with("nn") || loc.starts_with("no") { &NB } else { &EN }
        }
    }
}

static CURRENT_NB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Sets the language background threads use for their messages (errors).
pub fn set_current(tr: &'static Tr) {
    CURRENT_NB.store(std::ptr::eq(tr, &NB), std::sync::atomic::Ordering::Relaxed);
}

/// The language last chosen in the UI.
pub fn current() -> &'static Tr {
    if CURRENT_NB.load(std::sync::atomic::Ordering::Relaxed) { &NB } else { &EN }
}

pub fn fmt(template: &str, args: &[&str]) -> String {
    let mut s = template.to_owned();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

pub fn access_label(tr: &Tr, a: AccessType) -> &'static str {
    match a {
        AccessType::Public => tr.access_public,
        AccessType::Unlisted | AccessType::Private => tr.access_unlisted,
    }
}

pub fn sort_label(tr: &Tr, k: SortKey) -> &'static str {
    match k {
        SortKey::Title => tr.col_title,
        SortKey::Artist => tr.col_artist,
        SortKey::Album => tr.col_album,
        SortKey::Duration => tr.col_time,
        SortKey::Added => tr.col_added,
    }
}

strings! {
    app_title => "TIDAL Playlist Editor", "TIDAL spillelisteredigering";
    welcome_text => "Edit your TIDAL playlists: import and export them, add, find and remove songs, find new music, duplicate playlists, clean out songs that are no longer available, and play previews.",
        "Rediger spillelistene dine i TIDAL: importer og eksporter dem, legg til, finn og fjern sanger, finn ny musikk, dupliser spillelister, rydd bort sanger som ikke lenger er tilgjengelige, og spill av smakebiter.";
    checking_session => "Checking saved login…", "Sjekker lagret innlogging …";
    waiting_for_browser => "Log in on the TIDAL page that opened in your web browser. This window continues by itself when you are done. There is no time limit.",
        "Logg inn på TIDAL-siden som ble åpnet i nettleseren. Dette vinduet fortsetter av seg selv når du er ferdig. Det er ingen tidsgrense.";
    open_login_page_again => "Open login page again", "Åpne innloggingssiden igjen";
    copy_login_link => "Copy login link", "Kopier innloggingslenke";
    link_copied => "Link copied to the clipboard.", "Lenken er kopiert til utklippstavlen.";
    need_client_id => "Before you can log in, enter the Client ID of your own TIDAL developer app in API settings.",
        "Før du kan logge inn, må du skrive inn Client ID for din egen TIDAL-utviklerapp under API-innstillinger.";
    log_in_with_tidal => "Log in with TIDAL", "Logg inn med TIDAL";
    api_settings_dots => "API settings…", "API-innstillinger …";
    how_to_get_client_id => "How do I get a Client ID?", "Hvordan får jeg en Client ID?";
    client_id_steps => "1. Open the TIDAL developer portal and log in with your TIDAL account.\n2. Create a new app (any name).\n3. Add this redirect URI to the app exactly as written: {0}\n4. Give the app these scopes: user.read, collection.read, collection.write, playlists.read, playlists.write, search.read, playback.\n5. Copy the Client ID into API settings here and save. A client secret is not needed.",
        "1. Åpne TIDALs utviklerportal og logg inn med TIDAL-kontoen din.\n2. Lag en ny app (valgfritt navn).\n3. Legg til denne omdirigerings-URI-en i appen, nøyaktig slik: {0}\n4. Gi appen disse tilgangene (scopes): user.read, collection.read, collection.write, playlists.read, playlists.write, search.read, playback.\n5. Kopier Client ID inn i API-innstillinger her og lagre. Du trenger ikke klienthemmelighet.";
    developer_portal => "TIDAL developer portal", "TIDALs utviklerportal";
    logged_in_as => "Logged in as {0}.", "Logget inn som {0}.";
    signed_in_as_short => "{0} ({1})", "{0} ({1})";
    login_failed => "Login failed", "Innloggingen mislyktes";

    menu_file => "File", "Fil";
    menu_playlist => "Playlist", "Spilleliste";
    menu_tools => "Tools", "Verktøy";
    menu_view => "View", "Vis";
    menu_spotify => "Spotify", "Spotify";
    spotify_connect => "Connect Spotify", "Koble til Spotify";
    spotify_connect_dots => "Connect Spotify account…", "Koble til Spotify-konto …";
    spotify_disconnect => "Disconnect Spotify", "Koble fra Spotify";
    spotify_connected_as => "Spotify: connected as {0}", "Spotify: tilkoblet som {0}";
    spotify_waiting => "Log in to Spotify on the page that opened in your web browser, and allow access. This window continues by itself; there is no time limit.",
        "Logg inn på Spotify på siden som ble åpnet i nettleseren, og gi tilgang. Dette vinduet fortsetter av seg selv; det er ingen tidsgrense.";
    spotify_need_client => "Enter the Client ID of your Spotify developer app in Settings first.", "Skriv først inn Client ID for Spotify-utviklerappen din i Innstillinger.";
    spotify_expired => "The Spotify login has expired or was withdrawn. Connect Spotify again from the Spotify menu.",
        "Spotify-innloggingen er utløpt eller trukket tilbake. Koble til Spotify igjen fra Spotify-menyen.";
    spotify_login_failed => "Spotify login failed", "Spotify-innloggingen mislyktes";
    from_spotify_dots => "Copy playlists from Spotify to TIDAL…", "Kopier spillelister fra Spotify til TIDAL …";
    to_spotify_dots => "Copy playlists from TIDAL to Spotify…", "Kopier spillelister fra TIDAL til Spotify …";
    from_spotify => "Copy from Spotify to TIDAL", "Kopier fra Spotify til TIDAL";
    to_spotify => "Copy from TIDAL to Spotify", "Kopier fra TIDAL til Spotify";
    from_spotify_explain => "Choose Spotify playlists. Each song is looked up on TIDAL (same recording by ISRC first, then artist and title), and you check the result before anything is saved in TIDAL.",
        "Velg Spotify-spillelister. Hver sang slås opp i TIDAL (samme innspilling via ISRC først, så artist og tittel), og du sjekker resultatet før noe lagres i TIDAL.";
    to_spotify_explain => "Choose TIDAL playlists. Each song is looked up on Spotify (same recording by ISRC first, then artist and title), and you check the result before anything is saved in Spotify. Videos cannot be copied.",
        "Velg TIDAL-spillelister. Hver sang slås opp i Spotify (samme innspilling via ISRC først, så artist og tittel), og du sjekker resultatet før noe lagres i Spotify. Videoer kan ikke kopieres.";
    spotify_attribution => "Playlist and song information from Spotify.", "Informasjon om spillelister og sanger fra Spotify.";
    open_in_spotify => "Open in Spotify", "Åpne i Spotify";
    copy_n_to_spotify => "Copy {0} songs to Spotify", "Kopier {0} sanger til Spotify";
    liked_songs => "Liked Songs", "Likte sanger";
    find_matches => "Find matches", "Finn treff";
    reading_spotify => "Reading Spotify playlists and finding the songs on TIDAL", "Leser Spotify-spillelister og finner sangene i TIDAL";
    matching_on_spotify => "Finding the songs on Spotify", "Finner sangene i Spotify";
    review_to_spotify => "Check the copy to Spotify", "Sjekk kopien til Spotify";
    public_on_spotify => "Public on Spotify", "Offentlig på Spotify";
    copying_to_spotify => "Copying to Spotify: {0}", "Kopierer til Spotify: {0}";
    spotify_written => "Copied {0} songs to the Spotify playlist «{1}».", "Kopierte {0} sanger til Spotify-spillelisten «{1}».";
    skipped_n => "{0} items could not be copied (local files, podcast episodes, videos or songs without information).",
        "{0} elementer kunne ikke kopieres (lokale filer, podkastepisoder, videoer eller sanger uten informasjon).";
    spotify_setup_steps => "To copy playlists to and from Spotify: create an app at the Spotify developer dashboard, choose «Web API», add the redirect URI below exactly as written, and paste the app's Client ID here. A client secret is not needed.",
        "For å kopiere spillelister til og fra Spotify: lag en app i Spotifys utviklerpanel, velg «Web API», legg til omdirigerings-URI-en under nøyaktig slik den står, og lim inn appens Client ID her. Du trenger ikke klienthemmelighet.";
    spotify_client_id => "Spotify Client ID", "Spotify Client ID";
    spotify_redirect_uri => "Spotify redirect URI", "Omdirigerings-URI for Spotify";
    spotify_dashboard => "Spotify developer dashboard", "Spotifys utviklerpanel";
    spotify_redirect_invalid => "Spotify needs a redirect URI like http://127.0.0.1:8766/callback (127.0.0.1 with a port; «localhost» is not allowed).",
        "Spotify krever en omdirigerings-URI som http://127.0.0.1:8766/callback (127.0.0.1 med port; «localhost» er ikke tillatt).";
    menu_help => "Help", "Hjelp";
    import_dots => "Import…", "Importer …";
    export_dots => "Export…", "Eksporter …";
    backup_all => "Back up all playlists", "Sikkerhetskopier alle spillelister";
    restore_backup_dots => "Restore from backup…", "Gjenopprett fra sikkerhetskopi …";
    settings_dots => "Settings…", "Innstillinger …";
    settings => "Settings", "Innstillinger";
    log_out => "Log out", "Logg ut";
    quit => "Quit", "Avslutt";
    new_playlist_dots => "New playlist…", "Ny spilleliste …";
    new_playlist => "New playlist", "Ny spilleliste";
    new_short => "New", "Ny";
    edit_details_dots => "Edit name and description…", "Endre navn og beskrivelse …";
    edit_details => "Edit playlist", "Endre spilleliste";
    duplicate_dots => "Duplicate…", "Dupliser …";
    duplicate => "Duplicate", "Dupliser";
    duplicate_playlist => "Duplicate playlist", "Dupliser spilleliste";
    duplicate_explain => "Makes a new playlist with the same songs, in the same order, as «{0}».", "Lager en ny spilleliste med de samme sangene, i samme rekkefølge, som «{0}».";
    duplicating => "Duplicating {0}", "Dupliserer {0}";
    copy_of => "{0} (copy)", "{0} (kopi)";
    merge_dots => "Merge playlists…", "Slå sammen spillelister …";
    merge => "Merge", "Slå sammen";
    merge_playlists => "Merge playlists", "Slå sammen spillelister";
    merge_explain => "Choose two or more playlists. Their songs are put into a new playlist in the order shown.", "Velg to eller flere spillelister. Sangene legges i en ny spilleliste i rekkefølgen som vises.";
    merged_playlist => "Merged playlist", "Sammenslått spilleliste";
    merging_n => "Merging {0} playlists", "Slår sammen {0} spillelister";
    sort_permanently_dots => "Sort permanently…", "Sorter permanent …";
    sort_permanently => "Sort permanently", "Sorter permanent";
    sort_explain => "Changes the saved order of the playlist in TIDAL. You can undo it afterwards.", "Endrer den lagrede rekkefølgen på spillelisten i TIDAL. Du kan angre etterpå.";
    sort => "Sort", "Sorter";
    ascending => "Ascending (A–Z, shortest first, oldest first)", "Stigende (A–Å, korteste først, eldste først)";
    descending => "Descending", "Synkende";
    shuffle_order => "Shuffle order permanently", "Stokk rekkefølgen permanent";
    statistics => "Statistics", "Statistikk";
    open_in_tidal => "Open in TIDAL", "Åpne i TIDAL";
    refresh => "Refresh", "Oppdater";
    delete_playlist_dots => "Delete playlist…", "Slett spilleliste …";
    delete_playlist => "Delete playlist", "Slett spilleliste";
    confirm_delete => "Delete the playlist «{0}» from your TIDAL account?", "Vil du slette spillelisten «{0}» fra TIDAL-kontoen din?";
    delete_cannot_undo => "This cannot be undone. Tip: export or back up the playlist first.", "Dette kan ikke angres. Tips: eksporter eller sikkerhetskopier spillelisten først.";
    delete => "Delete", "Slett";
    find_unavailable_dots => "Find unavailable songs…", "Finn utilgjengelige sanger …";
    find_unavailable => "Find unavailable songs", "Finn utilgjengelige sanger";
    find_duplicates_dots => "Find duplicates…", "Finn duplikater …";
    recommend_songs_dots => "Recommend songs…", "Anbefal sanger …";
    similar_songs_dots => "Similar songs…", "Lignende sanger …";
    recommended_songs => "Recommended songs", "Anbefalte sanger";
    recommend_from_playlist => "Songs like those in «{0}», found by TIDAL. Tick the ones you want and add them.", "Sanger som ligner på dem i «{0}», funnet av TIDAL. Kryss av for dem du vil ha, og legg dem til.";
    recommend_from_songs => "Songs like the {0} chosen songs in «{1}», found by TIDAL. Tick the ones you want and add them.", "Sanger som ligner på de {0} valgte sangene i «{1}», funnet av TIDAL. Kryss av for dem du vil ha, og legg dem til.";
    recommend_not_owned => "This playlist belongs to someone else, so songs cannot be added to it here.", "Denne spillelisten tilhører noen andre, så sanger kan ikke legges til her.";
    find_more => "Find more", "Finn flere";
    no_recommendations => "TIDAL had no new suggestions. Try «Find more», or pick other songs.", "TIDAL hadde ingen nye forslag. Prøv «Finn flere», eller velg andre sanger.";
    suggested_by_n => "suggested by {0} of your songs", "foreslått av {0} av sangene dine";
    add_n_to_playlist => "Add {0} to the playlist", "Legg {0} til i spillelisten";
    no_seed_songs => "There are no playable songs to base suggestions on.", "Det finnes ingen spillbare sanger å basere forslag på.";
    finding_recommendations => "Finding recommendations", "Finner anbefalinger";
    generate_playlists_dots => "Generate playlists…", "Lag spillelister …";
    generate_playlists => "Generate playlists", "Lag spillelister";
    generate_explain => "Tick one or more genres, or type your own, and choose how many songs you want. The app searches TIDAL's playlists for each genre and makes new playlists. Nothing is changed in your other playlists.", "Kryss av for én eller flere sjangre, eller skriv inn din egen, og velg hvor mange sanger du vil ha. Programmet søker i TIDALs spillelister etter hver sjanger og lager nye spillelister. Ingenting endres i de andre spillelistene dine.";
    genres_picked_n => "Genres ({0} chosen)", "Sjangre ({0} valgt)";
    filter_genres => "Filter genres", "Filtrer sjangre";
    other_genre => "Other genre (type any genre, mood or style)", "Annen sjanger (skriv en hvilken som helst sjanger, stemning eller stil)";
    add_genre => "Add genre", "Legg til sjanger";
    number_of_songs => "Number of songs", "Antall sanger";
    songs_per_playlist => "Songs per playlist", "Sanger per spilleliste";
    one_mixed_playlist => "One playlist with all the genres", "Én spilleliste med alle sjangrene";
    one_playlist_per_genre => "One playlist per genre", "Én spilleliste per sjanger";
    generated_name => "New music: {0}", "Ny musikk: {0}";
    genre_placeholder => "genre", "sjanger";
    name_optional => "Name (leave empty to name it after the genres)", "Navn (la stå tomt for å bruke sjangrene)";
    skip_known_songs => "Leave out songs that are already in my playlists", "Utelat sanger som allerede er i spillelistene mine";
    generate => "Generate", "Lag";
    generating => "Generating playlists: {0}", "Lager spillelister: {0}";
    generated_description => "Generated from the genres {0}.", "Laget fra sjangrene {0}.";
    generated_none => "No new songs were found for those genres.", "Fant ingen nye sanger i de sjangrene.";
    generated_n => "Made {0} playlist(s) with {1} songs.", "Laget {0} spilleliste(r) med {1} sanger.";
    generated_fewer => "TIDAL had fewer new songs than the {0} asked for.", "TIDAL hadde færre nye sanger enn de {0} du ba om.";
    find_duplicates => "Find duplicates", "Finn duplikater";
    show_search_panel => "Show search panel (Ctrl+K)", "Vis søkepanel (Ctrl+K)";
    text_bigger => "Larger text (Ctrl +)", "Større tekst (Ctrl +)";
    text_smaller => "Smaller text (Ctrl −)", "Mindre tekst (Ctrl −)";
    text_reset => "Normal text size (Ctrl 0)", "Normal tekststørrelse (Ctrl 0)";
    show_log => "Show message log", "Vis meldingslogg";
    keyboard_shortcuts => "Keyboard shortcuts", "Hurtigtaster";
    accessibility_statement => "Accessibility statement", "Tilgjengelighetserklæring";
    about => "About", "Om";

    playlists => "Playlists", "Spillelister";
    filter_playlists => "Filter playlists (Ctrl+G)", "Filtrer spillelister (Ctrl+G)";
    type_to_filter => "Type to filter", "Skriv for å filtrere";
    loading => "Loading…", "Laster …";
    no_playlists_yet => "You have no playlists yet. Make one with «New».", "Du har ingen spillelister ennå. Lag en med «Ny».";
    n_items => "{0} items", "{0} elementer";
    collaborator => "shared with you", "delt med deg";
    select_a_playlist => "Choose a playlist on the left.", "Velg en spilleliste til venstre.";
    n_unavailable => "{0} unavailable", "{0} utilgjengelige";
    find_in_playlist => "Find in playlist (Ctrl+F)", "Finn i spillelisten (Ctrl+F)";
    find_hint => "Title, artist, album or ISRC", "Tittel, artist, album eller ISRC";
    clear => "Clear", "Tøm";
    view_order => "Show in order", "Vis i rekkefølge";
    order_playlist => "Playlist order", "Spillelistens rekkefølge";
    n_selected => "{0} selected", "{0} valgt";
    play_preview => "Play preview", "Spill smakebit";
    remove => "Remove", "Fjern";
    key_delete => "Delete key", "Delete-tasten";
    more_actions => "More ▾", "Mer ▾";
    move_up => "Move up", "Flytt opp";
    move_down => "Move down", "Flytt ned";
    copy_to_dots => "Copy to…", "Kopier til …";
    move_to_dots => "Move to…", "Flytt til …";
    move_to => "Move to playlists", "Flytt til spillelister";
    add_to => "Add to playlists", "Legg til i spillelister";
    add_to_dots => "Add to…", "Legg til i …";
    add_to_favourites => "Add to My Collection", "Legg til i Min samling";
    select_all => "Select all", "Velg alle";
    select_none => "Select none", "Velg ingen";
    select => "Select", "Velg";
    undo => "Undo", "Angre";
    redo => "Redo", "Gjør om";
    playlist_empty => "This playlist is empty. Find songs in the search panel and add them.", "Spillelisten er tom. Finn sanger i søkepanelet og legg dem til.";
    reorder_disabled_hint => "Moving songs is turned off while the list is filtered or shown in another order.", "Flytting av sanger er slått av mens listen er filtrert eller vist i en annen rekkefølge.";
    no_matches => "No songs match.", "Ingen sanger passer.";
    col_title => "Title", "Tittel";
    col_artist => "Artist", "Artist";
    col_album => "Album", "Album";
    col_time => "Length", "Lengde";
    col_added => "Added", "Lagt til";
    col_status => "Status", "Status";
    row => "Row", "Rad";
    unknown_item => "Unknown item (TIDAL id {0})", "Ukjent element (TIDAL-id {0})";
    status_ok => "Available", "Tilgjengelig";
    status_unavailable => "Unavailable", "Utilgjengelig";
    status_replaced => "Replaced by TIDAL", "Erstattet av TIDAL";
    drag_to_reorder => "Drag to move. With the keyboard: Alt+Up or Alt+Down.", "Dra for å flytte. Med tastaturet: Alt+Opp eller Alt+Ned.";
    copy_link => "Copy link", "Kopier lenke";
    cannot_play_unavailable => "This item is not available, so it cannot be played.", "Elementet er ikke tilgjengelig og kan ikke spilles av.";

    find_new_songs => "Find new songs", "Finn nye sanger";
    search_tidal => "Search TIDAL (Ctrl+K)", "Søk i TIDAL (Ctrl+K)";
    search_hint => "Artist, song or album", "Artist, sang eller album";
    search => "Search", "Søk";
    searching => "Searching…", "Søker …";
    adds_go_to => "«Add» puts songs into: {0}", "«Legg til» legger sanger i: {0}";
    select_playlist_to_add => "Choose a playlist on the left to add songs to it.", "Velg en spilleliste til venstre for å legge sanger i den.";
    search_intro => "Type a search and press Enter.", "Skriv et søk og trykk Enter.";
    did_you_mean => "Did you mean «{0}»?", "Mente du «{0}»?";
    tracks => "Tracks", "Spor";
    albums => "Albums", "Album";
    artists => "Artists", "Artister";
    load_more => "Load more", "Last inn flere";
    open => "Open", "Åpne";
    open_artist => "Open {0}", "Åpne {0}";
    back => "Back", "Tilbake";
    top_tracks => "Popular tracks", "Populære spor";
    add => "Add", "Legg til";
    add_n_here => "Add {0} selected", "Legg til {0} valgte";
    add_all_here => "Add all", "Legg til alle";
    explicit => "Explicit", "Eksplisitt";
    already_in_playlist => "already in playlist", "finnes allerede i spillelisten";

    play => "Play", "Spill";
    pause => "Pause", "Pause";
    stop => "Stop", "Stopp";
    playing => "Playing", "Spiller";
    paused => "Paused", "Pause";
    stopped => "Stopped", "Stoppet";
    preview_clip => "preview clip", "smakebit";
    full_track => "full track", "hele sporet";
    nothing_playing => "Nothing is playing. Double-click a song or press Ctrl+P to hear a preview.", "Ingenting spilles. Dobbeltklikk en sang eller trykk Ctrl+P for å høre en smakebit.";
    volume => "Volume", "Volum";
    audio_error => "Sound problem", "Lydproblem";
    loading_preview => "Loading preview: {0}", "Laster smakebit: {0}";
    preview_drm => "This track is copy protected (DRM) for this app. Use «Open in TIDAL» to hear it.", "Sporet er kopibeskyttet (DRM) for denne appen. Bruk «Åpne i TIDAL» for å høre det.";
    preview_geo => "This song cannot be played in your location (GEO_RESTRICTED). Tools → Find unavailable songs, with the playback check, finds all such songs.",
        "Sangen kan ikke spilles der du er (GEO_RESTRICTED). Verktøy → Finn utilgjengelige sanger, med avspillingssjekken, finner alle slike sanger.";
    preview_video => "Videos cannot be played here. Use «Open in TIDAL».", "Videoer kan ikke spilles av her. Bruk «Åpne i TIDAL».";
    preview_failed => "Could not play preview", "Kunne ikke spille smakebit";
    ready => "Ready.", "Klar.";
    error => "Error", "Feil";
    cancel => "Cancel", "Avbryt";
    cancelled => "cancelled", "avbrutt";
    err_network => "Network problem", "Nettverksproblem";
    err_not_logged_in => "Not logged in", "Ikke logget inn";
    err_login => "Login", "Innlogging";
    err_playback_check_refused => "TIDAL does not let this app check playback ({0}). Run the scan without the playback check.", "TIDAL lar ikke denne appen sjekke avspilling ({0}). Kjør søket uten avspillingssjekken.";
    err_no_stream => "No playable stream in the manifest", "Ingen spillbar strøm i manifestet";
    err_playlist_changed => "The playlist changed meanwhile; it has been reloaded. Please sort again.", "Spillelisten ble endret i mellomtiden og er lastet inn på nytt. Sorter igjen.";
    close => "Close", "Lukk";
    save => "Save", "Lagre";
    create => "Create", "Opprett";

    adding_to => "Adding {0} to {1}", "Legger {0} til i {1}";
    removing_n => "Removing {0}", "Fjerner {0}";
    moving_n => "Moving {0}", "Flytter {0}";
    results_waiting => "Results are ready. They open when you close this dialog.", "Resultatene er klare. De åpnes når du lukker denne dialogen.";
    added_n => "Added {0} items.", "La til {0} elementer.";
    added_n_skipped => "Added {0} items; skipped {1} already in the playlist.", "La til {0} elementer; hoppet over {1} som allerede var der.";
    removed_n => "Removed {0} items.", "Fjernet {0} elementer.";
    reordered => "New order saved.", "Ny rekkefølge er lagret.";
    created_playlist => "Created playlist «{0}».", "Opprettet spillelisten «{0}».";
    duplicated_as => "Duplicated as «{0}».", "Duplisert som «{0}».";
    merged_into => "Merged {0} items into «{1}».", "Slo sammen {0} elementer i «{1}».";
    playlist_saved => "Playlist saved.", "Spillelisten er lagret.";
    playlist_deleted => "Playlist deleted.", "Spillelisten er slettet.";
    restored => "Playlist restored.", "Spillelisten er gjenopprettet.";
    replaced_n => "Replaced {0} items.", "Erstattet {0} elementer.";
    favourited_n => "Added {0} to My Collection.", "La {0} til i Min samling.";
    unfavourited_n => "Removed {0} from My Collection.", "Fjernet {0} fra Min samling.";
    could_not_load_items => "Could not load the playlist", "Kunne ikke laste spillelisten";
    could_not_open => "Could not open link", "Kunne ikke åpne lenken";
    creating => "Creating «{0}»", "Oppretter «{0}»";

    name => "Name", "Navn";
    new_name => "Name of the new playlist", "Navn på den nye spillelisten";
    description => "Description", "Beskrivelse";
    visibility => "Who can see it", "Hvem kan se den";
    access_unlisted => "Only people with the link", "Bare de som har lenken";
    access_public => "Public", "Offentlig";
    will_add_n => "{0} songs will be added.", "{0} sanger blir lagt til.";
    skip_duplicates => "Leave out duplicates", "Utelat duplikater";
    skip_already_present => "Skip songs that are already in the playlist", "Hopp over sanger som allerede er i spillelisten";
    n_tracks_chosen => "{0} songs chosen.", "{0} sanger valgt.";
    or_new_playlist_named => "…or make a new playlist named", "… eller lag en ny spilleliste med navnet";

    stat_items => "Items", "Elementer";
    stat_videos => "Videos", "Videoer";
    stat_total_time => "Total length", "Total lengde";
    stat_duplicates => "Duplicates (same id)", "Duplikater (samme id)";
    top_artists => "Most frequent artists", "Hyppigste artister";

    unavailable_explain => "Looks for songs that TIDAL no longer offers in your country. Nothing is removed until you confirm.", "Ser etter sanger som TIDAL ikke lenger tilbyr i landet ditt. Ingenting fjernes før du bekrefter.";
    only_playlist => "Only «{0}»", "Bare «{0}»";
    all_my_playlists => "All my playlists ({0})", "Alle spillelistene mine ({0})";
    strict_check => "Thorough check (also asks TIDAL about streaming rights; slower)", "Grundig sjekk (spør også TIDAL om strømmerettigheter; tregere)";
    start_scan => "Start", "Start";
    scanning => "Looking for unavailable songs", "Ser etter utilgjengelige sanger";
    unavailable_songs => "Unavailable songs", "Utilgjengelige sanger";
    no_unavailable => "Good news: no unavailable songs were found.", "Gode nyheter: fant ingen utilgjengelige sanger.";
    found_n_unavailable => "Found {0} unavailable items. Ticked items are removed (or replaced, if you choose a replacement).", "Fant {0} utilgjengelige elementer. Avkryssede elementer fjernes (eller erstattes, om du velger en erstatning).";
    find_replacements => "Find replacements", "Finn erstatninger";
    replace_with => "Replace with", "Erstatt med";
    no_replacement_found => "No replacement found.", "Fant ingen erstatning.";
    apply_to_n => "Remove or replace {0}", "Fjern eller erstatt {0}";
    replacing_n => "Replacing {0}", "Erstatter {0}";
    reason_missing => "not offered by TIDAL any more", "tilbys ikke lenger av TIDAL";
    reason_not_streamable => "cannot be streamed", "kan ikke strømmes";
    geo_check => "Also check playback in your location — finds «Content is unavailable in your location (GEO_RESTRICTED)». Slow: one request per song.",
        "Sjekk også avspilling der du er — finner «Content is unavailable in your location (GEO_RESTRICTED)». Tregt: én forespørsel per sang.";
    reason_geo => "not playable in your location (GEO_RESTRICTED)", "kan ikke spilles der du er (GEO_RESTRICTED)";
    remove_all_geo => "Remove all GEO_RESTRICTED ({0})", "Fjern alle GEO_RESTRICTED ({0})";
    confirm_remove_geo => "Remove {0} GEO_RESTRICTED songs from {1} playlists? Ctrl+Z in each playlist undoes it.",
        "Fjerne {0} GEO_RESTRICTED-sanger fra {1} spillelister? Ctrl+Z i hver spilleliste angrer.";
    yes_remove => "Yes, remove", "Ja, fjern";
    no => "No", "Nei";
    removed_geo_n => "Removing {0} GEO_RESTRICTED songs from {1} playlists.", "Fjerner {0} GEO_RESTRICTED-sanger fra {1} spillelister.";
    reason_no_rule => "no streaming rights in your country", "ingen strømmerettigheter i landet ditt";

    compare_by => "Count as duplicates:", "Regn som duplikater:";
    dup_exact => "same TIDAL id", "samme TIDAL-id";
    dup_isrc => "same recording (ISRC)", "samme innspilling (ISRC)";
    dup_loose => "same artist and title", "samme artist og tittel";
    no_duplicates => "No duplicates found.", "Fant ingen duplikater.";
    keep => "Keep", "Behold";
    remove_n => "Remove {0}", "Fjern {0}";

    import_review => "Check the import", "Sjekk importen";
    import_review_explain => "Each line shows what was found in TIDAL. Untick anything that is wrong, or choose another match. Uncertain matches start unticked.", "Hver linje viser hva som ble funnet i TIDAL. Fjern krysset for det som er feil, eller velg et annet treff. Usikre treff er ikke krysset av fra start.";
    matched_of => "{0} of {1} found", "{0} av {1} funnet";
    new_playlist_named => "New playlist", "Ny spilleliste";
    or_add_to_existing => "or add to:", "eller legg til i:";
    choose_dots => "Choose…", "Velg …";
    other_match => "Other match", "Annet treff";
    match_id => "TIDAL id", "TIDAL-id";
    match_isrc => "ISRC", "ISRC";
    match_search => "search", "søk";
    match_none => "not found", "ikke funnet";
    import_n => "Import {0} songs", "Importer {0} sanger";
    matching => "Looking up songs", "Slår opp sanger";
    matching_n => "Looking up {0} songs", "Slår opp {0} sanger";
    playlist_files => "Playlist files", "Spillelistefiler";
    could_not_read_file => "Could not read the file", "Kunne ikke lese filen";
    file_has_no_tracks => "The file contains no songs that can be read.", "Filen inneholder ingen sanger som kan leses.";

    export_playlists => "Export playlists", "Eksporter spillelister";
    format => "Format:", "Format:";
    fmt_json_help => "JSON keeps everything (also for backup and restore).", "JSON tar vare på alt (også for sikkerhetskopi og gjenoppretting).";
    fmt_csv_help => "CSV opens in spreadsheets such as LibreOffice Calc or Excel.", "CSV kan åpnes i regneark som LibreOffice Calc eller Excel.";
    fmt_m3u_help => "M3U8 is a plain playlist with TIDAL links, readable by many players.", "M3U8 er en enkel spilleliste med TIDAL-lenker som mange spillere kan lese.";
    fmt_xspf_help => "XSPF is an open XML playlist format.", "XSPF er et åpent XML-format for spillelister.";
    choose_folder_and_export => "Choose folder and export", "Velg mappe og eksporter";
    exporting_n => "Exporting {0} playlists", "Eksporterer {0} spillelister";
    exported_n_to => "Wrote {0} files to {1}", "Skrev {0} filer til {1}";
    export_failed => "Export failed", "Eksporten mislyktes";
    backing_up => "Backing up all playlists", "Sikkerhetskopierer alle spillelister";

    restore_backup => "Restore from backup", "Gjenopprett fra sikkerhetskopi";
    restore_explain => "Compare each playlist in the backup with a playlist in your account, then restore it or import it as a new playlist.", "Sammenlign hver spilleliste i sikkerhetskopien med en spilleliste i kontoen din, og gjenopprett den eller importer den som ny spilleliste.";
    compare_with => "Compare with:", "Sammenlign med:";
    identical => "Same songs in the same order.", "Samme sanger i samme rekkefølge.";
    diff_summary => "{0} songs are missing now, {1} songs are new since the backup.", "{0} sanger mangler nå, {1} sanger er nye siden sikkerhetskopien.";
    only_order_differs => "The order differs.", "Rekkefølgen er forskjellig.";
    show_differences => "Show differences", "Vis forskjeller";
    restore_into_this => "Restore into this playlist", "Gjenopprett i denne spillelisten";
    import_as_new => "Import as new playlist", "Importer som ny spilleliste";
    restoring => "Restoring playlist", "Gjenoppretter spilleliste";
    confirm_restore => "Replace the contents of «{0}» with this backup? Ctrl+Z in the playlist undoes it.", "Erstatte innholdet i «{0}» med denne sikkerhetskopien? Ctrl+Z i spillelisten angrer det.";
    yes_restore => "Yes, restore", "Ja, gjenopprett";
    restore_started => "Restore started", "Gjenoppretting startet";
    not_a_backup_file => "This is not a backup made by TIDAL Playlist Editor.", "Dette er ikke en sikkerhetskopi laget av TIDAL spillelisteredigering.";

    appearance => "Appearance", "Utseende";
    language => "Language", "Språk";
    lang_auto => "Same as the system", "Samme som systemet";
    theme => "Colours", "Farger";
    follow_system => "Follow the system", "Følg systemet";
    theme_dark => "Dark", "Mørk";
    theme_light => "Light", "Lys";
    animations => "Animations", "Animasjoner";
    motion_reduced => "Reduced (no animations)", "Redusert (ingen animasjoner)";
    motion_full => "Normal", "Normal";
    text_size => "Text size", "Tekststørrelse";
    behaviour => "Behaviour", "Oppførsel";
    open_links_in => "Open TIDAL links in:", "Åpne TIDAL-lenker i:";
    tidal_app => "TIDAL app", "TIDAL-appen";
    web_browser => "Web browser", "Nettleser";
    tidal_api => "TIDAL API", "TIDAL-API";
    api_explain => "These come from your own app in the TIDAL developer portal. They are stored only on this computer.", "Disse kommer fra din egen app i TIDALs utviklerportal. De lagres bare på denne datamaskinen.";
    client_id => "Client ID", "Client ID";
    client_secret_optional => "Client secret (usually not needed)", "Klienthemmelighet (trengs vanligvis ikke)";
    paste => "Paste", "Lim inn";
    paste_failed => "Could not read the clipboard", "Kunne ikke lese utklippstavlen";
    show_secret => "Show", "Vis";
    paste_hint => "Tip: Ctrl+V pastes into any text field.", "Tips: Ctrl+V limer inn i alle tekstfelt.";
    redirect_uri => "Redirect URI (must match the app exactly)", "Omdirigerings-URI (må være lik appens)";
    redirect_invalid => "The redirect URI must be a full address with a port, for example http://localhost:8765/callback", "Omdirigerings-URI-en må være en hel adresse med port, for eksempel http://localhost:8765/callback";
    api_changed_relogin => "API settings changed. Log out and in again to use them.", "API-innstillingene er endret. Logg ut og inn igjen for å bruke dem.";

    shortcuts_text => "Everywhere\n  Tab / Shift+Tab — next / previous control\n  Enter or Space — press the focused button or tick a box\n  Esc — close a dialog\n  Ctrl+F — find in the playlist\n  Ctrl+K — search TIDAL for new songs\n  Ctrl+G — filter the playlist list\n  Ctrl+N — new playlist\n  Ctrl+D — duplicate playlist\n  Ctrl+I — import, Ctrl+E — export\n  Ctrl+Z — undo, Ctrl+Y — redo\n  Ctrl+Space — pause or resume playback\n  Ctrl + / Ctrl − / Ctrl 0 — larger, smaller, normal text\n  F5 — refresh, F1 — this list\n\nIn the song list (when a row number has focus)\n  Up / Down — previous / next song\n  Shift+Up / Shift+Down — extend the selection\n  Home / End — first / last song\n  Space or Enter — select or deselect the song\n  Ctrl+A — select all shown songs\n  Alt+Up / Alt+Down — move the selected songs\n  Delete — remove the selected songs (Ctrl+Z undoes it)\n  Ctrl+P — play preview, Ctrl+O — open in TIDAL",
        "Overalt\n  Tab / Shift+Tab — neste / forrige kontroll\n  Enter eller mellomrom — trykk knappen som har fokus, eller kryss av\n  Esc — lukk en dialog\n  Ctrl+F — finn i spillelisten\n  Ctrl+K — søk etter nye sanger i TIDAL\n  Ctrl+G — filtrer listen over spillelister\n  Ctrl+N — ny spilleliste\n  Ctrl+D — dupliser spilleliste\n  Ctrl+I — importer, Ctrl+E — eksporter\n  Ctrl+Z — angre, Ctrl+Y — gjør om\n  Ctrl+mellomrom — pause eller fortsett avspilling\n  Ctrl + / Ctrl − / Ctrl 0 — større, mindre, normal tekst\n  F5 — oppdater, F1 — denne listen\n\nI sanglisten (når et radnummer har fokus)\n  Opp / Ned — forrige / neste sang\n  Shift+Opp / Shift+Ned — utvid utvalget\n  Home / End — første / siste sang\n  Mellomrom eller Enter — velg eller fjern valget av sangen\n  Ctrl+A — velg alle viste sanger\n  Alt+Opp / Alt+Ned — flytt de valgte sangene\n  Delete — fjern de valgte sangene (Ctrl+Z angrer)\n  Ctrl+P — spill smakebit, Ctrl+O — åpne i TIDAL";
    about_text => "TIDAL Playlist Editor {0}\n\nA desktop editor for your own TIDAL playlists. It uses the official TIDAL API with your own developer app and your own TIDAL login. It is not made by or affiliated with TIDAL.\n\nPlayback: TIDAL gives third-party apps preview clips (about 30 seconds). Full tracks play in the TIDAL app through «Open in TIDAL».\n\nYour login is stored only on this computer, in your user's settings folder. The app sends nothing anywhere except to TIDAL, and to Spotify when you connect it for copying playlists. Spotify data is not stored. This program is not made by or affiliated with Spotify.\n\nOpen-source components are listed in the NOTICE file that comes with the program.",
        "TIDAL spillelisteredigering {0}\n\nEt skrivebordsprogram for å redigere dine egne spillelister i TIDAL. Det bruker TIDALs offisielle API med din egen utviklerapp og din egen TIDAL-innlogging. Programmet er ikke laget av eller tilknyttet TIDAL.\n\nAvspilling: TIDAL gir tredjepartsapper smakebiter (omtrent 30 sekunder). Hele spor spilles i TIDAL-appen via «Åpne i TIDAL».\n\nInnloggingen lagres bare på denne datamaskinen, i innstillingsmappen til brukeren din. Programmet sender ingenting noe sted, unntatt til TIDAL, og til Spotify når du kobler det til for å kopiere spillelister. Data fra Spotify lagres ikke. Programmet er ikke laget av eller tilknyttet Spotify.\n\nKomponenter med åpen kildekode står i NOTICE-filen som følger med programmet.";
    accessibility_text => "Accessibility statement — TIDAL Playlist Editor\n\nWhat works\n• Screen readers: the app exposes its controls through AccessKit (AT-SPI on Linux, UI Automation on Windows). Buttons, check boxes, text fields, lists and dialogs have names; each song row is read as row number, title, artist, album, length and status.\n• Keyboard: everything can be done without a mouse. Tab moves between controls, a strong yellow (dark theme) or blue (light theme) ring shows where the focus is, and Esc closes dialogs. Drag-and-drop has keyboard alternatives (Alt+Up / Alt+Down). See Help → Keyboard shortcuts.\n• Reduced motion: follows the system setting (KDE, GNOME, Windows), or can be set in Settings.\n• Colour and sound: nothing is shown only by colour (status has text and a symbol) and nothing is told only by sound.\n• Contrast: text colours meet WCAG 2.1 AA (at least 4.5:1).\n• Time: there are no time limits, also not while logging in.\n• Text size: 50 % to 300 %, kept between sessions. Long titles in the song table are shortened to fit their column; the full text of the current row is shown, wrapped, above the table, and it is also in the tooltip and the screen reader name. Columns can be made wider with the mouse.\n• Selection: selected rows are marked with ✓ in the number column, not only by colour.\n• Languages: Norwegian (bokmål) and English.\n\nKnown limitations\n• Logging in happens on TIDAL's web page in your browser; its accessibility is decided by TIDAL.\n• Screen reader support depends on AccessKit; some screen readers may not read changes in the status line automatically. The message log (View → Show message log) lists all messages.\n• Reduced motion on Windows is read from the «Show animations in Windows» setting.\n\nFeedback\nPlease report problems in writing; the README says where. Replies are given in writing.",
        "Tilgjengelighetserklæring — TIDAL spillelisteredigering\n\nDette fungerer\n• Skjermleser: programmet viser kontrollene sine gjennom AccessKit (AT-SPI på Linux, UI Automation på Windows). Knapper, avkrysningsbokser, tekstfelt, lister og dialoger har navn; hver sangrad leses som radnummer, tittel, artist, album, lengde og status.\n• Tastatur: alt kan gjøres uten mus. Tab flytter mellom kontrollene, en tydelig gul (mørkt tema) eller blå (lyst tema) ring viser hvor fokuset er, og Esc lukker dialoger. Dra-og-slipp har tastaturalternativ (Alt+Opp / Alt+Ned). Se Hjelp → Hurtigtaster.\n• Redusert bevegelse: følger systeminnstillingen (KDE, GNOME, Windows), eller kan velges i Innstillinger.\n• Farge og lyd: ingenting vises bare med farge (status har tekst og symbol), og ingenting formidles bare med lyd.\n• Kontrast: tekstfargene oppfyller WCAG 2.1 AA (minst 4,5:1).\n• Tid: det er ingen tidsgrenser, heller ikke ved innlogging.\n• Tekststørrelse: 50 % til 300 %, huskes mellom øktene. Lange titler i sangtabellen forkortes for å passe i kolonnen; hele teksten for gjeldende rad vises, med linjebryting, over tabellen, og den står også i verktøytipset og i skjermlesernavnet. Kolonnene kan gjøres bredere med musen.\n• Valg: valgte rader er merket med ✓ i nummerkolonnen, ikke bare med farge.\n• Språk: norsk (bokmål) og engelsk.\n\nKjente begrensninger\n• Innloggingen skjer på TIDALs nettside i nettleseren; tilgjengeligheten der bestemmes av TIDAL.\n• Skjermleserstøtten avhenger av AccessKit; noen skjermlesere leser kanskje ikke opp endringer i statuslinjen automatisk. Meldingsloggen (Vis → Vis meldingslogg) viser alle meldinger.\n• Redusert bevegelse på Windows leses fra innstillingen «Vis animasjoner i Windows».\n\nTilbakemelding\nMeld gjerne fra om problemer skriftlig; README-filen forteller hvor. Svar gis skriftlig.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert_eq!(fmt("{0} of {1}", &["a", "b"]), "a of b");
        // Both languages use the same placeholders for every key that has any.
        assert!(EN.adding_to.contains("{1}") && NB.adding_to.contains("{1}"));
        assert!(!NB.accessibility_text.to_lowercase().contains("ærlig"));
    }
}
