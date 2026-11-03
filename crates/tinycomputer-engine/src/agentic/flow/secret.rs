//! Host typing and redaction for opaque secret facts.

use tinycomputer_core::surface::{Candidate, Screen};

use super::{FlowRun, Halt, StepLog, backend::AgentBackend, view::target_payload};
use crate::SecretTypeRequest;

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    pub(super) fn redact_text(&self, text: &str) -> String {
        let text = self.secrets.mask(text);
        match (&self.secret_host, &self.task_id) {
            (Some(host), Some(task_id)) => host.redact(task_id, &text),
            _ => text,
        }
    }

    pub(super) fn redact_screen(&self, screen: &mut Screen) {
        screen.app = self.redact_text(&screen.app);
        screen.window = screen.window.as_deref().map(|text| self.redact_text(text));
        screen.surface = self.redact_text(&screen.surface);
        for text in &mut screen.context {
            *text = self.redact_text(text);
        }
        for candidate in &mut screen.candidates {
            self.redact_candidate(candidate);
        }
        for candidate in &mut screen.text_nodes {
            self.redact_candidate(candidate);
        }
    }

    pub(super) async fn type_handle(
        &mut self,
        log: &mut StepLog,
        name: &str,
        field: &Candidate,
    ) -> Result<(), Halt> {
        if self.actions >= self.max_actions {
            return Err(Halt::Stop(tinycomputer_bus::FlowStopReason::ActionBudget));
        }
        let Some(handle) = self.fact_handles.get(name) else {
            return Err(Halt::Failed(format!(
                "secret fact `{name}` has no value or host handle"
            )));
        };
        let (Some(host), Some(task_id)) = (&self.secret_host, &self.task_id) else {
            return Err(Halt::Failed(format!(
                "secret fact `{name}` cannot be typed without a configured host, task id, and known page URL"
            )));
        };
        let backend = self.backend.clone();
        let target = field.clone();
        let current_url =
            tokio::task::spawn_blocking(move || backend.authorize_secret_type(&target))
                .await
                .map_err(|_| Halt::Failed("the page policy check failed".to_owned()))?
                .map_err(|reason| Halt::Failed(self.redact_text(&reason)))?;
        let request = SecretTypeRequest {
            handle: handle.handle.clone(),
            mask: handle.mask.clone(),
            kind: handle.kind.clone(),
            field_role: field.role.clone(),
            field_name: field.name.clone(),
            field_ref: field.ref_id.clone(),
            current_url,
            task_id: task_id.clone(),
            actions_remaining: self.max_actions.saturating_sub(self.actions),
        };
        self.actions = self.actions.saturating_add(1);
        self.typed_last = Some(field.clone());
        let host = host.clone();
        let result = tokio::task::spawn_blocking(move || host.type_secret(&request))
            .await
            .map_err(|_| Halt::Failed("the secret host task failed".to_owned()))?;
        if let Err(reason) = result {
            return Err(Halt::Failed(self.redact_text(&reason)));
        }
        log.actions.push(tinycomputer_bus::FlowActionRecord {
            action: "host-type-secret".to_owned(),
            target: Some(target_payload(field)),
            ok: true,
            note: "typed by host".to_owned(),
        });
        Ok(())
    }

    fn redact_candidate(&self, candidate: &mut Candidate) {
        candidate.ref_id = self.redact_text(&candidate.ref_id);
        candidate.role = self.redact_text(&candidate.role);
        candidate.name = candidate.name.as_deref().map(|text| self.redact_text(text));
        candidate.description = candidate
            .description
            .as_deref()
            .map(|text| self.redact_text(text));
        candidate.value = candidate.value.take().map(|mut value| {
            redact_value(&mut value, &|text| self.redact_text(text));
            value
        });
        for text in &mut candidate.states {
            *text = self.redact_text(text);
        }
        for text in &mut candidate.available_actions {
            *text = self.redact_text(text);
        }
        for text in &mut candidate.path {
            *text = self.redact_text(text);
        }
        for child in &mut candidate.children {
            self.redact_candidate(child);
        }
    }
}

pub(super) fn redact_value(value: &mut serde_json::Value, redact: &impl Fn(&str) -> String) {
    match value {
        serde_json::Value::String(text) => *text = redact(text),
        serde_json::Value::Array(items) => {
            for item in items {
                redact_value(item, redact);
            }
        }
        serde_json::Value::Number(number) => {
            let text = number.to_string();
            let masked = redact(&text);
            if masked != text {
                *value = serde_json::Value::String(masked);
            }
        }
        serde_json::Value::Object(fields) => {
            let mut safe_fields = serde_json::Map::new();
            for (key, mut item) in std::mem::take(fields) {
                redact_value(&mut item, redact);
                let base = redact(&key);
                let mut safe_key = base.clone();
                let mut suffix = 2_u32;
                while safe_fields.contains_key(&safe_key) {
                    safe_key = format!("{base}#{suffix}");
                    suffix = suffix.saturating_add(1);
                }
                safe_fields.insert(safe_key, item);
            }
            *fields = safe_fields;
        }
        _ => {}
    }
}
