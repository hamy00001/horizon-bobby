// The mod's settings page: one button per option, labels pushed in order.
fn bobby_settings(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    let _ = game;
    if mod_id != "bobby" {
        return;
    }
    crate::game::bobby_mod::settings_buttons(buttons);
}
