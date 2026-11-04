// A click on button `button` of the mod's settings page.
fn bobby_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    let _ = game;
    if mod_id != "bobby" {
        return;
    }
    crate::game::bobby_mod::settings_click(button);
}
