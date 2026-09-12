use super::*;

/// Bounded preparation of an ordered group of geometry/bounds pairs.
/// Results are published together; no completed subset is exposed.
#[must_use = "retain the batch until completion or drop it to cancel mapping"]
pub struct GpuGeometryBatchPreparation {
    pending: Option<Vec<Entry>>,
    working_bytes: u64,
}

enum Entry {
    Pending(GpuGeometryPreparation),
    Ready(PreparedGpuGeometry),
}

impl GpuGeometryBatchPreparation {
    /// Checks all source identities and the aggregate working budget before packing
    /// or starting readbacks. Each entry counts its own packed output and staging,
    /// even when inputs are shared. Sources, deformation results and driver overhead
    /// are excluded. Limits across concurrent batches remain caller-owned.
    /// A later GPU failure cancels remaining mappings, not already submitted commands.
    pub fn new(
        inputs: &[(&GpuDeformationOutput, &WgpuScene3dGeometry)],
        bounds: &GpuDeformationBounds,
        max_working_bytes: Option<u64>,
    ) -> Result<Self> {
        ensure!(!bounds.context.device_lost(), "GPU bounds device is lost");
        let working_bytes =
            inputs
                .iter()
                .enumerate()
                .try_fold(0_u64, |total, (index, (output, source))| {
                    let bytes = output
                        .preparation_bytes(source, bounds)
                        .with_context(|| format!("GPU preparation input {index}"))?;
                    total
                        .checked_add(bytes)
                        .context("GPU batch preparation payload overflow")
                })?;
        ensure!(
            max_working_bytes.is_none_or(|limit| working_bytes <= limit),
            "GPU batch preparation requires {working_bytes} working bytes"
        );
        let pending = inputs
            .iter()
            .enumerate()
            .map(|(index, (output, source))| {
                output
                    .prepare_render_geometry(source, bounds, None)
                    .map(Entry::Pending)
                    .with_context(|| format!("GPU preparation input {index}"))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            pending: Some(pending),
            working_bytes,
        })
    }

    /// Aggregate admitted payload, retained after completion or failure.
    pub fn working_bytes(&self) -> u64 {
        self.working_bytes
    }

    /// Polls without waiting and returns pairs in input order once all are ready.
    /// Empty batches complete with an empty vector. Success and failure are terminal.
    pub fn try_read(&mut self) -> Result<Option<Vec<PreparedGpuGeometry>>> {
        let entries = self
            .pending
            .as_mut()
            .context("GPU batch preparation is finished")?;
        for (index, entry) in entries.iter_mut().enumerate() {
            if let Entry::Pending(request) = entry {
                match request.try_read() {
                    Ok(Some(prepared)) => *entry = Entry::Ready(prepared),
                    Ok(None) => {}
                    Err(error) => {
                        self.pending.take();
                        return Err(error.context(format!("GPU preparation input {index}")));
                    }
                }
            }
        }
        if entries
            .iter()
            .any(|entry| matches!(entry, Entry::Pending(_)))
        {
            return Ok(None);
        }
        Ok(Some(
            self.pending
                .take()
                .unwrap()
                .into_iter()
                .map(|entry| match entry {
                    Entry::Ready(prepared) => prepared,
                    Entry::Pending(_) => unreachable!("all entries completed"),
                })
                .collect(),
        ))
    }
}
