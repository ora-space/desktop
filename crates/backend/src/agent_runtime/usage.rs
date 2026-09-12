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
    fn decode(&self, agent_ref: &AgentRef, meta: Option<&Meta>) -> UsageSupplement;
}

/// Leaves private agent metadata untouched until Ora supports a documented extension contract.
pub(super) struct NoUsageExtensions;

impl UsageExtensionDecoder for NoUsageExtensions {
    fn decode(&self, _agent_ref: &AgentRef, _meta: Option<&Meta>) -> UsageSupplement {
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
    let supplement = decoder.decode(agent_ref, meta);
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
