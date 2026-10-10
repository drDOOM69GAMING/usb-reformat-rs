// Chiptune player for the RE-Formatter UI.
//
// Ported from the menu player in drDOOM69GAMING/re1hd-builder. The difference
// that matters here: tracks are read straight out of RT_RCDATA resources
// embedded in this exe (see seticon), not from files on disk and not unpacked
// to %TEMP%. `xmrs` parses from bytes, so nothing touches the filesystem.
//
// The whole shuffled playlist is played by one background thread: it loads a
// track, pumps its samples into rodio's channel, and when the module ends it
// moves to the next one in shuffled order. So playback starts with the UI and
// walks the collection without the UI thread doing anything.

use std::sync::mpsc::{sync_channel, Receiver};
use std::thread;

use rodio::{OutputStream, Sink, Source};
use xmrs::core::module::Module;
use xmrsplayer::xmrsplayer::XmrsPlayer;

use crate::win32;

const VOLUME: f32 = 0.35;
const SAMPLE_RATE: u32 = 44100;
const BUFFER_SAMPLES: usize = 8192;
/// Play each tune once, then move to the next shuffled track. Without this the
/// player repeats the same tune indefinitely (verified: `next()` never returns
/// None with the default loop count).
const MAX_LOOPS: usize = 1;
/// Last-resort runaway guard, only for modules whose internal pattern jump
/// never resolves so `next()` never returns None (verified: exactly one track,
/// "blank page.xm", which emits continuous audio for 15+ minutes). The longest
/// genuinely-ending track in the collection is 564.5s, so this sits ~1.6x above
/// anything real and normal tunes are never truncated.
const RUNAWAY_SAMPLES: i64 = SAMPLE_RATE as i64 * 900;

/// Fisher-Yates with a clock+pid seeded xorshift so every launch differs.
fn shuffle(files: &mut Vec<String>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut seed = now ^ (std::process::id() as u64).rotate_left(32);
    if seed == 0 {
        seed = 0xD1B5_4A32_D192_ED03;
    }
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let n = files.len();
    for i in (1..n).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        files.swap(i, j);
    }
}

pub struct MusicPlayer {
    // Must outlive the sink, otherwise the audio device is torn down early.
    // None when the machine has no usable audio device.
    _stream: Option<OutputStream>,
    sink: Option<Sink>,
    paused: bool,
}

// SAFETY: MusicPlayer is only ever touched from the single UI thread (the
// window procedure and its message loop). It never crosses threads.
unsafe impl Send for MusicPlayer {}

impl MusicPlayer {
    /// Build from whatever tracks were embedded and start playing straight
    /// away. If there is no audio device or no music, this still returns a
    /// usable object that simply does nothing.
    pub fn new() -> Self {
        let mut tracks = win32::music_index();
        tracks.sort();
        shuffle(&mut tracks);

        let (stream, sink) = match OutputStream::try_default() {
            Ok(s) => match Sink::try_new(&s.1) {
                Ok(k) => (Some(s.0), Some(k)),
                Err(_) => (Some(s.0), None),
            },
            Err(_) => (None, None),
        };

        let player = MusicPlayer {
            _stream: stream,
            sink,
            paused: false,
        };
        player.start();
        player
    }

    /// Hand the shuffled playlist to the pump thread and start playback.
    fn start(&self) {
        let Some(sink) = &self.sink else { return };
        let mut tracks = win32::music_index();
        tracks.sort();
        shuffle(&mut tracks);
        if tracks.is_empty() {
            return;
        }
        sink.append(PlaylistLoop::new(tracks));
        sink.set_volume(VOLUME);
        sink.play();
    }

    /// Pause / resume. Returns true when now paused.
    pub fn toggle_pause(&mut self) -> bool {
        self.paused = !self.paused;
        if let Some(s) = &self.sink {
            s.set_volume(if self.paused { 0.0 } else { VOLUME });
        }
        self.paused
    }
}

/// Streams every track in the shuffled list, moving on when one ends.
struct PlaylistLoop {
    receiver: Receiver<i16>,
}

impl PlaylistLoop {
    fn new(tracks: Vec<String>) -> Self {
        let (tx, receiver) = sync_channel::<i16>(BUFFER_SAMPLES);
        thread::spawn(move || {
            let n = tracks.len();
            let mut pos = 0usize;
            // Each iteration scopes a Module and the XmrsPlayer that borrows
            // it, so when a tune ends both are dropped and the next one is
            // loaded cleanly.
            'playlist: loop {
                let Some(data) = next_track_data(&tracks, &mut pos, n) else {
                    break;
                };
                let Ok(module) = Module::load(&data) else {
                    continue 'playlist;
                };
                let mut player = XmrsPlayer::new(&module, SAMPLE_RATE, 0);
                player.set_max_loop_count(MAX_LOOPS);
                let mut played: i64 = 0;
                loop {
                    match player.next() {
                        Some(sample) => {
                            if tx.send(sample).is_err() {
                                break 'playlist;
                            }
                            played += 1;
                            // Runaway guard only. Measured over all 100 tracks:
                            // 82 end naturally, 17 more end naturally but have
                            // quiet gaps part-way through, and only 1 never ends.
                            // A silence detector was tried here and removed - it
                            // truncated those 17 mid-tune (up to 123s early).
                            if played >= RUNAWAY_SAMPLES {
                                continue 'playlist;
                            }
                        }
                        // True end of module -> carry on with the next one.
                        // This is the only normal path to the next track.
                        None => continue 'playlist,
                    }
                }
            }
        });
        Self { receiver }
    }
}

/// Advance through the playlist and return the next track's bytes, reading
/// them straight out of the exe. Returns None only if nothing can be loaded.
fn next_track_data(tracks: &[String], pos: &mut usize, n: usize) -> Option<Vec<u8>> {
    for _ in 0..n {
        let name = &tracks[*pos];
        *pos = (*pos + 1) % n;
        if let Some(data) = win32::resource_bytes(name) {
            return Some(data);
        }
    }
    None
}

impl Iterator for PlaylistLoop {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (1024, None)
    }
}

impl Source for PlaylistLoop {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}