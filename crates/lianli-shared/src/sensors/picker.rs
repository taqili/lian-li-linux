use super::{NvidiaMetric, SensorCategory, SensorInfo, SensorSource, Unit};
use crate::media::SensorSourceConfig;
use std::path::Path;

/// Picks the most likely "CPU control temp" sensor — k10temp's `Tctl` /
/// coretemp's `Package id 0` first, then any other CPU temp sensor as a
/// fallback. Returns `None` if no CPU temp is exposed.
pub fn find_default_cpu_temp(sensors: &[SensorInfo]) -> Option<SensorSource> {
    let cpu_temps: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            s.unit == Unit::C
                && matches!(
                    &s.source,
                    SensorSource::Hwmon { name, .. } if name == "k10temp" || name == "coretemp"
                )
        })
        .collect();

    cpu_temps
        .iter()
        .find(|s| has_label(s, &["tctl", "package id 0"], "Control Temp"))
        .or_else(|| cpu_temps.first())
        .map(|s| s.source.clone())
}

/// Enumerated hwmon sources store the attribute prefix (`temp1`), so the
/// sysfs label is only visible through the display name derived from it.
fn has_label(sensor: &SensorInfo, labels: &[&str], display: &str) -> bool {
    let SensorSource::Hwmon { label, .. } = &sensor.source else {
        return false;
    };
    let label = label.to_lowercase();
    labels.iter().any(|needle| label.contains(needle))
        || sensor
            .sensor_name
            .as_ref()
            .is_some_and(|name| name.get_sensor_name() == display)
}

fn hwmon_device(sensor: &SensorInfo) -> Option<&str> {
    match &sensor.source {
        SensorSource::Hwmon { device_path, .. } if !device_path.is_empty() => Some(device_path),
        _ => None,
    }
}

/// Picks the most likely "GPU edge temp" sensor — NVIDIA first (if present),
/// then amdgpu's `edge` label, then any GPU-ish hwmon temp. When an APU's
/// integrated graphics also reports an edge temperature, the card that
/// reports a junction temperature is the discrete one and wins.
pub fn find_default_gpu_temp(sensors: &[SensorInfo]) -> Option<SensorSource> {
    if let Some(s) = sensors.iter().find(|s| {
        matches!(
            &s.source,
            SensorSource::NvidiaGpu {
                metric: NvidiaMetric::Temp,
                ..
            }
        )
    }) {
        return Some(s.source.clone());
    }
    let gpu_temps: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            s.unit == Unit::C
                && matches!(
                    &s.source,
                    SensorSource::Hwmon { name, .. } if name == "amdgpu" || name == "radeon"
                )
        })
        .collect();
    let discrete = |device: Option<&str>| {
        device.is_some_and(|device| {
            gpu_temps.iter().any(|s| {
                hwmon_device(s) == Some(device) && has_label(s, &["junction"], "Hotspot Temp")
            })
        })
    };
    let edges: Vec<&&SensorInfo> = gpu_temps
        .iter()
        .filter(|s| has_label(s, &["edge"], "Edge Temp"))
        .collect();
    edges
        .iter()
        .find(|s| discrete(hwmon_device(s)))
        .or_else(|| edges.first())
        .copied()
        .or_else(|| gpu_temps.first())
        .map(|s| s.source.clone())
}

/// The AMD junction (hotspot) temperature of the GPU that
/// [`find_default_gpu_temp`] picks, or of any AMD GPU when that pick has none.
pub fn find_default_gpu_hotspot(sensors: &[SensorInfo]) -> Option<SensorSource> {
    let junctions: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            s.unit == Unit::C
                && matches!(&s.source, SensorSource::Hwmon { name, .. } if name == "amdgpu")
                && has_label(s, &["junction"], "Hotspot Temp")
        })
        .collect();
    let edge = find_default_gpu_temp(sensors);
    let device = match &edge {
        Some(SensorSource::Hwmon { device_path, .. }) => Some(device_path.as_str()),
        _ => None,
    };
    junctions
        .iter()
        .find(|s| device.is_some() && hwmon_device(s) == device)
        .or_else(|| junctions.first())
        .map(|s| s.source.clone())
}

/// Resolve a `SensorCategory` to a concrete `SensorSourceConfig` based on
/// what the current machine exposes. Returns `None` when no suitable sensor
/// is available.
pub fn pick_source_for_category(
    category: SensorCategory,
    sensors: &[SensorInfo],
) -> Option<SensorSourceConfig> {
    match category {
        SensorCategory::CpuUsage => Some(SensorSourceConfig::CpuUsage),
        SensorCategory::MemUsage => Some(SensorSourceConfig::MemUsage),
        SensorCategory::MemUsed => Some(SensorSourceConfig::MemUsed),
        SensorCategory::MemFree => Some(SensorSourceConfig::MemFree),
        SensorCategory::CpuTemp => find_default_cpu_temp(sensors).map(source_to_config),
        SensorCategory::GpuTemp => find_default_gpu_temp(sensors).map(source_to_config),
        SensorCategory::GpuUsage => sensors
            .iter()
            .find(|s| {
                matches!(
                    &s.source,
                    SensorSource::NvidiaGpu {
                        metric: NvidiaMetric::Usage,
                        ..
                    } | SensorSource::AmdGpuUsage { .. }
                )
            })
            .map(|s| source_to_config(s.source.clone())),
        SensorCategory::NetworkRx => {
            default_route_iface().map(|iface| SensorSourceConfig::NetworkRx { iface })
        }
        SensorCategory::NetworkTx => {
            default_route_iface().map(|iface| SensorSourceConfig::NetworkTx { iface })
        }
        SensorCategory::DiskRead => {
            root_disk_device().map(|device| SensorSourceConfig::DiskRead { device })
        }
        SensorCategory::DiskWrite => {
            root_disk_device().map(|device| SensorSourceConfig::DiskWrite { device })
        }
    }
}

pub fn infer_sensor_category(source: &SensorSourceConfig) -> Option<SensorCategory> {
    match source {
        SensorSourceConfig::CpuUsage => Some(SensorCategory::CpuUsage),
        SensorSourceConfig::MemUsage => Some(SensorCategory::MemUsage),
        SensorSourceConfig::MemUsed => Some(SensorCategory::MemUsed),
        SensorSourceConfig::MemFree => Some(SensorCategory::MemFree),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::Temp,
            ..
        } => Some(SensorCategory::GpuTemp),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::Usage,
            ..
        } => Some(SensorCategory::GpuUsage),
        SensorSourceConfig::AmdGpuUsage { .. } => Some(SensorCategory::GpuUsage),
        SensorSourceConfig::Hwmon { name, label, .. } => {
            let l = label.to_lowercase();
            if name == "k10temp" || name == "coretemp" {
                if l.contains("tctl") || l.contains("package id 0") || l.starts_with("core") {
                    return Some(SensorCategory::CpuTemp);
                }
                return Some(SensorCategory::CpuTemp);
            }
            if (name == "amdgpu" || name == "radeon") && (l.contains("edge") || l.contains("temp"))
            {
                return Some(SensorCategory::GpuTemp);
            }
            None
        }
        SensorSourceConfig::NetworkRx { .. } => Some(SensorCategory::NetworkRx),
        SensorSourceConfig::NetworkTx { .. } => Some(SensorCategory::NetworkTx),
        SensorSourceConfig::DiskRead { .. } => Some(SensorCategory::DiskRead),
        SensorSourceConfig::DiskWrite { .. } => Some(SensorCategory::DiskWrite),
        SensorSourceConfig::Command { .. }
        | SensorSourceConfig::Constant { .. }
        | SensorSourceConfig::WirelessCoolant { .. } => None,
    }
}

fn source_to_config(source: SensorSource) -> SensorSourceConfig {
    use super::{DiskDirection, NetDirection};
    match source {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name,
            label,
            device_path,
        },
        SensorSource::NvidiaGpu { gpu_index, metric } => {
            SensorSourceConfig::NvidiaGpu { gpu_index, metric }
        }
        SensorSource::AmdGpuUsage { card_index } => SensorSourceConfig::AmdGpuUsage { card_index },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd },
        SensorSource::WirelessCoolant { device_id } => {
            SensorSourceConfig::WirelessCoolant { device_id }
        }
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
        SensorSource::NetworkRate { iface, direction } => match direction {
            NetDirection::Rx => SensorSourceConfig::NetworkRx { iface },
            NetDirection::Tx => SensorSourceConfig::NetworkTx { iface },
        },
        SensorSource::DiskRate { device, direction } => match direction {
            DiskDirection::Read => SensorSourceConfig::DiskRead { device },
            DiskDirection::Write => SensorSourceConfig::DiskWrite { device },
        },
    }
}

fn default_route_iface() -> Option<String> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[1] == "00000000" {
            return Some(fields[0].to_string());
        }
    }
    None
}

fn root_disk_device() -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let mut root_dev: Option<String> = None;
    for line in mounts.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[1] == "/" {
            root_dev = Some(fields[0].to_string());
            break;
        }
    }
    let dev = root_dev?;
    let partition = dev.strip_prefix("/dev/")?.to_string();
    let block = Path::new("/sys/class/block").join(&partition);
    let canon = std::fs::canonicalize(&block).ok()?;
    let parent = canon.parent()?;
    let parent_name = parent.file_name()?.to_string_lossy().to_string();
    if parent_name == "block" {
        Some(partition)
    } else {
        Some(parent_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::SensorName;

    fn temp(name: &str, prefix: &str, display: &str, device: &str) -> SensorInfo {
        SensorInfo {
            source: SensorSource::Hwmon {
                name: name.into(),
                label: prefix.into(),
                device_path: device.into(),
            },
            sensor_name: Some(SensorName {
                device_name: device.into(),
                sensor_name: display.into(),
            }),
            display_name: None,
            divider: 1000,
            unit: Unit::C,
            current_value: None,
        }
    }

    fn device_of(source: Option<SensorSource>) -> String {
        match source {
            Some(SensorSource::Hwmon {
                label, device_path, ..
            }) => format!("{device_path}/{label}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn discrete_gpu_edge_wins_over_an_earlier_integrated_gpu() {
        let sensors = [
            temp("amdgpu", "temp1", "Edge Temp", "0000:0f:00.0"),
            temp("amdgpu", "temp1", "Edge Temp", "0000:03:00.0"),
            temp("amdgpu", "temp2", "Hotspot Temp", "0000:03:00.0"),
            temp("amdgpu", "temp3", "VRAM Temp", "0000:03:00.0"),
        ];
        assert_eq!(
            device_of(find_default_gpu_temp(&sensors)),
            "0000:03:00.0/temp1"
        );
        assert_eq!(
            device_of(find_default_gpu_hotspot(&sensors)),
            "0000:03:00.0/temp2"
        );
        assert_eq!(find_default_gpu_hotspot(&sensors[..1]), None);
    }

    #[test]
    fn lone_integrated_gpu_and_cpu_control_temp_are_still_found() {
        let sensors = [
            temp("k10temp", "temp3", "Temp Die 1", "0000:00:18.3"),
            temp("k10temp", "temp1", "Control Temp", "0000:00:18.3"),
            temp("amdgpu", "temp1", "Edge Temp", "0000:0f:00.0"),
        ];
        assert_eq!(
            device_of(find_default_gpu_temp(&sensors)),
            "0000:0f:00.0/temp1"
        );
        assert_eq!(
            device_of(find_default_cpu_temp(&sensors)),
            "0000:00:18.3/temp1"
        );
    }
}
