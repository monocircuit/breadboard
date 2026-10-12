//! Running commands and talking to the user (lib/proc.mjs): the arguments as
//! today's commands read them, `step`, `fail` with its hint, child processes
//! with their output shown or kept.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// The arguments after the command, read as worktree.mjs and `flagValue` do.
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub all: Vec<String>,
}

impl Args {
    pub fn new(args: &[String]) -> Args {
        Args { all: args.to_vec() }
    }

    /// The first word that is no flag and no value of one of `valued`,
    /// wherever it stands: `wt:new --fix 12-x` is `wt:new 12-x --fix` (Node
    /// read only the first argument and printed the bare usage).
    pub fn first_word(&self, valued: &[&str]) -> Option<String> {
        self.words(valued).into_iter().next()
    }

    /// Whether `--name` stands on the command line as it is (`--x=y` counts
    /// only spelled so, as the Set of flags in worktree.mjs).
    pub fn has(&self, name: &str) -> bool {
        self.all.iter().any(|arg| arg == name)
    }

    /// `--name <value>` or `--name=<value>`; a value that starts with `--`
    /// counts as missing.
    pub fn value(&self, name: &str) -> Option<String> {
        if let Some(at) = self.all.iter().position(|arg| arg == name) {
            return self.all.get(at + 1).filter(|value| !value.starts_with("--")).cloned();
        }
        let prefix = format!("{name}=");
        self.all
            .iter()
            .find_map(|arg| arg.strip_prefix(&prefix).map(String::from))
    }

    /// The words that are no flag and no value of one of `valued`.
    pub fn words(&self, valued: &[&str]) -> Vec<String> {
        self.all
            .iter()
            .enumerate()
            .filter(|(at, arg)| !arg.starts_with("--") && !(*at > 0 && valued.contains(&self.all[at - 1].as_str())))
            .map(|(_, arg)| arg.clone())
            .collect()
    }
}

/// The first flag on the command line that the command doesn't take
/// (`--x=y` counts as `--x`), said with the one it is likely a typo of:
/// `unknown flag --shraed: did you mean --shared?`. After `--` nothing counts.
pub fn unknown_flag(all: &[String], known: &[&str]) -> Option<String> {
    let name = all
        .iter()
        .take_while(|arg| *arg != "--")
        .filter(|arg| arg.starts_with("--"))
        .map(|arg| arg.split_once('=').map_or(arg.as_str(), |(name, _)| name))
        .find(|name| !known.contains(name))?;
    let near = known
        .iter()
        .map(|flag| (distance(name, flag), flag))
        .filter(|(far, _)| *far <= 2)
        .min_by_key(|(far, _)| *far);
    Some(match near {
        Some((_, flag)) => format!("unknown flag {name}: did you mean {flag}?"),
        None => format!("unknown flag {name}"),
    })
}

/// What a command takes: its flags, those of them that need a value, how
/// many words (None: any), and its usage.
pub struct Shape {
    pub flags: &'static [&'static str],
    pub valued: &'static [&'static str],
    pub words: Option<usize>,
    pub usage: String,
}

/// What is wrong with the command line before anything happens, None when
/// nothing is: a flag the command doesn't take (with the one it is likely a
/// typo of), a flag without its value, a word too many.
pub fn misuse(args: &Args, shape: &Shape) -> Option<String> {
    if let Some(message) = unknown_flag(&args.all, shape.flags) {
        return Some(message);
    }
    let before_dashes: Vec<&String> = args.all.iter().take_while(|arg| *arg != "--").collect();
    if let Some(flag) = shape.valued.iter().find(|flag| {
        before_dashes
            .iter()
            .any(|arg| arg == *flag || arg.strip_prefix(**flag) == Some("="))
            && args.value(flag).is_none_or(|value| value.is_empty())
    }) {
        return Some(format!("{flag} needs a value"));
    }
    let words = args.words(shape.valued);
    match shape.words {
        Some(0) if !words.is_empty() => Some(format!("unexpected argument {}: it takes none", words[0])),
        Some(most) if words.len() > most => Some(format!("unexpected argument {}", words[most])),
        _ => None,
    }
}

/// `text` in kebab-case as a name would be: `Bad_Name` → `bad-name`.
pub fn kebab_of(text: &str) -> String {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit())
        .filter(|word| !word.is_empty())
        .collect();
    words.join("-")
}

/// Why `given` is no name for `what` (a feature, an orchestration): missing,
/// or not kebab-case with the name it likely meant; then the usage.
pub fn name_refusal(what: &str, given: Option<&str>, usage: &str) -> String {
    let Some(given) = given else {
        return format!("{what} is missing\n{usage}");
    };
    let meant = kebab_of(given);
    let hint = if meant.is_empty() {
        String::new()
    } else {
        format!(": {meant}?")
    };
    format!("{what} {given} is not kebab-case (lowercase letters, digits, single dashes){hint}\n{usage}")
}

/// Whether the command line asks for the usage (`--help`, `-h`).
pub fn wants_help(args: &Args) -> bool {
    args.all
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--help" || arg == "-h")
}

/// Refuses a command line [`misuse`] finds before anything happens: what is
/// wrong, then the command's usage (exit 1).
pub fn refuse_misuse(args: &Args, shape: &Shape) {
    if let Some(message) = misuse(args, shape) {
        fail(&format!(
            "{message}\n{}",
            breadboard_core::config::in_repo(&shape.usage)
        ));
    }
}

/// The edits from `a` to `b` (Levenshtein), for a typo's nearest flag.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(left != *right));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// A line on stdout that never panics: `println!` would on a closed pipe
/// (`breadboard commands | head -2`), and a setup command must finish its
/// steps when nobody reads them any more.
macro_rules! outln {
    () => {
        $crate::cli::setup::proc::write_line($crate::cli::setup::proc::Stream::Out, format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::cli::setup::proc::write_line($crate::cli::setup::proc::Stream::Out, format_args!($($arg)*))
    };
}

/// [`outln!`] on stderr.
macro_rules! errln {
    ($($arg:tt)*) => {
        $crate::cli::setup::proc::write_line($crate::cli::setup::proc::Stream::Err, format_args!($($arg)*))
    };
}

pub(crate) use {errln, outln};

/// Where a line goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

impl Stream {
    fn fd(self) -> libc::c_int {
        match self {
            Stream::Out => libc::STDOUT_FILENO,
            Stream::Err => libc::STDERR_FILENO,
        }
    }
}

/// `text` and a newline on `stream`. A stream nobody reads any more (EPIPE:
/// the reader of the pipe is gone) is pointed at /dev/null from then on, so
/// the next lines and the children after them write into nothing instead of
/// failing: the command's steps go on, its exit code stays its own.
pub fn write_line(stream: Stream, text: std::fmt::Arguments) {
    let written = match stream {
        Stream::Out => {
            let mut out = std::io::stdout().lock();
            out.write_fmt(text).and_then(|()| out.write_all(b"\n"))
        }
        Stream::Err => {
            let mut err = std::io::stderr().lock();
            err.write_fmt(text).and_then(|()| err.write_all(b"\n"))
        }
    };
    if written.is_err() {
        detach(stream);
    }
}

/// `bytes` on `stream` as they are, flushed; a closed one as [`write_line`].
pub fn write_bytes(stream: Stream, bytes: &[u8]) {
    let written = match stream {
        Stream::Out => {
            let mut out = std::io::stdout().lock();
            out.write_all(bytes).and_then(|()| out.flush())
        }
        Stream::Err => std::io::stderr().lock().write_all(bytes),
    };
    if written.is_err() {
        detach(stream);
    }
}

/// stdout flushed; a closed one as [`write_line`].
pub fn flush_out() {
    if std::io::stdout().flush().is_err() {
        detach(Stream::Out);
    }
}

/// `stream` pointed at /dev/null.
fn detach(stream: Stream) {
    // SAFETY: open(2) of a constant path and dup2(2)/close(2) of descriptors this process owns.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if null >= 0 {
            libc::dup2(null, stream.fd());
            libc::close(null);
        }
    }
    if stream == Stream::Out {
        // What stdout still buffers goes into /dev/null now, not again into the error.
        let _ = std::io::stdout().flush();
    }
}

/// Whether `stream` is a pipe or socket, whose reader can go away (a
/// terminal or a file can't): a child's output then passes through this
/// process (`forward`), so the child never meets the closed pipe itself,
/// which would end it (a child gets SIGPIPE back at its default).
fn is_pipe(stream: Stream) -> bool {
    // SAFETY: fstat(2) into a zeroed stat of a descriptor number.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let kind = (unsafe { libc::fstat(stream.fd(), &mut stat) } == 0).then_some(stat.st_mode & libc::S_IFMT);
    matches!(kind, Some(libc::S_IFIFO | libc::S_IFSOCK))
}

/// A child's output passed on to `stream` as it comes, until it ends.
fn forward(from: impl Read + Send + 'static, stream: Stream) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut from = from;
        let mut buffer = [0u8; 8192];
        while let Ok(read) = from.read(&mut buffer) {
            if read == 0 {
                break;
            }
            write_bytes(stream, &buffer[..read]);
        }
    })
}

static HINT: Mutex<String> = Mutex::new(String::new());
/// Files this process holds and drops when it fails (setup claims).
static HELD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// What `fail` adds under its message from here on.
pub fn set_hint(text: &str) {
    if let Ok(mut hint) = HINT.lock() {
        *hint = text.to_string();
    }
}

/// What `fail` adds under its message now.
pub fn hint() -> String {
    HINT.lock().map(|hint| hint.clone()).unwrap_or_default()
}

/// A file removed when this process fails or ends through `fail`.
pub fn hold(file: &Path) {
    if let Ok(mut held) = HELD.lock() {
        held.push(file.to_path_buf());
    }
}

/// The file no longer held.
pub fn unhold(file: &Path) {
    if let Ok(mut held) = HELD.lock() {
        held.retain(|known| known != file);
    }
}

/// Every held file let go, as the exit handlers of Node do.
pub fn release_held() {
    let files = HELD
        .lock()
        .map(|mut held| std::mem::take(&mut *held))
        .unwrap_or_default();
    for file in files {
        let _ = std::fs::remove_file(&file);
    }
}

/// `\n✗ <message>` (and the hint) on stderr, exit 1. Other processes read
/// the `✗ ` at the start of a line (wt:orchestrate, the overview).
pub fn fail(message: &str) -> ! {
    let message = breadboard_core::config::in_repo(message);
    let hint = breadboard_core::config::in_repo(&hint());
    flush_out();
    if hint.is_empty() {
        errln!("\n✗ {message}");
    } else {
        errln!("\n✗ {message}\n  {hint}");
    }
    release_held();
    std::process::exit(1)
}

/// `\n▸ <message>` on stdout.
pub fn step(message: &str) {
    outln!("\n▸ {}", breadboard_core::config::in_repo(message));
}

/// What a child process answered.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub is_ok: bool,
    pub out: String,
    pub err: String,
}

impl Ran {
    /// The output trimmed, as proc.mjs gives it.
    pub fn text(&self) -> &str {
        self.out.trim()
    }
}

/// How a child runs: in `cwd`, its output shown (`is_quiet` false) or kept.
#[derive(Debug, Clone, Default)]
pub struct How<'a> {
    pub cwd: Option<&'a Path>,
    pub is_quiet: bool,
    pub can_fail: bool,
    pub input: Option<&'a str>,
    pub env: Vec<(&'a str, String)>,
}

/// `command args` (proc.mjs `run`): shown or quiet; a failure fails unless
/// `can_fail`.
pub fn run(command: &str, args: &[&str], how: How) -> Ran {
    flush_out();
    let mut child = Command::new(command);
    child.args(args);
    if let Some(cwd) = how.cwd {
        child.current_dir(cwd);
    }
    for (key, value) in &how.env {
        child.env(key, value);
    }
    child.stdin(if how.input.is_some() {
        Stdio::piped()
    } else if how.is_quiet {
        Stdio::null()
    } else {
        Stdio::inherit()
    });
    let passes = |stream| !how.is_quiet && is_pipe(stream);
    let (passes_out, passes_err) = (passes(Stream::Out), passes(Stream::Err));
    let stdio = |is_kept: bool| if is_kept { Stdio::piped() } else { Stdio::inherit() };
    child.stdout(stdio(how.is_quiet || passes_out));
    child.stderr(stdio(how.is_quiet || passes_err));
    let ran = match child.spawn() {
        Ok(mut process) => {
            if let (Some(input), Some(mut stdin)) = (how.input, process.stdin.take()) {
                let _ = stdin.write_all(input.as_bytes());
            }
            let out = process
                .stdout
                .take_if(|_| passes_out)
                .map(|from| forward(from, Stream::Out));
            let err = process
                .stderr
                .take_if(|_| passes_err)
                .map(|from| forward(from, Stream::Err));
            let output = process.wait_with_output();
            for pump in [out, err].into_iter().flatten() {
                let _ = pump.join();
            }
            match output {
                Ok(output) => Ran {
                    is_ok: output.status.success(),
                    out: String::from_utf8_lossy(&output.stdout).into_owned(),
                    err: String::from_utf8_lossy(&output.stderr).into_owned(),
                },
                Err(error) => Ran {
                    err: error.to_string(),
                    ..Ran::default()
                },
            }
        }
        Err(error) => Ran {
            err: format!("{command}: {error}"),
            ..Ran::default()
        },
    };
    if !ran.is_ok && !how.can_fail {
        let quiet = if how.is_quiet {
            format!(":\n{}", ran.err)
        } else {
            String::new()
        };
        fail(&format!("{command} {} failed{quiet}", args.join(" ")));
    }
    ran
}

/// `command args` quiet, never failing: for reading.
pub fn sh(command: &str, args: &[&str], cwd: Option<&Path>) -> Ran {
    run(
        command,
        args,
        How {
            cwd,
            is_quiet: true,
            can_fail: true,
            ..How::default()
        },
    )
}

/// `git args` in `cwd`, quiet, never failing (proc.mjs `git`).
pub fn git(args: &[&str], cwd: &Path) -> Ran {
    sh("git", args, Some(cwd))
}

/// `git args` with `input` on its stdin.
pub fn git_with(args: &[&str], cwd: &Path, input: &str) -> Ran {
    run(
        "git",
        args,
        How {
            cwd: Some(cwd),
            is_quiet: true,
            can_fail: true,
            input: Some(input),
            ..How::default()
        },
    )
}

/// A shell word: single quotes, a `'` inside as `'\''`.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Whether claude runs this (Claude Code sets CLAUDECODE): an agent stays in its tab.
pub fn is_agent() -> bool {
    std::env::var_os("CLAUDECODE").is_some_and(|value| !value.is_empty())
}

/// This binary, for a command that calls another one in a child process.
pub fn entry() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("breadboard"))
}

/// Set for a child of this binary: its parent checked the herdr plugin (#230).
pub const PLUGIN_CHECKED: &str = "BREADBOARD_PLUGIN_CHECKED";

/// This binary as a child process, told the plugin is checked.
pub fn child() -> std::process::Command {
    let mut command = std::process::Command::new(entry());
    command.env(PLUGIN_CHECKED, "1");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        Args::new(&list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn names_a_flag_the_command_doesnt_take() {
        let known = ["--shared", "--fix", "--origin"];
        let all = |list: &[&str]| args(list).all;
        assert_eq!(unknown_flag(&all(&["12-x", "--shared", "--origin=b"]), &known), None);
        assert_eq!(
            unknown_flag(&all(&["12-x", "--shraed"]), &known).as_deref(),
            Some("unknown flag --shraed: did you mean --shared?")
        );
        assert_eq!(
            unknown_flag(&all(&["12-x", "--bogus=1"]), &known).as_deref(),
            Some("unknown flag --bogus")
        );
        assert_eq!(unknown_flag(&all(&["x", "--", "--bogus"]), &known), None);
        assert_eq!(distance("--fx", "--fix"), 1);
    }

    #[test]
    fn reads_flags_as_today() {
        let parsed = args(&["12-x", "--origin", "breadboard", "--shared"]);
        assert_eq!(parsed.first_word(&["--origin"]).as_deref(), Some("12-x"));
        assert!(parsed.has("--shared"));
        assert_eq!(parsed.value("--origin").as_deref(), Some("breadboard"));
        assert_eq!(args(&["--fix", "12-x"]).first_word(&[]).as_deref(), Some("12-x"));
        assert_eq!(
            args(&["--origin", "breadboard", "12-x"])
                .first_word(&["--origin"])
                .as_deref(),
            Some("12-x")
        );
        assert_eq!(args(&["--origin", "--shared"]).value("--origin"), None);
        assert_eq!(args(&["--since=2w"]).value("--since").as_deref(), Some("2w"));
        assert_eq!(args(&["a", "--type", "fixed", "b"]).words(&["--type"]), vec!["a", "b"]);
    }

    #[test]
    fn names_a_missing_value_and_a_word_too_many() {
        let shape = |words| Shape {
            flags: &["--port", "--all"],
            valued: &["--port"],
            words,
            usage: "usage: x".into(),
        };
        let wrong = |list: &[&str], words| misuse(&args(list), &shape(words));
        assert_eq!(wrong(&["a", "--port", "3002"], Some(1)), None);
        assert_eq!(wrong(&["--port=3002", "a"], Some(1)), None);
        assert_eq!(wrong(&["a", "b", "c"], None), None);
        assert_eq!(wrong(&["--port"], Some(1)).as_deref(), Some("--port needs a value"));
        assert_eq!(
            wrong(&["--port", "--all"], Some(1)).as_deref(),
            Some("--port needs a value")
        );
        assert_eq!(wrong(&["--port="], Some(1)).as_deref(), Some("--port needs a value"));
        assert_eq!(wrong(&["a", "b"], Some(1)).as_deref(), Some("unexpected argument b"));
        assert_eq!(
            wrong(&["foo"], Some(0)).as_deref(),
            Some("unexpected argument foo: it takes none")
        );
        assert!(wants_help(&args(&["a", "-h"])));
        assert!(!wants_help(&args(&["a", "--", "--help"])));
    }

    #[test]
    fn names_the_kebab_case_meant() {
        assert_eq!(kebab_of("Bad_Name"), "bad-name");
        assert_eq!(kebab_of("12 My  Feature!"), "12-my-feature");
        assert_eq!(
            name_refusal("the feature", Some("Bad_Name"), "usage: x"),
            "the feature Bad_Name is not kebab-case (lowercase letters, digits, single dashes): bad-name?\nusage: x"
        );
        assert_eq!(
            name_refusal("the feature", None, "usage: x"),
            "the feature is missing\nusage: x"
        );
    }

    #[test]
    fn quotes_for_every_shell() {
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}
