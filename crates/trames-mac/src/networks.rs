// SPDX-License-Identifier: GPL-3.0-or-later
//! Product root/authorization selection; cold network management is shared.
use super::environments::{self as env, Result};
use std::path::Path;
use trames_distro::{
    environment::Manifest,
    networks::{Bridge, Networks},
};
pub fn arguments(root: &Path, c: &Manifest) -> Result<Vec<String>> {
    Networks::at(root).arguments(
        c,
        &env::grants(&env::directory(root, &c.environment.uuid)?, c)?,
    )
}
pub fn dispatch(args: &[String]) -> Result<u8> {
    Networks::at(&env::root()?).dispatch(args)
}

pub(super) fn listing(root: &Path) -> Result<Vec<Bridge>> {
    Networks::at(root).listing()
}
