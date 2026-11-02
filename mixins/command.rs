/// `hook = "command"`: handles `/bobby ...` client commands.
fn bobby_on_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if *ran {
        return;
    }
    let trimmed = command.trim().strip_prefix('/').unwrap_or(command.trim());
    if trimmed == "bobby" || trimmed.starts_with("bobby ") {
        let rest = trimmed.strip_prefix("bobby").unwrap_or("").trim();
        let reply = crate::game::bobby_mod::execute_command(game, rest);
        game.native_chat(&reply);
        *ran = true;
    }
}

