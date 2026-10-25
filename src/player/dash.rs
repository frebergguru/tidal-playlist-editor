//! Just enough MPEG-DASH (MPD) parsing to fetch a single audio representation:
//! SegmentTemplate with SegmentTimeline or fixed duration, or a plain BaseURL.

use base64::Engine as _;
use quick_xml::events::{BytesStart, Event};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Representation {
    pub id: String,
    pub codecs: String,
    pub bandwidth: u64,
    pub urls: Vec<String>,
}

/// Decodes a `data:` URI (base64 or percent-encoded) or returns `None` for http(s).
pub fn decode_data_uri(uri: &str) -> Option<Result<String, String>> {
    let rest = uri.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',').unwrap_or(("", rest));
    Some(if meta.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .map_err(|e| e.to_string())
            .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
    } else {
        String::from_utf8(percent_decode(payload)).map_err(|e| e.to_string())
    })
}

/// RFC 3986 percent-decoding (unlike form decoding, `+` stays a plus).
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Upper bound on segments per representation, against hostile manifests.
const MAX_SEGMENTS: usize = 10_000;

#[derive(Default, Clone)]
struct Template {
    init: Option<String>,
    media: Option<String>,
    // `None` = not set here, so a Representation inherits the AdaptationSet's.
    start: Option<u64>,
    timescale: Option<u64>,
    duration: Option<u64>,
    timeline: Vec<(Option<u64>, u64, i64)>,
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

fn fill(tpl: &str, rep: &str, bw: u64, number: u64, time: u64) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        let (name, fmt) = token.split_once('%').map_or((token, None), |(n, f)| (n, Some(f)));
        let width =
            fmt.and_then(|f| f.trim_start_matches('0').trim_end_matches('d').parse::<usize>().ok()).unwrap_or(0);
        let val = match name {
            "" => "$".to_owned(),
            "RepresentationID" => rep.to_owned(),
            "Number" => format!("{number:0width$}"),
            "Time" => format!("{time:0width$}"),
            "Bandwidth" => format!("{bw:0width$}"),
            other => format!("${other}$"),
        };
        out.push_str(&val);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn join(base: &str, rel: &str) -> String {
    // Absolute when it has its own scheme (a query string may hold "://" too).
    if base.is_empty() || url::Url::parse(rel).is_ok() {
        return rel.to_owned();
    }
    url::Url::parse(base).and_then(|b| b.join(rel)).map(String::from).unwrap_or_else(|_| rel.to_owned())
}

/// Parses all audio representations with their full segment URL lists.
pub fn parse_mpd(xml: &str, manifest_url: &str) -> Result<Vec<Representation>, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut total_secs = 0f64;
    let mut base_stack: Vec<(String, String)> = vec![("root".into(), manifest_url.to_owned())];
    let mut set_tpl = Template::default();
    let mut rep_tpl: Option<Template> = None;
    let mut rep: Option<Representation> = None;
    let mut reps = Vec::new();
    let mut in_timeline_of_rep = false;
    let mut in_base = false;
    let mut base_text = String::new();
    let mut set_codecs = String::new();

    let base_now = |stack: &Vec<(String, String)>| stack.last().map(|b| b.1.clone()).unwrap_or_default();

    loop {
        let ev = reader.read_event().map_err(|e| format!("MPD: {e}"))?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            _ => (None, false),
        };
        if let Some(e) = e {
            let name = e.local_name().as_ref().to_owned();
            match name.as_str() {
                "MPD" => {
                    if let Some(d) = attr(&e, "mediaPresentationDuration") {
                        total_secs = f64::from(crate::api::parse_iso_duration(&d));
                    }
                }
                "AdaptationSet" => {
                    set_tpl = Template::default();
                    set_codecs = attr(&e, "codecs").unwrap_or_default();
                    if !empty {
                        base_stack.push(("AdaptationSet".into(), base_now(&base_stack)));
                    }
                }
                "Representation" => {
                    rep = Some(Representation {
                        id: attr(&e, "id").unwrap_or_default(),
                        codecs: attr(&e, "codecs").unwrap_or_else(|| set_codecs.clone()),
                        bandwidth: attr(&e, "bandwidth").and_then(|b| b.parse().ok()).unwrap_or(0),
                        urls: Vec::new(),
                    });
                    rep_tpl = None;
                    if empty {
                        // `<Representation .../>` has no End event; finish it now.
                        let base = base_now(&base_stack);
                        finish_rep(&mut rep, &mut rep_tpl, &set_tpl, &base, manifest_url, total_secs, &mut reps);
                    } else {
                        base_stack.push(("Representation".into(), base_now(&base_stack)));
                    }
                }
                "SegmentTemplate" => {
                    let parent =
                        if rep.is_some() { rep_tpl.get_or_insert_with(|| set_tpl.clone()) } else { &mut set_tpl };
                    if let Some(v) = attr(&e, "initialization") {
                        parent.init = Some(v);
                    }
                    if let Some(v) = attr(&e, "media") {
                        parent.media = Some(v);
                    }
                    if let Some(v) = attr(&e, "startNumber").and_then(|v| v.parse().ok()) {
                        parent.start = Some(v);
                    }
                    if let Some(v) = attr(&e, "timescale").and_then(|v| v.parse().ok()) {
                        parent.timescale = Some(v);
                    }
                    if let Some(v) = attr(&e, "duration").and_then(|v| v.parse().ok()) {
                        parent.duration = Some(v);
                    }
                    in_timeline_of_rep = rep.is_some();
                }
                // A Representation's own timeline replaces the inherited one.
                "SegmentTimeline" if in_timeline_of_rep => {
                    rep_tpl.get_or_insert_with(|| set_tpl.clone()).timeline.clear();
                }
                "S" => {
                    let s = (
                        attr(&e, "t").and_then(|v| v.parse().ok()),
                        attr(&e, "d").and_then(|v| v.parse().ok()).unwrap_or(0),
                        attr(&e, "r").and_then(|v| v.parse().ok()).unwrap_or(0),
                    );
                    if in_timeline_of_rep {
                        rep_tpl.get_or_insert_with(|| set_tpl.clone()).timeline.push(s);
                    } else {
                        set_tpl.timeline.push(s);
                    }
                }
                "BaseURL" if !empty => {
                    in_base = true;
                    base_text.clear();
                }
                _ => {}
            }
        }
        match ev {
            Event::Text(t) if in_base => base_text.push_str(&t.xml10_content()),
            Event::GeneralRef(r) if in_base => base_text.push_str(&crate::ops::files::ref_text(&r)),
            Event::End(e) => {
                let name = e.local_name().as_ref().to_owned();
                match name.as_str() {
                    "BaseURL" => {
                        in_base = false;
                        let resolved = join(&base_now(&base_stack), base_text.trim());
                        if let Some(top) = base_stack.last_mut() {
                            top.1 = resolved;
                        }
                    }
                    "Representation" => {
                        let base = base_now(&base_stack);
                        base_stack.pop();
                        finish_rep(&mut rep, &mut rep_tpl, &set_tpl, &base, manifest_url, total_secs, &mut reps);
                    }
                    "AdaptationSet" => {
                        base_stack.pop();
                    }
                    "SegmentTemplate" => in_timeline_of_rep = false,
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(reps)
}

#[allow(clippy::too_many_arguments)]
fn finish_rep(
    rep: &mut Option<Representation>,
    rep_tpl: &mut Option<Template>,
    set_tpl: &Template,
    base: &str,
    manifest_url: &str,
    total_secs: f64,
    reps: &mut Vec<Representation>,
) {
    if let Some(mut r) = rep.take() {
        let tpl = rep_tpl.take().unwrap_or_else(|| set_tpl.clone());
        r.urls = segment_urls(&tpl, &r, base, total_secs);
        if r.urls.is_empty() && !base.is_empty() && base != manifest_url {
            r.urls.push(base.to_owned());
        }
        reps.push(r);
    }
}

fn segment_urls(t: &Template, r: &Representation, base: &str, total_secs: f64) -> Vec<String> {
    let Some(media) = &t.media else { return Vec::new() };
    let timescale = t.timescale.unwrap_or(1).max(1);
    let mut urls = Vec::new();
    if let Some(init) = &t.init {
        urls.push(join(base, &fill(init, &r.id, r.bandwidth, 0, 0)));
    }
    let mut number = t.start.unwrap_or(1);
    if !t.timeline.is_empty() {
        let end_time = (total_secs * timescale as f64) as u64;
        let mut time = 0u64;
        for (i, &(start, d, rep)) in t.timeline.iter().enumerate() {
            if let Some(s) = start {
                time = s;
            }
            let count = if rep >= 0 {
                rep as u64 + 1
            } else {
                // r = -1: repeat until the next S@t, or the end of the period.
                let until = t.timeline.get(i + 1).and_then(|n| n.0).unwrap_or(end_time);
                if d == 0 { 1 } else { until.saturating_sub(time).div_ceil(d).max(1) }
            };
            for _ in 0..count {
                if urls.len() >= MAX_SEGMENTS {
                    return urls;
                }
                urls.push(join(base, &fill(media, &r.id, r.bandwidth, number, time)));
                number += 1;
                time = time.saturating_add(d);
            }
        }
    } else if let Some(duration) = t.duration.filter(|&d| d > 0)
        && total_secs > 0.0
    {
        let seg_secs = duration as f64 / timescale as f64;
        let count = ((total_secs / seg_secs).ceil() as u64).min(MAX_SEGMENTS as u64);
        for i in 0..count {
            urls.push(join(base, &fill(media, &r.id, r.bandwidth, number, i * duration)));
            number += 1;
        }
    }
    urls
}

/// Picks the representation our decoder handles best (AAC-LC > FLAC > HE-AAC > any).
pub fn pick(reps: &[Representation]) -> Option<&Representation> {
    let rank = |c: &str| {
        let c = c.to_ascii_lowercase();
        if c.starts_with("mp4a.40.2") {
            0
        } else if c.contains("flac") {
            1
        } else if c.starts_with("mp4a") {
            2
        } else {
            3
        }
    };
    reps.iter().filter(|r| !r.urls.is_empty()).min_by_key(|r| (rank(&r.codecs), std::cmp::Reverse(r.bandwidth)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT30.0S" type="static">
  <Period id="0">
    <AdaptationSet id="0" contentType="audio" mimeType="audio/mp4">
      <Representation id="AAC,44100" codecs="mp4a.40.2" bandwidth="320000" audioSamplingRate="44100">
        <SegmentTemplate timescale="44100" initialization="https://cdn.example/a/0.mp4?t=1&amp;x=2" media="https://cdn.example/a/$Number$.mp4?t=1" startNumber="1">
          <SegmentTimeline>
            <S d="176128" r="2"/>
            <S d="1000"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
      <Representation id="flac" codecs="flac" bandwidth="900000">
        <BaseURL>https://cdn.example/b/</BaseURL>
        <SegmentTemplate timescale="1000" duration="10000" initialization="init_$RepresentationID$.mp4" media="seg_$Number%03d$.mp4" startNumber="0"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn parses_timeline_and_duration_templates() {
        let reps = parse_mpd(MPD, "https://manifest.example/m.mpd").unwrap();
        assert_eq!(reps.len(), 2);
        let a = &reps[0];
        assert_eq!(a.codecs, "mp4a.40.2");
        assert_eq!(a.urls.len(), 5);
        assert_eq!(a.urls[0], "https://cdn.example/a/0.mp4?t=1&x=2");
        assert_eq!(a.urls[1], "https://cdn.example/a/1.mp4?t=1");
        assert_eq!(a.urls[4], "https://cdn.example/a/4.mp4?t=1");
        let b = &reps[1];
        assert_eq!(b.urls[0], "https://cdn.example/b/init_flac.mp4");
        assert_eq!(b.urls[1], "https://cdn.example/b/seg_000.mp4");
        assert_eq!(b.urls.len(), 4);
        assert_eq!(pick(&reps).unwrap().id, "AAC,44100");
    }

    #[test]
    fn data_uris() {
        let enc = base64::engine::general_purpose::STANDARD.encode("<MPD/>");
        assert_eq!(decode_data_uri(&format!("data:application/dash+xml;base64,{enc}")).unwrap().unwrap(), "<MPD/>");
        assert_eq!(decode_data_uri("data:text/plain,a%20b").unwrap().unwrap(), "a b");
        assert!(decode_data_uri("https://x").is_none());
        assert_eq!(fill("$$x$Number%05d$", "r", 1, 42, 0), "$x00042");
    }

    #[test]
    fn representation_inherits_set_template_and_may_self_close() {
        let mpd = r#"<MPD mediaPresentationDuration="PT20S"><Period><AdaptationSet codecs="mp4a.40.2">
          <SegmentTemplate timescale="1000" duration="10000" startNumber="0" media="s$Number$.mp4"/>
          <Representation id="a"><SegmentTemplate media="a$Number$.mp4"/></Representation>
          <Representation id="b" bandwidth="1"/>
        </AdaptationSet></Period></MPD>"#;
        let reps = parse_mpd(mpd, "https://m.example/x.mpd").unwrap();
        assert_eq!(reps.len(), 2);
        assert_eq!(reps[0].urls, ["https://m.example/a0.mp4", "https://m.example/a1.mp4"]);
        assert_eq!(reps[1].urls, ["https://m.example/s0.mp4", "https://m.example/s1.mp4"]);
    }

    #[test]
    fn open_ended_repeat_fills_the_period() {
        let mpd = r#"<MPD mediaPresentationDuration="PT10S"><Period><AdaptationSet><Representation id="a">
          <SegmentTemplate timescale="1" media="$Time$"><SegmentTimeline><S t="0" d="4" r="-1"/></SegmentTimeline></SegmentTemplate>
        </Representation></AdaptationSet></Period></MPD>"#;
        let reps = parse_mpd(mpd, "https://m.example/x.mpd").unwrap();
        assert_eq!(reps[0].urls, ["https://m.example/0", "https://m.example/4", "https://m.example/8"]);
    }

    #[test]
    fn own_timeline_replaces_the_inherited_one() {
        let mpd = r#"<MPD><Period><AdaptationSet>
          <SegmentTemplate timescale="1" media="s$Time$"><SegmentTimeline><S t="0" d="5" r="1"/></SegmentTimeline></SegmentTemplate>
          <Representation id="a">
            <SegmentTemplate media="a$Time$"><SegmentTimeline><S t="0" d="3" r="1"/></SegmentTimeline></SegmentTemplate>
          </Representation>
        </AdaptationSet></Period></MPD>"#;
        let reps = parse_mpd(mpd, "https://m.example/x.mpd").unwrap();
        assert_eq!(reps[0].urls, ["https://m.example/a0", "https://m.example/a3"]);
    }

    #[test]
    fn relative_url_with_a_url_in_its_query_is_resolved() {
        assert_eq!(join("https://m.example/x/m.mpd", "s1.mp4?r=https://o.example/"), "https://m.example/x/s1.mp4?r=https://o.example/");
        assert_eq!(join("https://m.example/x/m.mpd", "https://cdn.example/s1.mp4"), "https://cdn.example/s1.mp4");
    }

    #[test]
    fn percent_encoded_data_uri_keeps_plus() {
        assert_eq!(decode_data_uri("data:text/plain,a+b%2Bc%").unwrap().unwrap(), "a+b+c%");
    }
}
