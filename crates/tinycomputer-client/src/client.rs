//! Configuration-safe calls to `TinyComputer`'s contract families.

use std::{sync::Arc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use tinybus::{
    Proxy,
    module::{ConfigFingerprint, ModuleCallError, ReconfigurableModule},
};
use tokio::sync::{RwLock, RwLockReadGuard};
use tinycomputer_bus::{
    DesktopResponse,
    agent::{AgentResponse, Capabilities, names as agent_names},
    catalogue::{self, Family, Member},
    config::ComputerConfig,
    names,
};

use crate::ClientError;

/// Deadline for a bounded Jev goal plus transport and serialization overhead.
pub const RUN_GOAL_TIMEOUT: Duration = Duration::from_secs(330);

/// Deadline for task-member long polls, which the module caps at sixty seconds.
pub const TASK_CALL_TIMEOUT: Duration = Duration::from_secs(75);

/// A typed caller for the shared desktop, flow, browser, and task interface.
///
/// Calls with the same `ComputerConfig` run concurrently. A call with a
/// different configuration waits until every in-flight call has finished, so
/// no caller's configuration is replaced while its request is in flight, and
/// a long `RunGoal` or task poll does not block `CancelTask` or `Describe`.
#[derive(Clone)]
pub struct ComputerClient {
    module: Arc<ReconfigurableModule>,
    /// The configuration in-flight calls hold, shared by every clone.
    active: Arc<RwLock<Option<ConfigFingerprint>>>,
}

impl std::fmt::Debug for ComputerClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComputerClient")
            .finish_non_exhaustive()
    }
}

impl ComputerClient {
    /// Wrap a `TinyBus` configuration coordinator for one registered module.
    #[must_use]
    pub fn new(module: ReconfigurableModule) -> Self {
        Self {
            module: Arc::new(module),
            active: Arc::new(RwLock::new(None)),
        }
    }

    /// Record the configuration already applied by the module loader.
    ///
    /// This avoids a redundant reinitialization on the first call when the
    /// host has just loaded the module with this exact configuration.
    ///
    /// # Errors
    /// Returns an error if the configuration cannot be serialized.
    pub fn record_loaded_with(&self, config: &ComputerConfig) -> Result<(), serde_json::Error> {
        self.module.record_loaded_with(config)
    }

    /// Call a catalogued desktop, flow, browser, or accessibility member.
    ///
    /// The request and response remain typed by the caller's `TinyComputer` bus
    /// types. Confidential delivery is selected from the bus catalogue, and
    /// the successful result is validated by [`DesktopResponse::into_data`].
    ///
    /// # Errors
    /// Returns an error for an unknown or task-family member, module setup,
    /// transport, confidentiality, remote command, or response decoding.
    pub async fn call_desktop<Request, Response>(
        &self,
        config: &ComputerConfig,
        member: &str,
        request: Request,
    ) -> Result<Response, ClientError>
    where
        Request: Serialize + Send,
        Response: DeserializeOwned,
    {
        let entry = catalogued_member(member)?;
        match entry.family {
            Family::Task => {
                return Err(ClientError::WrongFamily {
                    member: member.to_owned(),
                    actual: entry.family,
                    expected: "desktop, flow, or browser",
                });
            }
            Family::Flow | Family::Desktop | Family::Browser => {}
        }

        let timeout = (member == names::methods::RUN_GOAL).then_some(RUN_GOAL_TIMEOUT);
        let response = self
            .call_wire::<_, DesktopResponse>(config, entry, request, timeout)
            .await?;
        response.into_data().map_err(ClientError::from)
    }

    /// Call a task member and decode its Agent envelope.
    ///
    /// `Describe` is deliberately separate: the contract serves its raw
    /// [`Capabilities`] value rather than an [`AgentResponse`].
    ///
    /// # Errors
    /// Returns an error for an unknown, non-task, or `Describe` member, module
    /// setup, transport, confidentiality, remote task, or response decoding.
    pub async fn call_task<Request, Response>(
        &self,
        config: &ComputerConfig,
        member: &str,
        request: Request,
    ) -> Result<Response, ClientError>
    where
        Request: Serialize + Send,
        Response: DeserializeOwned,
    {
        let entry = catalogued_member(member)?;
        if entry.family != Family::Task || member == agent_names::methods::DESCRIBE {
            return Err(ClientError::WrongFamily {
                member: member.to_owned(),
                actual: entry.family,
                expected: "task member other than Describe",
            });
        }

        let response = self
            .call_wire::<_, AgentResponse<Response>>(
                config,
                entry,
                request,
                Some(TASK_CALL_TIMEOUT),
            )
            .await?;
        response.into_data().map_err(ClientError::from)
    }

    /// Read capabilities through the catalogued `Describe` member.
    ///
    /// # Errors
    /// Returns an error if the module is unavailable, the bus call fails, or
    /// the reply does not decode as [`Capabilities`].
    pub async fn describe(&self, config: &ComputerConfig) -> Result<Capabilities, ClientError> {
        let entry = catalogued_member(agent_names::methods::DESCRIBE)?;
        let (_guard, proxy) = self.proxy(config, None).await?;
        self.call_entry::<(), Capabilities>(&proxy, entry, ()).await
    }

    async fn call_wire<Request, Reply>(
        &self,
        config: &ComputerConfig,
        entry: &'static Member,
        request: Request,
        timeout: Option<Duration>,
    ) -> Result<Reply, ClientError>
    where
        Request: Serialize + Send,
        Reply: DeserializeOwned,
    {
        let (_guard, proxy) = self.proxy(config, timeout).await?;
        self.call_entry(&proxy, entry, (request,)).await
    }

    async fn proxy(
        &self,
        config: &ComputerConfig,
        timeout: Option<Duration>,
    ) -> Result<(RwLockReadGuard<'_, Option<ConfigFingerprint>>, Proxy), ClientError> {
        let wanted =
            ConfigFingerprint::of(config).map_err(|_| ClientError::from(ModuleCallError::ModuleFault))?;
        let mut guard = self.active.read().await;
        if guard.as_ref() != Some(&wanted) {
            drop(guard);
            // Exclusive only while no call is in flight; then shared again so
            // later calls with this configuration run alongside.
            let mut switching = self.active.write().await;
            *switching = Some(wanted);
            guard = switching.downgrade();
        }
        let proxy = self.module.proxy(config).await?;
        let proxy = timeout.map_or(proxy.clone(), |duration| proxy.with_timeout(duration));
        Ok((guard, proxy))
    }

    async fn call_entry<Request, Reply>(
        &self,
        proxy: &Proxy,
        entry: &'static Member,
        request: Request,
    ) -> Result<Reply, ClientError>
    where
        Request: Serialize + Send,
        Reply: DeserializeOwned,
    {
        if entry.confidential {
            proxy
                .call_confidential(entry.name, request)
                .await
                .map_err(ClientError::from)
        } else {
            proxy
                .call(entry.name, request)
                .await
                .map_err(ClientError::from)
        }
    }
}

fn catalogued_member(member: &str) -> Result<&'static Member, ClientError> {
    catalogue::member(member).ok_or_else(|| ClientError::UnknownMember(member.to_owned()))
}
