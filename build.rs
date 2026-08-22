fn main() {
    compile_protos();
}

fn compile_protos() {
    let protos_dir =
        std::env::var("CONSTRUCT_PROTOS_DIR").unwrap_or_else(|_| "../construct-protos".to_string());

    println!("cargo:rerun-if-env-changed=CONSTRUCT_PROTOS_DIR");
    println!("cargo:rerun-if-changed={protos_dir}");

    let proto_files = [
        format!("{protos_dir}/core/crypto.proto"),
        format!("{protos_dir}/core/envelope.proto"),
        format!("{protos_dir}/core/identity.proto"),
        format!("{protos_dir}/core/pagination.proto"),
        format!("{protos_dir}/messaging/content.proto"),
        format!("{protos_dir}/messaging/e2ee.proto"),
        format!("{protos_dir}/signaling/presence.proto"),
        format!("{protos_dir}/signaling/webrtc.proto"),
        format!("{protos_dir}/services/auth_service.proto"),
        format!("{protos_dir}/services/key_service.proto"),
        format!("{protos_dir}/services/messaging_service.proto"),
        format!("{protos_dir}/services/user_service.proto"),
        format!("{protos_dir}/services/notification_service.proto"),
    ];

    let protos_path = std::path::Path::new(&protos_dir);
    if !protos_path.exists() {
        panic!(
            "construct-protos not found at '{protos_dir}'. \
             Clone it as a sibling of construct-tui, or set CONSTRUCT_PROTOS_DIR."
        );
    }

    let mut cfg = prost_build::Config::new();
    cfg.bytes(["."]);
    cfg.compile_protos(
        &proto_files.iter().map(String::as_str).collect::<Vec<_>>(),
        &[protos_dir.as_str()],
    )
    .unwrap_or_else(|e| panic!("proto compilation failed: {e}"));
}
