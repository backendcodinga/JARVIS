//! Opening a native library from one known file, the safe way (D-29).
//!
//! The wake word's ONNX Runtime library is opened by the `ort` crate through `libloading`, which on
//! Windows calls `LoadLibraryExW` with **no flags**. With no flags Windows looks for the library's
//! own dependencies in the usual places, which include the current folder and `PATH`: a planted
//! DLL there could be picked up. So the library is opened here **first**, with
//! `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`: its dependencies are looked for
//! in **its own folder and System32 only**. Once it is loaded, the later open of the same file by
//! `ort` returns the module that is already there, so nothing is searched again.
//!
//! The other systems search nothing for a library opened by absolute path, so there is nothing to
//! restrict there.

use std::path::Path;

/// Loads the library at `path` (absolute) so that its dependencies come only from its own folder
/// and System32, and keeps it loaded for the life of the process. Blocking.
#[cfg(windows)]
pub fn preload_restricted(path: &Path) -> Result<(), String> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
    };
    if !path.is_absolute() {
        return Err("it is not an absolute path".into());
    }
    // SAFETY: the file was checked against its pinned SHA-256 immediately before (wake/oww.rs), and
    // running a library's start-up code is the point of loading it.
    let lib = unsafe {
        Library::load_with_flags(
            path,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    }
    .map_err(|e| e.to_string())?;
    // It stays loaded, as the runtime does: unloading it under running models is not safe.
    std::mem::forget(lib);
    Ok(())
}

#[cfg(not(windows))]
pub fn preload_restricted(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("it is not an absolute path".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_refused_on_every_system() {
        assert!(preload_restricted(Path::new("onnxruntime.dll")).is_err());
        assert!(preload_restricted(Path::new("../onnxruntime.dll")).is_err());
    }

    /// Runs on the Windows CI runner: a real `LoadLibraryExW` with the restricted flags.
    #[cfg(windows)]
    #[test]
    fn a_system_library_loads_with_the_restricted_search_and_a_file_that_is_not_one_does_not() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let kernel = Path::new(&root).join("System32").join("version.dll");
        preload_restricted(&kernel).unwrap_or_else(|e| panic!("{e}"));
        let fake =
            std::env::temp_dir().join(format!("jarvis-not-a-dll-{}.dll", uuid::Uuid::now_v7()));
        std::fs::write(&fake, b"this is not a library").unwrap();
        assert!(preload_restricted(&fake).is_err());
        let _ = std::fs::remove_file(fake);
    }
}
