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
        Ok(url::form_urlencoded::parse(format!("x={payload}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default())
    })
}

#[derive(Default, Clone)]
struct Template {
    init: Option<String>,
    media: Option<String>,
    start: u64,
    timescale: u64,
    duration: u64,
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
    if rel.contains("://") || base.is_empty() {
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
                    if !empty {
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
                    parent.start = attr(&e, "startNumber").and_then(|v| v.parse().ok()).unwrap_or(1);
                    parent.timescale = attr(&e, "timescale").and_then(|v| v.parse().ok()).unwrap_or(1);
                    parent.duration = attr(&e, "duration").and_then(|v| v.parse().ok()).unwrap_or(0);
                    in_timeline_of_rep = rep.is_some();
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
                        if let Some(mut r) = rep.take() {
                            let tpl = rep_tpl.take().unwrap_or_else(|| set_tpl.clone());
                            r.urls = segment_urls(&tpl, &r, &base, total_secs);
                            if r.urls.is_empty() && !base.is_empty() && base != manifest_url {
                                r.urls.push(base);
                            }
                            reps.push(r);
                        }
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

fn segment_urls(t: &Template, r: &Representation, base: &str, total_secs: f64) -> Vec<String> {
    let Some(media) = &t.media else { return Vec::new() };
    let mut urls = Vec::new();
    if let Some(init) = &t.init {
        urls.push(join(base, &fill(init, &r.id, r.bandwidth, 0, 0)));
    }
    let mut number = t.start;
    if !t.timeline.is_empty() {
        let mut time = 0u64;
        for &(start, d, rep) in &t.timeline {
            if let Some(s) = start {
                time = s;
            }
            let count = if rep < 0 { 1 } else { rep as u64 + 1 };
            for _ in 0..count {
                urls.push(join(base, &fill(media, &r.id, r.bandwidth, number, time)));
                number += 1;
                time += d;
            }
        }
    } else if t.duration > 0 && total_secs > 0.0 {
        let seg_secs = t.duration as f64 / t.timescale.max(1) as f64;
        let count = (total_secs / seg_secs).ceil() as u64;
        for i in 0..count {
            urls.push(join(base, &fill(media, &r.id, r.bandwidth, number, i * t.duration)));
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
}
