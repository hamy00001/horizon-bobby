// `target = "horizon_client::game::native_hooks::Game<'_>::native_commands"`:
// registers the `/bobby` client command in chat auto-completion.
fn bobby_register_commands(commands: &mut Vec<&'static str>) {
    commands.push("bobby");
}
