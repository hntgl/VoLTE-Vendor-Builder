//! `modem` / `fix-modem` subcommands: BUILD MODEM and BUILD FIX MODEM.
//!
//! Dispatched from `main` by the first argv word, kept separate from the
//! five-flag vendor parser in `main.rs` so neither parser has to know
//! about the other's options. See `volte_core::modem` for the engine and
//! for what this integration intentionally does not port (GUI, zip
//! input, the `%APPDATA%` MBN cache/auto-discovery).

use std::path::{Path, PathBuf};

use volte_core::logging::Logger;
use volte_core::modem;

use crate::{is_option, Console};

const MODEM_USAGE: &str = "usage: volte-cli modem <modem.bin|modem.img> [--out OUT]";
const FIX_MODEM_USAGE: &str =
    "usage: volte-cli fix-modem <modem.bin|modem.img> (--donor DONOR | --mbn MBN_FILE) [--out OUT]";

/// Reads the value for a `--flag VALUE` pair; `argv[*i]` is the value slot.
fn take_value(argv: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    match argv.get(*i) {
        Some(v) if !is_option(v) => {
            *i += 1;
            Ok(v.clone())
        }
        _ => Err(format!("argument {flag}: expected one argument")),
    }
}

fn usage_error(usage: &str, msg: &str) -> i32 {
    eprintln!("{usage}\nvolte-cli: error: {msg}");
    2
}

fn out_path(out: &str) -> Option<PathBuf> {
    if out.is_empty() {
        None
    } else {
        Some(PathBuf::from(out))
    }
}

struct ModemArgs {
    image: String,
    out: String,
}

/// Parses `modem <image> [--out OUT | --out=OUT]`. `Err` carries the exit
/// code for a usage error already printed to stderr; `Ok(None)` means help
/// was printed (exit 0 is the caller's job).
fn parse_modem(argv: &[String]) -> Result<Option<ModemArgs>, i32> {
    let mut image = String::new();
    let mut out = String::new();
    let mut seen_image = false;
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].clone();
        i += 1;
        if is_option(&arg) {
            match arg.as_str() {
                "-h" | "--help" => {
                    println!("{MODEM_USAGE}");
                    return Ok(None);
                }
                "--out" => match take_value(argv, &mut i, "--out") {
                    Ok(v) => out = v,
                    Err(msg) => return Err(usage_error(MODEM_USAGE, &msg)),
                },
                other => match other.strip_prefix("--out=") {
                    Some(v) => out = v.to_string(),
                    None => {
                        return Err(usage_error(
                            MODEM_USAGE,
                            &format!("unrecognized arguments: {other}"),
                        ))
                    }
                },
            }
            continue;
        }
        if seen_image {
            return Err(usage_error(
                MODEM_USAGE,
                &format!("unrecognized arguments: {arg}"),
            ));
        }
        image = arg;
        seen_image = true;
    }
    if image.is_empty() {
        return Err(usage_error(
            MODEM_USAGE,
            "the following arguments are required: image",
        ));
    }
    Ok(Some(ModemArgs { image, out }))
}

enum FixSource {
    Donor(String),
    Mbn(String),
}

struct FixModemArgs {
    image: String,
    out: String,
    source: FixSource,
}

fn parse_fix_modem(argv: &[String]) -> Result<Option<FixModemArgs>, i32> {
    let mut image = String::new();
    let mut out = String::new();
    let mut donor: Option<String> = None;
    let mut mbn: Option<String> = None;
    let mut seen_image = false;
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].clone();
        i += 1;
        if is_option(&arg) {
            match arg.as_str() {
                "-h" | "--help" => {
                    println!("{FIX_MODEM_USAGE}");
                    return Ok(None);
                }
                "--out" => match take_value(argv, &mut i, "--out") {
                    Ok(v) => out = v,
                    Err(msg) => return Err(usage_error(FIX_MODEM_USAGE, &msg)),
                },
                "--donor" => match take_value(argv, &mut i, "--donor") {
                    Ok(v) => donor = Some(v),
                    Err(msg) => return Err(usage_error(FIX_MODEM_USAGE, &msg)),
                },
                "--mbn" => match take_value(argv, &mut i, "--mbn") {
                    Ok(v) => mbn = Some(v),
                    Err(msg) => return Err(usage_error(FIX_MODEM_USAGE, &msg)),
                },
                other => {
                    let (flag, val) = match other.split_once('=') {
                        Some((f, v)) => (f, Some(v.to_string())),
                        None => (other, None),
                    };
                    match (flag, val) {
                        ("--out", Some(v)) => out = v,
                        ("--donor", Some(v)) => donor = Some(v),
                        ("--mbn", Some(v)) => mbn = Some(v),
                        _ => {
                            return Err(usage_error(
                                FIX_MODEM_USAGE,
                                &format!("unrecognized arguments: {other}"),
                            ))
                        }
                    }
                }
            }
            continue;
        }
        if seen_image {
            return Err(usage_error(
                FIX_MODEM_USAGE,
                &format!("unrecognized arguments: {arg}"),
            ));
        }
        image = arg;
        seen_image = true;
    }
    if image.is_empty() {
        return Err(usage_error(
            FIX_MODEM_USAGE,
            "the following arguments are required: image",
        ));
    }
    let source = match (donor, mbn) {
        (Some(_), Some(_)) => {
            return Err(usage_error(
                FIX_MODEM_USAGE,
                "argument --mbn: not allowed with argument --donor",
            ))
        }
        (Some(d), None) => FixSource::Donor(d),
        (None, Some(m)) => FixSource::Mbn(m),
        (None, None) => {
            return Err(usage_error(
                FIX_MODEM_USAGE,
                "one of the arguments --donor --mbn is required",
            ))
        }
    };
    Ok(Some(FixModemArgs { image, out, source }))
}

fn report_result(result: volte_core::error::Result<modem::ModemReport>) -> i32 {
    match result {
        Ok(r) => {
            println!(
                "[OK] {} ({} bytes, Viettel: {})",
                r.dst.display(),
                r.size,
                r.viettel_rel
            );
            0
        }
        Err(e) => {
            println!("[ERROR] {}", e.message());
            2
        }
    }
}

pub fn run_modem(argv: &[String]) -> i32 {
    let args = match parse_modem(argv) {
        Ok(Some(a)) => a,
        Ok(None) => return 0,
        Err(code) => return code,
    };
    let mut log = Logger::new().with_sink(Console);
    let result = modem::build_modem_volte(
        Path::new(&args.image),
        out_path(&args.out).as_deref(),
        &mut log,
    );
    report_result(result)
}

pub fn run_fix_modem(argv: &[String]) -> i32 {
    let args = match parse_fix_modem(argv) {
        Ok(Some(a)) => a,
        Ok(None) => return 0,
        Err(code) => return code,
    };
    let mut log = Logger::new().with_sink(Console);
    let dest = Path::new(&args.image);
    let out = out_path(&args.out);
    let result = match args.source {
        FixSource::Donor(donor) => {
            modem::build_modem_fix_viettel(dest, Path::new(&donor), out.as_deref(), &mut log)
        }
        FixSource::Mbn(mbn_path) => match std::fs::read(&mbn_path) {
            Ok(bytes) => modem::build_modem_fix_from_mbn(dest, &bytes, out.as_deref(), &mut log),
            Err(e) => Err(volte_core::error::Error::from(e)),
        },
    };
    report_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn modem_parses_image_and_out() {
        let a = parse_modem(&argv(&["modem.bin", "--out", "out.bin"]))
            .unwrap()
            .unwrap();
        assert_eq!(a.image, "modem.bin");
        assert_eq!(a.out, "out.bin");
    }

    #[test]
    fn modem_accepts_out_equals_form() {
        let a = parse_modem(&argv(&["modem.bin", "--out=out.bin"]))
            .unwrap()
            .unwrap();
        assert_eq!(a.out, "out.bin");
    }

    #[test]
    fn modem_requires_an_image() {
        assert!(matches!(parse_modem(&argv(&[])), Err(2)));
        assert!(matches!(parse_modem(&argv(&["--out", "x.bin"])), Err(2)));
    }

    #[test]
    fn modem_help_prints_and_returns_none() {
        assert!(matches!(parse_modem(&argv(&["-h"])), Ok(None)));
        assert!(matches!(parse_modem(&argv(&["--help"])), Ok(None)));
    }

    #[test]
    fn modem_rejects_a_second_positional() {
        assert!(matches!(parse_modem(&argv(&["a.bin", "b.bin"])), Err(2)));
    }

    #[test]
    fn fix_modem_parses_donor() {
        let a = parse_fix_modem(&argv(&["dest.bin", "--donor", "donor.bin"]))
            .unwrap()
            .unwrap();
        assert_eq!(a.image, "dest.bin");
        assert!(matches!(a.source, FixSource::Donor(d) if d == "donor.bin"));
    }

    #[test]
    fn fix_modem_parses_mbn() {
        let a = parse_fix_modem(&argv(&["dest.bin", "--mbn", "viettel.mbn"]))
            .unwrap()
            .unwrap();
        assert!(matches!(a.source, FixSource::Mbn(m) if m == "viettel.mbn"));
    }

    #[test]
    fn fix_modem_rejects_both_donor_and_mbn() {
        assert!(matches!(
            parse_fix_modem(&argv(&["dest.bin", "--donor", "d.bin", "--mbn", "m.mbn"])),
            Err(2)
        ));
    }

    #[test]
    fn fix_modem_requires_a_source() {
        assert!(matches!(parse_fix_modem(&argv(&["dest.bin"])), Err(2)));
    }

    #[test]
    fn fix_modem_accepts_equals_forms() {
        let a = parse_fix_modem(&argv(&["dest.bin", "--mbn=viettel.mbn", "--out=o.bin"]))
            .unwrap()
            .unwrap();
        assert_eq!(a.out, "o.bin");
        assert!(matches!(a.source, FixSource::Mbn(m) if m == "viettel.mbn"));
    }

    #[test]
    fn fix_modem_unknown_option_is_a_usage_error() {
        assert!(matches!(
            parse_fix_modem(&argv(&["dest.bin", "--bogus"])),
            Err(2)
        ));
    }

    #[test]
    fn out_path_turns_empty_string_into_none() {
        assert!(out_path("").is_none());
        assert_eq!(out_path("x.bin"), Some(PathBuf::from("x.bin")));
    }
}
