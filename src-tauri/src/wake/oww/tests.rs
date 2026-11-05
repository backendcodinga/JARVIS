use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jarvis-oww-{tag}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

/// Writes `bytes` to `dir/name` and returns the pin for exactly that.
fn put(dir: &Path, name: &str, bytes: &[u8]) -> Pinned {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    Pinned {
        path,
        bytes: bytes.len() as u64,
        sha256: sha(bytes),
    }
}

/// Four fake files that each match their pin (none is a real model or library).
fn fake_files(dir: &Path) -> OwwFiles {
    OwwFiles {
        runtime: put(dir, "onnxruntime.fake", b"not a library"),
        melspectrogram: put(dir, "melspectrogram.onnx", b"mel"),
        embedding: put(dir, "embedding_model.onnx", b"embedding"),
        classifier: put(dir, "hey_jarvis_v0.1.onnx", b"classifier"),
    }
}

fn err_text(r: Result<OwwDetector, WakeError>) -> String {
    match r {
        Err(e) => e.to_string(),
        Ok(_) => panic!("it loaded"),
    }
}

#[test]
fn a_file_that_is_not_the_pinned_one_is_refused_before_anything_is_loaded() {
    let dir = temp_dir("pins");
    // Wrong content, same size.
    let f = fake_files(&dir);
    std::fs::write(&f.embedding.path, b"EMBEDDING").unwrap();
    assert!(err_text(OwwDetector::load(&f)).contains("embedding_model.onnx"));
    assert!(err_text(OwwDetector::load(&f)).contains("not the verified one"));
    // Wrong size (a file that grew is seen as wrong, not cut short).
    let f2 = fake_files(&dir);
    std::fs::write(&f2.classifier.path, b"classifier plus more").unwrap();
    assert!(err_text(OwwDetector::load(&f2)).contains("wrong size"));
    // Missing.
    std::fs::remove_file(&f2.melspectrogram.path).unwrap();
    assert!(err_text(OwwDetector::load(&f2)).contains("missing"));
    // A folder in place of a file (on fresh files, so the earlier change does not stop it first).
    let mut f3 = fake_files(&dir);
    f3.classifier.path = dir.join("a-folder");
    std::fs::create_dir(&f3.classifier.path).unwrap();
    assert!(err_text(OwwDetector::load(&f3)).contains("not a plain file"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_absolute_paths_are_accepted_so_nothing_is_searched_for() {
    let dir = temp_dir("relative");
    let mut f = fake_files(&dir);
    f.melspectrogram.path = PathBuf::from("melspectrogram.onnx");
    assert!(err_text(OwwDetector::load(&f)).contains("not an absolute path"));
    f.melspectrogram.path = PathBuf::from("../melspectrogram.onnx");
    assert!(err_text(OwwDetector::load(&f)).contains("not an absolute path"));
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_a_file_is_refused_even_if_what_it_points_at_is_right() {
    let dir = temp_dir("link");
    let f = fake_files(&dir);
    let real = dir.join("real-embedding");
    std::fs::write(&real, b"embedding").unwrap();
    std::fs::remove_file(&f.embedding.path).unwrap();
    std::os::unix::fs::symlink(&real, &f.embedding.path).unwrap();
    assert!(err_text(OwwDetector::load(&f)).contains("not a plain file"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_runtime_library_that_does_not_match_is_refused_and_nothing_is_started() {
    let dir = temp_dir("runtime");
    // The models are right; the library is missing, then changed.
    let f = fake_files(&dir);
    std::fs::remove_file(&f.runtime.path).unwrap();
    assert!(err_text(OwwDetector::load(&f)).contains("missing"));
    std::fs::write(&f.runtime.path, b"NOT A LIBRARY").unwrap();
    assert!(err_text(OwwDetector::load(&f)).contains("not the verified one"));
    // Right size and checksum but not a library: ONNX Runtime says so, and nothing crashes.
    std::fs::write(&f.runtime.path, b"not a library").unwrap();
    assert!(err_text(OwwDetector::load(&f)).contains("runtime could not be started"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_error_messages_name_a_file_but_never_a_folder() {
    let dir = temp_dir("names");
    let mut f = fake_files(&dir);
    f.classifier.sha256 = "0".repeat(64);
    let e = err_text(OwwDetector::load(&f));
    assert!(e.contains("hey_jarvis_v0.1.onnx"));
    assert!(!e.contains(dir.to_string_lossy().as_ref()), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

// ---- With the real files. Run with
// `JARVIS_TEST_WAKE_DIR=<folder> cargo test -- --ignored`, where the folder holds `real/` with
// `libonnxruntime.so` (or the dll or dylib of this system) and the three `.onnx` files.

fn real_pin(name: &str) -> Pinned {
    let dir =
        PathBuf::from(std::env::var("JARVIS_TEST_WAKE_DIR").expect("set JARVIS_TEST_WAKE_DIR"));
    let path = dir.join("real").join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Pinned {
        path,
        bytes: bytes.len() as u64,
        sha256: sha(&bytes),
    }
}

fn runtime_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn real_files() -> OwwFiles {
    OwwFiles {
        runtime: real_pin(runtime_name()),
        melspectrogram: real_pin("melspectrogram.onnx"),
        embedding: real_pin("embedding_model.onnx"),
        classifier: real_pin("hey_jarvis_v0.1.onnx"),
    }
}

/// Deterministic noise, so the test is repeatable.
fn noise(seconds: usize, amp: f32) -> Vec<f32> {
    let mut x: u32 = 0x1234_5678;
    (0..16_000 * seconds)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ((x as f32 / u32::MAX as f32) * 2.0 - 1.0) * amp
        })
        .collect()
}

#[test]
#[ignore = "needs the real ONNX Runtime library and the openWakeWord models"]
fn the_real_detector_scores_silence_and_noise_low_and_never_triggers() {
    use crate::wake::trigger::Trigger;
    let mut d = OwwDetector::load(&real_files()).unwrap_or_else(|e| panic!("{e}"));
    for (what, audio) in [
        ("silence", vec![0.0f32; 16_000 * 12]),
        ("noise", noise(12, 0.3)),
    ] {
        d.reset();
        let scores = d.feed(&audio).unwrap();
        // 12 s is 150 hops; scoring starts after about 2 s of audio (16 embeddings).
        assert!(
            scores.len() > 100 && scores.len() < 150,
            "{what}: {}",
            scores.len()
        );
        assert!(
            scores.iter().all(|s| (0.0..=1.0).contains(s)),
            "{what}: out of range"
        );
        let peak = scores.iter().copied().fold(0.0f32, f32::max);
        assert!(peak < 0.5, "{what}: peak {peak}");
        let mut t = Trigger::default();
        assert!(!scores.iter().any(|s| t.push(*s)), "{what} triggered");
    }
}

#[test]
#[ignore = "needs the real ONNX Runtime library and the openWakeWord models"]
fn the_real_detector_gives_the_same_scores_however_the_audio_is_cut_and_after_a_reset() {
    let mut d = OwwDetector::load(&real_files()).unwrap_or_else(|e| panic!("{e}"));
    let audio = noise(8, 0.2);
    let whole = d.feed(&audio).unwrap();
    d.reset();
    let mut pieces = Vec::new();
    for chunk in audio.chunks(333) {
        pieces.extend(d.feed(chunk).unwrap());
    }
    assert_eq!(whole, pieces, "the cut of the audio changed the scores");
    d.reset();
    assert_eq!(whole, d.feed(&audio).unwrap(), "a reset did not start over");
}

#[test]
#[ignore = "needs the real ONNX Runtime library and the openWakeWord models"]
fn the_real_runtime_starts_once_and_refuses_to_be_swapped_for_another_file() {
    let f = real_files();
    OwwDetector::load(&f).unwrap_or_else(|e| panic!("{e}"));
    // The same file again is fine.
    OwwDetector::load(&f).unwrap_or_else(|e| panic!("{e}"));
    // A copy of it at another path is refused: it cannot be swapped under running models.
    let dir = temp_dir("swap");
    let copy = dir.join(runtime_name());
    std::fs::copy(&f.runtime.path, &copy).unwrap();
    let mut other = f.clone();
    other.runtime.path = copy;
    assert!(err_text(OwwDetector::load(&other)).contains("another file"));
    // A tampered model is found before anything runs.
    let mut tampered = std::fs::read(&f.classifier.path).unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    let mut t = f.clone();
    t.classifier.path = dir.join("hey_jarvis_v0.1.onnx");
    std::fs::write(&t.classifier.path, tampered).unwrap();
    assert!(err_text(OwwDetector::load(&t)).contains("not the verified one"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "needs the real ONNX Runtime library and the openWakeWord models"]
fn the_real_detector_costs_well_under_a_hop_of_time_per_hop() {
    let mut d = OwwDetector::load(&real_files()).unwrap_or_else(|e| panic!("{e}"));
    // Warm up past the first 2 s, then time 10 s of audio (125 hops of 80 ms).
    d.feed(&noise(3, 0.1)).unwrap();
    let audio = noise(10, 0.1);
    let began = std::time::Instant::now();
    let scores = d.feed(&audio).unwrap();
    let per_hop = began.elapsed().as_secs_f64() * 1000.0 / scores.len() as f64;
    // The standby budget is about 2% of one thread (spec §10): one hop is 80 ms of audio, so under
    // 1.6 ms per hop. This prints the real number; the assertion only catches a disaster.
    println!(
        "wake word: {per_hop:.2} ms per 80 ms hop ({:.1}% of one thread)",
        per_hop / 80.0 * 100.0
    );
    assert!(per_hop < 40.0, "{per_hop} ms per hop");
}
