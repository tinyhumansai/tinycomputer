//! `Oracle`, a scripted Jev: it answers every question from the simulator's
//! state the way a well-behaved decision model would, unless a test's hook
//! answers first.

use super::*;

pub(super) type Hook = dyn Fn(&str, &Question, &Sim) -> Option<Answer> + Send + Sync;

pub(super) struct Oracle {
    pub(super) app: App,
    pub(super) hook: Box<Hook>,
    pub(super) requests: Mutex<Vec<EvaluationRequest>>,
    pub(super) fail: bool,
}

impl Evaluator for Oracle {
    fn evaluate<'a>(
        &'a self,
        request: &'a EvaluationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EvaluationResult, EvaluationFailure>> + Send + 'a>>
    {
        Box::pin(async move {
            request
                .validate()
                .expect("every request the flow builds is valid");
            self.requests.lock().unwrap().push(request.clone());
            if self.fail {
                return Err(EvaluationFailure {
                    error: Box::new(tinyinference_decisions::Error::RateLimited),
                    attempts: 1,
                    latency: Duration::ZERO,
                });
            }
            let sim = self.app.sim();
            let answers = request
                .questions
                .iter()
                .map(|(id, question)| {
                    let answer = self.answer(request, id, question, &sim);
                    (id.clone(), answer)
                })
                .collect::<BTreeMap<_, _>>();
            Ok(EvaluationResult {
                response: EvaluationResponse {
                    model: "typesafe/jev-test".to_owned(),
                    answers,
                    usage: tinyinference_decisions::Usage::default(),
                },
                request_id: None,
                attempts: 1,
                latency: Duration::from_millis(1),
            })
        })
    }
}

impl Oracle {
    /// The hooked or default answer; a negated question is answered as the
    /// inverse of its positive twin, so hooks only ever name the positive one
    /// (a condition's coverage may be hooked on its own).
    pub(super) fn answer(
        &self,
        request: &EvaluationRequest,
        id: &str,
        question: &Question,
        sim: &Sim,
    ) -> Answer {
        let near = id
            .strip_prefix("only_near_")
            .map(|index| format!("is_{index}"));
        // A coverage a test scripts outright stands; otherwise it follows its
        // condition's yes/no, as a negation does.
        if id == "coverage"
            && let Some(hooked) = (self.hook)(id, question, sim)
        {
            return hooked;
        }
        let twin = match id {
            "not_done" => Some("done"),
            "negated" | "coverage" => Some("holds"),
            "unintended" => Some("intended"),
            _ => near.as_deref(),
        };
        if let Some(twin) = twin
            && let Some(positive) = request.questions.get(twin)
            && let Answer::Noul(answer) = self.answer(request, twin, positive, sim)
        {
            return if id == "coverage" {
                level(if answer.noul >= 0.5 { 4 } else { 0 })
            } else {
                noul(1.0 - answer.noul)
            };
        }
        if let Some(hooked) = (self.hook)(id, question, sim) {
            return hooked;
        }
        // Unhooked progress follows the completion answer, so a test that
        // scripts only "done" gets a consistent pair.
        if id == "progress"
            && let Some(done) = request.questions.get("done")
            && let Answer::Noul(answer) = self.answer(request, "done", done, sim)
        {
            return level(if answer.noul >= 0.5 { 4 } else { 2 });
        }
        default_answer(id, question, sim)
    }
}

pub(super) fn text_of(question: &Question, field: &str) -> String {
    let instructions = match question {
        Question::Choice(choice) => &choice.instructions,
        Question::Score(score) => &score.instructions,
        Question::Noul(noul) => &noul.instructions,
        Question::Box(found) => &found.instructions,
    };
    instructions
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        })
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(super) fn noul(probability: f64) -> Answer {
    Answer::Noul(NoulAnswer { noul: probability })
}

/// A five-level Score sure to `probability` of its top level, the rest on
/// the level below.
pub(super) fn top_at(probability: f64) -> Answer {
    Answer::Score(ScoreAnswer {
        score: 0.0,
        legend: BTreeMap::new(),
        probabilities: (0..5)
            .map(|index| {
                let weight = match index {
                    4 => probability,
                    3 => 1.0 - probability,
                    _ => 0.0,
                };
                (index.to_string(), weight)
            })
            .collect(),
        confidence: probability,
    })
}

pub(super) fn level(position: usize) -> Answer {
    Answer::Score(ScoreAnswer {
        score: 0.0,
        legend: BTreeMap::new(),
        probabilities: (0..5)
            .map(|index| (index.to_string(), if index == position { 1.0 } else { 0.0 }))
            .collect(),
        confidence: 1.0,
    })
}

pub(super) fn pick(question: &Question, needle: &str, probability: f64) -> Answer {
    let Question::Choice(choice) = question else {
        panic!("pick needs a choice question");
    };
    let key = choice
        .criteria
        .iter()
        .find(|(key, description)| {
            *key == needle
                || description
                    .as_ref()
                    .is_some_and(|description| description.to_string().contains(needle))
        })
        .map_or_else(|| "none".to_owned(), |(key, _)| key.clone());
    let rest = (1.0 - probability) / f64::from(u32::try_from(choice.criteria.len()).unwrap());
    Answer::Choice(ChoiceAnswer {
        probabilities: choice
            .criteria
            .keys()
            .map(|option| {
                (
                    option.clone(),
                    if *option == key { probability } else { rest },
                )
            })
            .collect(),
        choice: key,
        confidence: 0.5,
    })
}

/// A Choice answer putting `weights` on the options whose key or description
/// holds each needle, the rest of the probability spread evenly over the
/// other options; the winner is the heaviest.
pub(super) fn weighted(question: &Question, weights: &[(&str, f64)]) -> Answer {
    let Question::Choice(choice) = question else {
        panic!("weighted needs a choice question");
    };
    let mut probabilities = BTreeMap::new();
    for (needle, weight) in weights {
        if let Some((key, _)) = choice.criteria.iter().find(|(key, description)| {
            !probabilities.contains_key(*key)
                && (*key == needle
                    || description
                        .as_ref()
                        .is_some_and(|description| description.to_string().contains(needle)))
        }) {
            probabilities.insert(key.clone(), *weight);
        }
    }
    let rest = choice.criteria.len().saturating_sub(probabilities.len());
    let left = (1.0 - probabilities.values().sum::<f64>()).max(0.0);
    for key in choice.criteria.keys() {
        if !probabilities.contains_key(key) {
            probabilities.insert(
                key.clone(),
                left / f64::from(u32::try_from(rest.max(1)).unwrap()),
            );
        }
    }
    let winner = probabilities
        .iter()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(key, _)| key.clone())
        .unwrap();
    Answer::Choice(ChoiceAnswer {
        choice: winner,
        probabilities,
        confidence: 0.5,
    })
}

pub(super) fn needle_for(purpose: &str) -> &'static str {
    if purpose.contains("send") {
        "Send"
    } else if purpose.contains("new email") || purpose.contains("editable fields") {
        "New Message"
    } else if purpose.contains("recipient") {
        "To"
    } else if purpose.contains("subject") {
        "Subject"
    } else if purpose.contains("body") {
        "Body"
    } else if purpose.contains("message 7") {
        "Message 7"
    } else if purpose.contains("search") {
        "Search"
    } else {
        "Archive"
    }
}

pub(super) fn default_answer(id: &str, question: &Question, sim: &Sim) -> Answer {
    match id {
        "done" => {
            let step = text_of(question, "step");
            let done = (step.contains("new email") || step.contains("editable fields"))
                && sim.compose_open;
            noul(if done { 0.95 } else { 0.05 })
        }
        "holds" => {
            let condition = text_of(question, "condition");
            let held = if condition.contains("has happened") {
                sim.sent
            } else if condition.contains("draft shows") {
                ["To", "Subject", "Body"].iter().all(|field| {
                    sim.fields
                        .get(*field)
                        .is_some_and(|value| !value.is_empty())
                })
            } else {
                sim.compose_open
            };
            noul(if held { 0.9 } else { 0.1 })
        }
        "progress" => level(2),
        "blocked" => noul(if sim.obstacle { 0.9 } else { 0.05 }),
        "move" => pick(question, "shortcut", 0.9),
        "shortcut" => pick(question, "new_item", 0.9),
        // Every action helps and no field shows an error, unless a test says.
        // A press left what the step asked for, unless a test says.
        "confirm" | "helped" | "dismiss_known" | "reflects" | "intended" => noul(0.9),
        // A contrasted finalist is the element, unless a test says.
        _ if id.starts_with("is_") => noul(0.9),
        _ if id.starts_with("known_") => noul(0.9),
        // A survey finds the step in "Region 1" and nothing distracting.
        _ if id.starts_with("relevance_") => {
            level(if text_of(question, "region").contains("region 1") {
                4
            } else {
                1
            })
        }
        _ if id.starts_with("distraction_") => noul(0.05),
        _ if id.starts_with("error_") || id == "strays" => noul(0.05),
        _ if id.starts_with("asks_") => noul(0.9),
        // Every ranked item belongs to the list picked from, unless a test says.
        _ if id.starts_with("belongs_") => noul(0.9),
        "dismiss" => pick(question, "Keep Editing", 0.9),
        "region" => pick(question, "Region 1", 0.9),
        _ if id.starts_with("slot_") => {
            pick(question, needle_for(&text_of(question, "purpose")), 0.9)
        }
        _ => pick(question, needle_for(&text_of(question, "purpose")), 0.9),
    }
}
