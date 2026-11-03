//! In-memory `TinyBus` tests for the public `TinyComputer` client.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use tinybus::module::ReconfigurableModule;
use tinybus::{
    Connection, Error as TinyBusError, Interface, InterfaceName, MemberName, ObjectPath, Proxy,
    broker::Broker,
    module::{
        AccessFuture, FailureReporter, FailureSink, ModuleAccess, ModuleHost, ProxyFailure,
        ReconfigureFailure, ReleaseRecord,
    },
    transport::memory::MemoryBus,
};
use tinycomputer_bus::JevConfig;
use tinycomputer_bus::{
    DesktopResponse,
    accessibility::{FocusQuery, FocusedTextContext},
    agent::{AgentResponse, Capabilities, names as task_names},
    browser::{
        ClaimProfileRequest, ClaimProfileResponse, NavigateRequest, OutputChunk, OutputId,
        PageState, ProfileClaim, ReadOutputRequest, SessionId, SessionRequest,
    },
    catalogue::{self, Family},
    config::ComputerConfig,
    names,
};
use tinycomputer_client::{ClientError, ComputerClient, RUN_GOAL_TIMEOUT, TASK_CALL_TIMEOUT};

#[path = "client_tests/support/mod.rs"]
mod support;
use support::*;

#[path = "client_tests/catalogue_tests.rs"]
mod catalogue_tests;
#[path = "client_tests/fail_closed_tests.rs"]
mod fail_closed_tests;
#[path = "client_tests/linked_module_tests.rs"]
mod linked_module_tests;
#[path = "client_tests/typed_round_trip_tests.rs"]
mod typed_round_trip_tests;
