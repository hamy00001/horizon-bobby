// `target = "horizon_client::game::native_hooks::Game<'_>::native_frame"`:
// runs Bobby's frame-budgeted `FakeChunkManager::update`, building static visual-only
// chunk meshes directly into `game.renderer` and bypassing `game.world`.
fn bobby_on_frame(game: &mut crate::game::Game<'_>) {
    if let Some(remote) = game.remote.as_ref() {
        let address = remote.session.address.clone();
        let seed_hash = remote.hashed_seed;
        let dimension = remote.level.clone();
        let has_skylight = remote.dimension.has_skylight;
        let needs_open = crate::game::bobby_mod::with_state(|state| {
            state.manager.is_none()
                || state.active_server != address
                || state.active_seed_hash != seed_hash
                || state.active_dimension != dimension
        });
        if needs_open && !dimension.is_empty() {
            crate::game::bobby_mod::open_dimension(&address, seed_hash, &dimension, has_skylight);
        }
    } else {
        let was_open = crate::game::bobby_mod::with_state(|state| state.manager.is_some());
        if was_open {
            let fakes = crate::game::bobby_mod::with_state(|state| {
                std::mem::take(&mut state.resident_fakes)
            });
            if let Some(renderer) = game.renderer.as_mut() {
                renderer.remove_columns(fakes);
            }
            crate::game::bobby_mod::close_dimension();
        }
        return;
    }

    let Some(player) = game.player.as_ref() else {
        return;
    };
    let player_chunk_x = (player.position.x.floor() as i32) >> 4;
    let player_chunk_z = (player.position.z.floor() as i32) >> 4;
    let client_rd = game.settings.render_distance;

    let (events, config) = crate::game::bobby_mod::with_state(|state| {
        let cfg = state.config.clone();
        let ev = state
            .manager
            .as_mut()
            .map(|mgr| {
                mgr.update(
                    player_chunk_x,
                    player_chunk_z,
                    client_rd,
                    crate::game::bobby_mod::DEFAULT_FRAME_BUDGET,
                )
            })
            .unwrap_or_default();
        for pos in &ev.unloaded_fake_chunks {
            state.resident_fakes.remove(&(pos.x, pos.z));
        }
        (ev, cfg)
    });

    // 1. Remove expired fake chunks from the renderer (visual-only, not in game.world)
    if !events.unloaded_fake_chunks.is_empty() {
        if let Some(renderer) = game.renderer.as_mut() {
            renderer.remove_columns(events.unloaded_fake_chunks.iter().map(|p| (p.x, p.z)));
        }
    }

    // 2. Build static visual-only meshes for newly loaded fake chunks and upload directly to renderer
    for cached in events.loaded_fake_chunks {
        let pos = (cached.pos.x, cached.pos.z);
        // If the chunk is currently loaded as a real ticking chunk in game.world, skip
        if game.world.chunks.contains_key(&pos) {
            continue;
        }
        if let Some(client_chunk) = crate::game::bobby_mod::to_client_chunk(game, &cached, &config) {
            let meshes = crate::game::bobby_mod::mesh_fake_chunk(game, &client_chunk);
            if let Some(renderer) = game.renderer.as_mut() {
                for (sy, mesh) in meshes {
                    renderer.upload((pos.0, sy, pos.1), &mesh);
                }
            }
            crate::game::bobby_mod::with_state(|state| {
                state.resident_fakes.insert(pos);
            });
        }
    }
}
