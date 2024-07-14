use crate::{
    action::{Action, ActionWrapper, ValidationKey},
    context::Context,
    instance::dispatch_action,
};
use futures::{future::Future, task::Poll};

use holochain_core_types::{
    chain_header::ChainHeader, error::HcResult, validation::ValidationPackage,
};
use snowflake::ProcessUniqueId;
use std::{pin::Pin, sync::Arc};

/// GetValidationPackage Action Creator
/// This triggers the network module to retrieve the validation package for the
/// entry given by the header.
///
/// Returns a future that resolves to Option<ValidationPackage> (or HolochainError).
/// If that is None this means that we couldn't get a validation package from the source.
#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
pub async fn get_validation_package(
    header: ChainHeader,
    context: &Arc<Context>,
) -> HcResult<Option<ValidationPackage>> {
    let entry_address = header.entry_address().clone();
    let key = ValidationKey {
        address: entry_address,
        id: snowflake::ProcessUniqueId::new().to_string(),
    };
    let action_wrapper = ActionWrapper::new(Action::GetValidationPackage((key.clone(), header)));
    dispatch_action(context.action_channel(), action_wrapper.clone());
    let id = ProcessUniqueId::new();
    GetValidationPackageFuture {
        context: context.clone(),
        key,
        id,
    }
    .await
}

/// GetValidationPackageFuture resolves to an Option<ValidationPackage>
/// which would be None if the source responded with None, indicating that it
/// is not the source.
pub struct GetValidationPackageFuture {
    context: Arc<Context>,
    key: ValidationKey,
    id: ProcessUniqueId,
}

#[holochain_tracing_macros::newrelic_autotrace(HOLOCHAIN_CORE)]
impl Future for GetValidationPackageFuture {
    type Output = HcResult<Option<ValidationPackage>>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context) -> Poll<Self::Output> {
        if let Some(err) = self
            .context
            .action_channel_error("GetValidationPackageFuture")
        {
            return Poll::Ready(Err(err));
        }

        self.context
            .register_waker(self.id.clone(), cx.waker().clone());

        if let Some(state) = self.context.try_state() {
            let state = state.network();
            if let Err(error) = state.initialized() {
                return Poll::Ready(Err(error));
            }

            match state.get_validation_package_results.get(&self.key) {
                Some(Some(result)) => {
                    dispatch_action(
                        self.context.action_channel(),
                        ActionWrapper::new(Action::ClearValidationPackageResult(self.key.clone())),
                    );
                    self.context.unregister_waker(self.id.clone());
                    Poll::Ready(result.clone())
                }
                _ => Poll::Pending,
            }
        } else {
            Poll::Pending
        }
    }
}
