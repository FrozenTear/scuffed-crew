fn main() {
    let code = stat_tracker::pack_cutter::run(std::env::args().skip(1));
    std::process::exit(code);
}
