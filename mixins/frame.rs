// `target = "horizon_client::game::native_hooks::Game<'_>::native_frame"`:
// runs Bobby's frame-budgeted `FakeChunkManager::update`, inserting newly decoded
// fake chunks into `game.world` and removing expired ones.
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

    for pos in events.unloaded_fake_chunks {
        let _ = game.streaming.forget(&mut game.world, pos.x, pos.z);
        if let Some(renderer) = game.renderer.as_mut() {
            renderer.remove_columns(std::iter::once((pos.x, pos.z)));
        }
    }

    for cached in events.loaded_fake_chunks {
        let key = (cached.pos.x, cached.pos.z);
        if let Some(client_chunk) = crate::game::bobby_mod::to_client_chunk(game, &cached, &config)
        {
            game.streaming.insert_remote(&mut game.world, client_chunk);
            crate::game::bobby_mod::with_state(|state| {
                state.resident_fakes.insert(key);
            });
        }
    }
}
