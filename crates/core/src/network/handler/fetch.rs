use crate::{
    action::{Action, ActionWrapper},
    context::Context,
    instance::dispatch_action,
    network::handler::{
        get_content_aspects, get_meta_aspects_from_chain, get_meta_aspects_from_dht_eav,
    },
};
use holochain_core_types::network::entry_aspect::EntryAspect;
use lib3h_protocol::{data_types::FetchEntryData, types::EntryHash};
use std::{collections::HashSet, sync::Arc};

/// The network has requested a DHT entry from us.
/// Lets try to get it and trigger a response.
#[autotrace]
#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub fn handle_fetch_entry(get_dht_data: FetchEntryData, context: Arc<Context>) {
    let entry_hash = get_dht_data.entry_address.clone();
    let aspect_set = fetch_aspects_for_entry(&entry_hash, context.clone());
    let aspects = aspect_set.into_iter().collect::<Vec<_>>();

    let action_wrapper = ActionWrapper::new(Action::RespondFetch((get_dht_data, aspects)));
    dispatch_action(context.action_channel(), action_wrapper);
}

pub fn fetch_aspects_for_entry(address: &EntryHash, context: Arc<Context>) -> HashSet<EntryAspect> {
    let mut aspects: HashSet<EntryAspect> = HashSet::new();

    // XXX: NB: we seem to be ignoring aspect_address_list and just attempting to get all aspects.
    // Is that right?

    match get_content_aspects(address, context.clone()) {
        Ok(content_aspects) => {
            // there may be more than one if the same entry data was committed twice
            for aspect in content_aspects {
                aspects.insert(aspect);
            }
            for result in &[
                get_meta_aspects_from_chain(&address, context.clone()),
                get_meta_aspects_from_dht_eav(&address, context.clone()),
            ] {
                match result {
                    Ok(meta_aspects) => meta_aspects.iter().for_each(|a| {
                        aspects.insert(a.clone());
                    }),
                    Err(get_meta_error) => {
                        log_error!(context, "net/handle_fetch_entry: Error getting meta aspects for entry ({:?}), error: {:?}",
                            address,
                            get_meta_error,
                        );
                    }
                }
            }
        }
        Err(get_content_error) => {
            log_debug!(context, "net/handle_fetch_entry: Could not get content aspects of requested entry ({:?}), error: {:?}",
                address,
                get_content_error,
            );
        }
    }

    aspects
}
