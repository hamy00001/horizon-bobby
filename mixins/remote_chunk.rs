/// `target = "horizon_client::game::remote_chunks::Game<'_>::remote_chunk"`:
/// intercepts incoming `LevelChunkWithLight` packets so any pending fake-chunk load is cancelled
/// and any resident fake chunk at `(p.x, p.z)` is marked as replaced by real server data.
fn bobby_on_remote_chunk(
    game: &mut crate::game::Game<'_>,
    p: &horizon_protocol::packets::play::LevelChunkWithLight,
) {
    let pos = horizon_core::position::ChunkPos::new(p.x, p.z);
    let _ = game;
    crate::game::bobby_mod::with_state(|state| {
        state.resident_fakes.remove(&(p.x, p.z));
        if let Some(mgr) = &mut state.manager {
            let placeholder = crate::game::bobby_mod::CachedChunk::new(pos, -4, 1, true, 0);
            let _ = mgr.on_real_chunk_received(&placeholder);
        }
    });
}

