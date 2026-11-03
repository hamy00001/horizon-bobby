// `target = "horizon_client::game::remote::Game<'_>::remote_chunk_in_range"`:
// extends the client chunk retention radius to `max(server_range, bobby_render_distance)`.
fn bobby_remote_chunk_in_range(
    game: &crate::game::Game<'_>,
    x: i32,
    z: i32,
    in_server_range: bool,
) -> bool {
    if in_server_range {
        return true;
    }
    let _ = game;
    crate::game::bobby_mod::with_state(|state| {
        state
            .manager
            .as_ref()
            .is_some_and(|mgr| mgr.should_be_loaded(x, z))
    })
}

