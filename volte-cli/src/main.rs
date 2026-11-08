//! Command-line front end.
//!
//! Port of `main` (`hd_vendor_build.py:1893`): the same five options and the
//! same exit codes (0 / 2), now reached through an explicit `vendor`
//! subcommand instead of a bare positional image. Three documented
//! divergences:
//! - the vendor build requires `volte-cli vendor <image> ...`; a bare
//!   `volte-cli <image>` (Python's and V1.0's original calling convention)
//!   is a usage error instead of a silent build, so a vendor image that
//!   happens to be named `modem` or `fix-modem` can never be misread as one
//!   of those subcommands (the risk the old positional dispatch had, noted
//!   in `README.md`);
//! - `--gui` (and a `vendor` run with no image, which Python sends to Tk)
//!   prints an error and exits 2, because V1.0 has no GUI;
//! - the console sink writes UTF-8 bytes, where Python re-encodes with the
//!   console code page and replaces unencodable characters.

use std::io::Write;
use std::path::{Path, PathBuf};

mod modem_cli;

use volte_core::avb::{Avb, HashtreeRequest, PythonAvbtool};
use volte_core::config;
use volte_core::env;
use volte_core::ext4_tools::E2fsck;
use volte_core::logging::{Logger, Sink};
use volte_core::pipeline::{self, Deps, Options};
use volte_core::rustfs_ext4::RustFsExt4Backend;

const TOP_USAGE: &str = "usage: volte-cli <vendor|modem|fix-modem> ... (see 'volte-cli --help')";

const USAGE: &str = "usage: volte-cli vendor [-h] [--qc] [--mtk] [--out OUT] [--gui] <image>";

const HELP: &str = "\
BUILD VENDOR CHUAN (MTK + QUALCOMM) - standalone

subcommands:
  vendor <image> [--qc] [--mtk] [--out OUT] [--gui]
      BUILD VENDOR: inject the VoLTE overlay into a vendor.img.

  modem <modem.bin|modem.img> [--out OUT]
      BUILD MODEM: the modem already has a Viettel MBN, move it to the front.

  fix-modem <modem.bin|modem.img> (--donor DONOR | --mbn MBN_FILE) [--out OUT]
      BUILD FIX MODEM: the modem is missing the Viettel MBN entirely.

'vendor' options:
  -h, --help  show this help message and exit
  --qc        force the Qualcomm branch
  --mtk       force the MTK branch
  --out OUT   output file path
  --gui       open the GUI (not part of V1.0)

Each subcommand also accepts -h/--help on its own, e.g. 'volte-cli vendor --help'.";

const GUI_DIVERGENCE: &str =
    "[ERROR] GUI is not part of V1.0 - pass a vendor.img path: volte-cli vendor <image> (see --help).";

/// The five `argparse` options of Python's `main`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    image: String,
    qc: bool,
    mtk: bool,
    out: String,
    gui: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Build(Args),
    Help,
    Usage(String),
}

fn is_option(arg: &str) -> bool {
    arg.starts_with('-') && arg != "-"
}

/// Mirrors `argparse` for `image`, `--qc`, `--mtk`, `--out`, `--gui`, `--help`
/// and `--`. Option abbreviations are not supported.
fn parse(argv: &[String]) -> Command {
    let mut a = Args::default();
    let mut positional_only = false;
    let mut seen_image = false;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        i += 1;
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && is_option(arg) {
            match arg.as_str() {
                "-h" | "--help" => return Command::Help,
                "--qc" => a.qc = true,
                "--mtk" => a.mtk = true,
                "--gui" => a.gui = true,
                "--out" => {
                    let value = match argv.get(i) {
                        Some(v) if !is_option(v) => v,
                        _ => {
                            return Command::Usage(
                                "argument --out: expected one argument".to_string(),
                            )
                        }
                    };
                    a.out = value.clone();
                    i += 1;
                }
                other => match other.strip_prefix("--out=") {
                    Some(v) => a.out = v.to_string(),
                    None => {
                        return Command::Usage(format!("unrecognized arguments: {other}"));
                    }
                },
            }
            continue;
        }
        if seen_image {
            return Command::Usage(format!("unrecognized arguments: {arg}"));
        }
        a.image = arg.clone();
        seen_image = true;
    }
    Command::Build(a)
}

/// `"qc" if a.qc else ("mtk" if a.mtk else "")` (`:1914`): QC wins silently.
fn chip_of(a: &Args) -> &'static str {
    if a.qc {
        "qc"
    } else if a.mtk {
        "mtk"
    } else {
        ""
    }
}

/// Python runs the Tk GUI when `--gui` is given or no image is passed.
fn opens_gui(a: &Args) -> bool {
    a.gui || a.image.is_empty()
}

/// Writes log lines to stdout, the console half of `BuildLog` (`:346`).
struct Console;

impl Sink for Console {
    fn emit(&mut self, line: &str) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(line.as_bytes());
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}

/// `avb_module` (`:1256`) loads `avbtool.py` only when the hashtree step runs,
/// so a missing tool degrades to the boot-safe SKIP path instead of aborting
/// the build up front.
struct LazyAvb {
    python: PathBuf,
    avbtool: PathBuf,
    inner: Option<PythonAvbtool>,
}

impl LazyAvb {
    fn new(python: PathBuf, avbtool: PathBuf) -> Self {
        LazyAvb {
            python,
            avbtool,
            inner: None,
        }
    }
}

impl Avb for LazyAvb {
    fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> volte_core::error::Result<()> {
        match self.inner.as_mut() {
            Some(tool) => tool.add_hashtree_footer(req),
            None => {
                let mut tool = PythonAvbtool::new(self.python.clone(), self.avbtool.clone())?;
                let res = tool.add_hashtree_footer(req);
                self.inner = Some(tool);
                res
            }
        }
    }
}

fn build(a: &Args) -> i32 {
    let root = env::exe_root();
    let tool = env::tool_dir(&root);
    let logs = root.join("Logs");
    let python = {
        let bundled = tool.join("python").join("python.exe");
        if bundled.is_file() {
            bundled
        } else {
            PathBuf::from("python.exe")
        }
    };

    let mut ops = RustFsExt4Backend::new();
    let mut fsck = E2fsck::new(&tool);
    let mut avb = LazyAvb::new(python, tool.join("avbtool.py"));
    let mut deps = Deps {
        ops: &mut ops,
        fsck: &mut fsck,
        avb: &mut avb,
    };
    let mut log = Logger::new().with_sink(Console);
    let mut progress = |_n: u8, _label: &str| {};
    let opts = Options {
        src: Path::new(&a.image),
        out_path: &a.out,
        chip: chip_of(a),
        keep_out: !a.out.is_empty(),
        root: &root,
        logs_dir: &logs,
    };

    let result = pipeline::run(&opts, &mut deps, &mut log, &mut progress);
    match &result {
        Ok(out) => println!("{}{}", config::TAG_OK, out.display()),
        Err(e) => println!("{}{}", config::TAG_ERROR, e.message()),
    }
    exit_code(result)
}

/// Any pipeline error, including a failed mandatory `e2fsck`, is exit code 2
/// (Python's `main` returns 2 for every `BuildError`).
fn exit_code(result: volte_core::error::Result<PathBuf>) -> i32 {
    if result.is_ok() {
        0
    } else {
        2
    }
}

/// Top-level dispatch: every run names a subcommand (`vendor`, `modem` or
/// `fix-modem`) first. There is no bare-image fallback: a vendor image is
/// never implicitly assumed from `argv[0]`, so an image literally named
/// `modem`/`fix-modem` can no longer be misread as a subcommand, and an
/// image literally named `vendor` is unambiguous too (it is the subcommand
/// name, not a path, in that position).
fn run(argv: &[String]) -> i32 {
    match argv.first().map(String::as_str) {
        Some("vendor") => run_vendor(&argv[1..]),
        Some("modem") => modem_cli::run_modem(&argv[1..]),
        Some("fix-modem") => modem_cli::run_fix_modem(&argv[1..]),
        Some("-h") | Some("--help") => {
            println!("{TOP_USAGE}\n\n{HELP}");
            0
        }
        Some(other) => {
            eprintln!("{TOP_USAGE}\nvolte-cli: error: unrecognized subcommand: {other}");
            2
        }
        None => {
            eprintln!("{TOP_USAGE}\nvolte-cli: error: a subcommand is required (vendor, modem, fix-modem)");
            2
        }
    }
}

/// Parses and runs the `vendor` subcommand (`argv` with `vendor` already
/// stripped off).
fn run_vendor(argv: &[String]) -> i32 {
    match parse(argv) {
        Command::Help => {
            println!("{USAGE}\n\n{HELP}");
            0
        }
        Command::Usage(msg) => {
            eprintln!("{USAGE}\nvolte-cli: error: {msg}");
            2
        }
        Command::Build(a) if opens_gui(&a) => {
            println!("{GUI_DIVERGENCE}");
            2
        }
        Command::Build(a) => build(&a),
    }
}

fn main() {
    std::process::exit(run(&std::env::args().skip(1).collect::<Vec<_>>()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn build_args(items: &[&str]) -> Args {
        match parse(&argv(items)) {
            Command::Build(a) => a,
            other => panic!("expected a build command, got {other:?}"),
        }
    }

    fn usage(items: &[&str]) -> String {
        match parse(&argv(items)) {
            Command::Usage(m) => m,
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    #[test]
    fn parse_accepts_the_five_python_options() {
        let a = build_args(&["vendor.img", "--qc", "--out", r"D:\out\v.img", "--gui"]);
        assert_eq!(a.image, "vendor.img");
        assert!(a.qc);
        assert!(!a.mtk);
        assert_eq!(a.out, r"D:\out\v.img");
        assert!(a.gui);
    }

    #[test]
    fn defaults_are_empty_like_argparse() {
        let a = build_args(&[]);
        assert_eq!(a, Args::default());
        assert!(opens_gui(&a), "no image sends Python to the GUI");
        assert_eq!(chip_of(&a), "");
    }

    #[test]
    fn qc_wins_over_mtk_like_python() {
        let a = build_args(&["vendor.img", "--mtk", "--qc"]);
        assert_eq!(chip_of(&a), "qc");
        assert_eq!(chip_of(&build_args(&["v.img", "--mtk"])), "mtk");
    }

    #[test]
    fn out_accepts_both_forms() {
        assert_eq!(build_args(&["v.img", "--out=x.img"]).out, "x.img");
        assert_eq!(build_args(&["v.img", "--out", "y.img"]).out, "y.img");
        assert_eq!(build_args(&["v.img", "--out="]).out, "");
    }

    #[test]
    fn help_wins_from_any_position() {
        assert_eq!(parse(&argv(&["-h"])), Command::Help);
        assert_eq!(parse(&argv(&["v.img", "--help", "--qc"])), Command::Help);
    }

    #[test]
    fn unknown_option_is_a_usage_error() {
        assert_eq!(usage(&["--bogus"]), "unrecognized arguments: --bogus");
        assert_eq!(usage(&["-q"]), "unrecognized arguments: -q");
        assert_eq!(usage(&["--outx=1"]), "unrecognized arguments: --outx=1");
    }

    #[test]
    fn missing_out_value_is_a_usage_error() {
        assert_eq!(
            usage(&["v.img", "--out"]),
            "argument --out: expected one argument"
        );
        assert_eq!(
            usage(&["v.img", "--out", "--qc"]),
            "argument --out: expected one argument"
        );
    }

    #[test]
    fn a_second_positional_is_a_usage_error() {
        assert_eq!(
            usage(&["one.img", "two.img"]),
            "unrecognized arguments: two.img"
        );
    }

    #[test]
    fn double_dash_ends_option_parsing() {
        let a = build_args(&["--", "--weird.img"]);
        assert_eq!(a.image, "--weird.img");
        assert!(!a.qc);
        let a = build_args(&["-", "--mtk"]);
        assert_eq!(a.image, "-");
        assert!(a.mtk);
    }

    #[test]
    fn gui_flag_alone_selects_the_divergence_not_a_build() {
        let a = build_args(&["--gui"]);
        assert!(opens_gui(&a));
        let with_image = build_args(&["vendor.img", "--gui"]);
        assert!(
            opens_gui(&with_image),
            "Python ignores the image when --gui is set"
        );
    }

    #[test]
    fn a_pipeline_error_exits_non_zero() {
        let failed: volte_core::error::Result<PathBuf> =
            Err(volte_core::error::Error::build("e2fsck failed"));
        assert_eq!(exit_code(failed), 2);
        let ok: volte_core::error::Result<PathBuf> = Ok(PathBuf::from("out.img"));
        assert_eq!(exit_code(ok), 0);
    }

    // -- top-level dispatch: `vendor` is now mandatory, not a bare image --

    #[test]
    fn no_subcommand_is_a_usage_error() {
        assert_eq!(run(&argv(&[])), 2);
    }

    #[test]
    fn a_bare_image_with_no_subcommand_is_rejected() {
        // The old calling convention (`volte-cli vendor.img`) must no longer
        // start a build: it is read as an unrecognized subcommand.
        assert_eq!(run(&argv(&["vendor.img"])), 2);
        assert_eq!(run(&argv(&["vendor.img", "--qc"])), 2);
    }

    #[test]
    fn unknown_subcommand_is_a_usage_error() {
        assert_eq!(run(&argv(&["build"])), 2);
        assert_eq!(run(&argv(&["--qc"])), 2);
    }

    #[test]
    fn top_level_help_is_not_a_subcommand() {
        assert_eq!(run(&argv(&["-h"])), 0);
        assert_eq!(run(&argv(&["--help"])), 0);
    }

    #[test]
    fn vendor_subcommand_help_still_works() {
        assert_eq!(run(&argv(&["vendor", "-h"])), 0);
        assert_eq!(run(&argv(&["vendor", "--help"])), 0);
    }

    #[test]
    fn vendor_subcommand_with_no_image_hits_the_gui_divergence() {
        // Same observable behaviour as before (Python sends a bare run to
        // Tk; V1.0 prints GUI_DIVERGENCE and exits 2), just reached through
        // `vendor` now instead of no argv at all.
        assert_eq!(run(&argv(&["vendor"])), 2);
        assert_eq!(run(&argv(&["vendor", "--gui"])), 2);
    }

    #[test]
    fn modem_and_fix_modem_subcommands_are_unaffected() {
        // These already required their own subcommand name; only the
        // `vendor` dispatch path changed. A missing image is a usage error
        // from modem_cli's own parser, exit 2 either way.
        assert_eq!(run(&argv(&["modem"])), 2);
        assert_eq!(run(&argv(&["fix-modem"])), 2);
    }
}
