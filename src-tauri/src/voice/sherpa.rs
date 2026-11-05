//! Speech to text with Moonshine through the `sherpa-onnx` crate (D-04, D-26).
//!
//! The model files are not part of the installer. They live in a folder the app is told about, and
//! are loaded the first time something is said (so a start-up never pays for the model, and an app
//! that never uses voice never holds it in memory). Until the files are there every request says
//! "The speech models are not installed yet."
//!
//! Runs only on the voice recognition thread (`VoiceController::transcribe_bounded`), never on a
//! Tokio worker, and never writes the audio anywhere.

use super::transcribe::{Transcriber, VoiceError};
use sherpa_onnx::{OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig};
use std::path::{Path, PathBuf};

/// The sample rate the capture engine delivers and the model expects.
const RATE: i32 = 16_000;

/// First file in `dir` whose lower-case name contains every `has` part, none of `not`, and ends
/// with `ext`. An `int8` file wins over a full-precision one. The sherpa-onnx release archives
/// differ a little in file names between model versions, so names are matched by pattern.
fn pick(dir: &Path, has: &[&str], not: &[&str], ext: &str) -> Option<String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .is_some_and(|n| {
                    n.ends_with(ext)
                        && has.iter().all(|h| n.contains(h))
                        && !not.iter().any(|x| n.contains(x))
                })
        })
        .collect();
    found.sort_by_key(|p| {
        let int8 = p.to_string_lossy().to_lowercase().contains("int8");
        (!int8, p.clone())
    });
    found
        .into_iter()
        .next()
        .map(|p| p.to_string_lossy().into_owned())
}

/// The Moonshine files found in a folder, before anything is loaded.
#[derive(Debug, PartialEq, Eq)]
struct Files {
    tokens: String,
    parts: Parts,
}

#[derive(Debug, PartialEq, Eq)]
enum Parts {
    /// Newer archives: one encoder and one merged decoder.
    Merged { encoder: String, decoder: String },
    /// The first release: preprocessor, encoder, and two decoders.
    Split {
        preprocessor: String,
        encoder: String,
        uncached: String,
        cached: String,
    },
}

/// Finds the files, or says which kind of problem it is: no folder or no tokens file means the
/// models are not installed; a folder with some files but not a whole model is a failure the
/// person can fix by downloading again.
fn find_files(dir: &Path) -> Result<Files, VoiceError> {
    if !dir.is_dir() {
        return Err(VoiceError::ModelMissing);
    }
    let Some(tokens) = pick(dir, &["tokens"], &[], ".txt") else {
        return Err(VoiceError::ModelMissing);
    };
    let incomplete =
        || VoiceError::Failed("the speech model folder is incomplete; download it again".into());
    let parts = if let Some(decoder) = pick(dir, &["merged"], &[], ".onnx") {
        Parts::Merged {
            encoder: pick(dir, &["encode"], &["merged"], ".onnx").ok_or_else(incomplete)?,
            decoder,
        }
    } else {
        Parts::Split {
            preprocessor: pick(dir, &["preprocess"], &[], ".onnx").ok_or_else(incomplete)?,
            encoder: pick(dir, &["encode"], &[], ".onnx").ok_or_else(incomplete)?,
            uncached: pick(dir, &["uncached"], &[], ".onnx").ok_or_else(incomplete)?,
            cached: pick(dir, &["cached"], &["uncached"], ".onnx").ok_or_else(incomplete)?,
        }
    };
    Ok(Files { tokens, parts })
}

fn config(f: Files, threads: i32) -> OfflineRecognizerConfig {
    let mut m = OfflineModelConfig {
        num_threads: threads,
        provider: Some("cpu".into()),
        tokens: Some(f.tokens),
        ..Default::default()
    };
    match f.parts {
        Parts::Merged { encoder, decoder } => {
            m.moonshine.encoder = Some(encoder);
            m.moonshine.merged_decoder = Some(decoder);
        }
        Parts::Split {
            preprocessor,
            encoder,
            uncached,
            cached,
        } => {
            m.moonshine.preprocessor = Some(preprocessor);
            m.moonshine.encoder = Some(encoder);
            m.moonshine.uncached_decoder = Some(uncached);
            m.moonshine.cached_decoder = Some(cached);
        }
    }
    OfflineRecognizerConfig {
        model_config: m,
        ..Default::default()
    }
}

/// Moonshine, loaded on first use from `dir`.
pub struct SherpaTranscriber {
    dir: PathBuf,
    threads: i32,
    recogniser: Option<OfflineRecognizer>,
}

impl SherpaTranscriber {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            // Two threads is plenty for a short utterance and leaves cores for the rest of the app.
            threads: 2,
            recogniser: None,
        }
    }

    fn load(&self) -> Result<OfflineRecognizer, VoiceError> {
        let files = find_files(&self.dir)?;
        OfflineRecognizer::create(&config(files, self.threads))
            .ok_or_else(|| VoiceError::Failed("the speech model could not be loaded".into()))
    }
}

impl Transcriber for SherpaTranscriber {
    fn transcribe(&mut self, audio: &[f32]) -> Result<String, VoiceError> {
        if self.recogniser.is_none() {
            // A model that is not there yet is checked again on every request, so installing it
            // does not need a restart.
            self.recogniser = Some(self.load()?);
        }
        let Some(rec) = self.recogniser.as_ref() else {
            return Err(VoiceError::ModelMissing);
        };
        let stream = rec.create_stream();
        stream.accept_waveform(RATE, audio);
        rec.decode(&stream);
        Ok(stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-stt-{tag}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(d: &Path, names: &[&str]) {
        for n in names {
            std::fs::write(d.join(n), b"x").unwrap();
        }
    }

    #[test]
    fn no_folder_or_no_tokens_means_the_models_are_not_installed() {
        let missing = std::env::temp_dir().join("jarvis-stt-does-not-exist");
        assert_eq!(find_files(&missing), Err(VoiceError::ModelMissing));
        let empty = dir("empty");
        assert_eq!(find_files(&empty), Err(VoiceError::ModelMissing));
        let mut t = SherpaTranscriber::new(missing);
        assert_eq!(t.transcribe(&[0.0; 160]), Err(VoiceError::ModelMissing));
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn a_partly_downloaded_model_is_a_failure_that_says_so_not_a_crash() {
        let d = dir("partial");
        touch(&d, &["tokens.txt", "encode.int8.onnx"]);
        match find_files(&d) {
            Err(VoiceError::Failed(m)) => assert!(m.contains("incomplete")),
            other => panic!("{other:?}"),
        }
        let mut t = SherpaTranscriber::new(d.clone());
        assert!(matches!(
            t.transcribe(&[0.0; 160]),
            Err(VoiceError::Failed(_))
        ));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn both_moonshine_layouts_are_found_and_int8_wins() {
        let split = dir("split");
        touch(
            &split,
            &[
                "tokens.txt",
                "preprocess.onnx",
                "encode.onnx",
                "encode.int8.onnx",
                "uncached_decode.int8.onnx",
                "cached_decode.int8.onnx",
            ],
        );
        let f = find_files(&split).unwrap();
        match f.parts {
            Parts::Split {
                encoder,
                uncached,
                cached,
                ..
            } => {
                assert!(encoder.ends_with("encode.int8.onnx"), "{encoder}");
                assert!(uncached.ends_with("uncached_decode.int8.onnx"));
                assert!(cached.ends_with("cached_decode.int8.onnx"));
                assert!(!cached.contains("uncached"));
            }
            other => panic!("{other:?}"),
        }
        let merged = dir("merged");
        touch(
            &merged,
            &[
                "tokens.txt",
                "encoder_model.onnx",
                "decoder_model_merged.onnx",
            ],
        );
        match find_files(&merged).unwrap().parts {
            Parts::Merged { encoder, decoder } => {
                assert!(encoder.ends_with("encoder_model.onnx"));
                assert!(decoder.ends_with("decoder_model_merged.onnx"));
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(split);
        let _ = std::fs::remove_dir_all(merged);
    }

    /// Needs the real model: `JARVIS_TEST_MOONSHINE_DIR=<folder> cargo test -- --ignored`.
    /// A second of silence must give an answer (empty text is fine), not an error or a crash.
    #[test]
    #[ignore = "needs the Moonshine model files"]
    fn the_real_model_answers_for_silence() {
        let Ok(d) = std::env::var("JARVIS_TEST_MOONSHINE_DIR") else {
            panic!("set JARVIS_TEST_MOONSHINE_DIR to the model folder");
        };
        let mut t = SherpaTranscriber::new(PathBuf::from(d));
        assert!(t.transcribe(&vec![0.0; 16_000]).is_ok());
    }

    /// The model's own sample recording, `test_wavs/0.wav` (16 kHz, 16-bit mono), against the
    /// sentence it ships with in `test_wavs/trans.txt`. Same environment variable.
    #[test]
    #[ignore = "needs the Moonshine model files"]
    fn the_real_model_hears_its_own_sample_sentence() {
        let Ok(d) = std::env::var("JARVIS_TEST_MOONSHINE_DIR") else {
            panic!("set JARVIS_TEST_MOONSHINE_DIR to the model folder");
        };
        let d = PathBuf::from(d);
        let bytes = std::fs::read(d.join("test_wavs/0.wav")).unwrap();
        // A plain 44-byte header, then 16-bit little-endian samples.
        let pcm = &bytes[44..];
        let audio: Vec<f32> = (0..pcm.len() / 2)
            .map(|i| i16::from_le_bytes([pcm[2 * i], pcm[2 * i + 1]]) as f32 / 32768.0)
            .collect();
        let expected = std::fs::read_to_string(d.join("test_wavs/trans.txt")).unwrap();
        let expected = expected
            .lines()
            .next()
            .unwrap()
            .split_once(' ')
            .map(|(_, t)| t)
            .unwrap_or_default()
            .to_lowercase();
        let mut t = SherpaTranscriber::new(d);
        let got = t.transcribe(&audio).unwrap().to_lowercase();
        let words = |s: &str| {
            s.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(words(&got), words(&expected), "heard {got:?}");
    }
}
