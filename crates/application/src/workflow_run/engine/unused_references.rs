//! Rejects references to excluded nodes before prerequisite preparation or session creation.

use super::graph::{GraphError, WorkflowGraph};
use super::loop_config::LoopInitialValue;
use std::collections::HashSet;

/// Visits typed executable configuration, never arbitrary labels or JSON schema strings.
pub(super) fn validate(graph: &WorkflowGraph, unused: &[String]) -> Result<(), GraphError> {
    let unused: HashSet<&str> = unused.iter().map(String::as_str).collect();
    if unused.is_empty() {
        return Ok(());
    }
    for scope in graph.execution_scopes() {
        for node in scope.nodes() {
            for producer in scope.input_producers(node) {
                require_used(&producer, &unused)?;
            }
        }
    }
    Ok(())
}

impl WorkflowGraph {
    /// Definition IDs read by a node's executable selectors and templates in this scope.
    pub fn input_producers(
        &self,
        node: &super::graph::WorkflowGraphNode,
    ) -> std::collections::BTreeSet<String> {
        let mut ids = std::collections::BTreeSet::new();
        for selector in node
            .condition_config
            .iter()
            .flat_map(|config| &config.cases)
            .flat_map(|case| &case.conditions)
            .map(|rule| &rule.variable_selector)
            .chain(
                node.output_config
                    .iter()
                    .flat_map(|config| &config.outputs)
                    .map(|binding| &binding.variable_selector),
            )
            .chain(
                node.aggregator_config
                    .iter()
                    .flat_map(|config| &config.variables),
            )
        {
            ids.insert(selector.node_id.clone());
        }
        if let Some(config) = &node.iteration_config {
            ids.insert(config.iterator_selector.node_id.clone());
            ids.insert(config.collect_selector.node_id.clone());
        }
        if let Some((config, _)) = self.loop_body(&node.id) {
            for variable in &config.variables {
                if let LoopInitialValue::Variable(selector) = &variable.initial {
                    ids.insert(selector.node_id.clone());
                }
                ids.insert(variable.feedback.node_id.clone());
            }
            for selector in config
                .outputs
                .iter()
                .map(|binding| &binding.variable_selector)
                .chain(
                    config
                        .until
                        .cases
                        .iter()
                        .flat_map(|case| &case.conditions)
                        .map(|rule| &rule.variable_selector),
                )
            {
                ids.insert(selector.node_id.clone());
            }
        }
        for text in node
            .agent_config
            .iter()
            .map(|config| config.prompt.as_str())
            .chain(
                node.condition_config
                    .iter()
                    .flat_map(|config| &config.cases)
                    .flat_map(|case| &case.conditions)
                    .filter_map(|rule| rule.value.as_ref().and_then(|value| value.as_str())),
            )
            .chain(
                self.loop_body(&node.id)
                    .into_iter()
                    .flat_map(|(config, _)| {
                        config
                            .until
                            .cases
                            .iter()
                            .flat_map(|case| &case.conditions)
                            .filter_map(|rule| rule.value.as_ref().and_then(|value| value.as_str()))
                    }),
            )
        {
            visit_template_node_ids(text, &mut ids);
        }
        ids
    }
}

/// Walks Dify-style `{{#node.variable#}}` placeholders and records their producers.
fn visit_template_node_ids(text: &str, ids: &mut std::collections::BTreeSet<String>) {
    let mut remaining = text;
    while let Some((_, after_open)) = remaining.split_once("{{#") {
        let Some((selector, rest)) = after_open.split_once("#}}") else {
            break;
        };
        if let Some((node_id, _)) = selector.split_once('.') {
            ids.insert(node_id.to_owned());
        }
        remaining = rest;
    }
}

/// References never implicitly activate spare nodes or substitute empty values.
fn require_used(node_id: &str, unused: &HashSet<&str>) -> Result<(), GraphError> {
    if unused.contains(node_id) {
        return Err(GraphError::InvalidNode {
            reason: format!("variable references unused node {node_id}"),
        });
    }
    Ok(())
}
