// `target = "horizon_client::game::remote::Game<'_>::forget_remote_columns"`:
// intercepts real chunks about to be unloaded by the server (`ForgetLevelChunk`,
// `SetChunkCacheCenter`, `SetChunkCacheRadius`), queues non-blocking Anvil `.mca` saving,
// and keeps columns that are still within Bobby's client render distance as `FakeChunk`s.
fn bobby_on_forget_remote_columns(game: &mut crate::game::Game<'_>, gone: &mut horizon_client::fast_hash::FxHashSet<(i32, i32)>) {
    if gone.is_empty() {
        return;
    }
    let config = crate::game::bobby_mod::with_state(|s| s.config.clone());
    if !config.enabled {
        return;
    }

    let mut keep_existing_fake = Vec::new();
    let mut keep_as_fake = Vec::new();
    for &(x, z) in gone.iter() {
        // Keep resident fake chunks that are still within Bobby's render distance;
        // only snapshot real server chunks that are about to be unloaded.
        let (is_fake, still_in_bobby_range) = crate::game::bobby_mod::with_state(|s| {
            let fake = s.resident_fakes.contains(&(x, z));
            let in_range = s
                .manager
                .as_ref()
                .is_some_and(|mgr| mgr.should_be_loaded(x, z));
            (fake, in_range)
        });
        if is_fake {
            if still_in_bobby_range {
                keep_existing_fake.push((x, z));
            }
            continue;
        }
        let Some(real_arc) = game.world.chunks.get(&(x, z)).cloned() else {
            continue;
        };
        let Some(snapshot) = crate::game::bobby_mod::snapshot_client_chunk(game, &real_arc, &config)
        else {
            continue;
        };
        let substitute = crate::game::bobby_mod::with_state(|state| {
            state
                .manager
                .as_mut()
                .and_then(|mgr| mgr.on_real_chunk_unloading(&snapshot))
        });
        if let Some(fake_chunk) = substitute {
            keep_as_fake.push(((x, z), fake_chunk));
        }
    }

    for (x, z) in keep_existing_fake {
        gone.remove(&(x, z));
    }

    // Any column that remains within Bobby's local render distance is removed from `gone`
    // and replaced in-place with its FakeChunk (tainted and/or stripped of block entities).
    for ((x, z), fake_cached) in keep_as_fake {
        gone.remove(&(x, z));
        if config.no_block_entities {
            game.world
                .block_entities
                .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
        }
        if config.taint_fake_chunks {
            if let Some(client_chunk) =
                crate::game::bobby_mod::to_client_chunk(game, &fake_cached, &config)
            {
                game.streaming.insert_remote(&mut game.world, client_chunk);
            }
        }
        crate::game::bobby_mod::with_state(|state| {
            state.resident_fakes.insert((x, z));
        });
    }
}

