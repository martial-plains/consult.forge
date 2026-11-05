# forge-plugin-consult

Consult-style completing-read commands for Forge, with live candidate
previews. `consult-line` and `consult-line-multi` use one prompt: every
nonblank line is a candidate, typing narrows the list, and point follows
the selected line as the input or selection changes. `C-g` restores the
original buffer and point.

## Commands

| Command | Description |
| --- | --- |
| `consult-buffer` | Switch to an open buffer, recent file, or bookmark. |
| `consult-buffer-other-window` | Switch to one of those in another window. |
| `consult-bookmark` | Jump to a bookmark. |
| `consult-compile-error` | Jump to an error in a compilation buffer. |
| `consult-dir` | Open the project or a directory from a recent file or bookmark. |
| `consult-fd` | Find a file by name with `fd`. |
| `consult-find` | Find a file by name with `find`. |
| `consult-flymake` | Jump to a diagnostic in this buffer, or all buffers with a prefix. |
| `consult-focus-lines` | Hide lines matching a regular expression. |
| `consult-git-grep` | Search the project with `git grep`. |
| `consult-global-mark` | Jump to a mark in any buffer. |
| `consult-goto-line` | Jump to a line number. |
| `consult-grep` | Search the project with `grep`. |
| `consult-grep-match` | Jump to a match in a Grep buffer. |
| `consult-history` | Insert a past answer from the current prompt's history (`M-r`). |
| `consult-imenu` | Jump to a definition in this buffer. |
| `consult-imenu-multi` | Jump to a definition in any open buffer. |
| `consult-isearch-history` | Pick a past Isearch string (`M-r` during Isearch). |
| `consult-keep-lines` | Keep only lines matching a regular expression. |
| `consult-kmacro` | Run a remembered keyboard macro. |
| `consult-line` | Jump to a line in this buffer, previewing as you type. |
| `consult-line-multi` | Jump to a line in project buffers, or all buffers with a prefix. |
| `consult-locate` | Find a file by name with `locate` (or `mdfind` on macOS). |
| `consult-ls-git` | Open a tracked or untracked project file. |
| `consult-man` | Read a manual page. |
| `consult-mark` | Jump to a mark in this buffer. |
| `consult-minor-mode-menu` | Toggle a minor mode in this buffer. |
| `consult-mode-command` | Run a command from the enabled modes' keymaps; with a prefix, list all commands. |
| `consult-org-agenda` | Jump to an Org agenda entry. |
| `consult-org-heading` | Jump to an Org heading with its outline path. |
| `consult-outline` | Jump to an Org or Markdown heading. |
| `consult-preview-current` | Preview the candidate under the cursor (`C-.` in a completion list). |
| `consult-project-buffer` | Switch to a buffer in this project. |
| `consult-recent-file` | Jump to a recently opened file. |
| `consult-register` | Insert a register's text or jump to a position register. |
| `consult-register-load` | Alias for `consult-register`. |
| `consult-ripgrep` | Search the project with ripgrep. |
| `consult-theme` | Apply a named theme, previewing candidates. |
| `consult-todo` | Jump to a TODO, FIXME, HACK, XXX, or BUG in the project. |
| `consult-xref` | Jump to a published cross-reference. |
| `consult-yank-from-kill-ring` | Insert an older kill-ring entry. |
| `consult-yank-pop` | Replace the last yank with the next kill-ring entry. |
| `consult-yank-replace` | Alias for `consult-yank-pop`. |

Commands that select a location preview it by moving point; cancelling with
`C-g` restores the original position. Commands with side effects, such as
running a macro or inserting a register, wait until a selection is confirmed.

## Building

This plugin uses `forge-plugin-sdk` from the Forge repository in `Cargo.toml`.
Install the target and build the WebAssembly component:

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
forge --check-plugin target/wasm32-wasip2/release/forge_consult.wasm
```

Copy the resulting `.wasm` to `~/.config/forge/plugins/`.
Live previews while typing use Forge's minibuffer input-change hook (SDK
0.1.30).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
