use crate::{
    context::Context,
    dht::pending_validations::PendingValidationWithTimeout,
    state_dump::{address_to_content_and_type, DumpOptions, StateDump},
};
use holochain_core_types::chain_header::ChainHeader;
use holochain_persistence_api::cas::content::{Address, AddressableContent};
use std::sync::Arc;

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
fn header_to_string(h: &ChainHeader) -> String {
    format!(
        r#"===========Header===========
Type: {:?}
Timestamp: {}
Sources: {:?}
Header address: {}
Prev. address: {:?}
----------Content----------"#,
        h.entry_type(),
        h.timestamp(),
        h.provenances()
            .iter()
            .map(|p| p.source().to_string())
            .collect::<Vec<String>>()
            .join(", "),
        h.address(),
        h.link().map(|l| l.to_string())
    )
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
fn address_to_content_string(address: &Address, context: Arc<Context>) -> String {
    let maybe_content = address_to_content_and_type(address, context);
    maybe_content
        .map(|(content_type, content)| {
            format!("* [{}] {}: {}", content_type, address.to_string(), content)
        })
        .unwrap_or_else(|err| {
            format!(
                "* [UNKNOWN] {}: Error trying to get type/content: {}",
                address.to_string(),
                err
            )
        })
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub fn state_dump(context: Arc<Context>, options: DumpOptions) {
    let dump = StateDump::new(context.clone(), options);

    let queued_holding_workflows_strings = dump
        .queued_holding_workflows
        .iter()
        .map(
            |PendingValidationWithTimeout {
                 pending, timeout, ..
             }| {
                format!(
                    "<{}({})> {}: depends on : {:?}, timeout: {}",
                    pending.workflow.to_string(),
                    pending.entry_with_header.header.entry_type(),
                    pending.entry_with_header.entry.address(),
                    pending
                        .dependencies
                        .iter()
                        .map(|addr| addr.to_string())
                        .collect::<Vec<_>>(),
                    if timeout.is_none() {
                        "Never".to_string()
                    } else {
                        format!("{}", timeout.as_ref().unwrap())
                    },
                )
            },
        )
        .collect::<Vec<String>>();

    let in_process_holding_workflows_strings = dump
        .in_process_holding_workflows
        .iter()
        .map(
            |PendingValidationWithTimeout {
                 pending, timeout, ..
             }| {
                format!(
                    "<{}({})> {}: depends on : {:?}, timeout: {:#?}",
                    pending.workflow.to_string(),
                    pending.entry_with_header.header.entry_type(),
                    pending.entry_with_header.entry.address(),
                    pending
                        .dependencies
                        .iter()
                        .map(|addr| addr.to_string())
                        .collect::<Vec<_>>(),
                    if timeout.is_none() {
                        "Never".to_string()
                    } else {
                        format!("{}", timeout.as_ref().unwrap())
                    },
                )
            },
        )
        .collect::<Vec<String>>();

    let holding_strings = dump
        .held_aspects
        .iter()
        .map(|(entry_address, aspect_set)| {
            format!(
                "[{}]:\n\t{}",
                entry_address,
                aspect_set
                    .iter()
                    .map(|aspect_address| aspect_address.to_string())
                    .collect::<Vec<String>>()
                    .join("\n\t")
            )
            //address_to_content_string(entry_address, context.clone())
        })
        .collect::<Vec<String>>();

    let source_chain_strings = dump
        .source_chain
        .iter()
        .map(|(ewh, _)| {
            format!(
                "{}\n=> {}",
                header_to_string(&ewh.header),
                address_to_content_string(ewh.header.entry_address(), context.clone())
            )
        })
        .collect::<Vec<String>>();

    let debug_dump = format!(
        r#"
=============STATE DUMP===============
Agent's Source Chain:
========

{source_chain}

Nucleus:
========
Queued zome calls: {queued_calls:?}
Running zome calls: {calls:?}
Zome call results: {call_results:?}
--------------------

Network:
--------
Running query flows: {flows:?}
------------------------
Running VALIDATION PACKAGE requests: {validation_packages:?}
------------------------------------
Running DIRECT MESSAGES: {direct_messages:?}

Dht:
====
Queued validations {qlen}:
{queued_holding_workflows_strings}

In-process validations {iplen}:
{in_process_holding_workflows_strings}
--------
Holding:
{holding_list}
--------
    "#,
        source_chain = source_chain_strings.join("\n\n"),
        queued_calls = dump.queued_calls,
        call_results = dump.call_results,
        calls = dump.running_calls,
        qlen = dump.queued_holding_workflows.len(),
        queued_holding_workflows_strings = queued_holding_workflows_strings.join("\n"),
        iplen = dump.in_process_holding_workflows.len(),
        in_process_holding_workflows_strings = in_process_holding_workflows_strings.join("\n"),
        flows = dump.query_flows,
        validation_packages = dump.validation_package_flows,
        direct_messages = dump.direct_message_flows,
        holding_list = holding_strings.join("\n")
    );

    log_info!(context, "debug/state_dump: {}", debug_dump);
}
