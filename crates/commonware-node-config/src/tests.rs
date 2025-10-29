// TODO: desired tests
// round trips for all custom serde impls
//
// Possibly also snapshot tests

#[test]
fn can_parse_config() {
    const INPUT: &str = r#"
signer = "0x81d35644dd13b5d712215023ab16615d9f8852c5a2fdfbd72dee06f538894b58"
share = "0x002ca4985d4850d2836b02a9597170ae3e122d4f858a11ed6d6447d1ca3ec3380d"
listen_addr = "0.0.0.0:8000"
metrics_port = 8001
storage_directory = "/Users/janis/dev/tempo/tempo-commonware/test_deployment/945fadcd1ea3bac97c86c2acbc539fce43219552d24aaa3188c3afc1df4d50a7/storage"
worker_threads = 3
message_backlog = 16384
mailbox_size = 16384
deque_size = 10
fee_recipient = "0x0000000000000000000000000000000000000000"

[p2p]
max_message_size_bytes = 1_048_576

[timeouts]
time_for_peer_response = "2s"
time_to_collect_notarizations = "2s"
time_to_propose = "2s"
time_to_retry_nullify_broadcast = "10s"
views_to_track = 256
views_until_leader_skip = 32
new_payload_wait_time = "500ms"
"#;

    toml::from_str::<crate::Config>(INPUT).expect("config must be valid");
}
