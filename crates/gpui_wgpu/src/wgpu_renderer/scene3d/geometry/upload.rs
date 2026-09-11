use parking_lot::Mutex;

pub(in crate::wgpu_renderer::scene3d) struct Upload<T> {
    state: Mutex<State<T>>,
}

struct State<T> {
    payload: Option<T>,
    encoded: bool,
    external: bool,
}

impl<T> Upload<T> {
    pub(in crate::wgpu_renderer::scene3d) fn new(payload: Option<T>) -> Self {
        Self {
            state: Mutex::new(State {
                payload,
                encoded: false,
                external: false,
            }),
        }
    }

    pub(in crate::wgpu_renderer::scene3d) fn encode(&self, copy: impl FnOnce(&T)) {
        let mut state = self.state.lock();
        if let Some(payload) = &state.payload {
            copy(payload);
            state.encoded = true;
        }
    }

    pub(in crate::wgpu_renderer::scene3d) fn commit(&self, submitted: bool) {
        let mut state = self.state.lock();
        if std::mem::take(&mut state.encoded) && submitted && !state.external {
            state.payload = None;
        }
    }

    pub(in crate::wgpu_renderer::scene3d) fn retain_external(&self) {
        let mut state = self.state.lock();
        state.external = true;
        state.encoded = false;
    }

    #[must_use]
    pub(in crate::wgpu_renderer::scene3d) fn replace(
        &mut self,
        payload: impl FnOnce() -> T,
    ) -> bool {
        let state = self.state.get_mut();
        if state.external {
            return false;
        }
        state.payload = Some(payload());
        state.encoded = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn unsubmitted_uploads_replay_and_release_only_after_submission() {
        let payload = Arc::new(vec![1, 3, 5]);
        let retained = Arc::downgrade(&payload);
        let upload = Upload::new(Some(payload));
        let mut submitted = vec![0; 3];
        let mut abandoned = Vec::new();
        upload.encode(|bytes| abandoned.extend_from_slice(bytes));
        upload.commit(false);
        assert_eq!(abandoned, [1, 3, 5]);
        assert!(retained.upgrade().is_some());
        upload.encode(|bytes| submitted.copy_from_slice(bytes));
        upload.commit(true);
        assert_eq!(submitted, [1, 3, 5]);
        assert!(retained.upgrade().is_none());
        upload.encode(|_| panic!("submitted geometry must not upload again"));
    }

    #[test]
    fn another_submission_does_not_commit_an_unencoded_or_abandoned_upload() {
        let payload = Arc::new(vec![2, 4]);
        let retained = Arc::downgrade(&payload);
        let upload = Upload::new(Some(payload));
        upload.commit(true);
        assert!(retained.upgrade().is_some());
        let mut external_commands = Vec::new();
        upload.encode(|bytes| external_commands.extend_from_slice(bytes));
        upload.commit(false);
        upload.commit(true);
        assert!(retained.upgrade().is_some());
        let mut retry = Vec::new();
        upload.encode(|bytes| retry.extend_from_slice(bytes));
        assert_eq!(retry, external_commands);
        upload.commit(true);
        assert!(retained.upgrade().is_none());
    }

    #[test]
    fn shared_passes_retain_commands_after_the_upload_owner_releases() {
        let upload = Arc::new(Upload::new(Some(Arc::new(vec![7, 9]))));
        let other_pass = upload.clone();
        let mut commands = Vec::new();
        upload.encode(|bytes| commands.push(bytes.clone()));
        other_pass.encode(|bytes| commands.push(bytes.clone()));
        let retained = Arc::downgrade(&commands[0]);
        upload.commit(true);
        other_pass.commit(true);
        other_pass.encode(|_| panic!("shared upload was already submitted"));
        assert_eq!(*commands[0], [7, 9]);
        assert_eq!(*commands[1], [7, 9]);
        assert!(retained.upgrade().is_some());
        drop(commands);
        assert!(retained.upgrade().is_none());
    }

    #[test]
    fn externally_encoded_destinations_reject_replacement_even_after_owned_submission() {
        for payload in [None, Some(vec![2, 4])] {
            let mut upload = Upload::new(payload.clone());
            let mut external_commands = Vec::new();
            upload.encode(|bytes| external_commands.extend_from_slice(bytes));
            upload.retain_external();
            for submitted in [false, true] {
                upload.commit(submitted);
                assert!(!upload.replace(|| panic!("protected destination allocated an upload")));
                let mut replay = Vec::new();
                upload.encode(|bytes| replay.extend_from_slice(bytes));
                assert_eq!(replay, external_commands);
            }
            assert_eq!(external_commands, payload.unwrap_or_default());
        }
    }

    #[test]
    fn owned_destinations_accept_new_snapshots_without_changing_encoded_payloads() {
        let mut upload = Upload::new(None);
        let mut commands = Vec::new();
        for bytes in [vec![2, 4], vec![6, 8]] {
            assert!(upload.replace(|| Arc::new(bytes)));
            upload.encode(|payload| commands.push(payload.clone()));
            upload.commit(true);
            upload.encode(|_| panic!("committed snapshot was uploaded again"));
        }
        assert_eq!(*commands[0], [2, 4]);
        assert_eq!(*commands[1], [6, 8]);
    }
}
