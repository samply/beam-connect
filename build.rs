fn main() {
    build_data::set_GIT_COMMIT_SHORT().expect("Could not determine git commit");
    build_data::set_GIT_DIRTY().unwrap();
    build_data::set_BUILD_DATE();
    build_data::set_BUILD_TIME();
    // build_data::no_debug_rebuilds();
}
