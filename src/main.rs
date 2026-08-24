mod app;
mod theme;

mod backend;
mod pages;
mod util;
mod widgets;

use app::App;
use util::perms::try_elevate;

fn main() -> anyhow::Result<()> {
    if !try_elevate() {
        return Ok(());
    }
    let mut app = App::new()?;
    app.run();

    Ok(())
}
