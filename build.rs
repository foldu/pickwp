include!("src/cli.rs");

const BIN: &str = "pickwp";

fn main() {
    let mut opt = Opt::clap();

    let outdir = match std::env::var_os("OUT_DIR") {
        None => return,
        Some(outdir) => outdir,
    };

    opt.gen_completions(BIN, structopt::clap::Shell::Zsh, &outdir);
    opt.gen_completions(BIN, structopt::clap::Shell::Bash, &outdir);
    opt.gen_completions(BIN, structopt::clap::Shell::Fish, &outdir);
}
