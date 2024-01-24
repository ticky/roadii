use anyhow::{anyhow, bail};
use anyhow::{Context, Result};
use clap::Parser;
use std::ffi::OsString;
use std::path::PathBuf;
use udev::{Device, Enumerator, Udev};

/// Wii Guitar mapping utility
#[derive(Parser, Debug)]
struct Args {
    /// The kernel name of the device to match, for example `input19`.
    /// If it is a Wiimote with a guitar attached it will be remapped.
    #[arg(short, long)]
    kernel_name: OsString,

    /// The path to the `evsieve` binary, useful if it isn't
    /// available in the `PATH` environment variable.
    ///
    /// If not supplied, `evsieve` will be run from the PATH.
    #[arg(short, long)]
    evsieve_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct Wiitar {
    wiimote: Option<Device>,
    guitar: Option<Device>,
    accel: Option<Device>,
}

impl Wiitar {
    fn from_kernel_name(kernel_name: OsString) -> Result<Self> {
        let udev = Udev::new().context("couldn't get access to Udev")?;

        Self::from_kernel_name_with_udev(kernel_name, udev)
    }

    fn from_kernel_name_with_udev(kernel_name: OsString, udev: Udev) -> Result<Self> {
        let guitar = {
            let mut kernel_name_enumerator = Enumerator::with_udev(udev.clone())
                .context("couldn't start a device enumerator")?;
            kernel_name_enumerator
                .match_sysname(&kernel_name)
                .unwrap_or_else(|_| {
                    panic!("couldn't set {:?} as parent device matcher", kernel_name)
                });

            let matching_devices: Vec<Device> = kernel_name_enumerator
                .scan_devices()
                .context("couldn't scan devices")?
                .collect();

            if matching_devices.len() != 1 {
                bail!(
                    "couldn't find a single matching device for {:?}",
                    kernel_name
                );
            }

            matching_devices[0].clone()
        };

        {
            // First up, we want to bail if this device doesn't pass our basic
            // sniff test. Theoretically the udev rule should guard against
            // this too but better to make sure than not!
            let name = guitar
                .attribute_value("name")
                .context("This device has no name? That's very strange.")?
                .to_string_lossy();

            // Unfortunately, despite an `extension` attribute on the hid-wiimote
            // driver, it isn't accessible after mount, so we may need to rely on
            // the display name, which is kind of strange, but if it works?
            if !name.contains("Wii") || !name.ends_with("Guitar") {
                bail!("That's a weird looking Wii Guitar (are the udev rules set right?)");
            }
        }

        // Next, we need to look at the parent device. Ultimately we want to
        // operate on the guitar device's siblings, but to get those we first
        // need to look at the parent, so, here we go...
        let wiimote = guitar
            .parent()
            .context("guitar didn't have a parent device")?;

        {
            // Sanity checks; the parent should be a hid-wiimote device
            if wiimote
                .subsystem()
                .context("The parent of the wiitar didn't have a subsystem")?
                != "hid"
            {
                bail!("The parent of the Wiitar is not a HID device?");
            }

            if wiimote
                .driver()
                .context("The parent of the wiitar didn't have a driver")?
                != "wiimote"
            {
                bail!("The parent of the Wiitar is an HID device but not a Wiimote?");
            }
        }

        println!(
            "Looks like {} is a Wiimote, with a guitar attached at {}!",
            wiimote.sysname().to_string_lossy(),
            guitar.sysname().to_string_lossy()
        );

        // Cool, let's get the party started, now we initialise our struct
        let mut inputs: Self = Default::default();

        {
            // Now we want to query siblings of the guitar
            let mut sibling_enumerator = Enumerator::with_udev(udev.clone())
                .context("couldn't start a device enumerator")?;
            sibling_enumerator
                .match_parent(&wiimote)
                .context("couldn't set wiimote as parent device matcher")?;
            sibling_enumerator
                .match_subsystem("input")
                .context("couldn't set input as device subsystem matcher")?;

            for device in sibling_enumerator
                .scan_devices()
                .context("couldn't scan sibling devices")?
                .filter(|device| {
                    device.syspath() != wiimote.syspath()
                        && device.parent().expect("device had no parent").syspath()
                            == wiimote.syspath()
                })
            {
                // Like mentioned above, the name is the best we can match
                // these on, thankfully these strings are constants in the
                // Linux kernel, and unlikely to change much, if at all.
                match device.attribute_value("name") {
                    Some(os_name) => match os_name.to_string_lossy().into_owned().as_str() {
                        "Nintendo Wii Remote" => {
                            if inputs.wiimote.is_none() {
                                let wiimote = Self::get_event_device_from_input_device_with_udev(
                                    &device,
                                    udev.clone(),
                                )?;
                                inputs.wiimote = Some(wiimote);
                            }
                        }
                        "Nintendo Wii Remote Guitar" => {
                            if inputs.guitar.is_none() {
                                let guitar = Self::get_event_device_from_input_device_with_udev(
                                    &device,
                                    udev.clone(),
                                )?;
                                inputs.guitar = Some(guitar);
                            }
                        }
                        "Nintendo Wii Remote Accelerometer" => {
                            if inputs.accel.is_none() {
                                let accel = Self::get_event_device_from_input_device_with_udev(
                                    &device,
                                    udev.clone(),
                                )?;
                                inputs.accel = Some(accel);
                            }
                        }
                        &_ => continue,
                    },
                    None => continue,
                };

                if inputs.is_complete() {
                    break;
                }
            }
        }

        if !inputs.is_complete() {
            bail!("Failed to find wiimote, guitar and accelerometer input devices");
        }

        Ok(inputs)
    }

    fn get_event_device_from_input_device_with_udev(device: &Device, udev: Udev) -> Result<Device> {
        let mut enumerator =
            Enumerator::with_udev(udev).context("couldn't start a device enumerator")?;
        enumerator
            .match_parent(device)
            .context("couldn't set device as parent device matcher")?;
        enumerator
            .match_subsystem("input")
            .context("couldn't set event as device subsystem matcher")?;

        for child in enumerator
            .scan_devices()
            .context("couldn't scan sibling devices")?
        {
            if child.syspath() == device.syspath() {
                continue;
            }

            if child.sysname().to_string_lossy().starts_with("event") {
                return Ok(child);
            }
        }

        bail!("didn't find a child event device")
    }

    fn is_complete(&self) -> bool {
        self.wiimote.is_some() && self.guitar.is_some() && self.accel.is_some()
    }
}

fn main() -> Result<()> {
    // We put this in a block so the main function can drop
    // everything else afterwards in preparation for exec'ing
    let mut evsieve = {
        let args = Args::parse();

        let parts = Wiitar::from_kernel_name(args.kernel_name)?;

        let mut evsieve = exec::Command::new(args.evsieve_path.unwrap_or("evsieve".into()));

        evsieve
            .arg("--input")
            .arg(
                parts
                    .wiimote
                    .ok_or(anyhow!("missing wiimote"))?
                    .devnode()
                    .ok_or(anyhow!("failed to retrieve wiimote devnode"))?,
            )
            .args(&["domain=wiimote", "grab", "persist=exit"]);

        evsieve.args(&["--map", "btn:south@wiimote", "btn:mode@wiitar"]);
        evsieve.args(&["--map", "btn:1@wiimote", "btn:thumbl@wiitar"]);
        evsieve.args(&["--map", "btn:2@wiimote", "btn:thumbr@wiitar"]);
        evsieve.args(&["--map", "btn:mode@wiimote", "btn:z@wiitar"]);
        evsieve.args(&["--map", "key:next@wiimote", "btn:start@wiitar"]);
        evsieve.args(&["--map", "key:previous@wiimote", "btn:select@wiitar"]);
        evsieve.args(&["--map", "key:left@wiimote", "btn:dpad_up@wiitar"]);
        evsieve.args(&["--map", "key:right@wiimote", "btn:dpad_down@wiitar"]);
        evsieve.args(&["--map", "key:up@wiimote", "btn:dpad_left@wiitar"]);
        evsieve.args(&["--map", "key:down@wiimote", "btn:dpad_right@wiitar"]);

        evsieve
            .arg("--input")
            .arg(
                parts
                    .guitar
                    .ok_or(anyhow!("missing wiimote guitar"))?
                    .devnode()
                    .ok_or(anyhow!("failed to retrieve wiimote guitar devnode"))?,
            )
            .args(&["domain=guitar", "grab", "persist=exit"]);

        evsieve.args(&["--map", "btn:south@wiimote", "btn:mode@wiitar"]);
        evsieve.args(&["--map", "btn:1@guitar", "btn:south@wiitar"]);
        evsieve.args(&["--map", "btn:2@guitar", "btn:east@wiitar"]);
        evsieve.args(&["--map", "btn:3@guitar", "btn:north@wiitar"]);
        evsieve.args(&["--map", "btn:4@guitar", "btn:west@wiitar"]);
        evsieve.args(&["--map", "btn:5@guitar", "btn:tl@wiitar"]);
        evsieve.args(&["--map", "btn:start@guitar", "btn:start@wiitar"]);
        evsieve.args(&["--map", "btn:select@guitar", "btn:select@wiitar"]);
        evsieve.args(&["--map", "btn:dpad_up@guitar", "btn:dpad_up@wiitar"]);
        evsieve.args(&["--map", "btn:dpad_down@guitar", "btn:dpad_down@wiitar"]);
        evsieve.args(&["--map", "abs:hat1x@guitar", "abs:rx:3x@wiitar"]);
        evsieve.args(&["--map", "abs:x@guitar", "abs:x@wiitar"]);
        evsieve.args(&["--map", "abs:y@guitar", "abs:y@wiitar"]);

        evsieve
            .arg("--input")
            .arg(
                parts
                    .accel
                    .ok_or(anyhow!("missing wiimote accelerometer"))?
                    .devnode()
                    .ok_or(anyhow!("failed to retrieve wiimote accelerometer devnode"))?,
            )
            .args(&["domain=accel", "grab", "persist=exit"]);

        evsieve.args(&["--block", "abs:rz@accel", "abs:rx@accel"]);
        evsieve.args(&["--map", "abs:ry:-59~..~-60@accel", "btn:select:1@wiitar"]);
        evsieve.args(&["--map", "abs:ry:~-60..-59~@accel", "btn:select:0@wiitar"]);

        // TODO: device-id et. al.
        evsieve.args(&["--output", "name=Wiitar", "@wiitar"]);

        evsieve
    };

    let error = evsieve.exec();

    Err(error.into())
}
