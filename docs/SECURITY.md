# Monitor: security

The threat model of Telamon Monitor: what it protects, who it defends against,
the rule each entry point follows, and the test that keeps the rule true.
`docs/DESIGN.md` ("Privilege") says what Monitor may do in one table; this file
says why that is safe, what comes in from outside, and what is left. Change it
together with the code it describes: some of the tests below read the QML and
fail when a rule here is skipped.

Report a vulnerability privately to the maintainer through GitHub's "Report a
vulnerability" on the repository (Security tab), not in a public issue.

## What Monitor is, security-wise

- It runs as the signed-in user, in the session, and has **no privilege of its
  own**: no setuid file, no root helper (it never calls the
  atlas-system-helper and adds no method to it), no polkit action, no system
  service. What needs more than the user is a D-Bus call to the service that
  owns the thing (systemd for services, udisks2 for drive health), and that
  service asks polkit itself. A bug in Monitor can therefore not do more than
  the user, or the user plus one polkit prompt they accept, could do.
- It reads a great deal that other programs write: every process's name, command
  line, cgroup and counters from `/proc`, desktop files and icon themes,
  autostart entries, unit files, podman's container lists, `/sys` attributes
  that devices fill in (a drive's model, a sensor's label), PipeWire's graph,
  fwupd's answers.
- It acts on three kinds of thing: **processes** (four signals), **services and
  startup items** (systemd, through its own polkit actions, and the user's own
  manager and `~/.config/autostart`), and **Energy Saver** (a CPU weight on an
  application's unit through the user's manager).
- It serves two D-Bus interfaces on the session bus: the single-instance one
  (`org.freedesktop.Application`, from KDBusService) and the focus object KWin's
  script reports to (section 9).
- It holds no secret. The one thing it puts on the clipboard is System Info
  ("Copy Details").

## Who we defend against

| Attacker | What they can reach | Defended? |
|---|---|---|
| **A hostile program of the session**, typically a sandboxed (Flatpak) app that was given a folder, or a compromised app | Process names and command lines (any program picks its own), cgroup folder names it can create, desktop files and icon themes in `~/.local/share`, entries in `~/.config/autostart`, user units, podman's lists, PipeWire's graph, the session bus | Yes: this is the main model. Everything arriving from it is cleaned, bounded and read without blocking (sections 1 and 2) |
| **Another local user** | Their processes (visible in `/proc` unless `hidepid`), their command lines, the names they give things | Yes: shown as plain text; their processes are never signalled (the kernel says no, and nothing escalates) |
| **A device** | A drive's model string, a USB device's name, a sensor's label, SMBIOS strings, firmware text | Yes: cleaned where read (`sysfs::read_string`, `hardware::clean`, `about::clean`) |
| **The system's services** (systemd, udisks2, fwupd) | Their replies | Mostly trusted (root-owned bus names, typed replies), but their text is cleaned and capped, wrong types are ignored |
| **The supply chain** | crates.io, the pinned git dependency, the build container, CI actions | Partly: locked and pinned builds, `cargo-deny`, `cargo-audit` (section 13) |

Out of scope, because it is not Monitor's to stop: code running as the same
user outside any sandbox (it can already do everything Monitor can, and read the
user's files, and `kill` the user's processes itself); a malicious or
compromised OS image; physical access; bugs in the services Monitor calls
(systemd, polkit, udisks2, fwupd, PipeWire, KWin); and what the pinned
`atlas-framework` crates (start-up, settings file, logging, crash reports) do
inside their own boundaries (we review how Monitor calls them, below).

## 1. Text from outside

**Rule.** Text that a program, a file or a device chose is cleaned *where it is
read* (`atlas-sysinfo`'s `text` module), not where it is drawn, so no table,
dialog, clipboard or log ever sees the raw string:

- `text::plain(s, max)`: one line of at most `max` characters; control
  characters (line breaks, escape, NUL, DEL, C1) and white space become one
  space between words; invisible format characters (bidirectional overrides and
  isolates, zero-width marks, the byte order mark, tag characters) are dropped;
  trimmed.
- `text::literal(s, max)`: for a path or a command line, where the spaces
  matter: the text as it is, but every control or invisible character becomes
  U+FFFD (so it shows that something was there), cut with `…`.
- `text::command_line(args)`: each argument through `literal`, at most 1024
  arguments and 16 384 characters in all, the rest replaced by one `…`.
- `text::icon(s)`: see section 6.
- **Joiners and fillers.** The zero-width non-joiner and joiner (U+200C, U+200D)
  are kept when they sit between two characters that are shown (Persian and
  Indic words, emoji sequences) and removed, or marked by `literal`, anywhere
  else (at an end, next to white space, a bidirectional control or another
  joiner); in an identifier or a path they are never kept. The fillers that
  draw blank (U+3164, U+FFA0, U+115F, U+1160, U+034F) are dropped like the other
  invisible characters, so a name made only of them is empty and gets the
  fallback (the file's or the ID's name). Bidirectional overrides and isolates,
  other zero-width characters and tag characters are always dropped. *Tests:*
  `text::tests::joiners_stay_between_two_shown_characters_only`,
  `blank_looking_names_are_empty`,
  `text::props::no_bidi_control_no_filler_and_joiners_between_letters`,
  `autostart::tests::a_planted_entry_shows_clean_text_and_no_remote_icon`.

| Text | Where it is cleaned | Limit |
|---|---|---|
| Process name (`comm`, or the program's file name where `comm` was cut) | `process::reuse`, when the scan first sees a name | 256 characters |
| Unit and container of a process (a cgroup folder's name, which whoever made the cgroup chose) | `process::reuse` | 256 |
| Details panel: name, state, user, parent's name, unit | `process::details` | 256 |
| Details panel: command line | `process::details::read_command_line` (reads at most 256 KiB of `cmdline`) | 1024 arguments, 16 384 characters |
| Details panel: executable path | `text::literal` in `src/details.rs` | 4096 |
| Application name and icon (desktop file), application ID (from a unit name) | `apps::desktop::parse_entry`, `app_id` | 256; an ID with a control character, a slash or a leading dot is no ID |
| Container name (podman's list) | `apps::container::parse` | 64 |
| Startup entries: name, comment, `Exec`, icon | `autostart::list_desktop` | 256, 1024, 1024 |
| Unit descriptions, sub-state, paths, documentation, systemd's error messages | `services` (`text()`, `strings()`, `parse_description`, `error_from_reply`) | 1024 |
| `/sys` attributes (drive model, sensor and battery labels, GPU names, MAC address) | `sysfs::read_string` | 1024, 16 KiB read |
| Devices page, SMBIOS, `os-release`, CPU model, fwupd attribute titles | `hardware::clean`, `about::clean` | 128 / 256 |

**Where it is drawn.** Every `Text`, `Label`, `Heading`, `TextEdit` and
`TextArea` of the QML says `textFormat: Text.PlainText` (a Qt `Label` guesses,
and `<img src="https://...">` in a process name would be fetched).
`TelamonLabel` and Telamon.Ui's rows, tables, dialogs and tooltips are plain by
default. Nothing asks for rich, styled, Markdown or auto-detected text. *Test:*
`tests/qml_text.rs` (`drawn_text_is_plain`, the checker's own tests, and
`checker_derived_types` for the inline `component NavHeading: Label`).

**The clipboard and the command line.**
- The only way to the clipboard is `Platform.copy`, called by System Info's
  "Copy Details" with the text of the page (hostname, product, firmware,
  graphics, fwupd's security level: all cleaned, no process data, no
  environment). *Test:* `copy_details_puts_only_the_pages_text_on_the_clipboard`.
- A command line is shown in a read-only plain `TextEdit` so a person can select
  and copy it. `quote_args` joins the arguments so that **pasting into a shell
  gives back the arguments that were shown**: anything outside
  `[A-Za-z0-9_@%+=:,./-]` is single-quoted (`;`, `|`, `&`, `$(...)`, backticks,
  `<`, `*`, `~`, `#`, quotes, spaces, line breaks), and a lone argument is
  shown as it is only if it has no shell character but spaces (Electron and
  Chromium rewrite their whole command line into one). Before this, only
  spaces, tabs, newlines, quotes, backslashes, `$` and backticks were quoted,
  so `a;rm -rf ~` pasted as two commands. *Tests:*
  `a_pasted_command_line_cannot_run_what_it_only_showed`, and the property tests
  `a_pasted_command_line_is_the_arguments` and `a_lone_argument_is_not_a_command`
  (a small POSIX tokenizer reads the output back).
- Control characters never reach a terminal that the text is pasted into: the
  reader replaced them (`literal`), so an escape sequence in an argument arrives
  as U+FFFD.

**Logs and crash reports.** Monitor logs what it did, not what it read: no
command line, no environment, no desktop file text is logged; the few lines that
name a thing name a pid or a cleaned name (`log::warn!("{action:?} for {pid}")`).
Nothing reads `/proc/<pid>/environ` (checked: no code path opens it). The crash
reports are the framework's (`telamon_app_init`; saved only when the user turned
them on in Telamon Updater, scrubbed there). The readers have no panicking path
on file contents: every parser is total (property tests, section 13), and
no panic message is built from process data.

*Tests:* `text::tests`, `text::props::*`, `process::tests::a_hostile_process_name_is_shown_clean`
(a real process whose file is called `ev\nil\x1b[2J‮x`),
`a_huge_process_name_is_capped`, `a_huge_command_line_is_read_in_part`,
`process::props::names_are_cleaned_where_they_are_read`,
`autostart::tests::a_planted_entry_shows_clean_text_and_no_remote_icon`,
`apps::tests::a_desktop_file_cannot_make_a_name_or_an_icon_of_its_own_choosing`,
`container_names_are_cleaned`, `services::props::*`, `sysfs::props::attributes_are_clean_lines`,
`hardware::props::*`, `about::props::*`.

## 2. Files and procfs: parsers that cannot be fed

**`/proc/<pid>`.** `parse_stat` splits at the *last* `)` (a name may hold
brackets, spaces, newlines, bytes that are not UTF-8), rejects numbers that
overflow instead of wrapping, and refuses a line that is cut short. The scan
reads each file into one shared buffer that **grows only to 1 MiB**
(`procfs::READ_MAX`): a process with a command line of megabytes (the kernel
allows it) does not make the buffer big for good. A process that exits mid-scan
is a failed read, skipped. Another user's `io` and `fd/` are unreadable: shown as
unknown, never as zero; their command lines are shown on request only if the
OS lets us read them (`hidepid` is the OS's switch), and are not logged.
*Tests:* `process::procfs::tests::a_huge_file_is_read_in_part`,
`process::props::stat_round_trips_whatever_the_name`,
`stat_of_anything_never_panics`, `stat_numbers_that_overflow_are_refused`,
`cgroup_files_give_units_and_containers_or_nothing`, `full_names_continue_the_comm`;
the unit tests in `process/parse.rs` (names with brackets, truncation at every
byte).

**Files that other programs can plant** are opened **without blocking**
(`O_NONBLOCK`, so a FIFO cannot hang the sampling thread in `open`), must be
**regular files** (checked on the descriptor, a link is followed to one as
Flatpak and dotfile managers make them), and are **read up to a cap**:

| File | Cap | Reader |
|---|---|---|
| Desktop files (`applications/`) | 256 KiB | `files::read_text_capped` |
| Icon theme `index.theme` | 1 MiB | `files::read_text_capped` |
| Icon files drawn from a path | regular, image extension, 4 MiB | `apps::icons::icon_file` |
| Autostart entries, unit files (descriptions) | 64 KiB | `autostart::read_whole`, `files::read_prefix` |
| podman's `containers.json`, `storage.conf` | 16 MiB, 256 KiB | `files::read_text_capped` |
| `.flatpak-info`, `installations.d/*.conf` | 64 KiB | `flatpak::read_info`, `files::read_text_capped` |
| Energy Saver's state file | 256 KiB, `O_NOFOLLOW` | `ease::system::load_state` |
| `/proc/<pid>/cmdline` | 256 KiB | `process::details::read_command_line` |
| `/sys` attributes | 16 KiB | `sysfs::read_string`, `read_uint` |
| Held kernel files | 8 MiB (the buffer's growth stops) | `sysfs::HeldFile` |
| `pw-dump` output | 32 MiB and 3 s | `ease::audio::run` |

Before this phase the desktop-file reader, the icon index, the container list
and `storage.conf` opened their file with a blocking `open` and no size cap: a
FIFO named `org.example.App.desktop` in `~/.local/share/applications` hung the
sampling thread (and with it every page) for good. *Tests:*
`files::tests::hostile_files_are_not_read`, `apps::tests::a_desktop_file_that_is_a_fifo_is_no_application`,
`an_icon_theme_index_that_is_a_fifo_does_not_hang_the_lookup`,
`a_container_list_that_is_a_fifo_does_not_hang_the_scan`,
`a_huge_desktop_file_is_not_read` (each runs the reader on a thread and fails
after 10 s), `autostart::tests::skips_a_fifo`, `files::props::the_cap_is_exact`.

## 3. Process actions

End Task (`SIGTERM`), Kill (`SIGKILL`), Stop (`SIGSTOP`) and Continue
(`SIGCONT`) are the **only** actions: an enum in Rust, an integer in QML that
`ProcessModel::act` maps to the enum (anything else is "failed"). There is no
renice and no priority change.

- **One process, by pid and start time.** The table's row carries the pid *and*
  the start time it showed. `process::act` opens a `pidfd` first, **then**
  reads `/proc/<pid>/stat` and compares the start time, and only then calls
  `pidfd_send_signal` on the descriptor. A pidfd stays bound to the process it
  was opened on and keeps its pid from being reused, so the signal goes to that
  process or to none, even if the pid is reused a moment later. A mismatch is
  "gone"; nothing is sent. *Tests:* `a_reused_pid_is_left_alone`,
  `a_process_that_is_gone`.
- **Never a group or everything.** `kill(2)` would take pid 0 for our process
  group, -1 for everything we may signal and a negative pid for a group; the
  pid here is a `u32` and a value above `i32::MAX` is the negative of one. `act`
  refuses 0 and anything above `i32::MAX` as "gone" before any system call, and
  **PID 1** as "not allowed" (its signals are the service manager's; for root
  `SIGTERM` would restart it). *Tests:* `process::tests::signals_never_reach_groups_or_pid_1`
  (with two bystander processes that must still be running),
  `process::props::no_pid_is_signalled_without_its_start_time` (every pid, with
  the start time no process has).
- **Other users' processes.** The kernel answers `EPERM`; the page says "Not
  allowed". Nothing escalates: no `pkexec`, no `kill` program, no helper, no
  D-Bus call. (`Command::new` appears in two places only, section 8.)
- **A group row** acts on each of its members by its own (pid, start time);
  a member that exited meanwhile is skipped, a reused pid fails the start-time
  check.
- Monitor can signal itself; that is the user's right to quit it.
- **Open File Location** shows `/proc/<pid>/exe` only if the file at that path
  is the program the kernel runs (same device and inode), through
  `org.freedesktop.FileManager1.ShowItems` with a percent-encoded `file://` URI
  (`files::file_uri`: every byte outside `A-Za-z0-9-._~/` encoded, non-UTF-8
  paths included), or by opening the folder's URI with the desktop.

## 4. Services (system manager) and Startup (user manager)

**Services.** `services::act` is the only caller of the system bus's actions and
it validates first, before any connection exists:

- `valid_name`: one unit name of a listed kind (`service socket timer path
  mount automount swap target`), at most 255 bytes, the characters systemd
  allows in a unit name (`[A-Za-z0-9:_.\@-]`), no `/`, no leading dot, no
  white space, control or non-ASCII character. A path (`/etc/...`, `../x`) is
  refused because `EnableUnitFiles` would link a file from anywhere. A name
  starting with `-` is legal and needed (`-.mount`, the root mount): names go
  over D-Bus as strings and are never the arguments of a program, so they cannot
  be options. Starting, stopping or restarting a **target** is refused
  (`poweroff.target` would turn the computer off).
- The methods called: `StartUnit`, `StopUnit`, `RestartUnit` with mode
  `"replace"`, `EnableUnitFiles(names, runtime=false, force=false)` and
  `DisableUnitFiles(names, runtime=false)`, each with the one validated name.
  No `LinkUnitFiles`, `MaskUnitFiles`, `SetUnitProperties`, `Reload`,
  `Kill` or `StartTransientUnit` on the system manager.
- **Polkit.** The message is flagged to allow interactive authorization;
  systemd asks polkit (`manage-units`, `manage-unit-files`) and polkit's agent
  prompts. Monitor never prompts, never caches a decision, never retries after
  a refusal and adds no rule of its own that could disagree with polkit. The
  system bus names are root-owned by the bus policy, so no program can answer in
  systemd's place.
- **Reading.** Listing and details need no privilege; `ServiceReader::details`
  refuses an invalid name; every string of a reply is cleaned and capped, every
  value of the wrong type is ignored (property tests with random types).

**Startup** uses the user's own files and manager:

- **Autostart entries.** `~/.config/autostart/<id>.desktop` only, where the id
  passes `valid_desktop_id` (ends `.desktop`, at most 255 bytes, no `/`, no
  leading dot, no control or invisible character). Switching an entry off
  writes `Hidden=true` into the user's file (a copy of the system entry when it
  came from `/etc/xdg`, which is never touched); switching on removes that copy
  or the line. The write is **atomic**: a file beside the target created with
  `O_EXCL` (mode 0644 less the umask), synced, then `rename`d over it; a symbolic
  link at the target is **replaced, never written through**, so a link planted
  to a system file leaves that file alone. The `Exec` line is parsed for
  display and `Runs` logic (`TryExec` is only looked up as a file) and is **never
  run, never through a shell**. *Tests:* `autostart::tests::replaces_a_link_rather_than_writing_through_it`,
  `refuses_bad_names_and_missing_entries`, `an_entry_named_with_control_characters_is_refused`,
  `skips_a_fifo`, `autostart::props::writes_are_whole_and_never_through_a_link`,
  `desktop_ids_are_file_names`, `hiding_and_showing_keep_the_entry`.
- **Units** through the user's manager (session bus; the user's own
  units): `EnableUnitFiles`, `DisableUnitFiles`, `MaskUnitFiles`,
  `UnmaskUnitFiles`, `Reload`, after `valid_unit_name`. Disabling a unit that
  had been `systemctl link`ed re-links it with `LinkUnitFiles`, using the target
  of the link that was there a moment before; that is the user's manager
  restoring the user's own link, no more than a planted link and the user's write
  access to `~/.config/systemd/user` could already do.
- The session's own units and Telamon Updater's tray are shown but cannot be
  switched off (`Lock`), checked again when the switch is made.

## 5. Energy Saver

`CPUWeight` as a runtime property on **an application's unit** through the
**user's** systemd manager (`SetUnitProperties(unit, runtime=true, ...)`),
restored on quit and after a crash. `app_unit` (an `app-*.scope|service` of 256
bytes in unit-name characters) is checked in `weight`, `set_weight` and for every
line of the state file, so a planted state file cannot name another unit. The
state file is `$XDG_RUNTIME_DIR/net.eterneon.telamon.monitor/eased`, in a
directory created 0700, written to a temporary file (`O_NOFOLLOW`, mode 0600) and
renamed, read with `O_NOFOLLOW|O_NONBLOCK` and a 256 KiB cap, and only a weight
systemd accepts is taken. *Tests:* `ease::props::state_files_give_application_units_only`,
`state_files_round_trip`, `weights_are_set_on_application_units_only`,
`cgroup_units_are_application_units`.

PipeWire's graph comes from `pw-dump` (section 8); its claims about who owns a
stream are checked against the socket's pid where PipeWire gives one
(`trusted`), and a client's own claim is used only to find an application.

## 6. Icons and images

`Kirigami.Icon` loads whatever it is given as a URL, **including `http://` and
`https://` from the network** (checked: an offscreen `Kirigami.Icon` with
`source: "http://127.0.0.1:.../x.png"` sends the request, repeatedly). So an
icon string from a file must never reach one unvetted. Before this phase the
Startup page passed the `Icon=` of every autostart entry straight to
`SectionRow.iconName`: an entry dropped into `~/.config/autostart` with
`Icon=https://tracker.example/pixel.png` made Monitor fetch it each time the
page was opened (an address leak, and a request from the user's machine to any
host or port on the local network). Checked on the packaged app, headless, with a
logging HTTP server: the build before this phase requested the address when the
Startup page opened; this one does not.

- `text::icon` accepts a **theme name** (`[A-Za-z0-9._+-]`, 128 bytes, not
  starting with a dot) or an **absolute path** with an image extension
  (`svg png svgz xpm`) and no control or invisible character, no `..`, no `?`,
  `#` or `%`; anything else is "" (no icon). It is used by the desktop-file
  parser and by the autostart reader.
- A path icon is drawn only if the file is a regular file of a known type and at
  most 4 MiB (`icons::icon_file`; Qt decodes on the GUI thread, and an SVG of
  hundreds of megabytes would stall it). The Apps table and the Startup page
  (`autostart::vetted_icon`) apply the same check, so a FIFO, a device, a huge
  file or a file of another type named by `Icon=` is no icon, and is not opened.
- The Apps table's and Energy Saver's icons come from `IconLookup`: a name the
  theme lists, or such a file. `index.theme` is read through the capped reader,
  its `Directories` cannot be absolute or climb out of the theme, and a theme
  name cannot contain `/`.
- The QML test (`icons_named_by_data_are_vetted`) lists every icon source that is
  not a literal or one of the app's own theme names, with the Rust function that
  vets it; a new one fails the test until it is listed. An `Image` is not used
  anywhere (`images_are_not_made_in_qml`). `os-release`'s `LOGO` is a name of
  `[A-Za-z0-9-_.+]` or nothing.

*Tests:* `text::tests::icons_are_names_or_image_files_only`,
`text::props::icons_are_safe_names_or_paths`, `urls_are_never_icons`,
`autostart::tests::a_planted_entry_shows_clean_text_and_no_remote_icon`,
`apps::tests::a_desktop_file_cannot_make_a_name_or_an_icon_of_its_own_choosing`.
*Residual:* an icon file in a *theme* folder (`~/.local/share/icons/...`) is
drawn without a size check: a stat for each of 20 000 files would double the
listing's cost, and the theme folders are the user's.

## 7. Firmware, drive health and system information

- **fwupd** (Host Security ID, System Info): the system bus, `Get`
  `HostSecurityId` and `GetHostSecurityAttrs`; the name is checked to be owned
  (or activatable) before calling, so a missing fwupd does not wait for a
  timeout; the name is root-owned by the bus policy. The reply is typed: an
  attribute of the wrong type is ignored, only the first 256 attributes are
  read, each title is cleaned and cut at 256 characters, the level is 0 to 5 or
  none and the version 32 characters of `[A-Za-z0-9.+-]`. Everything is read
  with a deadline of 8 s per call and 10 s in all (`about::TIMEOUT`, `DEADLINE`:
  the first call can start fwupd). There is **no network access**: Monitor does not fetch
  firmware, metadata or anything else. *Tests:* `about::props::security_from_survives_any_reply`,
  `a_flood_of_attributes_is_cut`, `hsi_ids_are_levels_and_versions`.
- **Drive health** is udisks2 over the system bus (`SmartGetAttributes`, its
  cached values); Monitor never runs `smartctl` or any program as root, and
  asks polkit for nothing. The reply is typed; the drive's name comes from
  sysfs (cleaned).
- **System information** is files: `os-release` (LOGO and HOME_URL checked, name
  cleaned), `/proc/cpuinfo`, DMI strings (placeholders dropped, cleaned), the
  Plasma metainfo, `uname`: each read with a cap, none through a shell or a
  program. The clipboard text is section 1.
- **Hardware** (PCI, USB, input): `pci.ids` and `usb.ids` from `hwdata` (root's),
  `/proc/bus/input/devices`, sysfs and udev's database; names are cleaned and
  capped at 128 characters.

## 8. Programs we start

Exactly two, by absolute or system path, never through a shell, never with text
from a file as an argument:

| Program | When | How |
|---|---|---|
| `/usr/bin/telamon-updater` (else `/usr/bin/atlas-updater`, this release only) | the "Open Telamon Updater" row of the Settings page | `Command::new(<absolute path>)`, no arguments, stdin/stdout/stderr null |
| `/usr/bin/pw-dump` (else `/bin/pw-dump`) | Energy Saver, when something is about to be eased and every 10 s while something is eased or busy | `ease::audio::find_program` looks in `/usr/bin` and `/bin` only (not the `PATH`), no arguments, stdout piped and read up to 32 MiB, killed after 3 s |

Before this phase both were looked up through the `PATH`, so a program of that
name in `~/.local/bin` (or any user-writable folder in front of `/usr/bin`) would
have run in their place. There is no `QProcess`, no `system()`, no `xdg-open`
(links go through `Qt.openUrlExternally`, section 10), no `pkexec`. The
environment is the process's own (`pw-dump` needs `XDG_RUNTIME_DIR` and
PipeWire's variables). *Test:* `ease::props::programs_are_found_by_file_name_only`;
the list above is held by `grep`-ing for `Command::new` in review, and CI's
shellcheck/QML lints cover the rest.

## 9. D-Bus objects we serve, and the signals we handle

- **`org.freedesktop.Application`** and **`org.kde.KDBusService`** on the session
  bus (KDBusService, `Unique`): any process of the session can ask the running
  window to raise itself (`Activate`) or to open a page (`--page <name>` in the
  arguments). The page name is at most 256 characters and Main.qml opens it only
  if it is one of its pages (anything else opens Overview). `Open` has no handler,
  so a document is never opened. A process that can call these can already raise
  any window it likes through the compositor; Flatpak's bus filter keeps
  sandboxed apps from the name unless they were given it.
- **The focus object `/Focus`** (Energy Saver, so the application in use is left
  alone): served on Monitor's *unique* bus name, which the KWin script is told.
  Any process of the session can list the bus's names and call it. A report is
  taken only from **KWin's own unique name** (`GetNameOwner("org.kde.KWin")`,
  compared with the sender of every call); anything else is dropped. The owner
  is found by polling (every 10 s, so a KWin that restarts is picked up late and
  reports from the new one are dropped until then), and while KWin is not on the
  bus a session-bus peer could claim the name `org.kde.KWin` and be believed.
  That peer is a process of the same session: it can already steer Energy Saver
  by being the focused window, and it is out of scope (section 14). The worst
  outcome is the one it started with: an application left alone or eased. Before this
  phase any process could set the "focused" pid, steering which application
  Energy Saver left alone or eased. *Tests:* `ease::kwin::tests::only_kwin_may_report`
  and `on_the_bus_only_kwin_is_believed` (a stranger and the real KWin on a private
  session bus). CI runs it under `dbus-run-session` with
  `TELAMON_REQUIRE_SESSION_BUS` set, which makes a missing bus a failure and not a
  skip.
- **The signal handler** (`main.cpp`, `SIGTERM`/`SIGINT`/`SIGHUP`) only writes a
  byte to a pipe; the event loop closes the window. A second signal kills as
  before.

## 10. Opening links

`Qt.openUrlExternally` appears twice, and `tests/qml_text.rs`
(`links_are_opened_in_known_places_only`) fails when a third is added or either
changes:

| Where | URL |
|---|---|
| About: "Source Code" | the constant `https://github.com/EternalCoder454/atlasos-monitor` |
| Apps: Open File Location, when no file manager answers | the `file://` URI of the program's folder, built in Rust (`file_uri`, every byte outside the unreserved set encoded): the desktop opens a folder, it does not run a file |

No link is drawn from outside text (`linkActivated` is not used anywhere).

## 11. unsafe, FFI and the CXX-Qt bridges

All `unsafe` in the shipped code, and why each is sound:

- `lib.rs`: `telamon_objects_new` and `telamon_icon_search_paths` take a C string
  from `main.cpp` (`QIcon::themeName()`), documented "null or NUL-terminated";
  turned into a `String` lossily.
- `process/details.rs`: `getpwuid_r` with a buffer that grows (up to 1 MiB) on
  `ERANGE`; the strings it returns are read inside the buffer's lifetime. `NSS`
  loads the system's modules, as every program does.
- `autostart/mod.rs`: `getuid()` (no preconditions). `power/mod.rs`:
  `clock_gettime`. `sysmem.rs`: `malloc_trim`.
- `gpu/nvml.rs`: `dlopen("libnvidia-ml.so.1")` and `dlsym`, only for a card bound
  to the `nvidia` driver (Telamon OS ships none; someone layered it), each
  function pointer cast to NVML's documented signature, the handle closed on
  drop. The library is found by soname, as the dynamic loader does for every
  library of the program: `LD_LIBRARY_PATH` is already trusted by the loader when
  the program starts, so this adds no trust.
- `settings.rs` and the tests: `set_var` and `mkfifo` in tests only.

The generated CXX-Qt bridges are the framework's and `cxx-qt`'s; Monitor's Rust
objects are made on the Qt thread and receive results only through
`qt_thread().queue`. The QML engine never owns them (`main.cpp` marks them
`CppOwnership` and deletes the sampler first).

## 12. Settings

`~/.config/telamon-monitorrc` is the framework's settings file (atomic write
through a temporary file and a rename, synced). What is read back is bounded:
the refresh interval snaps to the nearest offered one, a window size is 1 to
32 768 or ignored, the page is checked by Main.qml, booleans are `true`/`false`
(or `1`, `yes`, `on`), and lists (hidden columns, the applications Energy Saver
never eases) are trimmed, sorted, deduplicated words that are only compared with
ids, never used as paths or units.

## 13. Build hardening and supply chain

- **RPM flags.** The spec builds with Fedora's `%build_rustflags`,
  `%build_cflags`, `%build_cxxflags` and `%build_ldflags` (stack protector
  strong, `_FORTIFY_SOURCE`, stack clash protection, `-fcf-protection`, PIE, full
  RELRO and `BIND_NOW`, `-Werror=format-security`). The flags are *confirmed on
  the result*: `scripts/check-hardening.sh --cxx`, run by `%check` on the
  installed program, reads the ELF back and fails the package build without PIE,
  `GNU_RELRO` with `BIND_NOW`, a non-executable stack, no RPATH/RUNPATH/TEXTREL
  and stack protectors in the C++; then `annocheck` runs on it (PIE, RELRO, no
  writable GOT, ...). `scripts/test-check-hardening.sh` tests the check against
  programs built with and without each protection and runs in CI. *Not met:*
  Intel CET's IBT marking (rustc has no stable switch; the program carries
  `SHSTK`; the check notes it, `--require-cet` makes it an error). The
  build-path remaps keep the build tree out of the binary (checked in `%check`).
- **Locked and pinned.** Builds use `--locked`; the one git dependency
  (`atlas-framework`, by release tag) is pinned and the tag is checked against
  CI (the framework job); every GitHub Action and the reusable framework
  workflow are pinned by commit with the version in a comment.
- **`cargo-deny`** (`deny.toml`: advisories, licences, bans, sources; only
  crates.io and `atlas-framework`) and **`cargo-audit`** run in CI on every
  change to the dependencies and every week (`.github/workflows/security.yml`),
  so a new advisory against an unchanged `Cargo.lock` shows up without a commit.
  Dependabot opens weekly pull requests for crates and actions (the framework
  crates are moved by hand, CLAUDE.md).
- **CI.** Top-level `permissions: contents: read`; only the image job has
  `packages: write`, and its `GITHUB_TOKEN` is passed to the one step that logs in
  (through `env`, never interpolated into a script); every checkout has
  `persist-credentials: false`; no `pull_request_target`; no `github.event.*`
  text (titles, branch names) in a `run:`; caches are restored by every run and saved
  on push and schedule runs of the default branch (`main`) only, so neither a
  pull request nor another branch fills or evicts them.
- **Local builds.** `scripts/dev.sh` runs its container with `--ulimit core=0`
  (a crash leaves no core dump and no crash notification on the host) and
  `--security-opt no-new-privileges`; it never uses `--privileged`.
- **Property tests** (`proptest`, modules named `props`) of every parser of
  outside text: `PROPTEST_CASES=20000 cargo test --workspace --locked -- props`,
  run by CI. They check that nothing panics on hostile input (control
  characters, NUL, bidirectional overrides, huge strings, non-UTF-8 bytes, wrong
  types), that what is accepted has the safe shape, and that what is written is
  read back. A failure prints the input.

## 14. What is left

- **No sandbox for Monitor itself.** It is a normal session app that must read
  `/proc`, reach the system bus and `~/.config`. A Flatpak build is not planned.
- **Same-user programs.** A program running as the user outside a sandbox can
  already `kill` the user's processes, write `~/.config/autostart` and read their
  files. Monitor does not try to be a boundary against it; it only makes sure
  what such a program writes cannot make Monitor do more (fetch a URL, hang,
  paste a command, steer Energy Saver).
- **A peer that claims `org.kde.KWin`** while KWin is absent (section 9), and
  the polling delay in finding KWin's owner. Checking the owner's
  `GetConnectionCredentials` against our uid would not stop a process of the same
  user, which is the only peer that can do it.
- **Spoofing a name.** A process can call itself `systemd` or `Firefox`;
  cleaning removes what hides text, not what lies. The Details panel shows the
  pid, the executable, the command line, the user and the unit beside the name.
- **`setpriority`.** Monitor has no renice. Were one added, `setpriority` has no
  pidfd form, so the start time would have to be checked in `/proc` immediately
  before, and the small window between that check and the call documented.
- **Theme icon files** are drawn without a size check (section 6).
- **Hostile names make the scan do more.** A process whose `comm` is changed by
  cleaning is compared by value each tick instead of by bytes, and a cut name with
  a control character re-reads `cmdline` each tick: a cost only a hostile name
  pays.
- **The pinned framework crates** (start-up, settings file, logging, crash
  reports) and **telamon-ui** are reviewed at their call sites, not inside; they
  have their own `docs/SECURITY.md` in atlas-framework.
- **Fuzzing is property testing** (stable Rust, no coverage guidance);
  `cargo-fuzz` targets are a possible next step.
