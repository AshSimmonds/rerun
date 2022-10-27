//! This build script implements the second half of our cross-platform shader #import system.
//! The first half can be found in `src/file_resolver.rs`.
//!
//! It finds all WGSL shaders defined anywhere within our Cargo workspace, and embeds them
//! directly into the released artifact for our `re_renderer` library.
//!
//! At run-time, for release builds only, those shaders will be available through an hermetic
//! virtual filesystem.
//! To the user, it will look like business as usual.
//!
//! See `re_renderer/src/workspace_shaders.rs` for the end result.

// TODO(cmc): crash the build if someone is trying to create a non-hermetic artifact (e.g. one
// of the embedded shaders refer to stuff outside the repository).

use std::{path::Path, process::Command};
use walkdir::{DirEntry, WalkDir};

// ---

// Mapping to cargo:rerun-if-changed with glob support
fn rerun_if_changed(path: &str) {
    for path in glob::glob(path).unwrap() {
        println!("cargo:rerun-if-changed={}", path.unwrap().to_string_lossy());
    }
}

// ---

fn main() {
    // Don't run on CI!
    //
    // The code we're generating here is actual source code that gets committed into the
    // repository.
    if std::env::var("CI").is_ok() {
        return;
    }

    let root_path = Path::new(&std::env::var("CARGO_WORKSPACE_DIR").unwrap()).to_owned();
    let manifest_path = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_owned();
    let src_path = manifest_path.join("src");
    let file_path = src_path.join("workspace_shaders.rs");

    fn is_wgsl_or_dir(entry: &DirEntry) -> bool {
        let is_dir = entry.file_type().is_dir();
        let is_wgsl = entry
            .file_name()
            .to_str()
            .map_or(false, |s| s.ends_with(".wgsl"));
        is_dir || is_wgsl
    }

    let mut contents = r#"
        // This file is autogenerated via build.rs.
        // DO NOT EDIT.

        static ONCE: ::std::sync::atomic::AtomicBool = ::std::sync::atomic::AtomicBool::new(false);

        pub fn init() {
            if ONCE.swap(true, ::std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            use crate::file_system::FileSystem as _;
            let fs = crate::MemFileSystem::get();
        "#
    .to_owned();

    let walker = WalkDir::new(&root_path).into_iter();
    let entries = {
        let mut entries = walker
            .filter_entry(is_wgsl_or_dir)
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.path().cmp(b.path()));
        entries
    };

    for entry in entries {
        rerun_if_changed(entry.path().to_string_lossy().as_ref());

        // The relative path to get from the current shader file to `workspace_shaders.rs`.
        // We must make sure to pass relative paths to `include_str`!
        let relpath = pathdiff::diff_paths(entry.path(), &src_path).unwrap();
        // The hermetic path used in the virtual filesystem at run-time.
        //
        // This is using the exact same strip_prefix as the standard `file!()` macro, so that
        // hermetic paths generated by one will be comparable with the hermetic paths generated
        // by the other!
        let virtpath = entry.path().strip_prefix(&root_path).unwrap();

        contents += &format!(
            "
            {{
                let virtpath = ::std::path::Path::new({virtpath:?});
                fs.create_file(&virtpath, include_str!({relpath:?}).into()).unwrap();
            }}
            ",
        );
    }

    contents += "}";

    std::fs::write(&file_path, contents).unwrap();

    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
        .args(["fmt", "--", file_path.to_string_lossy().as_ref()])
        .output()
        .expect("failed to execute process");

    eprintln!("status: {}", output.status);
    eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));

    assert!(output.status.success());
}