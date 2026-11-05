// SPDX-License-Identifier: GPL-3.0-or-later
//! consult for Forge: jumping commands with a completing-read prompt and
//! live preview. Pick a candidate from a list; point moves to it as the
//! cursor moves over the list (C-n / C-p), RET jumps for good, C-g
//! returns to where you were.
//!
//! - `M-x consult-buffer` — switch to a buffer, recent file or bookmark
//!   (type `[recent]`, `[buffer]` or `[bookmark]` to narrow).
//! - Extensions: `consult-dir` (a directory), `consult-ls-git` (a tracked
//!   file), `consult-todo` (a TODO/FIXME in the project).
//! - `M-x consult-buffer-other-window` — the same in another window.
//! - `M-x consult-register-load`, `consult-yank-replace` — consult's names
//!   for `consult-register` and `consult-yank-pop`.
//! - `M-x consult-project-buffer` — a buffer in this project.
//! - `M-x consult-line` — a line in this buffer, starting from the line
//!   point is on. One prompt: type to narrow the lines (with your completion
//!   style), and point follows the selection live; C-g puts it back.
//! - `M-x consult-line-multi` — the same over any (project) buffer.
//! - `M-x consult-outline` — an Org or Markdown heading.
//! - `M-x consult-org-heading` — an Org heading, with its outline path.
//! - `M-x consult-goto-line` — a line number.
//! - `M-x consult-minor-mode-menu` — toggle a minor mode.
//! - `M-x consult-grep`, `consult-ripgrep`, `consult-git-grep` — grep the
//!   project in two stages: enter a pattern, pick a match.
//! - `M-x consult-find`, `consult-fd` — find a file by name, likewise.
//! - `M-x consult-locate`, `consult-man` — find with `locate`/`mdfind`,
//!   read a manual page with `man`.
//! - `M-x consult-grep-match` — a match in a Grep buffer.
//! - `M-x consult-compile-error` — an error in a compilation buffer.
//! - `M-x consult-keep-lines` — keep only lines matching a regexp.
//! - `M-x consult-focus-lines` — hide lines matching a regexp.
//! - `M-x consult-imenu` — jump to a definition in this buffer.
//! - `M-x consult-imenu-multi` — a definition in any buffer.
//! - `M-x consult-mode-command` — run a command of an enabled mode.
//! - `M-x consult-yank-from-kill-ring`, `consult-yank-pop` — yank an
//!   older kill.
//! - `M-x consult-recent-file`, `consult-bookmark` — jump to a recent
//!   file or a bookmark.
//! - `M-x consult-register`, `consult-kmacro`, `consult-mark`,
//!   `consult-global-mark` — insert a register (or jump to a position
//!   register), run a keyboard macro, or jump to a mark.
//! - `M-x consult-flymake` — a diagnostic in this buffer, or in every
//!   buffer with a prefix argument.
//! - `M-x consult-xref` — jump to a published cross-reference.
//! - `M-x consult-theme` — apply a named theme, previewing as you move.
//! - `M-x consult-org-agenda` — jump to an Org agenda entry.
//! - `M-x consult-history` (bound to `M-r` in prompts) — insert a past
//!   answer into the prompt you are in.
//! - `M-x consult-isearch-history` (bound to `M-r` during Isearch) — pick a
//!   past search string and resume the search with it.
//!
//! Preview on selection movement needs Forge 0.1.20
//! (`completion.on-selection-changed`); preview as the prompt narrows needs
//! Forge 0.1.30 (`minibuffer.on-input-changed` and
//! `minibuffer.on-selection-changed`). The kill-ring, session, imenu and
//! command-list commands need Forge 0.1.21;
//! the register, keyboard-macro and mark commands need Forge 0.1.22;
//! jumping to a position register needs Forge 0.1.23; the theme,
//! org-agenda and history commands need Forge 0.1.24. The commands work
//! without it; you just don't see the jump until RET.
//!
//!   cargo build --release --target wasm32-wasip2
//!   cp target/wasm32-wasip2/release/forge_consult.wasm ~/.config/forge/plugins/consult.wasm

use forge_plugin_sdk::{
    AgendaEntry, Buffer, Decoration, Position, Replacement, Scope, Span, bind_in_prompt, command,
    commands, completing_read, completing_read_over, decorations, forge_plugin, keymap, kill_ring,
    kmacro, marks, message, minibuffer, modes, on_minibuffer_input_changed,
    on_minibuffer_selection_changed, on_selection_changed, org, project_root, read_string,
    registers, selected, session, spawn, spawn_with, theme, xref,
};
use std::cell::{Cell, RefCell};

/// The command the selection-changed hook runs (consult's live preview).
const PREVIEW: &str = "consult-preview-current";

/// Never offer more than this many candidates; scanning every buffer in
/// a large session shouldn't hang the prompt.
const MAX_CANDIDATES: usize = 5000;

/// Where a candidate points.
#[derive(Clone, Debug)]
enum Target {
    /// A line in an open buffer (`consult-line`, `consult-outline`).
    Line { buffer: u32, line: u32 },
    /// A line in a file, which may need opening (grep results).
    File { path: String, line: u32 },
    /// A buffer to show (`consult-buffer`).
    Buffer { id: u32 },
    /// A line number in the buffer the command started in.
    GotoLine { line: u32 },
    /// A minor mode to turn on or off (`consult-minor-mode-menu`).
    /// Not previewed: toggling on every cursor move would be chaos.
    MinorMode { mode: String, enable: bool },
    /// A manual page (`consult-man`): section and page name.
    Man {
        section: Option<String>,
        name: String,
    },
    /// A kill-ring entry to insert (`consult-yank-from-kill-ring`).
    /// Not previewed: inserting on every cursor move would be chaos.
    Kill { index: u32, text: String },
    /// A place in a file, by char offset (`consult-bookmark`).
    FileAt { path: String, offset: u32 },
    /// A command to run (`consult-mode-command`). Not previewed: running
    /// on every cursor move would be chaos.
    RunCommand { name: String },
    /// Register `name` to insert (`consult-register`). `text` is what it
    /// holds; a position register is a `MarkAt` instead. Inserting on every
    /// cursor move would be chaos, so preview shows the text instead.
    Register { name: String, text: Option<String> },
    /// A remembered keyboard macro to run (`consult-kmacro`). Not
    /// previewed: running the macro on every cursor move would be chaos.
    Kmacro { index: u32 },
    /// A mark to jump to (`consult-mark`, `consult-global-mark`).
    MarkAt { buffer: u32, offset: u32 },
    /// A theme to apply (`consult-theme`). Previewed: moving over the
    /// list applies each theme in turn; C-g puts the old one back.
    Theme { name: String },
}

thread_local! {
    /// A consult prompt is open: the preview command may act.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// `consult-line` prompts preview the first line matching new input,
    /// even before the completion list has a selected candidate.
    static LINE_INPUT_PREVIEW: Cell<bool> = const { Cell::new(false) };
    /// Candidate text -> where it points, for the open prompt.
    static TARGETS: RefCell<Vec<(String, Target)>> = const { RefCell::new(Vec::new()) };
    /// Where point was before the prompt opened (C-g returns here).
    static ORIGIN: RefCell<Option<(u32, Position)>> = const { RefCell::new(None) };
    /// A grep process's output, gathered until it exits.
    static GREP_OUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// A locate/mdfind process's output.
    static LOCATE_OUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// A man page being fetched (killed if the prompt moves on).
    static MAN_PROC: RefCell<Option<forge_plugin_sdk::Process>> = const { RefCell::new(None) };
    /// The current man process's output.
    static MAN_OUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// `man -k`'s listing output.
    static MAN_K_OUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// The man page currently in its buffer (`section/name`), to skip refetching.
    static MAN_KEY: RefCell<String> = const { RefCell::new(String::new()) };
    /// The man page currently being fetched, if any.
    static MAN_PENDING: RefCell<String> = const { RefCell::new(String::new()) };
    /// Generation counter: a killed process's late output is discarded.
    static MAN_GEN: Cell<u64> = const { Cell::new(0) };
    /// A man fetch started during preview: don't show or warn if the
    /// prompt has since closed.
    static MAN_PREVIEWING: Cell<bool> = const { Cell::new(false) };
}

/// Candidates paired with their targets, as `completing_read` wants.
type CandidateSet = (Vec<String>, Vec<(String, Target)>);

// ------------------------------------------------------- the ask helper

/// Open the completing-read prompt over `candidates`, each pointing at
/// its `targets` entry. Saves where point was, so C-g can return there;
/// the selection-changed hook previews the candidate under the cursor.
fn ask(prompt: &str, candidates: Vec<String>, targets: Vec<(String, Target)>) {
    if candidates.is_empty() {
        message("No candidates");
        return;
    }
    save_origin();
    TARGETS.with(|t| *t.borrow_mut() = targets);
    ACTIVE.with(|a| a.set(true));
    completing_read(prompt, &candidates, true, |answer| {
        ACTIVE.with(|a| a.set(false));
        LINE_INPUT_PREVIEW.with(|p| p.set(false));
        let targets = TARGETS.with(|t| std::mem::take(&mut *t.borrow_mut()));
        match answer {
            Some(text) => {
                if let Some(t) = targets.iter().find(|(k, _)| *k == text).map(|(_, v)| v) {
                    if !matches!(t, Target::Man { .. }) {
                        // A man fetch left previewing mustn't land later.
                        cancel_man();
                    }
                    jump(t);
                }
                // Consumed: a later C-g shouldn't jump back here.
                ORIGIN.with(|o| *o.borrow_mut() = None);
            }
            None => restore_origin(),
        }
    });
}

fn ask_lines(prompt: &str, candidates: Vec<String>, targets: Vec<(String, Target)>) {
    LINE_INPUT_PREVIEW.with(|p| p.set(!candidates.is_empty()));
    ask(prompt, candidates, targets);
}

fn save_origin() {
    let b = Buffer::current();
    ORIGIN.with(|o| *o.borrow_mut() = Some((b.id(), b.point())));
}

fn restore_origin() {
    let origin = ORIGIN.with(|o| o.borrow_mut().take());
    if let Some((id, pos)) = origin
        && let Some(b) = Buffer::from_id(id)
        && b.alive()
    {
        b.show();
        b.set_point(pos);
    }
}

/// Move point to `t`: show the buffer (opening the file if needed) and
/// go to the line (1-based, as grep and Emacs count).
fn jump(t: &Target) {
    match t {
        Target::Line { buffer, line } => {
            if let Some(b) = Buffer::from_id(*buffer)
                && b.alive()
            {
                b.show();
                b.set_point(line_pos(*line));
            }
        }
        Target::File { path, line } => {
            if let Ok(b) = Buffer::visit(path) {
                b.show();
                b.set_point(line_pos(*line));
            }
        }
        Target::Buffer { id } => {
            if let Some(b) = Buffer::from_id(*id)
                && b.alive()
            {
                b.show();
            }
        }
        Target::GotoLine { line } => {
            Buffer::current().set_point(line_pos(*line));
        }
        Target::MinorMode { mode, enable } => {
            let b = Buffer::current();
            if *enable {
                let _ = b.enable_minor_mode(mode);
            } else {
                b.disable_minor_mode(mode);
            }
        }
        Target::Man { section, name } => {
            man_page(section.clone(), name.clone());
        }
        Target::Kill { index, text } => {
            kill_ring::select(*index);
            Buffer::current().insert(text);
        }
        Target::FileAt { path, offset } => {
            if let Ok(b) = Buffer::visit(path) {
                b.show();
                b.set_point(offset_to_position(&b.text(), *offset));
            }
        }
        Target::RunCommand { name } => {
            let _ = commands::run(name);
        }
        Target::Register { name, text } => match text {
            Some(t) => Buffer::current().insert(t),
            None => message(&format!("Register {name} holds no text")),
        },
        Target::Kmacro { index } => {
            kmacro::select(*index);
            let _ = commands::run("call-last-kbd-macro");
        }
        Target::MarkAt { buffer, offset } => {
            if let Some(b) = Buffer::from_id(*buffer)
                && b.alive()
            {
                b.show();
                b.set_point(offset_to_position(&b.text(), *offset));
            }
        }
        Target::Theme { name } => {
            if theme::load(name).is_err() {
                message(&format!("No theme named {name}"));
            }
        }
    }
}

/// A char offset in `text` as the (line, column) position `set_point`
/// wants.
fn offset_to_position(text: &str, offset: u32) -> Position {
    let mut line = 0u32;
    let mut column = 0u32;
    for (i, c) in text.chars().enumerate() {
        if i as u32 >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    Position { line, column }
}

/// A 1-based line number as the 0-based position `set_point` wants,
/// at column 0.
fn line_pos(line: u32) -> Position {
    Position {
        line: line.saturating_sub(1),
        column: 0,
    }
}

// ------------------------------------------------------------- preview

/// Show the candidate under the cursor in the open list (consult's live
/// preview, driven by `completion.on-selection-changed`).
fn preview_current(_prefix: Option<i64>) -> Result<(), String> {
    if !ACTIVE.with(Cell::get) {
        return Ok(());
    }
    let selection = selected().map(|sel| sel.text);
    let input = minibuffer::input();
    let text = if LINE_INPUT_PREVIEW.with(Cell::get) && !input.is_empty() {
        if selection
            .as_ref()
            .is_some_and(|candidate| line_matches(&input, candidate))
        {
            selection
        } else {
            TARGETS.with(|targets| first_input_match(&input, &targets.borrow()))
        }
    } else {
        selection
    };
    let Some(text) = text else {
        return Ok(());
    };
    let found = preview_target(&text);
    // Minor modes aren't previewed; the rest jump at once. Moving off a
    // man candidate cancels its fetch.
    if let Some(t) = found {
        if matches!(t, Target::Man { .. }) {
            jump(&t);
        } else {
            cancel_man();
            match t {
                Target::MinorMode { .. } | Target::RunCommand { .. } | Target::Kmacro { .. } => {}
                Target::Kill { text, .. } => message(&kill_preview(&text, 100)),
                Target::Register { text, .. } => {
                    if let Some(t) = text {
                        message(&kill_preview(&t, 100));
                    }
                }
                other => jump(&other),
            }
        }
    }
    Ok(())
}

fn preview_target(selection: &str) -> Option<Target> {
    TARGETS.with(|t| {
        t.borrow()
            .iter()
            .find(|(candidate, _)| candidate == selection)
            .map(|(_, target)| target.clone())
    })
}

fn first_input_match(query: &str, candidates: &[(String, Target)]) -> Option<String> {
    candidates
        .iter()
        .find(|(candidate, _)| line_matches(query, candidate))
        .map(|(candidate, _)| candidate.clone())
}

// ----------------------------------------------------------- matching

/// Orderless line matching, the way `consult-line` filters: every
/// space-separated part must appear, case-insensitive unless the query
/// has capitals.
fn line_matches(query: &str, text: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let case_sensitive = query.chars().any(|c| c.is_uppercase());
    let hay = if case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    query.split_whitespace().all(|part| {
        let needle = if case_sensitive {
            part.to_string()
        } else {
            part.to_lowercase()
        };
        hay.contains(&needle)
    })
}

// --------------------------------------------------------- the commands

/// The sources `consult-buffer` merges, as consult does: open buffers,
/// then recent files not open, then bookmarks. Each candidate ends with
/// its source ("buffer", "recent", "bookmark"), so typing that word
/// narrows to it the way consult's `b SPC` / `f SPC` / `m SPC` do.
fn buffer_sources() -> CandidateSet {
    let ids: Vec<u32> = Buffer::all()
        .iter()
        .filter(|b| b.alive())
        .map(|b| b.id())
        .collect();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut open_paths = Vec::new();
    for id in ids {
        let Some(b) = Buffer::from_id(id) else {
            continue;
        };
        let name = b.name();
        let file = b.path().map(|p| short_path(&p)).unwrap_or_default();
        if let Some(p) = b.path() {
            open_paths.push(p);
        }
        let display = if file.is_empty() {
            format!("{name}  [buffer]")
        } else {
            format!("{name}  {file}  [buffer]")
        };
        candidates.push(display.clone());
        targets.push((display, Target::Buffer { id }));
    }
    for path in session::recent_files() {
        if open_paths.contains(&path) {
            continue;
        }
        let display = format!("{}  [recent]", path);
        candidates.push(display.clone());
        targets.push((display, Target::File { path, line: 1 }));
    }
    for name in session::bookmark_names() {
        let Some(bm) = session::bookmark(&name) else {
            continue;
        };
        let display = format!("{name}  [bookmark]");
        candidates.push(display.clone());
        targets.push((
            display,
            Target::FileAt {
                path: bm.path,
                offset: bm.offset,
            },
        ));
    }
    (candidates, targets)
}

/// Switch to an open buffer, recent file or bookmark (`consult-buffer`).
fn consult_buffer(_prefix: Option<i64>) -> Result<(), String> {
    let (candidates, targets) = buffer_sources();
    ask("Switch to buffer: ", candidates, targets);
    Ok(())
}

/// The same, shown in another window (`consult-buffer-other-window`).
fn consult_buffer_other_window(_prefix: Option<i64>) -> Result<(), String> {
    let _ = commands::run("split-window-sensibly");
    let _ = commands::run("other-window");
    let (candidates, targets) = buffer_sources();
    ask("Switch to buffer in other window: ", candidates, targets);
    Ok(())
}

// ------------------------------------------------------ the extensions

/// Run `program` in the project and offer its output (no pattern prompt).
fn run_then_ask(
    prompt: &str,
    program: &str,
    args: Vec<String>,
    parse: fn(&str, &str) -> CandidateSet,
) {
    let dir = Buffer::current()
        .path()
        .and_then(|p| project_root(&p))
        .unwrap_or_else(|| ".".into());
    GREP_OUT.with(|o| o.borrow_mut().clear());
    let prompt = prompt.to_string();
    let for_exit = dir.clone();
    let spawned = spawn(
        program,
        &args,
        Some(&dir),
        |data| {
            if let Ok(text) = std::str::from_utf8(data) {
                GREP_OUT.with(|o| o.borrow_mut().push_str(text));
            }
        },
        move |_code| {
            let out = GREP_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
            let (candidates, targets) = parse(&out, &for_exit);
            ask(&prompt, candidates, targets);
        },
    );
    if let Err(e) = spawned {
        message(&format!("{program}: {e}"));
    }
}

/// Files git tracks (consult-ls-git).
fn consult_ls_git(_prefix: Option<i64>) -> Result<(), String> {
    run_then_ask(
        "Git file: ",
        "git",
        vec![
            "ls-files".into(),
            "--cached".into(),
            "--others".into(),
            "--exclude-standard".into(),
        ],
        parse_paths,
    );
    Ok(())
}

/// TODO, FIXME, HACK and XXX comments in the project (consult-todo).
fn consult_todo(prefix: Option<i64>) -> Result<(), String> {
    let _ = prefix;
    run_then_ask(
        "Todo: ",
        "rg",
        vec![
            "-n".into(),
            "--no-heading".into(),
            r"\b(TODO|FIXME|HACK|XXX|BUG)\b".into(),
        ],
        parse_grep,
    );
    Ok(())
}

/// Jump to a directory in Dired (consult-dir): the project, the folders of
/// recent files and of bookmarks.
fn consult_dir(_prefix: Option<i64>) -> Result<(), String> {
    let mut dirs: Vec<(String, String)> = Vec::new();
    let mut add = |d: String, why: &str| {
        if !d.is_empty() && !dirs.iter().any(|(x, _)| *x == d) {
            dirs.push((d, why.to_string()));
        }
    };
    if let Some(root) = Buffer::current().path().and_then(|p| project_root(&p)) {
        add(root, "project");
    }
    let parent = |p: &str| {
        p.rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default()
    };
    for f in session::recent_files() {
        add(parent(&f), "recent");
    }
    for name in session::bookmark_names() {
        if let Some(b) = session::bookmark(&name) {
            add(parent(&b.path), "bookmark");
        }
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for (d, why) in dirs {
        let display = format!("{d}  [{why}]");
        candidates.push(display.clone());
        targets.push((display, Target::File { path: d, line: 1 }));
    }
    ask("Directory: ", candidates, targets);
    Ok(())
}

/// `/home/me/x.rs` -> `x.rs`; `/home/me/src/x.rs` -> `src/x.rs`.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 2 {
        path.to_string()
    } else {
        parts[parts.len() - 2..].join("/")
    }
}

/// A line in this buffer (`consult-line`). One prompt, as in consult:
/// every line is a candidate, typing narrows them with the completion
/// style (orderless, if that's what you use), and point follows the
/// selection as it moves or the list narrows.
fn consult_line(_prefix: Option<i64>) -> Result<(), String> {
    let b = Buffer::current();
    let bid = b.id();
    let n = b.line_count();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    // Start at the line point is on and wrap round, as consult-line does,
    // so the nearest match is the first candidate.
    let from = b.point().line.min(n.saturating_sub(1));
    for i in (from..n).chain(0..from) {
        let text = b.line(i);
        if text.trim().is_empty() {
            continue;
        }
        let display = format!("{}: {}", i + 1, text.trim_end());
        candidates.push(display.clone());
        targets.push((
            display,
            Target::Line {
                buffer: bid,
                line: i + 1,
            },
        ));
        if candidates.len() >= MAX_CANDIDATES {
            message("Too many lines; showing the first ones");
            break;
        }
    }
    ask_lines("Consult line: ", candidates, targets);
    Ok(())
}

/// An Org (`*`) or Markdown (`#`) heading (`consult-outline`).
fn consult_outline(_prefix: Option<i64>) -> Result<(), String> {
    let bid = Buffer::current().id();
    let Some(b) = Buffer::from_id(bid) else {
        return Err("no buffer".into());
    };
    let n = b.line_count();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for i in 0..n {
        let text = b.line(i);
        if is_heading(&text) {
            let display = format!("{}: {}", i + 1, text.trim());
            candidates.push(display.clone());
            targets.push((
                display,
                Target::Line {
                    buffer: bid,
                    line: i + 1,
                },
            ));
        }
    }
    ask("Outline: ", candidates, targets);
    Ok(())
}

/// `* Heading` (Org) or `# Heading` (Markdown).
fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    for marker in ['*', '#'] {
        if t.starts_with(marker) {
            let rest = t.trim_start_matches(marker);
            if rest.is_empty() || rest.starts_with(' ') {
                return true;
            }
        }
    }
    false
}

/// A line number in this buffer (`consult-goto-line`).
fn consult_goto_line(_prefix: Option<i64>) -> Result<(), String> {
    let b = Buffer::current();
    let n = b.line_count();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for i in 0..n {
        let display = format!("{}: {}", i + 1, b.line(i).trim_end());
        candidates.push(display.clone());
        targets.push((display, Target::GotoLine { line: i + 1 }));
    }
    ask("Goto line: ", candidates, targets);
    Ok(())
}

/// Toggle a minor mode in this buffer (`consult-minor-mode-menu`).
fn consult_minor_mode_menu(_prefix: Option<i64>) -> Result<(), String> {
    let b = Buffer::current();
    let all = modes::minor_modes();
    let enabled: Vec<String> = b.minor_modes();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for mode in all {
        let on = enabled.contains(&mode);
        let display = format!("{mode} [{}]", if on { "on" } else { "off" });
        candidates.push(display.clone());
        targets.push((display, Target::MinorMode { mode, enable: !on }));
    }
    ask("Minor mode: ", candidates, targets);
    Ok(())
}

// ------------------------------------------------------- two-stage grep

/// Grep the project in two stages: `read_string` for the pattern, a
/// process for the matches, then a completing-read over them. Relative
/// paths in the output are read against `dir`, the directory the process
/// ran in.
fn process_two_stage(
    prompt: &str,
    program: &str,
    make_args: impl Fn(&str) -> Vec<String> + 'static,
    parse: fn(&str, &str) -> CandidateSet,
) {
    let prompt = prompt.to_string();
    let program = program.to_string();
    read_string(&format!("{prompt}: "), "", move |pattern| {
        let Some(pattern) = pattern else { return };
        if pattern.trim().is_empty() {
            return;
        }
        let args = make_args(&pattern);
        let dir = Buffer::current()
            .path()
            .and_then(|p| project_root(&p))
            .unwrap_or_else(|| ".".into());
        GREP_OUT.with(|o| o.borrow_mut().clear());
        let p2 = prompt.clone();
        let for_exit = dir.clone();
        let spawned = spawn(
            &program,
            &args,
            Some(&dir),
            |data| {
                if let Ok(text) = std::str::from_utf8(data) {
                    GREP_OUT.with(|o| o.borrow_mut().push_str(text));
                }
            },
            move |_code| {
                let out = GREP_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
                let (candidates, targets) = parse(&out, &for_exit);
                ask(&format!("{p2}: "), candidates, targets);
            },
        );
        if let Err(e) = spawned {
            message(&format!("{program}: {e}"));
        }
    });
}

/// `./src/a.rs` under `/p` becomes `/p/src/a.rs`; absolute paths and
/// `~/...` (which `buffer.visit` expands) are left alone.
fn absolutize(path: &str, dir: &str) -> String {
    if path.starts_with('/') || path.starts_with("~/") || dir.is_empty() {
        path.to_string()
    } else {
        let rel = path.strip_prefix("./").unwrap_or(path);
        format!("{}/{rel}", dir.trim_end_matches('/'))
    }
}

/// Parse `grep -n` / `rg` / `git grep` output: `path:line:text` per line.
fn parse_grep(out: &str, dir: &str) -> CandidateSet {
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(ln), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Ok(n) = ln.parse::<u32>() else { continue };
        let display = format!("{path}:{ln}:{text}");
        candidates.push(display.clone());
        targets.push((
            display,
            Target::File {
                path: absolutize(path, dir),
                line: n,
            },
        ));
    }
    (candidates, targets)
}

/// Parse `find` / `fd` / `locate` output: one path per line; open at line 1.
fn parse_paths(out: &str, dir: &str) -> CandidateSet {
    let candidates: Vec<String> = out
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let targets = candidates
        .iter()
        .map(|c| {
            (
                c.clone(),
                Target::File {
                    path: absolutize(c, dir),
                    line: 1,
                },
            )
        })
        .collect();
    (candidates, targets)
}

/// Grep the project (`consult-grep`).
fn consult_grep(_prefix: Option<i64>) -> Result<(), String> {
    process_two_stage(
        "Grep",
        "grep",
        |p| vec!["-rn".into(), "-e".into(), p.to_string(), ".".into()],
        parse_grep,
    );
    Ok(())
}

/// Ripgrep the project (`consult-ripgrep`).
fn consult_ripgrep(_prefix: Option<i64>) -> Result<(), String> {
    process_two_stage(
        "Ripgrep",
        "rg",
        |p| vec!["-n".into(), "--no-heading".into(), p.to_string()],
        parse_grep,
    );
    Ok(())
}

/// `git grep` the project (`consult-git-grep`).
fn consult_git_grep(_prefix: Option<i64>) -> Result<(), String> {
    process_two_stage(
        "Git grep",
        "git",
        |p| vec!["grep".into(), "-n".into(), "-e".into(), p.to_string()],
        parse_grep,
    );
    Ok(())
}

/// Find files by name with `find` (`consult-find`).
fn consult_find(_prefix: Option<i64>) -> Result<(), String> {
    process_two_stage(
        "Find",
        "find",
        |p| vec![".".into(), "-iname".into(), format!("*{p}*")],
        parse_paths,
    );
    Ok(())
}

/// Find files by name with `fd` (`consult-fd`).
fn consult_fd(_prefix: Option<i64>) -> Result<(), String> {
    process_two_stage("Fd", "fd", |p| vec![p.to_string()], parse_paths);
    Ok(())
}

// ------------------------------------------------ line-multi / project-buffer

/// A line in any buffer, or in this project's buffers with a prefix
/// (`consult-line-multi`).
fn consult_line_multi(prefix: Option<i64>) -> Result<(), String> {
    let all = prefix.is_some();
    let root = Buffer::current().path().and_then(|p| project_root(&p));
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut truncated = false;
    'buffers: for b in Buffer::all() {
        if !b.alive() {
            continue;
        }
        if !all && let Some(root) = &root {
            match b.path() {
                Some(p) if project_root(&p).as_deref() == Some(root.as_str()) => {}
                _ => continue,
            }
        }
        let name = b.name();
        let id = b.id();
        for (i, line) in b.text().lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let display = format!("{name}:{}: {}", i + 1, line.trim_end());
            candidates.push(display.clone());
            targets.push((
                display,
                Target::Line {
                    buffer: id,
                    line: i as u32 + 1,
                },
            ));
            if candidates.len() >= MAX_CANDIDATES {
                truncated = true;
                break 'buffers;
            }
        }
    }
    if truncated {
        message("Too many lines; showing the first ones");
    }
    ask_lines("Line (multi): ", candidates, targets);
    Ok(())
}

/// A buffer in this project (`consult-project-buffer`).
fn consult_project_buffer(_prefix: Option<i64>) -> Result<(), String> {
    let cur = Buffer::current();
    let Some(path) = cur.path() else {
        return Err("Not in a project".into());
    };
    let Some(root) = project_root(&path) else {
        return Err("Not in a project".into());
    };
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for b in Buffer::all() {
        if !b.alive() {
            continue;
        }
        let Some(p) = b.path() else { continue };
        if project_root(&p).as_deref() != Some(root.as_str()) {
            continue;
        }
        let name = b.name();
        let file = short_path(&p);
        let display = format!("{name}  {file}");
        candidates.push(display.clone());
        targets.push((display, Target::Buffer { id: b.id() }));
    }
    ask("Project buffer: ", candidates, targets);
    Ok(())
}

// ------------------------------------------------------- compile errors

/// Mirror of the host's `parse_location` regexes (crates/core/src/process.rs).
static RUST_LOC: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^\s*--> ([^\s:][^:]*):(\d+):(\d+)").unwrap());
static PYTHON_LOC: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r#"^\s*File "([^"]+)", line (\d+)"#).unwrap());
static GNU_LOC: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^([^\s:][^:\n]*?):(\d+)(?::(\d+))?[:\s]").unwrap()
});
static TSC_LOC: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^([^\s(][^(\n]*)\((\d+),(\d+)\): ").unwrap());

/// The file and 1-based line a line of compiler/grep output points at.
fn parse_location(line: &str) -> Option<(String, u32)> {
    let num = |s: &str| s.parse::<u32>().ok().filter(|&n| n > 0);
    if let Some(c) = RUST_LOC.captures(line).or_else(|| TSC_LOC.captures(line)) {
        return Some((c[1].to_string(), num(&c[2])?));
    }
    if let Some(c) = PYTHON_LOC.captures(line) {
        return Some((c[1].to_string(), num(&c[2])?));
    }
    let c = GNU_LOC.captures(line)?;
    let file = c[1].trim();
    // Not "Compilation started at 12:30:05" or "warning: 3 things".
    if file.contains(char::is_whitespace) || file.starts_with("http") || file.is_empty() {
        return None;
    }
    Some((file.to_string(), num(&c[2])?))
}

/// The `default-directory` a compilation/grep buffer's header line records.
fn header_dir(header: &str) -> Option<String> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"default-directory: "([^"]*)""#).unwrap());
    RE.captures(header).map(|c| c[1].to_string())
}

/// Whether a buffer's `default-directory` is inside (or is) the project
/// root. A `~/...` dir matches when the absolute root ends with the same
/// tail.
fn related(dir: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    if dir == root {
        return true;
    }
    if let Some(rest) = dir.strip_prefix("~/") {
        // dir = ~/a/b; the root may be the unknown home plus a prefix
        // of that, so match any leading run of segments.
        let rest = rest.trim_end_matches('/');
        let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        return (1..=segs.len()).any(|i| {
            let prefix = segs[..i].join("/");
            root == prefix || root.ends_with(&format!("/{prefix}"))
        });
    }
    dir.starts_with(&format!("{root}/")) || root.starts_with(&format!("{dir}/"))
}

/// A match in a Grep buffer (`consult-grep-match`), or an error in a
/// compilation buffer (`consult-compile-error`). Buffers of the right
/// mode whose `default-directory` belongs to this project are scanned;
/// with a prefix, jump stays inside that buffer instead of opening the
/// file.
fn compile_errors(prefix: Option<i64>, grep: bool) -> Result<(), String> {
    let mode = if grep {
        "grep-mode"
    } else {
        "compilation-mode"
    };
    let root = Buffer::current().path().and_then(|p| project_root(&p));
    let mut any_buffer = false;
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for b in Buffer::all() {
        if !b.alive() || b.major_mode() != mode {
            continue;
        }
        let header = b.line(0);
        let dir = header_dir(&header).unwrap_or_default();
        if let Some(root) = root.as_deref()
            && !dir.is_empty()
            && !related(&dir, root)
        {
            continue;
        }
        any_buffer = true;
        let name = b.name();
        let id = b.id();
        let mut seen = std::collections::HashSet::new();
        for (i, line) in b.text().lines().enumerate() {
            if i == 0 {
                continue;
            }
            let Some((file, ln)) = parse_location(line) else {
                continue;
            };
            let text = line.trim_end();
            let mut display = text.to_string();
            if !seen.insert(display.clone()) {
                display = format!("{text} [{name}:{}]", i + 1);
                seen.insert(display.clone());
            }
            let target = if prefix.is_some() {
                Target::Line {
                    buffer: id,
                    line: i as u32 + 1,
                }
            } else {
                Target::File {
                    path: absolutize(&file, &dir),
                    line: ln,
                }
            };
            candidates.push(display.clone());
            targets.push((display, target));
        }
    }
    if !any_buffer {
        return Err("No related buffers".into());
    }
    let prompt = if grep {
        "Go to match: "
    } else {
        "Go to error: "
    };
    ask(prompt, candidates, targets);
    Ok(())
}

/// A match in a Grep buffer (`consult-grep-match`).
fn consult_grep_match(prefix: Option<i64>) -> Result<(), String> {
    compile_errors(prefix, true)
}

/// An error in a compilation buffer (`consult-compile-error`).
fn consult_compile_error(prefix: Option<i64>) -> Result<(), String> {
    compile_errors(prefix, false)
}

// -------------------------------------------------------------- locate

/// Find files by name with `locate`, falling back to Spotlight's
/// `mdfind` when the locate database is missing (`consult-locate`).
fn consult_locate(_prefix: Option<i64>) -> Result<(), String> {
    read_string("Locate: ", "", |pattern| {
        let Some(pattern) = pattern else { return };
        if pattern.trim().is_empty() {
            return;
        }
        LOCATE_OUT.with(|o| o.borrow_mut().clear());
        let for_exit = pattern.clone();
        let spawned = spawn(
            "locate",
            &["-i".into(), pattern.clone()],
            None,
            |data| {
                if let Ok(text) = std::str::from_utf8(data) {
                    LOCATE_OUT.with(|o| o.borrow_mut().push_str(text));
                }
            },
            move |_code| {
                let out = LOCATE_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
                if out.lines().any(|l| l.starts_with('/')) {
                    locate_finish(out);
                } else {
                    mdfind(for_exit);
                }
            },
        );
        if let Err(e) = spawned {
            message(&format!("locate: {e}"));
            mdfind(pattern);
        }
    });
    Ok(())
}

/// Spotlight's `mdfind -name`, for when locate has no database.
fn mdfind(pattern: String) {
    LOCATE_OUT.with(|o| o.borrow_mut().clear());
    let spawned = spawn(
        "mdfind",
        &["-name".into(), pattern],
        None,
        |data| {
            if let Ok(text) = std::str::from_utf8(data) {
                LOCATE_OUT.with(|o| o.borrow_mut().push_str(text));
            }
        },
        move |_code| {
            let out = LOCATE_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
            locate_finish(out);
        },
    );
    if let Err(e) = spawned {
        message(&format!("mdfind: {e}"));
    }
}

/// Offer the path lines of a locate/mdfind run.
fn locate_finish(out: String) {
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for line in out.lines() {
        let path = line.trim_end();
        if !path.starts_with('/') {
            continue;
        }
        candidates.push(path.to_string());
        targets.push((
            path.to_string(),
            Target::File {
                path: path.to_string(),
                line: 1,
            },
        ));
        if candidates.len() >= MAX_CANDIDATES {
            break;
        }
    }
    ask("Locate: ", candidates, targets);
}

// ----------------------------------------------------------------- man

/// Read a manual page with `man -k`, then open the page chosen
/// (`consult-man`).
fn consult_man(_prefix: Option<i64>) -> Result<(), String> {
    read_string("Man: ", "", |pattern| {
        let Some(pattern) = pattern else { return };
        if pattern.trim().is_empty() {
            return;
        }
        MAN_K_OUT.with(|o| o.borrow_mut().clear());
        let spawned = spawn(
            "man",
            &["-k".into(), pattern],
            None,
            |data| {
                if let Ok(text) = std::str::from_utf8(data) {
                    MAN_K_OUT.with(|o| o.borrow_mut().push_str(text));
                }
            },
            move |_code| {
                let out = MAN_K_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
                let (candidates, targets) = parse_man_k(&out);
                ask("Man: ", candidates, targets);
            },
        );
        if let Err(e) = spawned {
            message(&format!("man: {e}"));
        }
    });
    Ok(())
}

/// Parse `man -k` lines: `BIO_printf(3) - formatted output...` — the
/// first name in parentheses is the page; noise without ` - ` is skipped.
fn parse_man_k(out: &str) -> CandidateSet {
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((names, _desc)) = line.split_once(" - ") else {
            continue;
        };
        let Some(open) = names.find('(') else {
            continue;
        };
        let Some(close) = names[open..].find(')') else {
            continue;
        };
        let name = names[..open].trim().trim_end_matches(',').to_string();
        let section = names[open + 1..open + close].trim();
        if name.is_empty() {
            continue;
        }
        let section = if section.is_empty() {
            None
        } else {
            Some(section.to_string())
        };
        let display = line.to_string();
        candidates.push(display.clone());
        targets.push((display, Target::Man { section, name }));
    }
    (candidates, targets)
}

/// Drop a pending man fetch: kill the process, so its late `on_exit`
/// (guarded by the generation counter) does nothing.
fn cancel_man() {
    MAN_GEN.with(|g| g.set(g.get() + 1));
    MAN_PENDING.with(|p| p.borrow_mut().clear());
    if let Some(p) = MAN_PROC.with(|p| p.borrow_mut().take()) {
        p.kill();
    }
}

/// Show a manual page in a `*man NAME*` buffer. If the page is already
/// there, just show it; if the same page is already being fetched (the
/// prompt moved away and back), wait for it.
fn man_page(section: Option<String>, name: String) {
    let key = format!("{}/{}", section.as_deref().unwrap_or(""), name);
    let buffer_name = format!("*man {name}*");
    if MAN_KEY.with(|k| k.borrow().clone()) == key
        && let Some(b) = Buffer::find(&buffer_name)
    {
        b.show();
        return;
    }
    if MAN_PENDING.with(|p| p.borrow().clone()) == key {
        MAN_PREVIEWING.with(|p| p.set(ACTIVE.with(Cell::get)));
        return;
    }
    cancel_man();
    MAN_GEN.with(|g| g.set(g.get() + 1));
    let fetch_id = MAN_GEN.with(Cell::get);
    MAN_OUT.with(|o| o.borrow_mut().clear());
    MAN_PENDING.with(|p| *p.borrow_mut() = key.clone());
    // Previewing: show the page only while the prompt is still open.
    let previewing = ACTIVE.with(Cell::get);
    MAN_PREVIEWING.with(|p| p.set(previewing));
    let mut args: Vec<String> = vec!["-P".into(), "cat".into()];
    if let Some(s) = &section {
        args.push(s.clone());
    }
    args.push(name.clone());
    let spawned = spawn_with(
        "man",
        &args,
        None,
        &[("MANPAGER", "cat"), ("PAGER", "cat")],
        move |data| {
            if MAN_GEN.with(Cell::get) == fetch_id {
                MAN_OUT.with(|o| o.borrow_mut().push_str(&String::from_utf8_lossy(data)));
            }
        },
        move |_code, stderr| {
            if MAN_GEN.with(Cell::get) != fetch_id {
                return; // killed while another page is fetching
            }
            MAN_PENDING.with(|p| p.borrow_mut().clear());
            let out = MAN_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
            let text = strip_man_formatting(&out);
            let show = !MAN_PREVIEWING.with(Cell::get) || ACTIVE.with(Cell::get);
            if text.trim().is_empty() {
                if show {
                    let first = stderr.lines().find(|l| !l.trim().is_empty());
                    message(first.unwrap_or(&format!("No manual entry for {name}")));
                }
                return;
            }
            let b = match Buffer::find(&buffer_name) {
                Some(b) => b,
                None => Buffer::create(&buffer_name),
            };
            let _ = b.set_major_mode("special-mode");
            let _ = b.set_option("read-only", "true");
            let span = Span {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: u32::MAX,
                    column: u32::MAX,
                },
            };
            b.replace(span, &text);
            MAN_KEY.with(|k| *k.borrow_mut() = key.clone());
            if show {
                b.show();
            }
        },
    );
    match spawned {
        Ok(p) => MAN_PROC.with(|proc| *proc.borrow_mut() = Some(p)),
        Err(e) => {
            MAN_PENDING.with(|p| p.borrow_mut().clear());
            message(&format!("man: {e}"));
        }
    }
}

/// Undo `man -P cat`'s backspace overstrikes (`p\bp` -> `p`) and strip
/// ANSI escape sequences.
fn strip_man_formatting(s: &str) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x08' => {
                out.pop();
            }
            '\x1b' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out.into_iter().collect()
}

// ---------------------------------------------------------- org heading

/// Org keywords a heading's first word may be.
const ORG_TODO: &[&str] = &[
    "TODO",
    "DONE",
    "NEXT",
    "WAITING",
    "HOLD",
    "CANCELLED",
    "CANCELED",
];

/// Level, todo keyword, priority, title, tags — the parts of an Org heading.
type OrgParts = (usize, Option<String>, Option<char>, String, String);

/// An Org heading line: `* TODO [#A] Title :tag:tag:`.
fn parse_org_line(line: &str) -> Option<OrgParts> {
    if !line.starts_with('*') {
        return None;
    }
    let level = line.bytes().take_while(|&b| b == b'*').count();
    let mut rest = &line[level..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    rest = rest.trim_start();
    let mut todo = None;
    if let Some(first) = rest.split_whitespace().next()
        && ORG_TODO.contains(&first)
    {
        todo = Some(first.to_string());
        rest = rest[first.len()..].trim_start();
    }
    let mut priority = None;
    if rest.len() >= 4 && rest.starts_with("[#") && rest.as_bytes()[3] == b']' {
        priority = Some(rest.as_bytes()[2] as char);
        rest = rest[4..].trim_start();
    }
    static TAGS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?:^|\s)(:[\w@#%:]+:)\s*$").unwrap());
    let mut tags = String::new();
    if let Some(m) = TAGS.find(rest) {
        tags = m.as_str().trim().to_string();
        rest = rest[..m.start()].trim_end();
    }
    Some((level, todo, priority, rest.to_string(), tags))
}

/// An Org heading, labeled with its outline path (`consult-org-heading`).
/// With a prefix, headings come from every Org buffer.
fn consult_org_heading(prefix: Option<i64>) -> Result<(), String> {
    let all = prefix.is_some();
    let cur = Buffer::current();
    if !all && cur.major_mode() != "org-mode" {
        return Err("Not in an Org buffer".into());
    }
    let buffers: Vec<(u32, String)> = if all {
        Buffer::all()
            .into_iter()
            .filter(|b| b.alive() && b.major_mode() == "org-mode")
            .map(|b| (b.id(), b.name()))
            .collect()
    } else {
        vec![(cur.id(), cur.name())]
    };
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for (id, buf_name) in buffers {
        let Some(b) = Buffer::from_id(id) else {
            continue;
        };
        let mut stack: Vec<(usize, String)> = Vec::new();
        for (i, line) in b.text().lines().enumerate() {
            let Some((level, todo, priority, title, tags)) = parse_org_line(line) else {
                continue;
            };
            while stack.last().map(|(l, _)| *l >= level).unwrap_or(false) {
                stack.pop();
            }
            let path: Vec<&str> = stack.iter().map(|(_, t)| t.as_str()).collect();
            let mut display = String::new();
            if all {
                display.push_str(&format!("{buf_name}:"));
            }
            display.push_str(&format!("{}: ", i + 1));
            if let Some(t) = &todo {
                display.push_str(&format!("{t} "));
            }
            if let Some(p) = priority {
                display.push_str(&format!("[#{p}] "));
            }
            let mut outline = path.join("/");
            if !outline.is_empty() {
                outline.push('/');
            }
            outline.push_str(&title);
            display.push_str(&outline);
            if !tags.is_empty() {
                display.push_str(&format!(" {tags}"));
            }
            stack.push((level, title));
            candidates.push(display.clone());
            targets.push((
                display,
                Target::Line {
                    buffer: id,
                    line: i as u32 + 1,
                },
            ));
        }
    }
    ask("Org heading: ", candidates, targets);
    Ok(())
}

// --------------------------------------------------------- org agenda

/// Jump to an Org agenda entry (`consult-org-agenda`). The entries come
/// from the configured agenda files (or the open Org buffers, when none
/// are set). The prefix argument picks the view: none, today's agenda;
/// `C-u`, the week; `C-u C-u`, the TODO list; `C-u C-u C-u`, the stuck
/// projects.
fn consult_org_agenda(prefix: Option<i64>) -> Result<(), String> {
    let (prompt, entries) = match prefix.unwrap_or(0) {
        16 => ("Org TODO: ", org::todos(None)?),
        64 => ("Org stuck: ", org::stuck()?),
        4 => ("Org week: ", org::span(7, None, false)?),
        _ => ("Org agenda: ", org::span(1, None, false)?),
    };
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for e in &entries {
        let display = org_entry_display(e);
        candidates.push(display.clone());
        targets.push((
            display,
            Target::Line {
                buffer: e.buffer,
                line: e.line + 1,
            },
        ));
    }
    if candidates.is_empty() {
        message("No agenda entries");
        return Ok(());
    }
    ask(prompt, candidates, targets);
    Ok(())
}

/// An agenda entry as one line, the way the agenda view labels it.
fn org_entry_display(e: &AgendaEntry) -> String {
    let mut s = String::new();
    if let Some(d) = &e.date {
        s.push_str(d);
        s.push(' ');
    }
    if let Some(t) = &e.time {
        s.push_str(t);
        s.push(' ');
    }
    if let Some(k) = &e.keyword {
        s.push_str(k);
        s.push(' ');
    }
    if !e.label.is_empty() {
        s.push_str(&e.label);
        s.push(' ');
    }
    s.push_str(&e.file);
    s.push(':');
    if !e.category.is_empty() {
        s.push_str(&e.category);
        s.push(':');
    }
    s.push(' ');
    s.push_str(&e.title);
    if !e.tags.is_empty() {
        s.push_str(&format!("  :{}:", e.tags.join(":")));
    }
    s
}

// ------------------------------------------------ keep-lines / focus-lines

/// Keep only the lines matching a regexp; delete the rest
/// (`consult-keep-lines`).
fn consult_keep_lines(_prefix: Option<i64>) -> Result<(), String> {
    let bid = Buffer::current().id();
    read_string("Keep lines (regexp): ", "", move |query| {
        let Some(query) = query else { return };
        let Ok(re) = regex::Regex::new(&query) else {
            message("Invalid regexp");
            return;
        };
        let Some(b) = Buffer::from_id(bid) else {
            return;
        };
        if !b.alive() {
            return;
        }
        let n = b.line_count();
        let total = n;
        let kept: Vec<String> = (0..n)
            .filter(|&i| re.is_match(&b.line(i)))
            .map(|i| b.line(i))
            .collect();
        let mut new_text = kept.join("\n");
        if !new_text.is_empty() {
            new_text.push('\n');
        }
        let span = Span {
            start: Position { line: 0, column: 0 },
            end: Position {
                line: u32::MAX,
                column: u32::MAX,
            },
        };
        b.replace(span, &new_text);
        message(&format!("Kept {} of {total} lines", kept.len()));
    });
    Ok(())
}

/// Hide the lines matching a regexp (`consult-focus-lines`): they stay
/// in the buffer, drawn as nothing, until the plugin is unloaded or the
/// mode is turned off again.
fn consult_focus_lines(_prefix: Option<i64>) -> Result<(), String> {
    let bid = Buffer::current().id();
    read_string("Focus lines (regexp): ", "", move |query| {
        let Some(query) = query else { return };
        let Ok(re) = regex::Regex::new(&query) else {
            message("Invalid regexp");
            return;
        };
        let Some(b) = Buffer::from_id(bid) else {
            return;
        };
        if !b.alive() {
            return;
        }
        let n = b.line_count();
        let mut items = Vec::new();
        for i in 0..n {
            let text = b.line(i);
            if re.is_match(&text) {
                let end_col = text.chars().count() as u32;
                // Hide the line's text; the newline stays so the lines
                // below keep their place.
                items.push(Replacement {
                    span: Span {
                        start: Position { line: i, column: 0 },
                        end: Position {
                            line: i,
                            column: end_col,
                        },
                    },
                    text: None,
                    face: None,
                });
            }
        }
        let hidden = items.len();
        if let Err(e) = decorations::set_display(&b, &items) {
            message(&e);
            return;
        }
        message(&format!("Hid {hidden} of {n} lines"));
    });
    Ok(())
}

// ------------------------------------------------- kill ring & session

/// One kill-ring entry as a short single line, for a candidate or the
/// echo area.
fn kill_preview(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        match c {
            '\n' => out.push('⏎'),
            '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    if out.trim().is_empty() {
        "(empty)".to_string()
    } else {
        out
    }
}

/// Insert an older kill (`consult-yank-from-kill-ring`): pick one from
/// the kill ring and yank it, making it the current one so M-y rotates
/// from there.
fn consult_yank_from_kill_ring(_prefix: Option<i64>) -> Result<(), String> {
    let entries = kill_ring::entries();
    if entries.is_empty() {
        message("Kill ring is empty");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for (i, text) in entries.into_iter().enumerate() {
        let display = format!("{}: {}", i + 1, kill_preview(&text, 100));
        candidates.push(display.clone());
        targets.push((
            display,
            Target::Kill {
                index: i as u32,
                text,
            },
        ));
    }
    ask("Yank: ", candidates, targets);
    Ok(())
}

/// Replace the last yank with the next kill (`consult-yank-pop`), the
/// way M-y does; if the text before point isn't the current kill, insert
/// the next one instead.
fn consult_yank_pop(_prefix: Option<i64>) -> Result<(), String> {
    if kill_ring::length() == 0 {
        message("Kill ring is empty");
        return Ok(());
    }
    let b = Buffer::current();
    let p = b.point();
    if let Some(cur) = kill_ring::current()
        && !cur.contains('\n')
    {
        let len = cur.chars().count() as u32;
        if p.column >= len {
            let start = Position {
                line: p.line,
                column: p.column - len,
            };
            if b.text_in(Span { start, end: p }) == cur {
                let next = kill_ring::rotate().unwrap_or_default();
                b.replace(Span { start, end: p }, &next);
                return Ok(());
            }
        }
    }
    match kill_ring::rotate() {
        Some(text) => b.insert(&text),
        None => message("Kill ring is empty"),
    }
    Ok(())
}

/// Jump to a recently opened file (`consult-recent-file`).
fn consult_recent_file(_prefix: Option<i64>) -> Result<(), String> {
    let files = session::recent_files();
    if files.is_empty() {
        message("No recent files");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for path in files {
        // The path is the candidate, so two files whose tails match stay
        // apart.
        candidates.push(path.clone());
        targets.push((path.clone(), Target::File { path, line: 1 }));
    }
    ask("Recent file: ", candidates, targets);
    Ok(())
}

/// Jump to a bookmark (`consult-bookmark`).
fn consult_bookmark(_prefix: Option<i64>) -> Result<(), String> {
    let names = session::bookmark_names();
    if names.is_empty() {
        message("No bookmarks");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for name in names {
        let Some(b) = session::bookmark(&name) else {
            continue;
        };
        candidates.push(name.clone());
        targets.push((
            name,
            Target::FileAt {
                path: b.path,
                offset: b.offset,
            },
        ));
    }
    ask("Bookmark: ", candidates, targets);
    Ok(())
}

/// Jump to a definition in this buffer (`consult-imenu`), from the
/// grammar's tags query (the same list M-g i offers).
fn consult_imenu(_prefix: Option<i64>) -> Result<(), String> {
    let bid = Buffer::current().id();
    let Some(b) = Buffer::from_id(bid) else {
        return Ok(());
    };
    let defs = b.definitions();
    if defs.is_empty() {
        message("No definitions");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for d in defs {
        let line = d.position.line + 1;
        let display = format!("{} ({}, line {})", d.name, d.kind, line);
        candidates.push(display.clone());
        targets.push((display, Target::Line { buffer: bid, line }));
    }
    ask("Imenu: ", candidates, targets);
    Ok(())
}

/// Jump to a definition in any buffer (`consult-imenu-multi`).
fn consult_imenu_multi(_prefix: Option<i64>) -> Result<(), String> {
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut truncated = false;
    'buffers: for b in Buffer::all() {
        if !b.alive() {
            continue;
        }
        let name = b.name();
        let id = b.id();
        for d in b.definitions() {
            let line = d.position.line + 1;
            let display = format!("{name}: {} ({}, line {})", d.name, d.kind, line);
            candidates.push(display.clone());
            targets.push((display, Target::Line { buffer: id, line }));
            if candidates.len() >= MAX_CANDIDATES {
                truncated = true;
                break 'buffers;
            }
        }
    }
    if truncated {
        message("Too many definitions; showing the first ones");
    }
    if candidates.is_empty() {
        message("No definitions");
        return Ok(());
    }
    ask("Imenu (multi): ", candidates, targets);
    Ok(())
}

/// Run a command of an enabled mode (`consult-mode-command`): the
/// commands bound in the major and minor modes' keymaps, or, with a
/// prefix argument, every registered command.
fn consult_mode_command(prefix: Option<i64>) -> Result<(), String> {
    let b = Buffer::current();
    let mut seen = std::collections::HashSet::new();
    let mut names: Vec<String> = Vec::new();
    if prefix.is_some() {
        for name in commands::names() {
            if seen.insert(name.clone()) {
                names.push(name);
            }
        }
    } else {
        let mut scopes = vec![Scope::Global, Scope::Mode(b.major_mode())];
        for m in b.minor_modes() {
            scopes.push(Scope::Mode(m));
        }
        for scope in scopes {
            for (_keys, command) in keymap::bindings(&scope) {
                if !command.is_empty() && seen.insert(command.clone()) {
                    names.push(command);
                }
            }
        }
        // No keymap is installed (a bare editor): offer every command.
        if names.is_empty() {
            for name in commands::names() {
                if seen.insert(name.clone()) {
                    names.push(name);
                }
            }
        }
    }
    if names.is_empty() {
        message("No commands");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for name in names {
        candidates.push(name.clone());
        targets.push((name.clone(), Target::RunCommand { name }));
    }
    ask("Mode command: ", candidates, targets);
    Ok(())
}

/// Insert a register (`consult-register`): pick one and insert the text it
/// holds, or jump to a position register (C-x r SPC).
fn consult_register(_prefix: Option<i64>) -> Result<(), String> {
    let names = registers::names();
    if names.is_empty() {
        message("No registers are set");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for name in names {
        let desc = registers::describe(&name).unwrap_or_else(|| "(empty)".to_string());
        let display = format!("{name}: {desc}");
        // A position register has no text to insert: jump to it instead.
        let target = match registers::position(&name) {
            Some(m) => Target::MarkAt {
                buffer: m.buffer,
                offset: m.offset,
            },
            None => Target::Register {
                text: registers::text(&name),
                name,
            },
        };
        candidates.push(display.clone());
        targets.push((display, target));
    }
    ask("Register: ", candidates, targets);
    Ok(())
}

/// Run a remembered keyboard macro (`consult-kmacro`): pick one and run
/// it (it becomes the last macro, so C-x e repeats it).
fn consult_kmacro(_prefix: Option<i64>) -> Result<(), String> {
    let entries = kmacro::entries();
    if entries.is_empty() {
        message("No keyboard macros");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for (i, keys) in entries.into_iter().enumerate() {
        let label = if keys.is_empty() {
            "(empty macro)".to_string()
        } else {
            keys.join(" ")
        };
        let display = format!("{}: {}", i + 1, kill_preview(&label, 100));
        candidates.push(display.clone());
        targets.push((display, Target::Kmacro { index: i as u32 }));
    }
    ask("Keyboard macro: ", candidates, targets);
    Ok(())
}

/// Jump to a mark on this buffer's mark ring (`consult-mark`).
fn consult_mark(_prefix: Option<i64>) -> Result<(), String> {
    let text = Buffer::current().text();
    let marks = marks::buffer_marks();
    if marks.is_empty() {
        message("No marks");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for m in marks {
        let display = format!(
            "{}: {}",
            m.position.line + 1,
            line_at(&text, m.position.line)
        );
        candidates.push(display.clone());
        targets.push((
            display,
            Target::MarkAt {
                buffer: m.buffer,
                offset: m.offset,
            },
        ));
    }
    ask("Mark: ", candidates, targets);
    Ok(())
}

/// Jump to a mark in any buffer (`consult-global-mark`).
fn consult_global_mark(_prefix: Option<i64>) -> Result<(), String> {
    let marks = marks::global_marks();
    if marks.is_empty() {
        message("No marks");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for m in marks {
        let (name, text) = match Buffer::from_id(m.buffer) {
            Some(b) if b.alive() => (b.name(), b.text()),
            _ => (format!("#{}", m.buffer), String::new()),
        };
        let display = format!(
            "{}:{}:{} {}",
            name,
            m.position.line + 1,
            m.position.column + 1,
            line_at(&text, m.position.line)
        );
        candidates.push(display.clone());
        targets.push((
            display,
            Target::MarkAt {
                buffer: m.buffer,
                offset: m.offset,
            },
        ));
    }
    ask("Global mark: ", candidates, targets);
    Ok(())
}

/// The `line`-th line of `text` (0-based), trimmed, or empty.
fn line_at(text: &str, line: u32) -> String {
    text.lines()
        .nth(line as usize)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// List this buffer's diagnostics (`consult-flymake`), or every buffer's
/// with a prefix: the error, warning and info decorations the editor and
/// plugins put on a buffer.
fn consult_flymake(prefix: Option<i64>) -> Result<(), String> {
    let all = prefix.is_some();
    let buffers: Vec<Buffer> = if all {
        Buffer::all().into_iter().filter(|b| b.alive()).collect()
    } else {
        vec![Buffer::current()]
    };
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for b in buffers {
        let bid = b.id();
        let name = b.name();
        for d in buffer_diagnostics(&b) {
            let line = d.span.start.line + 1;
            let col = d.span.start.column + 1;
            let label = d.message.clone().unwrap_or_else(|| d.face.clone());
            let display = if all {
                format!("{name}:{line}:{col} {} {label}", d.face)
            } else {
                format!("{line}:{col} {} {label}", d.face)
            };
            candidates.push(display.clone());
            targets.push((display, Target::Line { buffer: bid, line }));
        }
    }
    if candidates.is_empty() {
        message("No diagnostics");
        return Ok(());
    }
    ask("Diagnostic: ", candidates, targets);
    Ok(())
}

/// The error, warning and info decorations on `b`.
fn buffer_diagnostics(b: &Buffer) -> Vec<Decoration> {
    decorations::get(b)
        .into_iter()
        .filter(|d| matches!(d.face.as_str(), "error" | "warning" | "info"))
        .collect()
}

/// Jump to a published cross-reference (`consult-xref`).
fn consult_xref(_prefix: Option<i64>) -> Result<(), String> {
    let refs = xref::all();
    if refs.is_empty() {
        message("No cross-references");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for r in refs {
        let display = format!("{} {}:{}:{} [{}]", r.name, r.path, r.line, r.column, r.kind);
        candidates.push(display.clone());
        targets.push((
            display,
            Target::File {
                path: r.path,
                line: r.line,
            },
        ));
    }
    ask("Xref: ", candidates, targets);
    Ok(())
}

/// Apply a named theme (`consult-theme`). Moving over the list applies
/// each in turn (live preview); RET keeps it, C-g puts the old one back.
fn consult_theme(_prefix: Option<i64>) -> Result<(), String> {
    let names = theme::names();
    if names.is_empty() {
        message("No themes are defined (see forge.theme.define)");
        return Ok(());
    }
    let before = theme::current();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for name in names {
        let label = match theme::describe(&name) {
            Some(d) if !d.is_empty() => format!("{name}  {d}"),
            _ => name.clone(),
        };
        candidates.push(label.clone());
        targets.push((label, Target::Theme { name }));
    }
    save_origin();
    TARGETS.with(|t| *t.borrow_mut() = targets);
    ACTIVE.with(|a| a.set(true));
    completing_read("Theme: ", &candidates, true, move |answer| {
        ACTIVE.with(|a| a.set(false));
        let targets = TARGETS.with(|t| std::mem::take(&mut *t.borrow_mut()));
        match answer {
            Some(text) => {
                if let Some(t) = targets.iter().find(|(k, _)| *k == text).map(|(_, v)| v) {
                    jump(t);
                }
            }
            None => {
                // Put back the theme that was active when we started.
                match &before {
                    Some(name) => {
                        let _ = theme::load(name);
                    }
                    None => theme::disable(),
                }
                restore_origin();
            }
        }
    });
    Ok(())
}

/// Insert a string from the current prompt's history (`consult-history`).
/// Meant to be bound in a prompt map (e.g. `M-r`): it offers the history of
/// the prompt you are in, and the pick replaces that prompt's input.
fn consult_history(_prefix: Option<i64>) -> Result<(), String> {
    if !minibuffer::active() {
        message("No prompt is open (bind consult-history in a prompt map)");
        return Ok(());
    }
    let Some(kind) = minibuffer::history_kind() else {
        message("This prompt keeps no history");
        return Ok(());
    };
    let mut candidates = Vec::new();
    // The current input leads, so RET on it keeps what is already typed.
    let current = minibuffer::input();
    if !current.is_empty() {
        candidates.push(current);
    }
    for item in session::history(&kind) {
        if !candidates.contains(&item) {
            candidates.push(item);
        }
    }
    if candidates.is_empty() {
        message("History is empty");
        return Ok(());
    }
    completing_read_over("History: ", &candidates, false, |_| {});
    Ok(())
}

/// Insert a past search string (`consult-isearch-history`). Bound in the
/// isearch key map (`M-r`): it offers the search history and the pick
/// resumes the search with that string.
fn consult_isearch_history(_prefix: Option<i64>) -> Result<(), String> {
    if minibuffer::history_kind().as_deref() != Some("search") {
        message("Not in Isearch (bind consult-isearch-history in the isearch map)");
        return Ok(());
    }
    let mut candidates = Vec::new();
    let current = minibuffer::input();
    if !current.is_empty() {
        candidates.push(current);
    }
    for item in session::history("search") {
        if !candidates.contains(&item) {
            candidates.push(item);
        }
    }
    if candidates.is_empty() {
        message("Search history is empty");
        return Ok(());
    }
    completing_read_over("I-search: ", &candidates, false, |_| {});
    Ok(())
}

// ----------------------------------------------------------------- init

fn init() -> Result<(), String> {
    // Live preview: selection changes cover cursor movement; input changes
    // cover the list narrowing as the user types. The latter needs Forge
    // 0.1.30.
    command(
        PREVIEW,
        "Preview the candidate under the cursor",
        preview_current,
    )?;
    on_selection_changed(PREVIEW);
    on_minibuffer_input_changed(PREVIEW);
    on_minibuffer_selection_changed(PREVIEW);
    // C-. previews on demand, for when the hook isn't enough.
    bind_in_prompt("completion-list", "C-.", PREVIEW)?;

    command(
        "consult-buffer",
        "Switch to a buffer, recent file or bookmark",
        consult_buffer,
    )?;
    command(
        "consult-buffer-other-window",
        "Switch to a buffer, recent file or bookmark in another window",
        consult_buffer_other_window,
    )?;
    command(
        "consult-ls-git",
        "Open a file git tracks (or would, if added)",
        consult_ls_git,
    )?;
    command(
        "consult-todo",
        "Jump to a TODO, FIXME, HACK or XXX in the project",
        consult_todo,
    )?;
    command(
        "consult-dir",
        "Open a directory: the project, recent files' and bookmarks' folders",
        consult_dir,
    )?;
    command(
        "consult-register-load",
        "Insert a register's text, or jump to a position register",
        consult_register,
    )?;
    command(
        "consult-yank-replace",
        "Replace the text just yanked with an older kill",
        consult_yank_pop,
    )?;
    command(
        "consult-project-buffer",
        "Switch to a buffer in this project",
        consult_project_buffer,
    )?;
    command(
        "consult-line",
        "Jump to a line in this buffer",
        consult_line,
    )?;
    command(
        "consult-line-multi",
        "Jump to a line in any (project) buffer",
        consult_line_multi,
    )?;
    command(
        "consult-outline",
        "Jump to an Org or Markdown heading",
        consult_outline,
    )?;
    command(
        "consult-org-heading",
        "Jump to an Org heading, with its outline path",
        consult_org_heading,
    )?;
    command(
        "consult-goto-line",
        "Jump to a line number",
        consult_goto_line,
    )?;
    command(
        "consult-minor-mode-menu",
        "Toggle a minor mode in this buffer",
        consult_minor_mode_menu,
    )?;
    command("consult-grep", "Grep the project (grep)", consult_grep)?;
    command(
        "consult-ripgrep",
        "Grep the project (ripgrep)",
        consult_ripgrep,
    )?;
    command(
        "consult-git-grep",
        "Grep the project (git grep)",
        consult_git_grep,
    )?;
    command(
        "consult-grep-match",
        "Jump to a match in a Grep buffer",
        consult_grep_match,
    )?;
    command(
        "consult-compile-error",
        "Jump to an error in a compilation buffer",
        consult_compile_error,
    )?;
    command("consult-find", "Find a file by name (find)", consult_find)?;
    command("consult-fd", "Find a file by name (fd)", consult_fd)?;
    command(
        "consult-locate",
        "Find a file by name (locate)",
        consult_locate,
    )?;
    command("consult-man", "Read a manual page (man)", consult_man)?;
    command(
        "consult-keep-lines",
        "Keep only lines matching a regexp",
        consult_keep_lines,
    )?;
    command(
        "consult-focus-lines",
        "Hide lines matching a regexp",
        consult_focus_lines,
    )?;
    command(
        "consult-yank-from-kill-ring",
        "Insert a kill from the kill ring",
        consult_yank_from_kill_ring,
    )?;
    command(
        "consult-yank-pop",
        "Replace the last yank with the next kill",
        consult_yank_pop,
    )?;
    command(
        "consult-recent-file",
        "Jump to a recently opened file",
        consult_recent_file,
    )?;
    command("consult-bookmark", "Jump to a bookmark", consult_bookmark)?;
    command(
        "consult-imenu",
        "Jump to a definition in this buffer",
        consult_imenu,
    )?;
    command(
        "consult-imenu-multi",
        "Jump to a definition in any buffer",
        consult_imenu_multi,
    )?;
    command(
        "consult-mode-command",
        "Run a command of an enabled mode",
        consult_mode_command,
    )?;
    command(
        "consult-register",
        "Insert or jump to a register",
        consult_register,
    )?;
    command(
        "consult-kmacro",
        "Run a remembered keyboard macro",
        consult_kmacro,
    )?;
    command(
        "consult-mark",
        "Jump to a mark in this buffer",
        consult_mark,
    )?;
    command(
        "consult-global-mark",
        "Jump to a mark in any buffer",
        consult_global_mark,
    )?;
    command(
        "consult-flymake",
        "Jump to a diagnostic in this buffer",
        consult_flymake,
    )?;
    command("consult-xref", "Jump to a cross-reference", consult_xref)?;
    command("consult-theme", "Apply a named theme", consult_theme)?;
    command(
        "consult-history",
        "Insert a string from the current prompt's history",
        consult_history,
    )?;
    // M-r in any prompt offers that prompt's own history.
    bind_in_prompt("prompt", "M-r", "consult-history")?;
    bind_in_prompt("completion-list", "M-r", "consult-history")?;
    command(
        "consult-isearch-history",
        "Insert a past search string (during Isearch)",
        consult_isearch_history,
    )?;
    // During Isearch, M-r offers the search history instead.
    bind_in_prompt("isearch", "M-r", "consult-isearch-history")?;
    command(
        "consult-org-agenda",
        "Jump to an Org agenda entry",
        consult_org_agenda,
    )?;
    Ok(())
}

forge_plugin!(init);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_agenda_entry_display() {
        let e = AgendaEntry {
            buffer: 1,
            file: "plan.org".into(),
            line: 3,
            heading: "* TODO Meet".into(),
            title: "Meet".into(),
            category: "work".into(),
            keyword: Some("TODO".into()),
            done: false,
            tags: vec!["urgent".into()],
            priority: Some("A".into()),
            effort: Some(30),
            kind: forge_plugin_sdk::org::AgendaKind::Scheduled,
            date: Some("2024-05-06".into()),
            time: Some("10:00".into()),
            label: "Scheduled:".into(),
        };
        let s = org_entry_display(&e);
        for part in ["2024-05-06", "10:00", "TODO", "work:", "Meet", ":urgent:"] {
            assert!(s.contains(part), "{part:?} missing from {s:?}");
        }
    }

    #[test]
    fn orderless_line_matching() {
        assert!(line_matches("", "anything"));
        assert!(line_matches("foo", "a foo b"));
        assert!(line_matches("foo bar", "bar foo baz"));
        assert!(!line_matches("foo", "a bar b"));
        assert!(
            line_matches("FOO", "a FOO b"),
            "capitals make it case-sensitive"
        );
        assert!(!line_matches("FOO", "a foo b"));
    }

    #[test]
    fn preview_tracks_the_selected_candidate_after_narrowing() {
        TARGETS.with(|targets| {
            *targets.borrow_mut() = vec![
                ("1: first line".into(), Target::Line { buffer: 7, line: 1 }),
                (
                    "2: matching line".into(),
                    Target::Line { buffer: 7, line: 2 },
                ),
            ];
        });

        assert!(matches!(
            preview_target("2: matching line"),
            Some(Target::Line { buffer: 7, line: 2 })
        ));
        assert!(preview_target("not in the narrowed list").is_none());
        TARGETS.with(|targets| targets.borrow_mut().clear());
    }

    #[test]
    fn input_preview_chooses_the_first_matching_line() {
        let candidates = vec![
            ("1: unrelated".into(), Target::Line { buffer: 7, line: 1 }),
            (
                "2: matching phrase".into(),
                Target::Line { buffer: 7, line: 2 },
            ),
            (
                "3: phrase also matching".into(),
                Target::Line { buffer: 7, line: 3 },
            ),
        ];

        assert_eq!(
            first_input_match("matching phrase", &candidates).as_deref(),
            Some("2: matching phrase")
        );
    }

    #[test]
    fn headings_are_org_and_markdown() {
        assert!(is_heading("* Heading"));
        assert!(is_heading("** Sub"));
        assert!(is_heading("# Title"));
        assert!(is_heading("### Deep"));
        assert!(!is_heading("*no space"));
        assert!(!is_heading("normal text"));
        assert!(!is_heading("#hashtag"));
    }

    #[test]
    fn grep_output_parses_to_targets() {
        let out = "src/a.rs:10:fn main() {\nsrc/b.rs:3:let x = 1;\nnot-a-match\n";
        let (cands, targets) = parse_grep(out, "/proj");
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0], "src/a.rs:10:fn main() {");
        match &targets[0].1 {
            Target::File { path, line } => {
                assert_eq!(path, "/proj/src/a.rs");
                assert_eq!(*line, 10);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn find_output_parses_to_file_targets() {
        let out = "src/a.rs\nsrc/b.rs\n";
        let (cands, targets) = parse_paths(out, "/proj");
        assert_eq!(cands, vec!["src/a.rs", "src/b.rs"]);
        assert!(
            targets
                .iter()
                .all(|(_, t)| matches!(t, Target::File { line: 1, .. }))
        );
    }

    #[test]
    fn short_path_shows_the_tail() {
        assert_eq!(short_path("/home/me/x.rs"), "me/x.rs");
        assert_eq!(short_path("/a/b/c/d.rs"), "c/d.rs");
        assert_eq!(short_path("x.rs"), "x.rs");
    }

    #[test]
    fn absolutize_joins_relative_paths() {
        assert_eq!(absolutize("./src/a.rs", "/proj"), "/proj/src/a.rs");
        assert_eq!(absolutize("src/a.rs", "/proj/"), "/proj/src/a.rs");
        assert_eq!(absolutize("/abs/a.rs", "/proj"), "/abs/a.rs");
        assert_eq!(absolutize("~/a.rs", "/proj"), "~/a.rs");
        assert_eq!(absolutize("a.rs", ""), "a.rs");
    }

    #[test]
    fn locations_parse_from_compiler_output() {
        assert_eq!(
            parse_location("  --> src/main.rs:10:5"),
            Some(("src/main.rs".into(), 10))
        );
        assert_eq!(
            parse_location(r#"  File "app.py", line 3"#),
            Some(("app.py".into(), 3))
        );
        assert_eq!(
            parse_location("src/a.rs:7: nope"),
            Some(("src/a.rs".into(), 7))
        );
        assert_eq!(
            parse_location("src/a.rs:7:12: nope"),
            Some(("src/a.rs".into(), 7))
        );
        assert_eq!(
            parse_location("app.ts(12,3): oops"),
            Some(("app.ts".into(), 12))
        );
        assert_eq!(parse_location("Grep started at 12:30:05"), None);
        assert_eq!(parse_location("warning: 3 things"), None);
        assert_eq!(parse_location("http://example.com/x:1: "), None);
        assert_eq!(parse_location("plain prose"), None);
    }

    #[test]
    fn man_k_lines_parse_to_pages() {
        let out = "printf(1) - format and print data\n\
                   BIO_printf(3ssl), BIO_vprintf(3ssl) - formatted output\n\
                   makewhatis:\n\
                   \n";
        let (cands, targets) = parse_man_k(out);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0], "printf(1) - format and print data");
        match &targets[0].1 {
            Target::Man { section, name } => {
                assert_eq!(section.as_deref(), Some("1"));
                assert_eq!(name, "printf");
            }
            other => panic!("{other:?}"),
        }
        match &targets[1].1 {
            Target::Man { section, name } => {
                assert_eq!(section.as_deref(), Some("3ssl"));
                assert_eq!(name, "BIO_printf");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn man_formatting_strips_overstrikes_and_escapes() {
        assert_eq!(strip_man_formatting("p\u{8}p"), "p");
        assert_eq!(strip_man_formatting("x\x1b[1m!"), "x!");
        assert_eq!(strip_man_formatting("plain"), "plain");
    }

    #[test]
    fn org_lines_parse_todo_priority_and_tags() {
        let (level, todo, prio, title, tags) =
            parse_org_line("* TODO [#A] Write docs :work:urgent:").unwrap();
        assert_eq!(level, 1);
        assert_eq!(todo.as_deref(), Some("TODO"));
        assert_eq!(prio, Some('A'));
        assert_eq!(title, "Write docs");
        assert_eq!(tags, ":work:urgent:");

        let (level, todo, prio, title, tags) = parse_org_line("** Plain heading").unwrap();
        assert_eq!(level, 2);
        assert!(todo.is_none());
        assert!(prio.is_none());
        assert_eq!(title, "Plain heading");
        assert!(tags.is_empty());

        assert!(parse_org_line("*no space").is_none());
        assert!(parse_org_line("not a heading").is_none());
    }

    #[test]
    fn related_compares_dirs_to_root() {
        assert!(related("/proj", "/proj"));
        assert!(related("/proj/src", "/proj"));
        assert!(related("~/proj", "/home/me/proj"));
        assert!(related("~/proj/src", "/home/me/proj"));
        assert!(related("/proj", "/proj/src"), "root under dir counts");
        assert!(!related("/other", "/proj"));
        assert!(!related("~/work", "/home/me/proj"));
    }

    #[test]
    fn header_dir_reads_the_first_line() {
        assert_eq!(
            header_dir(r#"-*- mode: grep; default-directory: "~/proj" -*-"#).as_deref(),
            Some("~/proj")
        );
        assert_eq!(header_dir("Grep started at 12:30:05"), None);
    }

    #[test]
    fn char_offsets_become_positions() {
        let text = "abc\ndef\ngh";
        let at = |o| {
            let p = offset_to_position(text, o);
            (p.line, p.column)
        };
        assert_eq!(at(0), (0, 0));
        assert_eq!(at(2), (0, 2));
        assert_eq!(at(4), (1, 0));
        assert_eq!(at(9), (2, 1));
        assert_eq!(at(999), (2, 2));
    }

    #[test]
    fn kill_previews_are_one_line_and_bounded() {
        assert_eq!(kill_preview("one\ntwo", 100), "one⏎two");
        assert_eq!(kill_preview("   ", 100), "(empty)");
        assert_eq!(kill_preview("abcdef", 3), "abc…");
    }

    #[test]
    fn line_at_reads_zero_based_lines() {
        let text = "first\n  second  \n\nfourth";
        assert_eq!(line_at(text, 0), "first");
        assert_eq!(line_at(text, 1), "second");
        assert_eq!(line_at(text, 2), "");
        assert_eq!(line_at(text, 3), "fourth");
        assert_eq!(line_at(text, 99), "");
    }
}
