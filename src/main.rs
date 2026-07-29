use bootmux::env::Env;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = Env::from_process();
    bootmux::cli::bootstrap(args, env);
}
