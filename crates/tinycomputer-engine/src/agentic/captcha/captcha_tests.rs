//! Tests for the captcha solver: reading a scan, mapping pixels to the
//! viewport, and the loop against a scripted page and a scripted Jev.

use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::{ImageFormat, RgbaImage};
use tinyinference_decisions::{
    Answer, BoxAnswer, BoxEdges, ChoiceAnswer, EvaluationFailure, EvaluationRequest,
    EvaluationResponse, EvaluationResult, NoulAnswer, Usage,
};

use super::*;
use crate::agentic::{Evaluator, JevRuntime};

mod pixels_tests;
mod scan_tests;
mod solve_tests;

/// A PNG of `width` by `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    RgbaImage::new(width, height)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

/// A widget of `role` at the CSS box (`x`, `y`, `width`, `height`).
fn widget(role: CaptchaRole, x: f64, y: f64, width: f64, height: f64) -> CaptchaWidget {
    CaptchaWidget {
        vendor: CaptchaVendor::Recaptcha,
        role,
        visible: true,
        rect: Rect {
            x,
            y,
            width,
            height,
        },
    }
}

/// A scan of a 400 by 300 viewport holding `widgets`.
fn scan(widgets: Vec<CaptchaWidget>, solved: bool) -> CaptchaScan {
    CaptchaScan {
        widgets,
        solved,
        viewport: Viewport {
            width: 400.0,
            height: 300.0,
        },
    }
}

/// A page answering scans from a script (repeating the last), photographing
/// an 800 by 600 viewport (a device pixel ratio of two), and recording
/// presses.
struct ScriptedPage {
    scans: Mutex<VecDeque<Option<CaptchaScan>>>,
    last: Mutex<Option<CaptchaScan>>,
    shot: Option<Vec<u8>>,
    presses: Mutex<Vec<(f64, f64)>>,
    presses_work: bool,
}

impl ScriptedPage {
    fn new(scans: Vec<Option<CaptchaScan>>) -> Self {
        Self {
            scans: Mutex::new(scans.into()),
            last: Mutex::new(None),
            shot: Some(png(800, 600)),
            presses: Mutex::new(Vec::new()),
            presses_work: true,
        }
    }

    fn presses(&self) -> Vec<(f64, f64)> {
        self.presses.lock().unwrap().clone()
    }
}

impl CaptchaPage for ScriptedPage {
    fn scan(&self) -> CaptchaPageFuture<'_, Option<CaptchaScan>> {
        let next = self.scans.lock().unwrap().pop_front();
        let scan = match next {
            Some(scan) => {
                self.last.lock().unwrap().clone_from(&scan);
                scan
            }
            None => self.last.lock().unwrap().clone(),
        };
        Box::pin(async move { scan })
    }

    fn shoot(&self) -> CaptchaPageFuture<'_, Option<Vec<u8>>> {
        let shot = self.shot.clone();
        Box::pin(async move { shot })
    }

    fn press(&self, x: f64, y: f64) -> CaptchaPageFuture<'_, bool> {
        self.presses.lock().unwrap().push((x, y));
        let works = self.presses_work;
        Box::pin(async move { works })
    }

    fn pause(&self, _ms: u64) -> CaptchaPageFuture<'_, ()> {
        Box::pin(async {})
    }
}

type Answers = Box<dyn Fn(&EvaluationRequest) -> Option<BTreeMap<String, Answer>> + Send + Sync>;

/// A Jev that answers each request with `answers`, or fails when it gives
/// none, and keeps every request it was asked.
struct ScriptedJev {
    answers: Answers,
    asked: Arc<Mutex<Vec<EvaluationRequest>>>,
}

impl Evaluator for ScriptedJev {
    fn evaluate<'a>(
        &'a self,
        request: &'a EvaluationRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<EvaluationResult, EvaluationFailure>>
                + Send
                + 'a,
        >,
    > {
        self.asked.lock().unwrap().push(request.clone());
        let answers = (self.answers)(request);
        Box::pin(async move {
            match answers {
                Some(answers) => Ok(EvaluationResult {
                    response: EvaluationResponse {
                        model: "jev-preview".to_owned(),
                        answers,
                        usage: Usage::default(),
                    },
                    request_id: None,
                    attempts: 1,
                    latency: Duration::from_millis(1),
                }),
                None => Err(EvaluationFailure {
                    error: Box::new(tinyinference_decisions::Error::Timeout),
                    attempts: 1,
                    latency: Duration::from_millis(1),
                }),
            }
        })
    }
}

fn jev(
    answers: impl Fn(&EvaluationRequest) -> Option<BTreeMap<String, Answer>> + Send + Sync + 'static,
) -> (JevRuntime, Arc<Mutex<Vec<EvaluationRequest>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let runtime = JevRuntime {
        client: Arc::new(ScriptedJev {
            answers: Box::new(answers),
            asked: Arc::clone(&asked),
        }),
        configuration: tinycomputer_bus::JevConfiguration {
            provider: tinycomputer_bus::JevProvider::TypeSafe,
            model: "jev-preview".to_owned(),
            endpoint_url: None,
            fast: false,
        },
        #[cfg(feature = "desktop")]
        pending: Arc::new(Mutex::new(std::collections::HashMap::new())),
        journal: crate::agentic::journal::Journal::default(),
        copies: Arc::default(),
        usage: None,
    };
    (runtime, asked)
}

fn found(left: f64, top: f64, right: f64, bottom: f64, confidence: f64) -> Answer {
    Answer::Box(BoxAnswer {
        edges: BoxEdges {
            top,
            bottom,
            left,
            right,
        },
        confidence,
    })
}

fn chose(option: &str) -> Answer {
    Answer::Choice(ChoiceAnswer {
        choice: option.to_owned(),
        probabilities: BTreeMap::from([(option.to_owned(), 1.0)]),
        confidence: 1.0,
    })
}

fn yes(probability: f64) -> Answer {
    Answer::Noul(NoulAnswer { noul: probability })
}

fn near(actual: (f64, f64), expected: (f64, f64)) -> bool {
    (actual.0 - expected.0).abs() < 0.6 && (actual.1 - expected.1).abs() < 0.6
}

#[test]
fn settings_clamp_their_attempts() {
    assert_eq!(
        CaptchaSettings::default().max_attempts,
        DEFAULT_CAPTCHA_ATTEMPTS
    );
    assert_eq!(CaptchaSettings::with_attempts(0).max_attempts, 1);
    assert_eq!(
        CaptchaSettings::with_attempts(99).max_attempts,
        MAX_CAPTCHA_ATTEMPTS
    );
    assert_eq!(CaptchaSettings::with_attempts(4).max_attempts, 4);
}
