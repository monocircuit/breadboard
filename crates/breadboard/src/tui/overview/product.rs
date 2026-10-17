//! `overview --product <name>` (#405): the view of a product's herdr space,
//! every orchestration of the product a section of its own. Each is the
//! orchestration's overview as it was, with its readers and its actor, in
//! this one process; a click goes to the one it was drawn by. An
//! orchestration made later joins, one that ended leaves.

use super::{Act, Args, Msg, Overview, look, once, start_reading};
use crate::daemon;
use crate::tui::theme::{self, Tone};
use crate::tui::{App, Flow, Header, Input, Page, Section, view};
use breadboard_core::reader::{LocalReader, View};
use breadboard_core::record;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// A click or a pick of a section: the orchestration it belongs to and its own act.
type Tagged = (String, Act);

/// How often the records are looked at for an orchestration that came or went.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// One orchestration of the product: its overview and whether its readers still deliver.
struct Part {
    name: String,
    overview: Overview,
    /// Cleared when it leaves: its forwarder stops, its readers end with it.
    is_alive: Arc<AtomicBool>,
}

/// What a live view needs to start the readers of an orchestration that joins.
struct Live {
    runtime: tokio::runtime::Handle,
    sender: mpsc::UnboundedSender<(String, Msg)>,
}

pub struct Product {
    name: String,
    main: PathBuf,
    args: Args,
    parts: Vec<Part>,
    live: Option<Live>,
    looked_at: Instant,
    is_replaced: bool,
}

pub fn main(name: &str, args: &Args, main: &Path, cwd: &Path) -> ExitCode {
    let mut app = Product::new(name, args, main);
    if args.is_once {
        for (part, name) in app.parts.iter_mut().zip(record::orchestrations_of_product(main, name)) {
            let Some(reader) = LocalReader::new(main, View::Orchestration(name)) else {
                continue;
            };
            let code = once(&mut part.overview, reader);
            if code != ExitCode::SUCCESS {
                return code;
            }
        }
        print!("{}", view::once(&app, crate::tui::columns()));
        return ExitCode::SUCCESS;
    }
    let binary = crate::binary::Started::now();
    let runtime = match crate::tui::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("overview: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (sender, models) = mpsc::unbounded_channel();
    daemon::client::share_reads();
    app.live = Some(Live {
        runtime: runtime.handle().clone(),
        sender: sender.clone(),
    });
    let names: Vec<String> = app.parts.drain(..).map(|part| part.name).collect();
    for name in names {
        app.join(&name);
    }
    // A build installed over the binary: every part hears it.
    let replaced = Arc::new(binary);
    runtime.spawn(super::watch_binary(
        replaced.clone(),
        forward(
            &runtime.handle().clone(),
            String::new(),
            sender,
            Arc::new(AtomicBool::new(true)),
        ),
    ));
    let code = crate::tui::serve(runtime, &mut app, models);
    if app.is_replaced {
        let _ = std::env::set_current_dir(cwd);
        let error = replaced.exec(std::env::args_os().skip(1), &[]);
        eprintln!("overview: the new build didn't start: {error}");
    }
    code
}

/// A sender of a part's models that hands them on tagged with `name`, while `is_alive`.
fn forward(
    runtime: &tokio::runtime::Handle,
    name: String,
    out: mpsc::UnboundedSender<(String, Msg)>,
    is_alive: Arc<AtomicBool>,
) -> mpsc::UnboundedSender<Msg> {
    let (sender, mut models) = mpsc::unbounded_channel();
    runtime.spawn(async move {
        while let Some(model) = models.recv().await {
            if !is_alive.load(Ordering::Relaxed) || out.send((name.clone(), model)).is_err() {
                return;
            }
        }
    });
    sender
}

impl Product {
    pub fn new(name: &str, args: &Args, main: &Path) -> Self {
        let mut product = Product {
            name: name.to_string(),
            main: main.to_path_buf(),
            args: args.clone(),
            parts: Vec::new(),
            live: None,
            looked_at: Instant::now(),
            is_replaced: false,
        };
        for name in record::orchestrations_of_product(main, name) {
            product.join(&name);
        }
        product
    }

    /// The orchestration `name` as a part, its readers started in a live view.
    fn join(&mut self, name: &str) {
        let args = Args {
            view: View::Orchestration(name.to_string()),
            product: None,
            is_herdr: false,
            ..self.args.clone()
        };
        let mut overview = Overview::new(&args, &self.main);
        let is_alive = Arc::new(AtomicBool::new(true));
        if let Some(live) = &self.live
            && let Some(reader) = LocalReader::new(&self.main, args.view.clone())
        {
            let sender = forward(&live.runtime, name.to_string(), live.sender.clone(), is_alive.clone());
            start_reading(&mut overview, reader, &live.runtime, &sender);
        }
        self.parts.push(Part {
            name: name.to_string(),
            overview,
            is_alive,
        });
    }

    /// The parts as the records say: a new orchestration of the product
    /// joins, one whose record went leaves (not while it is busy).
    fn follow_records(&mut self) {
        if self.live.is_none() || self.looked_at.elapsed() < LOOK_EVERY {
            return;
        }
        self.looked_at = Instant::now();
        let names = record::orchestrations_of_product(&self.main, &self.name);
        self.parts.retain(|part| {
            let is_kept = names.contains(&part.name) || part.overview.actor.is_busy();
            if !is_kept {
                part.is_alive.store(false, Ordering::Relaxed);
            }
            is_kept
        });
        for name in names {
            if !self.parts.iter().any(|part| part.name == name) {
                self.join(&name);
            }
        }
        self.parts.sort_by(|a, b| a.name.cmp(&b.name));
    }

    fn part_mut(&mut self, name: &str) -> Option<&mut Overview> {
        self.parts
            .iter_mut()
            .find(|part| part.name == name)
            .map(|part| &mut part.overview)
    }
}

impl App for Product {
    type Action = Tagged;
    type Model = (String, Msg);

    /// The band: the product's name, the rest as its first orchestration's (GitLab, the orchestrator).
    fn header(&self) -> Header {
        let mut header = match self.parts.first() {
            Some(part) => part.overview.header(),
            None => Header::new(""),
        };
        header.title = self.name.clone();
        if self.args.is_once {
            header.keys = String::new();
        }
        header
    }

    fn build(&self, page: &mut Page<Tagged>) {
        if self.parts.is_empty() {
            page.section(Section::new(self.name.to_uppercase()), |scene| {
                look::flow(
                    scene,
                    &[look::dim(format!(
                        "no orchestration of {} yet: wt:orchestrate <name> <issue>… starts one",
                        self.name
                    ))],
                );
            });
            return;
        }
        // One orchestration keeps its NEXT on top; of several, each scrolls under its title.
        let is_alone = self.parts.len() == 1;
        let picked = page.picked().cloned();
        for part in &self.parts {
            let title = match part.overview.orchestration() {
                Some(model) => model.label.clone(),
                None => format!("{} {}", record::ORCHESTRATOR, part.name),
            };
            let mut section = Section::new(title.clone()).plain();
            if is_alone {
                section = section.sticky();
            }
            page.section(section, |scene| {
                scene.pad_to(1);
                scene.text(&title, theme::bold().patch(theme::tone(Tone::Cyan)));
            });
            let own = picked
                .as_ref()
                .filter(|(name, _)| *name == part.name)
                .map(|(_, act)| act.clone());
            let mut inner = Page::new(page.width()).picking(own);
            part.overview.build(&mut inner);
            let name = part.name.clone();
            page.embed(inner, move |act| (name.clone(), act), is_alone);
        }
    }

    fn update(&mut self, (name, message): (String, Msg)) {
        if name.is_empty() {
            // The binary was replaced: every part goes on in the new build.
            if matches!(message, Msg::Replaced) {
                self.is_replaced = true;
                for part in &mut self.parts {
                    part.overview.update(Msg::Replaced);
                }
            }
            return;
        }
        if let Some(overview) = self.part_mut(&name) {
            overview.update(message);
        }
        self.follow_records();
    }

    fn act(&mut self, input: Input<Tagged>) -> Flow {
        match input {
            Input::Refresh => {
                for part in &mut self.parts {
                    part.overview.act(Input::Refresh);
                }
                Flow::Same
            }
            Input::Click((name, act)) => match self.part_mut(&name) {
                Some(overview) => overview.act(Input::Click(act)),
                None => Flow::Same,
            },
            Input::Key(key, picked) => {
                // A prompt typed into, else the picked block's part, else the first.
                let typing = self.parts.iter().position(|part| part.overview.is_typing());
                let (at, act) = match (typing, picked) {
                    (Some(at), _) => (Some(at), None),
                    (None, Some((name, act))) => (self.parts.iter().position(|part| part.name == name), Some(act)),
                    (None, None) => (Some(0), None),
                };
                match at.and_then(|at| self.parts.get_mut(at)) {
                    Some(part) => part.overview.act(Input::Key(key, act)),
                    None => Flow::Same,
                }
            }
        }
    }

    fn picked(&mut self, picked: Option<&Tagged>) -> Flow {
        let Some((name, act)) = picked else {
            return Flow::Same;
        };
        match self.part_mut(name) {
            Some(overview) => overview.picked(Some(act)),
            None => Flow::Same,
        }
    }

    fn is_over(&self) -> bool {
        self.is_replaced && self.parts.iter().all(|part| part.overview.is_over())
    }

    fn is_typing(&self) -> bool {
        self.parts.iter().any(|part| part.overview.is_typing())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_product_without_orchestrations_says_how_to_start_one() {
        let main = std::env::temp_dir().join(format!("product-view-{}", std::process::id()));
        let args = Args {
            view: View::Everything,
            is_once: true,
            is_herdr: false,
            product: Some("vita".into()),
        };
        let app = Product::new("vita", &args, &main);
        let text = view::once(&app, 80);
        assert!(text.contains("vita"), "{text}");
        assert!(text.contains("no orchestration of vita yet"), "{text}");
    }
}
