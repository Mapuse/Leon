//! Chainloading boot entries with the UEFI image loader.
//!
//! lbl boots every entry exactly the way the firmware Boot Manager does: it
//! builds a full device path for `\EFI\<vendor>\<name>.efi` on the volume it
//! was itself loaded from, then calls `LoadImage` with the boot-selection
//! policy and `StartImage`. Leon's own kernel, Linux's EFI stub, Windows, and
//! every other UEFI application are indistinguishable here.

use alloc::vec::Vec;

use uefi::boot::{self, LoadImageSource, image_handle};
use uefi::data_types::CStr16;
use uefi::proto::BootPolicy;
use uefi::proto::device_path::build::DevicePathBuilder;
use uefi::proto::device_path::{DevicePath, DeviceSubType, DeviceType};
use uefi::proto::loaded_image::LoadedImage;
use uefi::{Result, Status};

use super::entries::{self, Entry};

/// Loads and starts the entry. On success this only returns if the image
/// itself returned (e.g. it chose not to hand off), never if it took over.
pub fn boot(entry: &Entry) -> Result {
    let path = BootDevicePath::build(&entry.path)?;
    match boot::load_image(
        image_handle(),
        LoadImageSource::FromDevicePath {
            device_path: path.as_path(),
            boot_policy: BootPolicy::BootSelection,
        },
    ) {
        Ok(image) => boot::start_image(image)?,
        Err(err) => {
            // The most actionable failure: the firmware's image verification
            // rejected the entry because Secure Boot is on and it is unsigned
            // or not enrolled. Firmware reports this as either
            // SECURITY_VIOLATION (spec) or ACCESS_DENIED (OVMF/EDK2).
            let rejected_by_sb = matches!(
                err.status(),
                Status::SECURITY_VIOLATION | Status::ACCESS_DENIED
            ) && crate::secure_boot::state()
                == crate::secure_boot::SecureBootState::Enabled;
            if rejected_by_sb {
                crate::log_error!(
                    "boot: {} rejected by Secure Boot - sign it or enroll its key",
                    entries::cstr_lossy(entry.path.as_ref())
                );
            }
            return Err(err);
        }
    }
    Ok(())
}

/// A full device path for a `\EFI\...` file on lbl's own boot volume: the
/// device path of the volume lbl was loaded from, followed by the target
/// file path.
struct BootDevicePath {
    bytes: Vec<u8>,
}

impl BootDevicePath {
    /// Builds the device path, or `Status::LOAD_ERROR` on any malformed path.
    fn build(target: &CStr16) -> Result<Self> {
        // The loaded image's device handle is the ESP; its DevicePath protocol
        // is the volume part of the path (`PciRoot(0)/Pci(..)/HD(1,..)/`).
        let loaded = boot::open_protocol_exclusive::<LoadedImage>(image_handle())?;
        let device = loaded.device().ok_or(Status::LOAD_ERROR)?;
        let volume = boot::open_protocol_exclusive::<DevicePath>(device)?;

        let mut bytes = Vec::new();
        let mut builder = DevicePathBuilder::with_vec(&mut bytes);
        // Copy the volume nodes, skipping any trailing file-path nodes the
        // volume handle may expose.
        for node in volume.node_iter() {
            if node.full_type() == (DeviceType::MEDIA, DeviceSubType::MEDIA_FILE_PATH) {
                break;
            }
            builder = builder.push(&node).map_err(|_| Status::LOAD_ERROR)?;
        }
        drop(volume);
        builder
            .push(&uefi::proto::device_path::build::media::FilePath { path_name: target })
            .map_err(|_| Status::LOAD_ERROR)?
            .finalize()
            .map_err(|_| Status::LOAD_ERROR)?;
        Ok(Self { bytes })
    }

    /// Reinterprets the owned bytes as a device path for `LoadImage`.
    fn as_path(&self) -> &DevicePath {
        // SAFETY: `bytes` holds the node sequence built by `DevicePathBuilder`
        // (volume nodes + file path node + END_ENTIRE), which is exactly the
        // layout `DevicePath` wraps, and it outlives this reference.
        unsafe { DevicePath::from_ffi_ptr(self.bytes.as_ptr().cast()) }
    }
}
