//! Reports what the recorded takes of a project's scene say, with plots.
//!
//! ```text
//! cargo run -p charcraft-record --example takes_report -- PROJECT OUTDIR [--scene NAME] [--prefix NAME]
//! ```
//!
//! For every puppet track it lists the takes, plots the decoded face curves of each face take as a PNG, and compares
//! the voice with the scene's sound: the openness of the recorded mouth shapes and of the performer's mouth against
//! the loudness of the vocals (Pearson correlation over a range of lags), and the recorded mouth shapes against the
//! ones `anim-lipsync` finds in the same sound.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::many_single_char_names,
    clippy::useless_let_if_seq,
    clippy::too_many_lines,
    clippy::suboptimal_flops,
    clippy::print_stdout,
    reason = "a report tool"
)]

use std::path::{Path, PathBuf};

use anim_core::Tick;
use anim_lipsync::Viseme;
use charcraft_behaviors::Inputs;
use charcraft_behaviors::behaviors::lipsync::openness;
use charcraft_project::Project;
use charcraft_record::take::resolve;
use charcraft_record::{MediaStore, ParamGroup, PuppetTrack, Take, import_scene, load_sounds};

type Error = Box<dyn std::error::Error>;

// ---- a tiny plotting canvas -------------------------------------------------------------------------------------

const GLYPHS: &[(char, [&str; 5])] = &[
    ('A', ["010", "101", "111", "101", "101"]),
    ('B', ["110", "101", "110", "101", "110"]),
    ('C', ["011", "100", "100", "100", "011"]),
    ('D', ["110", "101", "101", "101", "110"]),
    ('E', ["111", "100", "110", "100", "111"]),
    ('F', ["111", "100", "110", "100", "100"]),
    ('G', ["011", "100", "101", "101", "011"]),
    ('H', ["101", "101", "111", "101", "101"]),
    ('I', ["111", "010", "010", "010", "111"]),
    ('J', ["001", "001", "001", "101", "010"]),
    ('K', ["101", "101", "110", "101", "101"]),
    ('L', ["100", "100", "100", "100", "111"]),
    ('M', ["101", "111", "111", "101", "101"]),
    ('N', ["110", "101", "101", "101", "101"]),
    ('O', ["010", "101", "101", "101", "010"]),
    ('P', ["110", "101", "110", "100", "100"]),
    ('Q', ["010", "101", "101", "110", "011"]),
    ('R', ["110", "101", "110", "101", "101"]),
    ('S', ["011", "100", "010", "001", "110"]),
    ('T', ["111", "010", "010", "010", "010"]),
    ('U', ["101", "101", "101", "101", "111"]),
    ('V', ["101", "101", "101", "101", "010"]),
    ('W', ["101", "101", "111", "111", "101"]),
    ('X', ["101", "101", "010", "101", "101"]),
    ('Y', ["101", "101", "010", "010", "010"]),
    ('Z', ["111", "001", "010", "100", "111"]),
    ('0', ["111", "101", "101", "101", "111"]),
    ('1', ["010", "110", "010", "010", "111"]),
    ('2', ["110", "001", "010", "100", "111"]),
    ('3', ["110", "001", "010", "001", "110"]),
    ('4', ["101", "101", "111", "001", "001"]),
    ('5', ["111", "100", "110", "001", "110"]),
    ('6', ["011", "100", "111", "101", "111"]),
    ('7', ["111", "001", "010", "010", "010"]),
    ('8', ["111", "101", "111", "101", "111"]),
    ('9', ["111", "101", "111", "001", "110"]),
    ('-', ["000", "000", "111", "000", "000"]),
    ('.', ["000", "000", "000", "000", "010"]),
    (':', ["000", "010", "000", "010", "000"]),
    ('/', ["001", "001", "010", "100", "100"]),
    ('=', ["000", "111", "000", "111", "000"]),
    ('+', ["000", "010", "111", "010", "000"]),
    ('(', ["001", "010", "010", "010", "001"]),
    (')', ["100", "010", "010", "010", "100"]),
    (',', ["000", "000", "000", "010", "100"]),
    ('%', ["101", "001", "010", "100", "101"]),
];

type Rgb = [u8; 3];

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<Rgb>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![[255, 255, 255]; w * h] }
    }

    fn set(&mut self, x: i64, y: i64, c: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }

    fn rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgb) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    fn line(&mut self, a: (f64, f64), b: (f64, f64), c: Rgb) {
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            self.set((a.0 + (b.0 - a.0) * t).round() as i64, (a.1 + (b.1 - a.1) * t).round() as i64, c);
        }
    }

    fn text(&mut self, x: i64, y: i64, text: &str, scale: i64, c: Rgb) {
        let mut cx = x;
        for ch in text.chars().map(|c| c.to_ascii_uppercase()) {
            if let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) {
                for (ry, row) in rows.iter().enumerate() {
                    for (rx, bit) in row.chars().enumerate() {
                        if bit == '1' {
                            self.rect(cx + rx as i64 * scale, y + ry as i64 * scale, scale, scale, c);
                        }
                    }
                }
            }
            cx += 4 * scale;
        }
    }

    fn save(&self, path: &Path) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, self.w as u32, self.h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let flat: Vec<u8> = self.px.iter().flatten().copied().collect();
        encoder.write_header()?.write_image_data(&flat)?;
        Ok(())
    }
}

const COLORS: [Rgb; 6] = [[200, 40, 40], [30, 110, 200], [20, 150, 80], [170, 90, 0], [130, 50, 170], [0, 140, 150]];

struct Series<'a> {
    name: &'a str,
    points: Vec<(f64, f64)>,
}

/// Draws one panel: a frame, the curves, a label per curve and the value range at the left edge.
fn panel(canvas: &mut Canvas, (x, y, w, h): (i64, i64, i64, i64), title: &str, series: &[Series<'_>], (t0, t1): (f64, f64)) {
    canvas.rect(x, y, w, h, [250, 250, 250]);
    for edge in [(x, y, x + w, y), (x, y + h, x + w, y + h), (x, y, x, y + h), (x + w, y, x + w, y + h)] {
        canvas.line((edge.0 as f64, edge.1 as f64), (edge.2 as f64, edge.3 as f64), [150, 150, 150]);
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in series {
        for &(_, v) in &s.points {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || hi - lo < 1e-9 {
        lo -= 0.5;
        hi += 0.5;
    }
    let pad = (hi - lo) * 0.05;
    let (lo, hi) = (lo - pad, hi + pad);
    let to_px = |t: f64, v: f64| (x as f64 + (t - t0) / (t1 - t0) * w as f64, y as f64 + h as f64 - (v - lo) / (hi - lo) * h as f64);
    if lo < 0.0 && hi > 0.0 {
        canvas.line(to_px(t0, 0.0), to_px(t1, 0.0), [210, 210, 210]);
    }
    for (i, s) in series.iter().enumerate() {
        let c = COLORS[i % COLORS.len()];
        for pair in s.points.windows(2) {
            canvas.line(to_px(pair[0].0, pair[0].1), to_px(pair[1].0, pair[1].1), c);
        }
        canvas.text(x + 6 + i as i64 * 150, y + 4, s.name, 2, c);
    }
    canvas.text(x + w - 6 - title.len() as i64 * 8, y + 4, title, 2, [90, 90, 90]);
    canvas.text(x + 4, y + h - 14, &format!("{lo:.2}"), 2, [120, 120, 120]);
    canvas.text(x + 4, y + 18, &format!("{hi:.2}"), 2, [120, 120, 120]);
}

fn sheet(path: &Path, title: &str, range: (f64, f64), panels: &[(&str, Vec<Series<'_>>)]) -> Result<(), Error> {
    let (w, ph) = (1400_usize, 190_usize);
    let mut canvas = Canvas::new(w, 40 + panels.len() * (ph + 8));
    canvas.text(10, 10, title, 3, [20, 20, 20]);
    for (i, (name, series)) in panels.iter().enumerate() {
        panel(&mut canvas, (10, 40 + (i * (ph + 8)) as i64, (w - 20) as i64, ph as i64), name, series, range);
    }
    canvas.save(path)
}

// ---- measurements -------------------------------------------------------------------------------------------------

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 {
        return f64::NAN;
    }
    let (a, b) = (&a[..n], &b[..n]);
    let (ma, mb) = (a.iter().sum::<f64>() / n as f64, b.iter().sum::<f64>() / n as f64);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    if saa < 1e-12 || sbb < 1e-12 { f64::NAN } else { sab / (saa * sbb).sqrt() }
}

/// Correlation of `a(t)` with `b(t + lag)` for lags in `-max..=max` samples; returns `(best lag, r)` and `r` at lag 0.
fn best_lag(a: &[f64], b: &[f64], max: isize) -> ((isize, f64), f64) {
    let mut best = (0, f64::NEG_INFINITY);
    let mut zero = f64::NAN;
    for lag in -max..=max {
        let (xs, ys): (Vec<f64>, Vec<f64>) =
            (0..a.len() as isize).filter_map(|i| Some((a[usize::try_from(i).ok()?], *b.get(usize::try_from(i + lag).ok()?)?))).unzip();
        let r = pearson(&xs, &ys);
        if lag == 0 {
            zero = r;
        }
        if r.is_finite() && r > best.1 {
            best = (lag, r);
        }
    }
    (best, zero)
}

const RATE_HZ: f64 = 25.0;

fn times(duration: f64) -> Vec<f64> {
    (0..(duration * RATE_HZ) as usize).map(|i| i as f64 / RATE_HZ).collect()
}

fn inputs_at(track: &PuppetTrack, t: f64) -> Inputs {
    resolve(&track.takes, Inputs::at(Tick::from_seconds_f64(t)))
}

fn rms_envelope(samples: &[f32], rate: u32, start: f64, trim: f64, duration: f64) -> Vec<f64> {
    let window = (f64::from(rate) / RATE_HZ) as usize;
    times(duration)
        .into_iter()
        .map(|t| {
            let at = ((t - start + trim) * f64::from(rate)).round();
            if at < 0.0 {
                return 0.0;
            }
            let a = at as usize;
            let slice = samples.get(a..(a + window).min(samples.len())).unwrap_or(&[]);
            if slice.is_empty() { 0.0 } else { (slice.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / slice.len() as f64).sqrt() }
        })
        .collect()
}

fn describe(take: &Take) -> String {
    let groups: Vec<&str> = take.groups.iter().map(|g| g.label()).collect();
    format!(
        "{:<46} take {:>2} {} at {:>8.3} s for {:>8.3} s, {:>6} samples [{}] fade {:.2}/{:.2}",
        take.name,
        take.number,
        if take.enabled { "on " } else { "off" },
        take.start.seconds(),
        take.duration.seconds(),
        take.samples.len(),
        groups.join(","),
        take.blend_in.seconds(),
        take.blend_out.seconds()
    )
}

fn main() -> Result<(), Error> {
    let mut positional = Vec::new();
    let mut scene_name: Option<String> = None;
    let mut prefix = String::from("ca-takes");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scene" => scene_name = args.next(),
            "--prefix" => prefix = args.next().ok_or("--prefix needs a name")?,
            _ => positional.push(arg),
        }
    }
    let input = positional.first().ok_or("usage: takes_report PROJECT OUTDIR [--scene NAME] [--prefix NAME]")?;
    let out = PathBuf::from(positional.get(1).ok_or("usage: takes_report PROJECT OUTDIR")?);
    let project = Project::open(input)?;
    let model = project.model()?;
    let imported = import_scene(&project, &model, scene_name.as_deref())?;
    let scene = &imported.scene;
    println!("scene {:?}: {:.2} s, {} puppet tracks, warnings {:?}", scene.name, scene.duration.seconds(), scene.tracks.len(), imported.warnings);
    let mut media = MediaStore::new();
    println!("sounds: {:?}", load_sounds(&project, &imported, &mut media));
    let duration = scene.duration.seconds();

    // The scene's sound, as the loudness the vocals have over scene time, and the mouth shapes anim-lipsync finds.
    let mut loudness: Option<Vec<f64>> = None;
    let mut audio_keys = Vec::new();
    if let Some(clip) = scene.audio.iter().flat_map(|t| &t.clips).find(|c| media.get(&c.media).is_some())
        && let Some(buffer) = media.get(&clip.media)
    {
        let mono = buffer.to_mono();
        println!(
            "sound {:?}: {:.1} s at {} Hz, starts at {:.3} s, trim {:.3} s",
            clip.media,
            buffer.seconds(),
            mono.sample_rate,
            clip.start.seconds(),
            clip.trim_in.seconds()
        );
        loudness = Some(rms_envelope(&mono.samples, mono.sample_rate, clip.start.seconds(), clip.trim_in.seconds(), duration));
        audio_keys = anim_lipsync::analyze(&mono.samples, mono.sample_rate);
    }
    let ts = times(duration);
    let loud_scale = loudness.as_ref().map_or(1.0, |l| l.iter().copied().fold(1e-9, f64::max));

    for (index, track) in scene.tracks.iter().enumerate() {
        if track.takes.is_empty() {
            continue;
        }
        println!("\ntrack {index} {:?} ({}): {} takes", track.name, track.puppet, track.takes.len());
        for take in &track.takes {
            println!("  {}", describe(take));
        }
        let slug: String = track.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();

        // Each face take: its decoded curves.
        for take in track.takes.iter().filter(|t| t.groups.contains(&ParamGroup::Head)) {
            let (t0, t1) = (take.start.seconds(), take.end().seconds());
            let pick = |f: &dyn Fn(&charcraft_behaviors::FaceInput) -> f64| -> Vec<(f64, f64)> {
                take.samples.iter().filter_map(|s| Some((t0 + s.t.seconds(), f(s.frame.face.as_ref()?)))).collect()
            };
            let panels: Vec<(&str, Vec<Series<'_>>)> = vec![
                (
                    "head turn (degrees)",
                    vec![
                        Series { name: "yaw", points: pick(&|f| f.head_yaw) },
                        Series { name: "pitch", points: pick(&|f| f.head_pitch) },
                        Series { name: "roll", points: pick(&|f| f.head_roll) },
                    ],
                ),
                (
                    "head position and scale",
                    vec![
                        Series { name: "x", points: pick(&|f| f.head_translation.x) },
                        Series { name: "y", points: pick(&|f| f.head_translation.y) },
                        Series { name: "scale", points: pick(&|f| f.head_scale - 1.0) },
                    ],
                ),
                (
                    "eyes open and brows",
                    vec![
                        Series { name: "eye L", points: pick(&|f| f.eye_open_left) },
                        Series { name: "eye R", points: pick(&|f| f.eye_open_right) },
                        Series { name: "brow L", points: pick(&|f| f.eyebrow_left) },
                        Series { name: "brow R", points: pick(&|f| f.eyebrow_right) },
                    ],
                ),
                (
                    "gaze and mouth",
                    vec![
                        Series { name: "gaze x", points: pick(&|f| f.gaze.x) },
                        Series { name: "gaze y", points: pick(&|f| f.gaze.y) },
                        Series { name: "mouth open", points: pick(&|f| f.mouth_open) },
                        Series { name: "smile", points: pick(&|f| f.smile) },
                    ],
                ),
            ];
            let path = out.join(format!("{prefix}-curves-{}-{slug}-take{}.png", scene.name, take.number));
            sheet(&path, &format!("{} - {} - {}", scene.name, track.name, take.name), (t0, t1), &panels)?;
            println!("  plot {}", path.display());
        }

        // The voice: recorded mouth shapes against the sound.
        let mouth_open: Vec<f64> = ts.iter().map(|&t| inputs_at(track, t).face.map_or(0.0, |f| f.mouth_open)).collect();
        let has_face = ts.iter().any(|&t| inputs_at(track, t).face.is_some());
        let recorded: Vec<Option<Viseme>> = ts.iter().map(|&t| inputs_at(track, t).viseme).collect();
        if recorded.iter().any(Option::is_some) {
            let recorded_open: Vec<f64> = recorded
                .iter()
                .map(|v| v.map_or(0.0, |v| openness(charcraft_puppet::Viseme::from_label(v.ca_layer_name()).unwrap_or(charcraft_puppet::Viseme::Neutral))))
                .collect();
            let neutral_share = recorded.iter().flatten().filter(|v| **v == Viseme::Neutral).count() as f64 / recorded.iter().flatten().count().max(1) as f64;
            println!(
                "  recorded voice: {} of {} sampled moments have a mouth shape, {:.0}% of them neutral",
                recorded.iter().flatten().count(),
                recorded.len(),
                neutral_share * 100.0
            );
            let mut panels: Vec<(&str, Vec<Series<'_>>)> =
                vec![("recorded mouth openness", vec![Series { name: "openness", points: ts.iter().copied().zip(recorded_open.iter().copied()).collect() }])];
            if let Some(loud) = &loudness {
                let ((lag, best), zero) = best_lag(&recorded_open, loud, 5 * RATE_HZ as isize);
                println!("  recorded voice vs the sound's loudness: r = {zero:.3} at lag 0, best r = {best:.3} at lag {:+.2} s", lag as f64 / RATE_HZ);
                panels.push(("sound loudness", vec![Series { name: "rms", points: ts.iter().copied().zip(loud.iter().map(|l| l / loud_scale)).collect() }]));
                // Against the shapes the audio analysis finds.
                if !audio_keys.is_empty() {
                    let analysed: Vec<Viseme> = ts.iter().map(|&t| anim_lipsync::viseme_at(&audio_keys, Tick::from_seconds_f64(t))).collect();
                    let (mut both_silent, mut either_silent, mut same, mut counted) = (0_usize, 0_usize, 0_usize, 0_usize);
                    for (rec, ana) in recorded.iter().zip(&analysed) {
                        let Some(rec) = rec else { continue };
                        counted += 1;
                        let (a, b) = (*rec == Viseme::Neutral, *ana == Viseme::Neutral);
                        both_silent += usize::from(a && b);
                        either_silent += usize::from(a || b);
                        same += usize::from(rec == ana);
                    }
                    println!(
                        "  recorded shapes vs anim-lipsync on the sound: same shape {:.0}%, silent together {:.0}% of the moments either is silent",
                        same as f64 / counted.max(1) as f64 * 100.0,
                        both_silent as f64 / either_silent.max(1) as f64 * 100.0
                    );
                    let analysed_open: Vec<f64> = analysed
                        .iter()
                        .map(|v| openness(charcraft_puppet::Viseme::from_label(v.ca_layer_name()).unwrap_or(charcraft_puppet::Viseme::Neutral)))
                        .collect();
                    let ((alag, abest), azero) = best_lag(&recorded_open, &analysed_open, 5 * RATE_HZ as isize);
                    println!(
                        "  recorded openness vs anim-lipsync openness: r = {azero:.3} at lag 0, best r = {abest:.3} at lag {:+.2} s",
                        alag as f64 / RATE_HZ
                    );
                }
            }
            if has_face && let Some(loud) = &loudness {
                let ((lag, best), zero) = best_lag(&mouth_open, loud, 5 * RATE_HZ as isize);
                println!("  performer's mouth opening vs the sound: r = {zero:.3} at lag 0, best r = {best:.3} at lag {:+.2} s", lag as f64 / RATE_HZ);
                let ((lag, best), zero) = best_lag(&mouth_open, &recorded_open, 5 * RATE_HZ as isize);
                println!(
                    "  performer's mouth opening vs the recorded mouth shapes: r = {zero:.3} at lag 0, best r = {best:.3} at lag {:+.2} s",
                    lag as f64 / RATE_HZ
                );
                panels
                    .push(("performer mouth open", vec![Series { name: "mouth open", points: ts.iter().copied().zip(mouth_open.iter().copied()).collect() }]));
            }
            let path = out.join(format!("{prefix}-voice-{}-{slug}.png", scene.name));
            sheet(&path, &format!("{} - {} - voice against sound", scene.name, track.name), (0.0, duration), &panels)?;
            println!("  plot {}", path.display());
        }
    }
    Ok(())
}
