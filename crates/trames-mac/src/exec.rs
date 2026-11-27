// SPDX-License-Identifier: GPL-3.0-or-later
//! Mac lifecycle uses the shared host server and independent CLI client.
pub use trames_cli_common::exec::{
    capture, run, run_input, run_interactive_streams_deadline as run_streams_deadline, Request,
};
pub use trames_host::exec::{path, Server};
const _: () = {
    assert!(trames_cli_common::exec::LIMIT == trames_host::exec::LIMIT);
    assert!(
        u64::from_be_bytes(*trames_cli_common::exec::MAGIC)
            == u64::from_be_bytes(*trames_host::exec::MAGIC)
    );
};
