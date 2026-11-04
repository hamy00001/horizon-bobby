// `target = "horizon_client::game::remote::Game<'_>::forget_remote_columns"`:
// intercepts real chunks about to be unloaded by the server, queues non-blocking
// Anvil saving, and allows real chunks to naturally unload from game.world.
fn bobby_on_forget_remote_columns(
    game: &mut crate::game::Game<'_>,
    gone: &mut horizon_client::fast_hash::FxHashSet<(i32, i32)>,
) {
    if gone.is_empty() {
        return;
    }
    let config = crate::game::bobby_mod::with_state(|s| s.config.clone());
    if !config.enabled {
        return;
    }

    for &(x, z) in gone.iter() {
        // Snapshot the real server chunk before it unloads from game.world
        if let Some(real_arc) = game.world.chunks.get(&(x, z)).cloned() {
            if let Some(snapshot) =
                crate::game::bobby_mod::snapshot_client_chunk(game, &real_arc, &config)
            {
                crate::game::bobby_mod::with_state(|state| {
                    if let Some(mgr) = state.manager.as_mut() {
                        let _ = mgr.on_real_chunk_unloading(&snapshot);
                    }
                });
            }
        }
    }

    // Crucial: Allow the real chunks to naturally unload from game.world (do NOT call gone.remove).
    // The engine frees them from game.world.chunks, stopping ticks/physics/fluid updates.
    // Bobby's frame loop will supply static visual-only meshes directly to game.renderer.
}
