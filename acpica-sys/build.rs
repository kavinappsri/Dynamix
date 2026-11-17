//! Compiles the vendored ACPICA and generates its Rust bindings.
//!
//! Needs `clang` (compiling for UEFI means COFF objects and the Windows x64
//! ABI, which gcc cannot produce) and `libclang` for bindgen. Set `CC` /
//! `CC_x86_64_unknown_uefi` to pick a specific clang, `LIBCLANG_PATH` to pick
//! a specific libclang.
//!
//! The archive is built with `llvm-ar` (found next to the clang, or on PATH)
//! rather than the host `ar`: Apple's `ar` cannot index COFF objects, which
//! leaves the archive without a symbol table and the link fails with
//! "undefined symbol: Acpi...". Set `AR_x86_64_unknown_uefi` to override.
//!
//! The source list is every `.c` file in the nine ACPICA component
//! directories (see scripts/vendor_acpica.sh for what is vendored and why)
//! minus `SKIP`, plus our own `c/osl_printf.c`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const COMPONENTS: &[&str] = &[
    "tables", "namespace", "parser", "dispatcher", "executer", "events", "hardware", "resources", "utilities",
];

/// Debug-dump helpers; they only compile with ACPI_DEBUG_OUTPUT/ACPI_DEBUGGER.
const SKIP: &[&str] = &["rsdump.c", "rsdumpinfo.c"];

/// Defines every ACPICA translation unit (and bindgen) is built with.
const DEFINES: &[&str] = &[
    // Selects our platform header (c/acdynamix.h); see third_party/acpica-dynamix.patch.
    "_DYNAMIX",
    // Compiler-provided va_list, so no C library header is needed at all.
    "ACPI_USE_BUILTIN_STDARG",
];

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    println!("cargo::rerun-if-changed=build.rs");

    // ACPI is only used on x86_64; elsewhere this crate is an empty shell, so
    // the aarch64/armv7 builds need neither clang nor libclang.
    if arch != "x86_64" {
        fs::write(out_dir.join("bindings.rs"), "").unwrap();
        fs::write(out_dir.join("status_codes.rs"), "").unwrap();
        return;
    }

    let acpica = manifest.join("../third_party/acpica/source");
    if !acpica.join("include/acpi.h").exists() {
        panic!("third_party/acpica is missing; run scripts/vendor_acpica.sh from the repository root");
    }
    let include_dirs = [acpica.join("include"), manifest.join("c")];

    println!("cargo::rerun-if-changed=c");
    println!("cargo::rerun-if-changed=wrapper.h");
    println!("cargo::rerun-if-changed=../third_party/acpica");

    build_c(&acpica, &manifest, &include_dirs);
    generate_bindings(&manifest, &out_dir, &include_dirs);
    generate_status_codes(&acpica, &out_dir);
}

fn build_c(acpica: &Path, manifest: &Path, include_dirs: &[PathBuf]) {
    let mut files = vec![manifest.join("c/osl_printf.c")];
    for component in COMPONENTS {
        let dir = acpica.join("components").join(component);
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if name.ends_with(".c") && !SKIP.contains(&name) {
                files.push(path);
            }
        }
    }
    files.sort(); // stable order, reproducible archive

    let mut build = cc::Build::new();
    build.files(&files).warnings(false).extra_warnings(false);
    for dir in include_dirs {
        build.include(dir);
    }
    for define in DEFINES {
        build.define(define, None);
    }
    build
        // The kernel is freestanding: no hosted headers/builtins, and no
        // stack protector (there is no runtime to provide the canary).
        .flag("-ffreestanding")
        .flag("-fno-stack-protector")
        .flag("-mno-red-zone")
        // Lets the linker drop whatever the kernel never calls.
        .flag("-ffunction-sections")
        .flag("-fdata-sections");

    // gcc cannot target UEFI, and `cc` would otherwise pick the system
    // compiler. An explicit CC still wins.
    let target = env::var("TARGET").unwrap();
    let underscored = target.replace('-', "_");
    let overridden = [format!("CC_{target}"), format!("CC_{underscored}"), "TARGET_CC".to_string(), "CC".to_string()]
        .iter()
        .any(|var| env::var_os(var).is_some());
    if !overridden {
        build.compiler("clang");
    }

    pick_archiver(&mut build, &target, &underscored);

    build.compile("acpica");
    check_archive_index(&PathBuf::from(env::var("OUT_DIR").unwrap()).join("libacpica.a"));
}

/// Use `llvm-ar` unless the user chose an archiver. See the module comment.
fn pick_archiver(build: &mut cc::Build, target: &str, underscored: &str) {
    let overridden = [format!("AR_{target}"), format!("AR_{underscored}"), "TARGET_AR".to_string(), "AR".to_string()]
        .iter()
        .any(|var| env::var_os(var).is_some());
    if overridden {
        return;
    }

    let compiler = build.get_compiler();
    let compiler_path = compiler.path();
    let compiler_path = if compiler_path.components().count() > 1 {
        Some(compiler_path.to_path_buf())
    } else {
        find_in_path(compiler_path.to_str().unwrap())
    };
    let sibling = compiler_path.and_then(|path| path.parent().map(|dir| dir.join("llvm-ar"))).filter(|path| path.is_file());

    if let Some(llvm_ar) = sibling.or_else(|| find_in_path("llvm-ar")) {
        build.archiver(llvm_ar);
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths).map(|dir| dir.join(name)).find(|path| path.is_file())
}

/// lld-link resolves symbols through the archive's GNU-style symbol table (a
/// first member named "/"). An archive without one (what Apple's `ar` makes
/// from COFF objects) builds fine and then fails to link with a wall of
/// "undefined symbol" errors, so refuse it here with the actual reason.
fn check_archive_index(lib: &Path) {
    let data = fs::read(lib).unwrap_or_else(|e| panic!("cannot read {}: {e}", lib.display()));
    // 8 byte magic, then a 60 byte member header: name[16] date[12] uid[6] gid[6] mode[8] size[10] magic[2].
    let has_index = data.starts_with(b"!<arch>\n")
        && data.len() >= 72
        && data[8..10] == *b"/ "
        && u32::from_be_bytes(data[68..72].try_into().unwrap()) > 0;
    if !has_index {
        panic!(
            "{} has no GNU-style symbol table, so lld-link would not find any ACPICA symbols. \
             The host `ar` cannot index COFF objects (Apple's `ar` is the usual culprit). \
             Install LLVM (`brew install llvm`) and put its bin directory on PATH, or point \
             AR_x86_64_unknown_uefi at its llvm-ar.",
            lib.display()
        );
    }
}

fn generate_bindings(manifest: &Path, out_dir: &Path, include_dirs: &[PathBuf]) {
    // The same target the C code is compiled for (cc maps the UEFI targets to
    // the Windows ones for clang), so struct layouts match.
    let target = env::var("TARGET").unwrap();
    let clang_target = if target.contains("uefi") || target.contains("windows") {
        "x86_64-unknown-windows-gnu".to_string()
    } else {
        target
    };

    let mut args = vec![format!("--target={clang_target}"), "-ffreestanding".to_string()];
    args.extend(DEFINES.iter().map(|define| format!("-D{define}")));
    args.extend(include_dirs.iter().map(|dir| format!("-I{}", dir.display())));

    let bindings = bindgen::Builder::default()
        .header(manifest.join("wrapper.h").to_str().unwrap())
        .clang_args(args)
        .use_core()
        .ctypes_prefix("core::ffi")
        .layout_tests(false)
        .generate_comments(false)
        .prepend_enum_name(false)
        // ACPICA's public interface, and nothing from the compiler's headers.
        .allowlist_function("Acpi.*")
        .allowlist_type("ACPI_.*|Acpi.*")
        .allowlist_var("ACPI_.*|Acpi.*|AE_.*")
        // The OS services layer is implemented by the kernel, not called by it.
        .blocklist_function("AcpiOs.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen could not process ACPICA's headers (is libclang installed?)");

    bindings.write_to_file(out_dir.join("bindings.rs")).expect("cannot write bindings.rs");
}

/// bindgen cannot evaluate ACPICA's status codes (`AE_NOT_FOUND` is
/// `EXCEP_ENV (0x0005)`, a function-like macro with a cast), so they are
/// generated from `acexcep.h` here instead of being copied by hand. The
/// header's layout is regular: `#define AE_NAME  EXCEP_<kind> (0x<code>)`,
/// plus `AE_OK` itself.
fn generate_status_codes(acpica: &Path, out_dir: &Path) {
    let header = acpica.join("include/acexcep.h");
    let text = fs::read_to_string(&header).unwrap();

    let mut out = String::from("// Generated by build.rs from acexcep.h\n");
    let mut count = 0;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("#define AE_") else { continue };
        let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let name = format!("AE_{}", &rest[..name_end]);
        let value = rest[name_end..].trim();

        let code = if name == "AE_OK" {
            Some(0)
        } else {
            // EXCEP_ENV (0x0005) -> base | 0x0005
            let (kind, tail) = value.strip_prefix("EXCEP_").map(|v| v.split_at(3)).unwrap_or(("", ""));
            let base = match kind {
                "ENV" => Some(0x0000u32),
                "PGM" => Some(0x1000),
                "TBL" => Some(0x2000),
                "AML" => Some(0x3000),
                "CTL" => Some(0x4000),
                _ => None,
            };
            let digits = tail.trim().strip_prefix('(').and_then(|t| t.split(')').next()).and_then(|t| t.trim().strip_prefix("0x"));
            match (base, digits.and_then(|d| u32::from_str_radix(d, 16).ok())) {
                (Some(base), Some(code)) => Some(base | code),
                _ => None,
            }
        };
        if let Some(code) = code {
            out.push_str(&format!("pub const {name}: u32 = {code:#06x};\n"));
            count += 1;
        }
    }
    // Guards against acexcep.h changing shape and the scan silently finding nothing.
    assert!(count > 80, "only found {count} status codes in acexcep.h; has its format changed?");
    fs::write(out_dir.join("status_codes.rs"), out).unwrap();
}
