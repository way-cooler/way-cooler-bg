
#[macro_export]
macro_rules! get_wayland {
    ($env_id: tt, $registry: expr, $event_queue: expr, $type: ty, $name: tt) => {{
        let state = $event_queue.state();
        let env = state.get_handler::<EnvHandler<WaylandEnv>>($env_id);
        let mut value = None;
        for &(name, ref interface, version) in env.globals() {
            if interface == $name {
                value = Some($registry.bind::<$type>(version, name));
                break;
            }
        }
        match value {
            Some(v) => v,
            _ => {
                for &(name, ref interface, version) in env.globals() {
                    println!("{:4} : {} (version {})", name, interface, version);
                }
                panic!(concat!("Could not find ", $name, " to bind to"));
            }
        }
    }}
}
