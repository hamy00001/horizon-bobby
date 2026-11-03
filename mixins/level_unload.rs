// `target = "horizon_client::game::native_hooks::Game<'_>::native_client_level_unload"`:
// flushes saves and closes region files when leaving a level.
fn bobby_on_level_unload(game: &mut crate::game::Game<'_>, dimension: &str) {
    let _ = game;
    let should_close = crate::game::bobby_mod::with_state(|state| {
        state.active_dimension.is_empty() || state.active_dimension == dimension
    });
    if should_close {
        crate::game::bobby_mod::close_dimension();
    }
}
