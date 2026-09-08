/// CPU geometry changes and GPU residency are separate. A failed upload never
/// acknowledges either, and a new renderer needs even unchanged geometry.
#[derive(Default)]
pub(super) struct DynamicUploadState {
    dirty: bool,
    uploaded_epoch: Option<u64>,
}

impl DynamicUploadState {
    pub(super) fn needs_upload(&mut self, rebuilt: bool, renderer_epoch: u64) -> bool {
        self.dirty |= rebuilt;
        self.dirty || self.uploaded_epoch != Some(renderer_epoch)
    }

    pub(super) fn uploaded(&mut self, renderer_epoch: u64) {
        self.dirty = false;
        self.uploaded_epoch = Some(renderer_epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::DynamicUploadState;

    #[test]
    fn first_use_and_new_renderer_require_upload_even_without_rebuild() {
        let mut state = DynamicUploadState::default();
        assert!(state.needs_upload(false, 1));
        state.uploaded(1);
        assert!(!state.needs_upload(false, 1));
        assert!(state.needs_upload(false, 2));
        assert!(state.needs_upload(false, 2)); // Still pending, not acknowledged.
        state.uploaded(2);
        assert!(!state.needs_upload(false, 2));
    }

    #[test]
    fn failed_upload_retains_dirty_geometry_until_success() {
        let mut state = DynamicUploadState::default();
        state.uploaded(1);
        assert!(state.needs_upload(true, 1));
        // Upload failed; don't acknowledge. Next frame need not rebuild again.
        assert!(state.needs_upload(false, 1));
        state.uploaded(1);
        assert!(!state.needs_upload(false, 1));
    }

    #[test]
    fn changed_or_cleared_geometry_requires_upload_on_same_renderer() {
        let mut state = DynamicUploadState::default();
        state.uploaded(7);
        for _ in 0..3 {
            assert!(state.needs_upload(true, 7));
            state.uploaded(7);
            assert!(!state.needs_upload(false, 7));
        }
    }
}
