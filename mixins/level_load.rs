/// `target = "horizon_client::game::native_hooks::Game<'_>::native_client_level_load"`:
/// initializes `FakeChunkManager` when joining a remote dimension.
fn bobby_on_level_load(game: &mut crate::game::Game<'_>, dimension: &str) {
    let Some(remote) = game.remote.as_ref() else {
        return;
    };
    let address = remote.session.address.clone();
    let seed_hash = remote.hashed_seed;
    let has_skylight = remote.dimension.has_skylight;
    crate::game::bobby_mod::open_dimension(&address, seed_hash, dimension, has_skylight);
}
