// The mod's row in Options > Mods.
fn bobby_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: "bobby".into(),
        title: "Bobby".into(),
        description: "Allows for render distances greater than the server's view-distance setting.".into(),
        keys: Vec::new(),
    });
}
