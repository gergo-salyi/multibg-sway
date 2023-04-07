use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// directory with: wallpaper_dir/output/workspace_name.{jpg|png|...}
    pub wallpaper_dir: String,
}
