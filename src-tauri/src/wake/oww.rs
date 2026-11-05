//! openWakeWord's pretrained "hey jarvis" model, run through ONNX Runtime (D-29).
//!
//! Pipeline (as in openWakeWord's `AudioFeatures`, and as proven in the S1 spike): 16 kHz audio in
//! 1280-sample hops (80 ms) -> the mel-spectrogram model over the hop plus 480 samples of context
//! (8 new frames) -> scaled `x / 10 + 2` -> the embedding model over the last 76 frames (a vector
//! of 96 numbers) -> the classifier over the last 16 embeddings (about 1.3 s) -> one score between
//! 0 and 1 per hop.
//!
//! **What gets loaded, and how** (the rule written into D-29): only the files the model manager
//! installed and verified, each given as an **absolute path** with its pinned size and SHA-256.
//! Nothing is searched for (not `PATH`, not the current folder, not the program's folder). Each
//! file must be a plain file (not a link), of the pinned size, and its SHA-256 is checked **right
//! before it is used**. The three models are built from the very bytes that were hashed, so the
//! file cannot change between the check and the use. The runtime library has to be opened by path;
//! it is checked immediately before, and the window that remains needs write access to the app's
//! own data folder. If anything does not match, the wake word stays off with a plain error.
//! Nothing here writes a file or records audio.

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const HOP: usize = 1280;
const CONTEXT: usize = 480;
const MEL_BANDS: usize = 32;
const MEL_WINDOW: usize = 76;
const EMBED: usize = 96;
const CLASSIFIER_WINDOW: usize = 16;
/// Never read more than this from a file we are about to load (the models are a few MB).
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WakeError {
    /// A file is missing, is not a plain file, is a link, or is not the pinned size or content.
    #[error("a wake word file is not what was installed: {0}")]
    Changed(String),
    #[error("the wake word runtime could not be started: {0}")]
    Runtime(String),
    #[error("a wake word model could not be loaded: {0}")]
    Model(String),
    #[error("the wake word detector failed: {0}")]
    Run(String),
}

/// A file to load with what it must be.
#[derive(Debug, Clone)]
pub struct Pinned {
    /// Absolute.
    pub path: PathBuf,
    pub bytes: u64,
    /// Lower-case hex SHA-256.
    pub sha256: String,
}

/// The four files the detector needs.
#[derive(Debug, Clone)]
pub struct OwwFiles {
    /// `onnxruntime.dll`, `libonnxruntime.dylib` or `libonnxruntime.so`.
    pub runtime: Pinned,
    pub melspectrogram: Pinned,
    pub embedding: Pinned,
    pub classifier: Pinned,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a file".into())
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks that `p` is an absolute path to a plain file of the pinned size.
fn check_plain_file(p: &Pinned) -> Result<(), WakeError> {
    let changed = |why: &str| WakeError::Changed(format!("{}: {why}", name_of(&p.path)));
    if !p.path.is_absolute() {
        return Err(changed("not an absolute path"));
    }
    let meta = std::fs::symlink_metadata(&p.path).map_err(|_| changed("it is missing"))?;
    if !meta.is_file() {
        return Err(changed("it is not a plain file"));
    }
    if meta.len() != p.bytes {
        return Err(changed("it has the wrong size"));
    }
    Ok(())
}

/// Reads a pinned file once, checks it, and returns the checked bytes.
fn read_verified(p: &Pinned) -> Result<Vec<u8>, WakeError> {
    check_plain_file(p)?;
    if p.bytes > MAX_MODEL_BYTES {
        return Err(WakeError::Changed(format!(
            "{}: it is larger than a model can be",
            name_of(&p.path)
        )));
    }
    let mut f = std::fs::File::open(&p.path)
        .map_err(|_| WakeError::Changed(format!("{}: it cannot be read", name_of(&p.path))))?;
    let mut bytes = Vec::with_capacity(p.bytes as usize);
    // One byte more than pinned, so a file that grew is seen as wrong and not cut short.
    (&mut f)
        .take(p.bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| WakeError::Changed(format!("{}: it cannot be read", name_of(&p.path))))?;
    if bytes.len() as u64 != p.bytes || hex(&Sha256::digest(&bytes)) != p.sha256 {
        return Err(WakeError::Changed(format!(
            "{}: its content is not the verified one",
            name_of(&p.path)
        )));
    }
    Ok(bytes)
}

/// Checks the runtime library by streaming it through SHA-256 (it is tens of MB).
fn verify_library(p: &Pinned) -> Result<(), WakeError> {
    check_plain_file(p)?;
    let changed = || {
        WakeError::Changed(format!(
            "{}: its content is not the verified one",
            name_of(&p.path)
        ))
    };
    let mut f = std::fs::File::open(&p.path)
        .map_err(|_| WakeError::Changed(format!("{}: it cannot be read", name_of(&p.path))))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|_| changed())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    if hex(&h.finalize()) != p.sha256 {
        return Err(changed());
    }
    Ok(())
}

/// ONNX Runtime is started once per process, from one file. A later request for the same file
/// reuses it; a request for another file is refused (it cannot be swapped under running models).
static RUNTIME: OnceLock<(PathBuf, &'static Environment)> = OnceLock::new();
static RUNTIME_START: Mutex<()> = Mutex::new(());

fn runtime(lib: &Pinned) -> Result<&'static Environment, WakeError> {
    let _one_at_a_time = RUNTIME_START.lock().unwrap_or_else(|e| e.into_inner());
    verify_library(lib)?;
    if let Some((path, env)) = RUNTIME.get() {
        return if *path == lib.path {
            Ok(env)
        } else {
            Err(WakeError::Runtime(
                "it is already running from another file".into(),
            ))
        };
    }
    // On Windows the library is opened first with a search that is limited to its own folder and
    // System32 (`ort` opens it with no flags, which would also search the current folder and
    // `PATH`); `ort`'s own open of the same file then reuses it. Nothing to do elsewhere.
    crate::platform::dll::preload_restricted(&lib.path)
        .map_err(|e| WakeError::Runtime(format!("cannot open {}: {e}", name_of(&lib.path))))?;
    let env = ort::init_from(&lib.path)
        .map_err(|e| WakeError::Runtime(format!("cannot load {}: {e}", name_of(&lib.path))))?
        .build()
        .map_err(|e| WakeError::Runtime(e.to_string()))?;
    // Kept for the life of the process: the sessions borrow it, and it cannot be unloaded safely.
    let env: &'static Environment = Box::leak(Box::new(env));
    let _ = RUNTIME.set((lib.path.clone(), env));
    Ok(env)
}

fn load(env: &Environment, bytes: &[u8], what: &str) -> Result<Session, WakeError> {
    let fail = |e: &dyn std::fmt::Display| WakeError::Model(format!("{what}: {e}"));
    // These models are tiny and run every 80 ms: one thread each, so standby never wakes a pool of
    // worker threads and never competes with the rest of the app.
    let b = Session::builder(env).map_err(|e| fail(&e))?;
    let b = b.with_intra_threads(1).map_err(|e| fail(&e))?;
    let mut b = b.with_inter_threads(1).map_err(|e| fail(&e))?;
    b.commit_from_memory(bytes).map_err(|e| fail(&e))
}

/// Runs a session with one `f32` input of the given shape and returns its first output, flat.
fn run(s: &mut Session, shape: Vec<i64>, data: Vec<f32>) -> Result<Vec<f32>, WakeError> {
    let fail = |e: &dyn std::fmt::Display| WakeError::Run(e.to_string());
    let name = s.inputs()[0].name().to_string();
    let out_name = s.outputs()[0].name().to_string();
    let tensor = Tensor::from_array((shape, data)).map_err(|e| fail(&e))?;
    let outputs = s
        .run(ort::inputs![name.as_str() => tensor])
        .map_err(|e| fail(&e))?;
    let (_, values) = outputs[out_name.as_str()]
        .try_extract_tensor::<f32>()
        .map_err(|e| fail(&e))?;
    Ok(values.to_vec())
}

/// The detector. One per listening session; not shared between threads (runs take `&mut`).
pub struct OwwDetector {
    mel: Session,
    embedding: Session,
    classifier: Session,
    pending: Vec<f32>,
    raw: VecDeque<f32>,
    frames: VecDeque<[f32; MEL_BANDS]>,
    features: VecDeque<[f32; EMBED]>,
}

impl OwwDetector {
    /// Checks every file against its pin, starts the runtime, and loads the three models from the
    /// checked bytes. Blocking: call it from a thread that may take a moment, never from the audio
    /// callback or a Tokio worker.
    pub fn load(files: &OwwFiles) -> Result<Self, WakeError> {
        // Every file is checked before anything is started, so a changed model is found before the
        // runtime is touched.
        let mel_bytes = read_verified(&files.melspectrogram)?;
        let embed_bytes = read_verified(&files.embedding)?;
        let class_bytes = read_verified(&files.classifier)?;
        let env = runtime(&files.runtime)?;
        let mut d = Self {
            mel: load(env, &mel_bytes, "the spectrogram model")?,
            embedding: load(env, &embed_bytes, "the embedding model")?,
            classifier: load(env, &class_bytes, "the wake word model")?,
            pending: Vec::new(),
            raw: VecDeque::new(),
            frames: VecDeque::new(),
            features: VecDeque::new(),
        };
        d.reset();
        Ok(d)
    }

    /// Forgets everything heard: a new listening session.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.raw = std::iter::repeat_n(0.0, CONTEXT).collect();
        self.frames.clear();
        self.features.clear();
    }

    /// Feeds audio (16 kHz mono, -1..1, any length) and returns one score per complete 80 ms hop.
    /// A score is produced only once enough audio has been heard (about 2 s after a reset).
    pub fn feed(&mut self, samples: &[f32]) -> Result<Vec<f32>, WakeError> {
        self.pending.extend_from_slice(samples);
        let mut scores = Vec::new();
        while self.pending.len() >= HOP {
            let hop: Vec<f32> = self.pending.drain(..HOP).collect();
            self.raw.extend(hop);
            while self.raw.len() > HOP + CONTEXT {
                self.raw.pop_front();
            }
            // The mel model takes raw 16-bit sample values as floats, not -1..1.
            let input: Vec<f32> = self.raw.iter().map(|s| s * 32767.0).collect();
            let n = input.len() as i64;
            let mel = run(&mut self.mel, vec![1, n], input)?;
            for f in mel.as_chunks::<MEL_BANDS>().0 {
                let mut row = [0.0f32; MEL_BANDS];
                for (r, v) in row.iter_mut().zip(f) {
                    *r = v / 10.0 + 2.0;
                }
                self.frames.push_back(row);
            }
            while self.frames.len() > 400 {
                self.frames.pop_front();
            }
            if self.frames.len() >= MEL_WINDOW {
                let window: Vec<f32> = self
                    .frames
                    .iter()
                    .skip(self.frames.len() - MEL_WINDOW)
                    .flat_map(|f| f.iter().copied())
                    .collect();
                let e = run(
                    &mut self.embedding,
                    vec![1, MEL_WINDOW as i64, MEL_BANDS as i64, 1],
                    window,
                )?;
                if e.len() < EMBED {
                    return Err(WakeError::Run(format!(
                        "the embedding model returned {} numbers",
                        e.len()
                    )));
                }
                let mut row = [0.0f32; EMBED];
                row.copy_from_slice(&e[..EMBED]);
                self.features.push_back(row);
                while self.features.len() > 64 {
                    self.features.pop_front();
                }
            }
            if self.features.len() >= CLASSIFIER_WINDOW {
                let window: Vec<f32> = self
                    .features
                    .iter()
                    .skip(self.features.len() - CLASSIFIER_WINDOW)
                    .flat_map(|f| f.iter().copied())
                    .collect();
                let out = run(
                    &mut self.classifier,
                    vec![1, CLASSIFIER_WINDOW as i64, EMBED as i64],
                    window,
                )?;
                scores.push(out.first().copied().unwrap_or(0.0));
            }
        }
        Ok(scores)
    }
}

#[cfg(test)]
mod tests;
