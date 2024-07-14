use crate::{
    action::QueryKey,
    content_store::GetContent,
    context::Context,
    dht::pending_validations::PendingValidationWithTimeout,
    network::{direct_message::DirectMessage, entry_with_header::EntryWithHeader},
    nucleus::{ZomeFnCall, ZomeFnCallState},
};
use holochain_core_types::{
    chain_header::ChainHeader,
    eav::{EaviQuery, EntityAttributeValueIndex},
    entry::{entry_type::EntryType, Entry},
    error::HolochainError,
};
use holochain_json_api::json::JsonString;
use holochain_net::aspect_map::AspectMapBare;
use holochain_persistence_api::{
    cas::content::{Address, AddressableContent},
    eav::IndexFilter,
};
use std::{collections::VecDeque, convert::TryInto, sync::Arc};

#[derive(Serialize)]
pub struct StateDump {
    pub queued_calls: Vec<ZomeFnCall>,
    pub running_calls: Vec<(ZomeFnCall, Option<ZomeFnCallState>)>,
    pub call_results: Vec<(ZomeFnCall, Result<JsonString, HolochainError>)>,
    pub query_flows: Vec<QueryKey>,
    pub validation_package_flows: Vec<Address>,
    pub direct_message_flows: Vec<(String, DirectMessage)>,
    pub queued_holding_workflows: VecDeque<PendingValidationWithTimeout>,
    pub in_process_holding_workflows: VecDeque<PendingValidationWithTimeout>,
    pub held_aspects: AspectMapBare,
    pub source_chain: Vec<(EntryWithHeader, Address)>,
    pub eavis: Option<Vec<EntityAttributeValueIndex>>,
}

#[derive(Clone)]
pub struct DumpOptions {
    pub include_eavis: bool,
}

impl StateDump {
    pub fn new(context: Arc<Context>, options: DumpOptions) -> StateDump {
        let (agent, nucleus, network, dht) = {
            let state_lock = context.state().expect("No state?!");
            (
                (*state_lock.agent()).clone(),
                (*state_lock.nucleus()).clone(),
                (*state_lock.network()).clone(),
                (*state_lock.dht()).clone(),
            )
        };

        let source_chain: Vec<ChainHeader> = agent.iter_chain().collect();
        let source_chain: Vec<(EntryWithHeader, Address)> = source_chain
            .into_iter()
            .rev()
            .filter_map(|header| {
                let ewh = EntryWithHeader {
                    entry: agent
                        .chain_store()
                        .get(&header.entry_address())
                        .unwrap()
                        .unwrap(),
                    header: header.clone(),
                };
                // for now just drop the DNA entry
                if ewh.entry.entry_type() == EntryType::Dna {
                    None
                } else {
                    Some((ewh, header.address()))
                }
            })
            .collect();

        let queued_calls: Vec<ZomeFnCall> = nucleus.queued_zome_calls.into_iter().collect();
        let invocations = nucleus.hdk_function_calls;
        let running_calls: Vec<(ZomeFnCall, Option<ZomeFnCallState>)> = nucleus
            .running_zome_calls
            .into_iter()
            .map(|call| {
                let state = invocations.get(&call).cloned();
                (call, state)
            })
            .collect();
        let call_results: Vec<(ZomeFnCall, Result<_, _>)> =
            nucleus.zome_call_results.into_iter().collect();

        let query_flows: Vec<QueryKey> = network
            .get_query_results
            //using iter so that we don't copy this again and again if it is a scheduled job that runs everytime
            //it might be slow if copied
            .iter()
            .filter(|(_, result)| result.is_none())
            .map(|(key, _)| key.clone())
            .collect();

        let validation_package_flows: Vec<Address> = network
            .get_validation_package_results
            .into_iter()
            .filter(|(_, result)| result.is_none())
            .map(|(key, _)| key.address)
            .collect();

        let direct_message_flows: Vec<(String, DirectMessage)> = network
            .direct_message_connections
            .into_iter()
            .map(|(s, dm)| (s, dm))
            .collect();

        let queued_holding_workflows = dht.queued_holding_workflows().clone();
        let in_process_holding_workflows = dht.in_process_holding_workflows().clone();

        let held_aspects = dht.get_holding_map().bare().clone();

        let maybe_eavis = if options.include_eavis {
            let query = EaviQuery::new(
                Default::default(),
                Default::default(),
                Default::default(),
                IndexFilter::Range(Some(0), Some(std::i64::MAX)),
                None,
            );
            let eavis: Vec<EntityAttributeValueIndex> = dht
                .fetch_eavi(&query)
                .expect("should be ok")
                .iter()
                .cloned()
                .collect();
            Some(eavis)
        } else {
            None
        };

        StateDump {
            queued_calls,
            running_calls,
            call_results,
            query_flows,
            validation_package_flows,
            direct_message_flows,
            queued_holding_workflows,
            in_process_holding_workflows,
            held_aspects,
            source_chain,
            eavis: maybe_eavis,
        }
    }
}

impl From<Arc<Context>> for StateDump {
    fn from(context: Arc<Context>) -> StateDump {
        StateDump::new(
            context,
            DumpOptions {
                include_eavis: false,
            },
        )
    }
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub fn address_to_content_and_type(
    address: &Address,
    context: Arc<Context>,
) -> Result<(String, String), HolochainError> {
    let raw_content = context
        .dht_storage
        .read()?
        .fetch(address)?
        .ok_or(HolochainError::EntryNotFoundLocally)?;
    let maybe_entry: Result<Entry, _> = raw_content.clone().try_into();
    if let Ok(entry) = maybe_entry {
        let mut entry_type = entry.entry_type().to_string();
        let content = match entry {
            Entry::Dna(_) => String::from("DNA omitted"),
            Entry::AgentId(agent_id) => agent_id.nick,
            Entry::LinkAdd(link) | Entry::LinkRemove((link, _)) => format!(
                "({}#{})\n\t{} => {}",
                link.link.link_type(),
                link.link.tag(),
                link.link.base(),
                link.link.target(),
            ),
            Entry::App(app_type, app_value) => {
                entry_type = app_type.to_string();
                app_value.to_string()
            }
            _ => entry.content().to_string(),
        };
        Ok((entry_type, content))
    } else {
        Ok((String::from("UNKNOWN"), raw_content.to_string()))
    }
}
