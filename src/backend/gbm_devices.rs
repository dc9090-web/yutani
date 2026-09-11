//! Opened gbm devices, keyed by DRM dev id (as reported by the compositor).

use std::collections::hash_map::{self, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Default)]
pub struct GbmDevices {
    devices: HashMap<u64, (PathBuf, gbm::Device<fs::File>)>,
}

impl GbmDevices {
    pub fn gbm_device(&mut self, dev: u64) -> io::Result<Option<(&Path, &gbm::Device<fs::File>)>> {
        Ok(match self.devices.entry(dev) {
            hash_map::Entry::Occupied(entry) => {
                let (path, gbm) = entry.into_mut();
                Some((path, gbm))
            }
            hash_map::Entry::Vacant(entry) => match find_gbm_device(dev)? {
                Some(value) => {
                    let (path, gbm) = entry.insert(value);
                    Some((path, gbm))
                }
                None => None,
            },
        })
    }
}

fn find_gbm_device(dev: u64) -> io::Result<Option<(PathBuf, gbm::Device<fs::File>)>> {
    for entry in fs::read_dir("/dev/dri")? {
        let entry = entry?;
        if entry.metadata()?.rdev() == dev {
            let file = fs::File::options().read(true).write(true).open(entry.path())?;
            tracing::info!("opened gbm device {}", entry.path().display());
            return Ok(Some((entry.path(), gbm::Device::new(file)?)));
        }
    }
    Ok(None)
}
