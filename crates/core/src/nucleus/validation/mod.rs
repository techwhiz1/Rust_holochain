use crate::{context::Context, workflows::get_entry_result::get_entry_with_meta_workflow};
use holochain_core_types::{
    chain_header::ChainHeader,
    entry::{entry_type::EntryType, Entry, EntryWithMeta},
    error::HolochainError,
    time::Timeout,
    validation::{EntryValidationData, ValidationData},
};
use holochain_persistence_api::cas::content::Address;

use std::sync::Arc;

mod agent_entry;
mod app_entry;
pub mod build_from_dht;
mod header_address;
mod link_entry;
mod provenances;
mod remove_entry;

#[derive(Clone, Debug, PartialEq, Serialize)]
/// A failed validation.
pub enum ValidationError {
    /// `Fail` means the validation function did run successfully and recognized the entry
    /// as invalid. The String parameter holds the non-zero return value of the app validation
    /// function.
    Fail(String),

    /// The entry could not get validated because known dependencies (like base and target
    /// for links) were not present yet.
    UnresolvedDependencies(Vec<Address>),

    /// A validation function for the given entry could not be found.
    /// This can happen if the entry's type is not defined in the DNA (which can only happen
    /// if somebody is sending wrong entries..) or there is no native implementation for a
    /// system entry type yet.
    NotImplemented,

    /// An error occurred that is out of the scope of validation (no state?, I/O errors..)
    Error(HolochainError),
}

/// Result of validating an entry.
/// Either Ok(()) if the entry is valid,
/// or any specialization of ValidationError.
pub type ValidationResult = Result<(), ValidationError>;

impl From<ValidationError> for HolochainError {
    fn from(ve: ValidationError) -> Self {
        match ve {
            ValidationError::Fail(reason) => HolochainError::ValidationFailed(reason),
            ValidationError::UnresolvedDependencies(_) => {
                HolochainError::ValidationFailed("Missing dependencies".to_string())
            }
            ValidationError::NotImplemented => {
                HolochainError::NotImplemented("Validation not implemented".to_string())
            }
            ValidationError::Error(e) => e,
        }
    }
}

/// enum for specifying if validation is being called for the purpose of holding data
/// or as part of authoring entries
pub enum ValidationContext {
    Authoring,
    Holding,
}

/// Main validation workflow.
/// This is the high-level validate function that wraps the whole validation process and is what should
/// be called from other workflows for validating an entry.
///
/// 1. Checks if the entry's address matches the address in given header provided by
///    the validation package.
/// 2. Validates provenances given in the header by verifying the cryptographic signatures
///    against the source agent addresses.
/// 3. Finally spawns a thread to run the type specific validation callback in a Ribosome.
///
/// All of this actually happens in the functions of the sub modules. This function is the
/// main validation entry point and, like a workflow, stays high-level.
#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub async fn validate_entry(
    entry: Entry,
    link: Option<Address>,
    mut validation_data: ValidationData,
    context: &Arc<Context>,
    validation_context: ValidationContext,
) -> ValidationResult {
    log_debug!(context, "workflow/validate_entry: {:?}", entry);
    //check_entry_type(entry.entry_type(), context)?;

    // cleanup validation data which may include too much
    if let Some(ref mut headers) = validation_data.package.source_chain_headers {
        let t = validation_data.package.chain_header.timestamp();
        headers.retain(|header| header.timestamp() < t);
    }

    header_address::validate_header_address(&entry, &validation_data.package.chain_header)?;
    provenances::validate_provenances(&validation_data)?;

    match entry.entry_type() {
        // DNA entries are not validated currently and always valid
        // TODO: Specify when DNA can be commited as an update and how to implement validation of DNA entries then.
        EntryType::Dna => Ok(()),

        EntryType::App(app_entry_type) => {
            app_entry::validate_app_entry(
                entry.clone(),
                app_entry_type.clone(),
                context,
                link,
                validation_data,
            )
            .await
        }

        EntryType::LinkAdd => {
            link_entry::validate_link_entry(
                entry.clone(),
                validation_data,
                context,
                validation_context,
            )
            .await
        }

        EntryType::LinkRemove => {
            link_entry::validate_link_entry(
                entry.clone(),
                validation_data,
                context,
                validation_context,
            )
            .await
        }

        // Deletion entries are not validated currently and always valid
        // TODO: Specify how Deletion can be commited to chain.
        EntryType::Deletion => {
            remove_entry::validate_remove_entry(entry.clone(), validation_data, context).await
        }

        // a grant should always be private, so it should always pass
        EntryType::CapTokenGrant => Ok(()),

        EntryType::AgentId => {
            agent_entry::validate_agent_entry(entry.clone(), validation_data, context).await
        }

        // chain headers always pass for now. In future this should check that the entry is valid
        EntryType::ChainHeader => Ok(()),

        _ => Err(ValidationError::NotImplemented),
    }
}

/// interprets the validation error from validate_entry. for use by the various workflows
pub fn process_validation_err(
    src: &str,
    context: Arc<Context>,
    err: ValidationError,
    addr: Address,
) -> HolochainError {
    match err {
        ValidationError::UnresolvedDependencies(dependencies) => {
            log_debug!(context, "workflow/{}: {} could not be validated due to unresolved dependencies and will be tried later. List of missing dependencies: {:?}",
                       src,
                       addr,
                       dependencies,
            );
            HolochainError::ValidationPending
        }
        ValidationError::Fail(_) => {
            log_warn!(
                context,
                "workflow/{}: Entry {} is NOT valid! Validation error: {:?}",
                src,
                addr,
                err,
            );
            HolochainError::from(err)
        }
        ValidationError::Error(HolochainError::Timeout(e)) => {
            log_warn!(
                context,
                "workflow/{}: Entry {} got timeout({}) during validation, retrying",
                src,
                addr,
                e,
            );
            HolochainError::ValidationPending
        }
        _ => {
            log_warn!(
                context,
                "workflow/{}: Entry {} Unexpected error during validation: {:?}",
                src,
                addr,
                err,
            );
            HolochainError::from(err)
        }
    }
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub fn entry_to_validation_data(
    context: Arc<Context>,
    entry: &Entry,
    maybe_link_update_delete: Option<Address>,
    validation_data: ValidationData,
) -> Result<EntryValidationData<Entry>, HolochainError> {
    match entry {
        Entry::App(_, _) => maybe_link_update_delete
            .map(|link_update| {
                get_entry_with_header(context.clone(), &link_update)
                    .map(|entry_with_header| {
                        Ok(EntryValidationData::Modify {
                            old_entry: entry_with_header.0.entry.clone(),
                            new_entry: entry.clone(),
                            old_entry_header: entry_with_header.1,
                            validation_data: validation_data.clone(),
                        })
                    })
                    .unwrap_or_else(|e| match e {
                        HolochainError::Timeout(_) => Err(e),
                        _ => Err(HolochainError::ErrorGeneric(format!(
                            "Could not find App Entry during validation, got err: {}",
                            e
                        ))),
                    })
            })
            .unwrap_or_else(|| {
                Ok(EntryValidationData::Create {
                    entry: entry.clone(),
                    validation_data: validation_data.clone(),
                })
            }),
        Entry::Deletion(deletion_entry) => {
            let deletion_address = deletion_entry.deleted_entry_address().clone();
            get_entry_with_header(context, &deletion_address)
                .map(|entry_with_header| {
                    Ok(EntryValidationData::Delete {
                        old_entry: entry_with_header.0.entry.clone(),
                        old_entry_header: entry_with_header.1,
                        validation_data: validation_data.clone(),
                    })
                })
                .unwrap_or_else(|e| match e {
                    HolochainError::Timeout(_) => Err(e),
                    _ => Err(HolochainError::ErrorGeneric(format!(
                        "Could not find Delete Entry during validation, got err: {}",
                        e
                    ))),
                })
        }
        Entry::CapTokenGrant(_) => Ok(EntryValidationData::Create {
            entry: entry.clone(),
            validation_data,
        }),
        _ => Err(HolochainError::NotImplemented(
            "Not implemented".to_string(),
        )),
    }
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
fn get_entry_with_header(
    context: Arc<Context>,
    address: &Address,
) -> Result<(EntryWithMeta, ChainHeader), HolochainError> {
    let pair = context.block_on(get_entry_with_meta_workflow(
        &context,
        address,
        &Timeout::default(),
    ))?;
    let entry_with_meta = pair.ok_or("Could not get chain")?;
    let latest_header = entry_with_meta
        .headers
        .last()
        .ok_or("Could not get last entry from chain")?;
    Ok((entry_with_meta.entry_with_meta, latest_header.clone()))
}
