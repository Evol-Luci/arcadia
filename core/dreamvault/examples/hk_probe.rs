#[tokio::main]
async fn main() {
    let engine = dreamvault::Engine::bootstrap().await.expect("bootstrap");
    let ids = [
        ("n64", "787bc817-3770-4e27-864c-284ed982b227"),
        ("virtualboy", "14f41032-2bcf-46d1-9844-e367fc10a03d"),
        ("lynx", "8585b5a0-2926-4067-855e-44357e3f5e6a"),
        ("snes", "4dd47ffd-798f-4141-b1ac-b0a365655ceb"),
        ("gba", "a78bc97d-ab11-4d17-99c6-925246f4b961"),
        ("gb", "78dea598-545d-483d-ab36-de51c92082b1"),
    ];
    for (plat, id) in ids {
        let hk = engine.game_hotkeys(id).await.unwrap_or_default();
        println!("[{plat}] -> {} hotkeys", hk.len());
        for h in &hk {
            let tag = if h.is_default { "default" } else { "user" };
            println!("    {:<18} {:<14} ({tag})", h.label, h.binding);
        }
    }
}
