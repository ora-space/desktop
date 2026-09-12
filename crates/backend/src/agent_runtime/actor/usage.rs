use agent_client_protocol_schema::v1::{Meta, Usage};
use ora_contracts::{TokenAccountingScope, TokenUsageReport};
use ora_domain::AgentRef;

/// Optional values decoded from an agent-specific, namespaced metadata contract.
///
/// Standard ACP fields always take precedence over these supplements. An extension should return
/// only values whose semantics are documented by that agent rather than inferring them from a
/// provider name or from how counters changed between turns.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct UsageSupplement {
    pub accounting_scope: Option<TokenAccountingScope>,
    pub thought_tokens: Option<u64>,
    pub cached_read_tokens: Option<u64>,
    pub cached_write_tokens: Option<u64>,
}

/// Decodes explicitly supported usage extensions without coupling the runtime to private metadata.
///
/// Implementations are expected to recognize a documented, namespaced `_meta` shape for a known
/// agent version. Unknown metadata must produce an empty supplement.
pub(super) trait UsageExtensionDecoder {
    fn decode(
        &self,
        agent_ref: &AgentRef,
        usage_meta: Option<&Meta>,
        response_meta: Option<&Meta>,
    ) -> UsageSupplement;
}

/// Leaves private agent metadata untouched until Ora supports a documented extension contract.
pub(super) struct NoUsageExtensions;

impl UsageExtensionDecoder for NoUsageExtensions {
    fn decode(
        &self,
        _agent_ref: &AgentRef,
        _usage_meta: Option<&Meta>,
        _response_meta: Option<&Meta>,
    ) -> UsageSupplement {
        UsageSupplement::default()
    }
}

/// Converts ACP's draft response usage into Ora's stable, presentation-neutral contract.
pub(super) fn normalize_token_usage<D: UsageExtensionDecoder>(
    agent_ref: &AgentRef,
    usage: Option<&Usage>,
    meta: Option<&Meta>,
    decoder: &D,
) -> Option<TokenUsageReport> {
    let usage = usage?;
    let supplement = decoder.decode(agent_ref, usage.meta.as_ref(), meta);
    Some(TokenUsageReport {
        accounting_scope: supplement
            .accounting_scope
            .unwrap_or(TokenAccountingScope::Unspecified),
        total_tokens: usage.total_tokens,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        thought_tokens: usage.thought_tokens.or(supplement.thought_tokens),
        cached_read_tokens: usage.cached_read_tokens.or(supplement.cached_read_tokens),
        cached_write_tokens: usage.cached_write_tokens.or(supplement.cached_write_tokens),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    struct FixtureExtension;

    impl UsageExtensionDecoder for FixtureExtension {
        fn decode(
            &self,
            agent_ref: &AgentRef,
            usage_meta: Option<&Meta>,
            response_meta: Option<&Meta>,
        ) -> UsageSupplement {
            assert_eq!(agent_ref.as_str(), "fixture/agent");
            assert_eq!(
                usage_meta.and_then(|meta| meta.get("usage")),
                Some(&json!(true))
            );
            assert_eq!(
                response_meta.and_then(|meta| meta.get("response")),
                Some(&json!(true))
            );
            UsageSupplement {
                accounting_scope: Some(TokenAccountingScope::Turn),
                thought_tokens: Some(999),
                cached_read_tokens: Some(200),
                cached_write_tokens: Some(100),
            }
        }
    }

    #[test]
    fn normalizes_anonymized_agent_fixtures_without_guessing_scope() {
        let fixtures = [
            (
                "opencode",
                r#"{"inputTokens":149,"outputTokens":29,"totalTokens":11877,"thoughtTokens":51,"cachedReadTokens":11648}"#,
                TokenUsageReport {
                    accounting_scope: TokenAccountingScope::Unspecified,
                    total_tokens: 11_877,
                    input_tokens: 149,
                    output_tokens: 29,
                    thought_tokens: Some(51),
                    cached_read_tokens: Some(11_648),
                    cached_write_tokens: None,
                },
            ),
            (
                "claude",
                r#"{"inputTokens":2,"outputTokens":53,"cachedReadTokens":35719,"cachedWriteTokens":2273,"totalTokens":38047}"#,
                TokenUsageReport {
                    accounting_scope: TokenAccountingScope::Unspecified,
                    total_tokens: 38_047,
                    input_tokens: 2,
                    output_tokens: 53,
                    thought_tokens: None,
                    cached_read_tokens: Some(35_719),
                    cached_write_tokens: Some(2_273),
                },
            ),
            (
                "codex",
                r#"{"totalTokens":19163,"inputTokens":970,"cachedReadTokens":18176,"outputTokens":17,"thoughtTokens":0}"#,
                TokenUsageReport {
                    accounting_scope: TokenAccountingScope::Unspecified,
                    total_tokens: 19_163,
                    input_tokens: 970,
                    output_tokens: 17,
                    thought_tokens: Some(0),
                    cached_read_tokens: Some(18_176),
                    cached_write_tokens: None,
                },
            ),
        ];

        for (agent, fixture, expected) in fixtures {
            let usage: Usage = serde_json::from_str(fixture)
                .unwrap_or_else(|error| panic!("parse {agent} fixture: {error}"));
            let agent_ref = AgentRef::parse(format!("fixture/{agent}"))
                .unwrap_or_else(|error| panic!("parse fixture agent: {error}"));
            assert_eq!(
                normalize_token_usage(&agent_ref, Some(&usage), None, &NoUsageExtensions),
                Some(expected),
            );
        }
    }

    #[test]
    fn returns_none_when_the_prompt_response_has_no_usage() {
        let agent_ref = AgentRef::parse("fixture/agent").unwrap();
        assert_eq!(
            normalize_token_usage(&agent_ref, None, None, &NoUsageExtensions),
            None,
        );
    }

    #[test]
    fn extensions_only_fill_fields_missing_from_standard_acp_usage() {
        let mut usage = Usage::new(40, 10, 20);
        usage.thought_tokens = Some(7);
        usage.meta = Some(serde_json::from_value(json!({ "usage": true })).unwrap());
        let response_meta = serde_json::from_value(json!({ "response": true })).unwrap();
        let agent_ref = AgentRef::parse("fixture/agent").unwrap();

        assert_eq!(
            normalize_token_usage(
                &agent_ref,
                Some(&usage),
                Some(&response_meta),
                &FixtureExtension,
            ),
            Some(TokenUsageReport {
                accounting_scope: TokenAccountingScope::Turn,
                total_tokens: 40,
                input_tokens: 10,
                output_tokens: 20,
                thought_tokens: Some(7),
                cached_read_tokens: Some(200),
                cached_write_tokens: Some(100),
            }),
        );
    }
}
