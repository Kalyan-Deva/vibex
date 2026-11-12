# Security policy

## Reporting a vulnerability

Please report security problems **privately**: use [Report a vulnerability](https://github.com/Kalyan-Deva/vibex/security/advisories/new) on the repository's Security tab. Don't open a public issue.

Include what's affected, how to reproduce it, and what an attacker could do with it. You'll get a reply within a week. A fix ships in a new release, with credit if you'd like it.

## What counts

Anything that lets output, a file or a remote party do something you didn't ask for. For example:

- escape sequences that run commands, write files, or read the clipboard without permission
- a way around the secret guard, the risky-command check or the audit log's hash chain
- the update path accepting an unsigned or downgraded installer
- the bundled-editor updater installing a file that doesn't match its published SHA-256

## Supported versions

Only the latest release gets security fixes.

## How Vibex protects you

- **Updates** are signed with Ed25519. The installer and its version are both signed, so an old installer can't be passed off as new, and nothing installs until the signature checks out. The public key is in `crates/vibex/src/update.rs`. The private key never touches the repository.
- **Bundled editors** (nano, Neovim) are pinned by SHA-256 in `tools/bundle.lock`. Updates are checked the same way, against Neovim's published release digest and MSYS2's package database.
- **Clipboard reads** by programs (OSC 52) are off unless you turn them on.
- **Remote control** (`vibex cli`) is off by default. It's a per-user named pipe, never a network port.
- **Secrets** that the secret guard matches are masked on screen, kept out of history, recordings and the audit log, and blurred in screenshots and exports.
