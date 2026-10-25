//! Audio preview player on its own thread (rodio + symphonia).

pub mod dash;

use std::io::Cursor;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source as _;

pub enum PlayerCmd {
    Play { bytes: Vec<u8>, hint: String, title: String, track_id: String, full: bool },
    TogglePause,
    Stop,
    Volume(f32),
}

#[derive(Clone, Debug, Default)]
pub struct PlayerState {
    pub title: String,
    pub track_id: String,
    pub full: bool,
    pub playing: bool,
    pub paused: bool,
    pub pos_secs: f32,
    pub total_secs: Option<f32>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct PlayerHandle {
    tx: mpsc::Sender<PlayerCmd>,
    pub state: Arc<Mutex<PlayerState>>,
}

impl PlayerHandle {
    pub fn spawn(volume: f32, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(PlayerState::default()));
        let st = state.clone();
        std::thread::Builder::new()
            .name("player".into())
            .spawn(move || run(rx, st, volume, repaint))
            .expect("player thread");
        Self { tx, state }
    }

    pub fn send(&self, cmd: PlayerCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> PlayerState {
        self.state.lock().expect("player state").clone()
    }
}

fn run(rx: mpsc::Receiver<PlayerCmd>, state: Arc<Mutex<PlayerState>>, mut volume: f32, repaint: impl Fn()) {
    let mut sink: Option<rodio::MixerDeviceSink> = None;
    let mut player: Option<rodio::Player> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(PlayerCmd::Play { bytes, hint, title, track_id, full }) => {
                if let Some(p) = player.take() {
                    p.stop();
                }
                if sink.is_none() {
                    match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(mut s) => {
                            s.log_on_drop(false);
                            sink = Some(s);
                        }
                        Err(e) => {
                            set_error(&state, format!("audio output: {e}"));
                            repaint();
                            continue;
                        }
                    }
                }
                let len = bytes.len() as u64;
                let decoder = rodio::Decoder::builder()
                    .with_data(Cursor::new(bytes))
                    .with_byte_len(len)
                    .with_hint(&hint)
                    .with_seekable(true)
                    .build();
                match decoder {
                    Ok(dec) => {
                        let total = dec.total_duration().map(|d| d.as_secs_f32());
                        let p = rodio::Player::connect_new(sink.as_ref().expect("sink").mixer());
                        p.set_volume(volume);
                        p.append(dec);
                        player = Some(p);
                        *state.lock().expect("state") = PlayerState {
                            title,
                            track_id,
                            full,
                            playing: true,
                            paused: false,
                            pos_secs: 0.0,
                            total_secs: total,
                            error: None,
                        };
                    }
                    Err(e) => set_error(&state, format!("decode: {e}")),
                }
                repaint();
            }
            Ok(PlayerCmd::TogglePause) => {
                if let Some(p) = &player {
                    if p.is_paused() {
                        p.play()
                    } else {
                        p.pause()
                    }
                    state.lock().expect("state").paused = p.is_paused();
                    repaint();
                }
            }
            Ok(PlayerCmd::Stop) => {
                if let Some(p) = player.take() {
                    p.stop();
                }
                let mut s = state.lock().expect("state");
                s.playing = false;
                s.paused = false;
                s.pos_secs = 0.0;
                drop(s);
                repaint();
            }
            Ok(PlayerCmd::Volume(v)) => {
                volume = v.clamp(0.0, 1.0);
                if let Some(p) = &player {
                    p.set_volume(volume);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(p) = &player {
            let mut s = state.lock().expect("state");
            let finished = p.empty();
            let pos = p.get_pos().as_secs_f32();
            if finished || (pos - s.pos_secs).abs() >= 0.5 {
                s.pos_secs = pos;
                if finished {
                    s.playing = false;
                    s.paused = false;
                }
                drop(s);
                if finished {
                    player = None;
                }
                repaint();
            }
        }
    }
}

fn set_error(state: &Arc<Mutex<PlayerState>>, msg: String) {
    tracing::warn!("{msg}");
    let mut s = state.lock().expect("state");
    s.playing = false;
    s.error = Some(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preview pipeline end to end, offline: MPD → segment URLs → concatenated
    /// fragmented MP4 → rodio/symphonia decode. Fixture made with ffmpeg (8 s, 440 Hz, AAC-LC).
    #[test]
    fn decodes_concatenated_dash_segments() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dash-aac");
        let xml = std::fs::read_to_string(dir.join("manifest.mpd")).unwrap();
        let reps = dash::parse_mpd(&xml, "https://cdn.example/x/manifest.mpd").unwrap();
        let rep = dash::pick(&reps).unwrap();
        assert_eq!(rep.urls.len(), 5, "init + 4 segments");
        let mut bytes = Vec::new();
        for u in &rep.urls {
            let name = u.rsplit('/').next().unwrap();
            assert!(u.starts_with("https://cdn.example/x/"), "relative URLs resolve against the manifest: {u}");
            bytes.extend(std::fs::read(dir.join(name)).unwrap());
        }
        let len = bytes.len() as u64;
        let dec = rodio::Decoder::builder()
            .with_data(Cursor::new(bytes))
            .with_byte_len(len)
            .with_hint("mp4")
            .with_seekable(true)
            .build()
            .unwrap();
        let (rate, channels) = (dec.sample_rate().get(), dec.channels().get());
        let samples = dec.count();
        let secs = samples as f32 / (rate as f32 * f32::from(channels));
        assert!((7.5..=8.5).contains(&secs), "decoded {secs} s");
    }
}
