/// `hook = "client_level_unload"`: flushes saves and closes region files when leaving a level.
fn bobby_on_level_unload(_game: &mut crate::game::Game<'_>, _dimension: &str) {
    crate::game::bobby_mod::close_dimension();
}

