//! the plugin as its own program, for looking at the editor without a host

fn main() {
  nice_plug::wrapper::standalone::nice_export_standalone::<canwetalk::CanWeTalk>();
}
