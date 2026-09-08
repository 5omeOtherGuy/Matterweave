//! Optional graphics-queue timestamps, read only after the existing frame fence.
use super::{err, Device, Result};
use ash::vk;
use std::{sync::Arc, time::Instant};

/// A completed GPU submission, excluding CPU work and presentation. Availability
/// is explicit: absent queries never become fabricated zero GPU measurements.
#[derive(Clone, Copy, Debug)]
pub struct GpuTimings {
    pub frame_id: u64,
    pub render_ms: f64,
    /// None when disabled or when this submission reused the stored depth map.
    pub shadow_ms: Option<f64>,
    pub shadows: bool,
    /// This submission executed a depth pass, including a first-use clear.
    pub shadow_map_updated: bool,
    pub shadow_map_size: u32,
}

pub(crate) struct TimestampQueries {
    device: Arc<Device>,
    pool: vk::QueryPool,
    bits: u32,
    period_ns: f64,
    pending: bool,
    recorded_at: Option<Instant>,
    frame_id: u64,
    shadows: bool,
    shadow_map_updated: bool,
    size: u32,
    pub completed: Option<GpuTimings>,
}

fn elapsed_ms(start: u64, end: u64, bits: u32, period_ns: f64) -> Option<f64> {
    if !(1..=64).contains(&bits) || !period_ns.is_finite() || period_ns <= 0. {
        return None;
    }
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    };
    Some((end.wrapping_sub(start) & mask) as f64 * period_ns / 1_000_000.)
}

impl TimestampQueries {
    pub fn new(device: Arc<Device>) -> Result<Option<Self>> {
        // SAFETY: physical device and selected family are live. TOP/BOTTOM_OF_PIPE
        // timestamps work even when timestampComputeAndGraphics is false.
        unsafe {
            let props = device
                .instance
                .raw
                .get_physical_device_properties(device.physical);
            let queues = device
                .instance
                .raw
                .get_physical_device_queue_family_properties(device.physical);
            let bits = queues[device.family as usize].timestamp_valid_bits;
            let period_ns = f64::from(props.limits.timestamp_period);
            if elapsed_ms(0, 0, bits, period_ns).is_none() {
                return Ok(None);
            }
            let pool = device
                .raw
                .create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(3),
                    None,
                )
                .map_err(err)?;
            Ok(Some(Self {
                device,
                pool,
                bits,
                period_ns,
                pending: false,
                recorded_at: None,
                frame_id: 0,
                shadows: false,
                shadow_map_updated: false,
                size: 0,
                completed: None,
            }))
        }
    }

    pub fn read_completed(&mut self) -> Result<()> {
        if !self.pending {
            return Ok(());
        }
        // Modulo subtraction is ambiguous if a submission could span a full
        // counter period. CPU recording-to-read time is a conservative upper
        // bound; reject such samples instead of reporting a truncated duration.
        let wrap_ns = 2_f64.powi(self.bits as i32) * self.period_ns;
        if self
            .recorded_at
            .is_none_or(|t| t.elapsed().as_secs_f64() * 1_000_000_000. >= wrap_ns)
        {
            self.pending = false;
            self.completed = None;
            return Ok(());
        }
        let mut values = [[0_u64; 2]; 3];
        // SAFETY: the caller completed the submit fence; query pool is owned and
        // results use Vulkan's 64-bit value/availability pair layout. No WAIT flag.
        let result = unsafe {
            self.device.raw.get_query_pool_results(
                self.pool,
                0,
                &mut values,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WITH_AVAILABILITY,
            )
        };
        self.pending = false;
        self.completed = None;
        match result {
            Ok(()) if values.iter().all(|v| v[1] != 0) => {
                if let Some(render_ms) =
                    elapsed_ms(values[0][0], values[2][0], self.bits, self.period_ns)
                {
                    self.completed = Some(GpuTimings {
                        frame_id: self.frame_id,
                        render_ms,
                        shadow_ms: if self.shadows && self.shadow_map_updated {
                            elapsed_ms(values[0][0], values[1][0], self.bits, self.period_ns)
                        } else {
                            None
                        },
                        shadows: self.shadows,
                        shadow_map_updated: self.shadow_map_updated,
                        shadow_map_size: self.size,
                    });
                }
                Ok(())
            }
            Ok(()) | Err(vk::Result::NOT_READY) => Ok(()),
            Err(e) => Err(err(e)),
        }
    }
    pub fn begin(&mut self, cmd: vk::CommandBuffer) {
        self.recorded_at = Some(Instant::now());
        // SAFETY: prior use completed before reset; caller is recording this command buffer.
        unsafe {
            self.device.raw.cmd_reset_query_pool(cmd, self.pool, 0, 3);
            self.device.raw.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                self.pool,
                0,
            );
        }
    }
    pub fn mark(&self, cmd: vk::CommandBuffer, query: u32) {
        // SAFETY: internal callers use query 1 or 2 once per reset while recording.
        unsafe {
            self.device.raw.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                self.pool,
                query,
            );
        }
    }
    pub fn submitted(&mut self, shadows: bool, shadow_map_updated: bool, size: u32) {
        self.pending = true;
        self.frame_id += 1;
        self.shadows = shadows;
        self.shadow_map_updated = shadow_map_updated;
        self.size = size;
    }
}
impl Drop for TimestampQueries {
    fn drop(&mut self) {
        // SAFETY: Renderer waits for the last frame before fields are destroyed.
        unsafe {
            self.device.raw.destroy_query_pool(self.pool, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::elapsed_ms;
    #[test]
    fn timestamp_wrap_and_period_are_respected() {
        assert_eq!(elapsed_ms(250, 5, 8, 1000.), Some(0.011));
        assert_eq!(elapsed_ms(u64::MAX - 3, 2, 64, 1000.), Some(0.006));
        assert_eq!(elapsed_ms(10, 30, 32, 500_000.), Some(10.));
    }
    #[test]
    fn unsupported_or_invalid_timestamps_remain_unavailable() {
        assert_eq!(elapsed_ms(0, 2, 0, 1.), None);
        assert_eq!(elapsed_ms(0, 2, 65, 1.), None);
        assert_eq!(elapsed_ms(0, 2, 32, f64::NAN), None);
        assert_eq!(elapsed_ms(0, 2, 32, 0.), None);
    }
}
