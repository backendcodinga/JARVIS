//! The wake word (M3, D-29): openWakeWord's pretrained "hey jarvis" through ONNX Runtime.
//!
//! This is the detector only (step 2): it turns audio into a score per 80 ms and decides when the
//! phrase was said. Nothing here opens a microphone or starts a turn (step 3), and nothing is
//! downloaded here (the model manager does that, D-28). Nothing here records audio: it works on
//! the sound it is handed and keeps no more than about two seconds of derived numbers.
//!
//! The model is licensed CC BY-NC-SA 4.0, for personal non-commercial use by the owner and
//! Developer B only (D-23, D-29); it is never in the installer.

// Used by the standby engine in the next step; until then only the tests call it.
#[allow(dead_code)]
pub mod oww;
#[allow(dead_code)]
pub mod trigger;
