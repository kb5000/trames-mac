// SPDX-License-Identifier: GPL-3.0-or-later
//! Product lifecycle stays here; host console transport is shared with Linux.
pub use trames_cli_common::console::{attach, list};
pub use trames_host::console::Console;
const _: () = {
    assert!(
        u64::from_be_bytes(*trames_cli_common::console::MAGIC)
            == u64::from_be_bytes(*trames_host::console::MAGIC)
    );
    assert!(
        u64::from_be_bytes(*trames_cli_common::console::LIST)
            == u64::from_be_bytes(*trames_host::console::LIST)
    );
};
