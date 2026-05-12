//! per-subcommand handlers. each module owns one `cmd_*` entry point that
//! `main::run` dispatches to. shared helpers live in `util`.

pub(crate) mod add;
pub(crate) mod diff;
pub(crate) mod help_all;
pub(crate) mod refute;
pub(crate) mod show;
pub(crate) mod suspect;
pub(crate) mod util;
