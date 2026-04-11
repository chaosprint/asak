use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};

pub(crate) fn available_output_devices() -> Result<(Vec<String>, Option<String>)> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|device| device.name().ok());
    let mut names = host
        .output_devices()?
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    move_default_to_front(&mut names, default_name.as_deref());
    Ok((names, default_name))
}

pub(crate) fn available_input_devices() -> Result<(Vec<String>, Option<String>)> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut names = host
        .input_devices()?
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    move_default_to_front(&mut names, default_name.as_deref());
    Ok((names, default_name))
}

pub(super) fn resolve_output_device(preferred_name: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = preferred_name {
        if let Some(device) = host
            .output_devices()?
            .find(|device| device.name().ok().as_deref() == Some(name))
        {
            return Ok(device);
        }
    }

    host.default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device available"))
}

pub(super) fn resolve_input_device(preferred_name: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = preferred_name {
        if let Some(device) = host
            .input_devices()?
            .find(|device| device.name().ok().as_deref() == Some(name))
        {
            return Ok(device);
        }
    }

    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))
}

fn move_default_to_front(devices: &mut Vec<String>, default_name: Option<&str>) {
    let Some(default_name) = default_name else {
        return;
    };

    if let Some(index) = devices.iter().position(|device| device == default_name) {
        if index != 0 {
            let default_device = devices.remove(index);
            devices.insert(0, default_device);
        }
    }
}
