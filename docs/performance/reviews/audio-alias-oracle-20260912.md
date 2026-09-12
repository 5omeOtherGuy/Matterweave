## Finding: unsound for concurrent registration/rendering

Perfect command synchronization and disjoint PCM ranges can establish **data-race freedom**, but do not establish the **reference exclusivity** this design requires.

### Exact source

Paths relative to `crates/matterweave-audio/src/`:

- **`service.rs:333–334`** writes through the live core:
  ```rust
  self.core.as_mut().expect("core present").pool[start as usize..start as usize + samples]
      .copy_from_slice(spec.samples);
  ```
- **`mixer.rs:125`** stores PCM as ordinary uniquely owned storage:
  ```rust
  pub pool: Box<[f32]>,
  ```
- **`backend/aaudio.rs:125–127`** invokes:
  ```rust
  (*core_ptr).render(out, channels);
  ```
  This implicitly creates the mutable receiver required by **`mixer.rs:248`**, `render(&mut self, ...)`.
- **`mixer.rs:280–283`** reads `self.pool[base]` and, for stereo, `self.pool[base + 1]`.

The callback takes an exclusive reference to the mixer containing the owned PCM pool, while registration accesses that pool through an independent mutable path. Ordinary slice indexing also creates slice-reference access; selecting disjoint indices afterward is not equivalent to establishing disjoint ownership with appropriately scoped borrows.

Thus, the argument “the callback never reads the samples being written” is insufficient. The abstraction permits conflicting reference permissions even when actual sample loads/stores do not overlap. Bounds checking and stable allocation addresses do not repair this.

The supplied [official `UnsafeCell` rule](https://doc.rust-lang.org/std/cell/struct.UnsafeCell.html#aliasing-rules) is decisive: it relaxes shared-reference immutability, **not mutable-reference uniqueness**, and supplies no synchronization. Simply wrapping the existing core or pool in `UnsafeCell` while retaining conflicting mutable references would not fix the design.

## Smallest correct boundary

Preserve the fixed pool, range allocator and command transport; separate their access ownership:

1. **Callback-exclusive mixer state.** Registration must never traverse or mutably borrow the live `MixerCore`. Control access to mixer state requires callback quiescence.
2. **Separately shared PCM allocation.** Both sides hold a preconstructed `Arc<PcmPool>`, with private storage such as:
   ```rust
   samples: Box<[UnsafeCell<f32>]>
   ```
3. **Narrow cell access.** Read samples by value and write selected samples through pointers obtained from their cells. Never manufacture a whole-pool `&mut [f32]` or ordinary `&[f32]` spanning concurrently writable cells. Keep unsafe operations and any `unsafe impl Sync` inside this abstraction; access must require the range-ownership protocol.
4. Allocate storage and clone ownership handles before callbacks. Rendering needs only bounded indexing and sample copies—no allocation, deallocation or locks.

### Concrete invariant

Each sample range is either:

- **Writer-owned:** only the control thread may access it; or
- **Published/retiring:** callbacks may read it, and nobody writes it.

Initialization happens-before publication. Reclamation occurs only after an acknowledgement proving the final possible read has completed and no queued command can revive access. Access-derived references cannot survive reclamation. Storage remains allocated until all callbacks are quiescent.

Under the stipulated perfect synchronization, that boundary addresses both aliasing and data races without expanding the callback’s work.

## Uncertainty and engineering log

- Source inspection only; frozen revision `22070ca` was supplied, not independently Git-verified.
- No Miri execution or operational alias-model proof; exact diagnostic behavior is unverified.
- Command acknowledgement correctness and platform shutdown guarantees were assumed, not approved.
- Traced registration → PCM ownership → callback receiver → sample reads; checked cited line locations.
- No edits, commands, descendants, other reviewer material, or acceptance decision.
