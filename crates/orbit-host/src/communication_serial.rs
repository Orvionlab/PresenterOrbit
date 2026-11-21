use serde::Deserialize;
use serde_json::Value;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::{io::Write, time::Duration};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Settings {
    path: String,
    baud_rate: u32,
    data_bits: u8,
    parity: String,
    stop_bits: u8,
    flow_control: String,
}

fn settings(device: &Value) -> Result<Settings, String> {
    let settings: Settings = serde_json::from_value(device["serial"].clone())
        .map_err(|error| format!("Invalid serial settings: {error}"))?;
    if settings.path.trim().is_empty() || settings.path.trim() != settings.path {
        return Err("A serial port path is required without surrounding whitespace.".into());
    }
    if !(1..=4_000_000).contains(&settings.baud_rate)
        || !(5..=8).contains(&settings.data_bits)
        || !matches!(settings.stop_bits, 1 | 2)
        || !matches!(settings.parity.as_str(), "none" | "odd" | "even")
        || !matches!(
            settings.flow_control.as_str(),
            "none" | "software" | "hardware"
        )
    {
        return Err(
            "Invalid serial baud rate, data bits, parity, stop bits, or flow control.".into(),
        );
    }
    Ok(settings)
}

pub(crate) fn validate(device: &Value) -> Result<(), String> {
    settings(device).map(|_| ())
}

pub(crate) fn ports() -> Result<Value, String> {
    let mut ports = serialport::available_ports()
        .map_err(|error| format!("Could not enumerate serial ports: {error}"))?
        .into_iter()
        .map(|port| port.port_name)
        .collect::<Vec<_>>();
    ports.sort();
    ports.dedup();
    Ok(serde_json::json!(ports))
}

pub(crate) fn open(device: &Value) -> Result<Box<dyn serialport::SerialPort>, String> {
    let settings = settings(device)?;
    serialport::new(&settings.path, settings.baud_rate)
        .data_bits(match settings.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            8 => DataBits::Eight,
            _ => unreachable!("validated data bits"),
        })
        .parity(match settings.parity.as_str() {
            "none" => Parity::None,
            "odd" => Parity::Odd,
            "even" => Parity::Even,
            _ => unreachable!("validated parity"),
        })
        .stop_bits(if settings.stop_bits == 1 {
            StopBits::One
        } else {
            StopBits::Two
        })
        .flow_control(match settings.flow_control.as_str() {
            "none" => FlowControl::None,
            "software" => FlowControl::Software,
            "hardware" => FlowControl::Hardware,
            _ => unreachable!("validated flow control"),
        })
        .timeout(Duration::from_secs(3))
        .open()
        .map_err(|error| format!("Could not open serial port {}: {error}", settings.path))
}

pub(crate) fn send(device: &Value, payload: &[u8]) -> Result<(), String> {
    let mut port = open(device)?;
    // A successful write is local transport feedback, not a device acknowledgement.
    port.write_all(payload)
        .map_err(|error| format!("Serial write failed: {error}"))?;
    Ok(())
}
