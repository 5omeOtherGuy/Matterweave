use super::*;
use crate::pcm::PcmPool;

#[test]
fn shared_pool_disjoint_reads_and_writes_do_not_touch_neighboring_samples() {
    let pool = Arc::new(PcmPool::new(8));
    assert_eq!(pool.len(), 8);
    // SAFETY: no readers exist yet; the destination is exclusively owned here.
    unsafe { pool.write(0, &[0.125; 4]) };
    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            start.wait();
            for _ in 0..10_000 {
                for index in 0..4 {
                    // SAFETY: this published range is never written in this scope.
                    assert_eq!(unsafe { pool.read(index) }, 0.125);
                }
            }
        });
        start.wait();
        for _ in 0..10_000 {
            // SAFETY: only this thread accesses cells 4..8; source is separate.
            unsafe { pool.write(4, &[-0.25; 4]) };
        }
    });
    // SAFETY: the writer and reader have both finished.
    assert_eq!(unsafe { pool.read(7) }, -0.25);
}

#[test]
fn registration_while_mutable_render_borrow_is_live_uses_separate_pool_owner() {
    let mut service = playing();
    let ptr = service.core.ptr();
    let mut registered = None;
    let mut out = [0.0; 2];
    // SAFETY: the synchronous mock cannot invoke another callback. The closure
    // only registers disjoint PCM, never reenters render or accesses the core.
    unsafe {
        (*ptr.0).render_with_after_drain(&mut out, 2, || {
            registered = Some(service.register_clip(ClipSpec::mono(&[-0.25; 4])).unwrap());
        });
    }
    assert_eq!(out, [0.25; 2]);
    service.play(registered.unwrap(), PlayOptions::default()).unwrap();
    service.mock_render(&mut out).unwrap();
    assert_eq!(out, [0.125; 2]); // original frame 0.375 plus new frame -0.25
    assert_eq!(service.health().rt_rejected_commands, 0);
}

// Taking CorePtr as a whole keeps the Send wrapper captured, not its raw field.
unsafe fn render_once(ptr: CorePtr, out: &mut [f32]) {
    // SAFETY: caller guarantees the unique callback borrow and allocation lifetime.
    unsafe { (*ptr.0).render(out, 2) };
}

#[test]
fn concurrent_service_registration_and_render_preserve_published_pcm() {
    let mut service = AudioService::new().unwrap();
    let samples = vec![0.125; MAX_PCM_BYTES / 8]; // half of the fixed pool
    let clip = service.register_clip(ClipSpec::mono(&samples)).unwrap();
    service.play(clip, PlayOptions::default()).unwrap();
    service.mock_render(&mut [0.0; 2]).unwrap();
    // Retire the mock before transferring callback access to the scoped thread.
    service.backend.as_mut().unwrap().close().unwrap();
    service.backend = None;
    let ptr = service.core.ptr();
    let start = std::sync::Barrier::new(2);
    let additions = [0.75; 16_384];
    std::thread::scope(|scope| {
        scope.spawn(|| {
            start.wait();
            let mut out = [0.0; 128];
            for _ in 0..2_000 {
                // SAFETY: backend is closed; this is the only core dereferencer.
                // Scope joins even on panic before service/owner destruction.
                unsafe { render_once(ptr, &mut out) };
                assert_eq!(out, [0.125; 128]);
            }
        });
        start.wait();
        for _ in 0..16 {
            service.register_clip(ClipSpec::mono(&additions)).unwrap();
        }
    });
    assert_eq!(service.health().callback_count, 2_001);
    assert_eq!(service.retained_pcm_bytes(), MAX_PCM_BYTES * 3 / 4);
    assert_eq!(service.health().rt_rejected_commands, 0);
}

#[test]
fn pool_handles_outlive_service_but_not_through_callback_reference_borrows() {
    let pool = {
        let service = playing();
        assert_eq!(Arc::strong_count(&service.pool), 2); // service and mixer
        service.pool.clone()
    }; // service closes backend, drops core's Arc, then its own Arc
    assert_eq!(Arc::strong_count(&pool), 1);
    // SAFETY: no service, callback or writer remains; payload is still alive.
    assert_eq!(unsafe { pool.read(0) }, 0.125);
}
