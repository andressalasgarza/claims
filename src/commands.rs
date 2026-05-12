//! per-subcommand handlers. each module owns one `cmd_*` entry point that
//! `main::run` dispatches to. shared helpers live in `util`.

pub(crate) mod util;
