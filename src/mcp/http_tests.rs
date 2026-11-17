use super::super::{MAX_CONCURRENT_CALLS, catalog};
use super::{
    as_rmcp_tool, call_result, command_arg, http_protocol_versions, tool_concurrency_gate,
};
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

#[test]
fn http_tool_definitions_match_the_cli_catalog() {
    let catalog = catalog::tools();
    let converted = catalog
        .iter()
        .map(as_rmcp_tool)
        .collect::<Result<Vec<_>, _>>();
    let converted = converted.expect("catalog tool definitions should convert");
    assert_eq!(converted.len(), catalog.len());
    for (source, target) in catalog.iter().zip(converted) {
        let value = serde_json::to_value(target).expect("rmcp tool serializes");
        assert_eq!(value["name"], source.name);
        assert_eq!(value["inputSchema"], source.definition()["inputSchema"]);
        assert_eq!(value["description"], source.definition()["description"]);
    }
}

#[test]
fn http_call_results_preserve_stdio_envelope_and_error_mapping() {
    let success = json!({"content":[{"type":"text","text":"{\"ok\":true}"}],"structuredContent":{"ok":true},"isError":false});
    let mapped = call_result(success).expect("success result converts");
    assert_eq!(mapped.is_error, Some(false));
    assert_eq!(
        serde_json::to_value(&mapped).expect("result serializes")["resultType"],
        "complete"
    );
    assert!(matches!(&mapped.content[0], ContentBlock::Text(text) if text.text == "{\"ok\":true}"));
    let failure = json!({"content":[{"type":"text","text":"failed"}],"isError":true});
    let mapped = call_result(failure).expect("error result converts");
    assert_eq!(mapped.is_error, Some(true));
    assert_eq!(
        serde_json::to_value(&mapped).expect("error result serializes")["resultType"],
        "complete"
    );
}

#[tokio::test]
async fn http_tool_calls_never_exceed_the_stdio_concurrency_limit() {
    let gate = tool_concurrency_gate();
    let permits = (0..MAX_CONCURRENT_CALLS)
        .map(|_| gate.clone().try_acquire_owned())
        .collect::<Result<Vec<_>, _>>();
    assert!(permits.is_ok());
    assert!(gate.clone().try_acquire_owned().is_err());
    drop(permits);
    assert!(gate.try_acquire_owned().is_ok());
}

#[test]
fn malformed_http_call_result_is_rejected() {
    let result = call_result(Value::Null);
    assert!(result.is_err());
}

#[test]
fn streamable_http_transport_is_selected_by_the_mcp_flag() {
    let matches = command_arg(clap::Command::new("mcp"))
        .try_get_matches_from(["mcp", "--transport", "streamable-http"])
        .expect("transport flag parses");
    assert_eq!(
        matches.get_one::<String>("transport").map(String::as_str),
        Some("streamable-http")
    );
}

#[test]
fn http_does_not_advertise_the_superseded_http_sse_revision() {
    assert_eq!(
        http_protocol_versions()
            .first()
            .map(|version| version.as_str()),
        Some("2025-03-26")
    );
    assert_eq!(
        http_protocol_versions()
            .last()
            .map(|version| version.as_str()),
        Some("2026-07-28")
    );
}
