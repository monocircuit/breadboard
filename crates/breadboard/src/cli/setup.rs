//! The commands that set up (#314): `COMMANDS` below.
//! Names, flags and wording as today's Node commands (rewrite rules 2, 3, 5);
//! what other groups and core should share is in their own modules here
//! until breadboard-core offers it (herdr, GitLab, local time).

mod browser;
pub(in crate::cli) mod changelog;
mod check;
mod commands;
mod conversations;
pub(in crate::cli) mod epic;
mod fragments;
mod gitlab;
mod herdr;
mod hook;
pub(crate) mod init;
mod new;
mod orchestrate;
mod proc;
mod repo;
mod stack;
mod sweep;
mod view;

pub(super) use proc::PLUGIN_CHECKED;
use proc::{Args, Shape, outln, refuse_misuse, wants_help};
use std::process::ExitCode;

/// A command: its name, what it takes and what runs it.
type Command = (&'static str, &'static Shape, fn(&Args) -> ExitCode);

/// The group's commands, sorted, one per line (#420: two MRs adding one each
/// change different lines, `cli::tests` checks the order): what each takes
/// (flags, those with a value, how many words) and its usage, kept in its
/// own module; anything else is refused before the command does anything (a
/// typo of `--shared` would make a worktree on its own stack and push its
/// claim; `wt:check foo` would check what `foo` doesn't name).
const COMMANDS: &[Command] = &[
    ("browser", &browser::SHAPE, |_| browser::command()),
    ("changelog", &changelog::SHAPE, changelog::overall_command),
    ("changelog:add", &fragments::ADD, |args| fragments::command("add", args)),
    ("changelog:check", &fragments::CHECK, |args| {
        fragments::command("check", args)
    }),
    ("changelog:notes", &fragments::OF_VERSION, |args| {
        fragments::command("notes", args)
    }),
    ("changelog:release", &fragments::OF_VERSION, |args| {
        fragments::command("release", args)
    }),
    ("changelog:unreleased", &fragments::UNRELEASED, |args| {
        fragments::command("unreleased", args)
    }),
    ("commands", &commands::SHAPE, |args| {
        commands::command(args.has("--all"))
    }),
    ("init", &init::SHAPE, init::command),
    ("stack:down", &stack::SHAPE, |args| stack::control("down", args)),
    ("stack:ps", &stack::SHAPE, |args| stack::control("ps", args)),
    ("stack:reset", &stack::SHAPE, |args| stack::control("reset", args)),
    ("stack:up", &stack::SHAPE, |args| stack::control("up", args)),
    ("wt:check", &check::SHAPE, check::command),
    ("wt:new", &new::SHAPE, new::command),
    ("wt:orchestrate", &orchestrate::SHAPE, orchestrate::command),
    ("wt:stack", &stack::WT_SHAPE, |_| stack::wt_stack()),
];

/// The commands no person types, which `usage` leaves out: git's pre-push hook.
const INTERNAL: &[Command] = &[("hook", &hook::SHAPE, |args| hook::command(&args.all))];

/// The group's commands, as `usage` lists them.
pub fn names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, ..)| *name).collect()
}

/// The name of the root package.json for the internal names that the herdr
/// plugin, tabs and the Node overview spawn today (`worktree.mjs control up`,
/// `platform-changelog check`, `new`, …), with the arguments after it.
fn public_of<'a>(command: &str, args: &'a [String]) -> Option<(String, &'a [String])> {
    let sub = || {
        args.first()
            .filter(|sub| !sub.starts_with("--"))
            .cloned()
            .unwrap_or_default()
    };
    let rest = args.get(1..).unwrap_or_default();
    Some(match command {
        "new" | "orchestrate" | "stack" | "check" => (format!("wt:{command}"), args),
        "control" if !sub().is_empty() => (format!("stack:{}", sub()), rest),
        "control" => ("stack:".to_string(), args),
        "platform-changelog" if !sub().is_empty() => (format!("changelog:{}", sub()), rest),
        "platform-changelog" => ("changelog:".to_string(), args),
        _ => return None,
    })
}

/// #230: a `wt:*` or `stack:*` run inside herdr finds the plugin linked
/// from a path that is gone and links it again (lifecycle's heal, once).
/// Each command calls it once nothing refused: a usage error asks herdr
/// nothing; a child of this binary (`proc::child`) leaves it to its parent.
pub fn heal_plugin() {
    if std::env::var_os(proc::PLUGIN_CHECKED).is_none()
        && let Some(main) = repo::find_main()
    {
        super::lifecycle::plugin::heal(&main);
    }
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let (command, args) = public_of(command, args).unwrap_or_else(|| (command.to_string(), args));
    let command = command.as_str();
    let args = Args::new(args);
    let Some((_, shape, run)) = COMMANDS.iter().chain(INTERNAL).find(|(name, ..)| *name == command) else {
        // A `stack:` or `changelog:` of no command: it says which there are.
        return if let Some(action) = command.strip_prefix("stack:") {
            Some(stack::control(action, &args))
        } else {
            command
                .strip_prefix("changelog:")
                .map(|sub| fragments::command(sub, &args))
        };
    };
    if wants_help(&args) {
        outln!("{}", breadboard_core::config::in_repo(shape.usage));
        return Some(ExitCode::SUCCESS);
    }
    refuse_misuse(&args, shape);
    Some(run(&args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn takes_the_internal_names_of_today() {
        let args = strings(&["up", "--port", "3002"]);
        let (name, rest) = public_of("control", &args).unwrap();
        assert_eq!((name.as_str(), rest), ("stack:up", &args[1..]));
        let args = strings(&["check", "--mr"]);
        assert_eq!(public_of("platform-changelog", &args).unwrap().0, "changelog:check");
        let args = strings(&["12-x", "--shared"]);
        assert_eq!(public_of("new", &args).unwrap(), ("wt:new".to_string(), &args[..]));
        assert_eq!(public_of("control", &[]).unwrap().0, "stack:");
        assert!(public_of("wt:new", &args).is_none());
    }

    #[test]
    fn every_command_names_its_flags() {
        for (command, shape, _) in COMMANDS.iter().chain(INTERNAL) {
            assert!(shape.usage.starts_with("usage:"), "{command}");
            assert!(shape.valued.iter().all(|flag| shape.flags.contains(flag)), "{command}");
        }
    }
}
