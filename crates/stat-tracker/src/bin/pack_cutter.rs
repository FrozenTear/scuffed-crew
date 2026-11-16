fn main() {
    let code = stat_tracker::pack_cutter::run(std::env::args_os().skip(1));
    std::process::exit(code);
}
