//! AWS Cedar Policy Engine scenario.
//!
//! Demonstrates declarative ABAC/ReBAC policy evaluation with sub-100µs decision latency,
//! fail-closed protection against skipped `forbid` rules, and policy diagnostic explanations.

use std::str::FromStr;
use std::time::Instant;

use authkestra_policy::cedar_policy::EntityUid;
use authkestra_policy::{AuthorizationRequest, MemoryResourceLoader, PolicyEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitFragment, KitLink,
    Scenario, ScenarioContext, ScenarioOption,
};
use crate::error::ApiError;
use crate::events::Step;

pub struct PolicyScenario;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyEvaluationResult {
    pub decision: String,
    pub allowed: bool,
    pub diagnostics: Vec<String>,
    pub latency_micros: u64,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct EvaluateBody {
    principal: Option<String>,
    action: Option<String>,
    resource: Option<String>,
    context: Option<Value>,
}

const TEMPLATE_ADMIN_ONLY: &str = r#"
@id("admin-access")
permit(
    principal,
    action,
    resource
) when {
    context.is_admin == true
};

@id("block-unauthenticated")
forbid(
    principal,
    action,
    resource
) when {
    context.is_anonymous == true
};
"#;

const TEMPLATE_DEPARTMENT_ABAC: &str = r#"
@id("department-match")
permit(
    principal,
    action,
    resource
) when {
    context.department == "engineering" && context.clearance == "confidential"
};

@id("block-untrusted-network")
forbid(
    principal,
    action,
    resource
) when {
    context.untrusted_ip == true
};
"#;

const TEMPLATE_EMERGENCY_BREAK_GLASS: &str = r#"
@id("emergency-break-glass")
permit(
    principal,
    action,
    resource
) when {
    context.break_glass == true && context.justification != ""
};

@id("standard-operator")
permit(
    principal,
    action,
    resource
) when {
    context.role == "operator" && context.is_on_call == true
};
"#;

#[async_trait::async_trait]
impl Scenario for PolicyScenario {
    fn id(&self) -> &'static str {
        "policy"
    }

    fn name(&self) -> &'static str {
        "Cedar Policy Engine (ABAC)"
    }

    fn summary(&self) -> &'static str {
        "Declarative AWS Cedar authorization: fine-grained ABAC/ReBAC policies, \
         sub-100µs decision latency, fail-closed evaluation guarantees, and auditable diagnostic verdicts."
    }

    fn control(&self) -> ControlShape {
        ControlShape::SelectOne {
            options: vec![
                ScenarioOption::new("department_abac", "Department & Clearance (ABAC)"),
                ScenarioOption::new("admin_only", "Admin Role (Strict RBAC)"),
                ScenarioOption::new(
                    "emergency_break_glass",
                    "Emergency Break-Glass (Context ReBAC)",
                ),
            ],
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectOne { selected: None }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["evaluate"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        Consequences {
            routes: vec!["GET /protected/resource".to_string()],
            requirements: vec![
                "AWS Cedar policy engine evaluates requests in-process in sub-100µs.".to_string(),
                "Zero database I/O inside the engine: entity hydration is caller-supplied."
                    .to_string(),
                "Fail-closed evaluation: skipped forbid rules abort with UnreliableDecision."
                    .to_string(),
            ],
            crates: vec![
                CrateRequirement::new("authkestra-policy", &[]),
                CrateRequirement::new("authkestra-resource", &[]),
            ],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        let template_choice = match ctx.value {
            ControlValue::SelectOne { selected: Some(t) } => t.as_str(),
            _ => "department_abac",
        };

        match action {
            "evaluate" => {
                let body: EvaluateBody = serde_json::from_value(body).unwrap_or(EvaluateBody {
                    principal: None,
                    action: None,
                    resource: None,
                    context: None,
                });

                let principal_str = body
                    .principal
                    .unwrap_or_else(|| r#"User::"alice""#.to_string());
                let action_str = body
                    .action
                    .unwrap_or_else(|| r#"Action::"readDocument""#.to_string());
                let resource_str = body
                    .resource
                    .unwrap_or_else(|| r#"Document::"q3_roadmap""#.to_string());

                let principal = EntityUid::from_str(&principal_str).map_err(|e| {
                    ApiError::InvalidValue(format!("invalid principal entity UID: {e}"))
                })?;
                let action_uid = EntityUid::from_str(&action_str).map_err(|e| {
                    ApiError::InvalidValue(format!("invalid action entity UID: {e}"))
                })?;
                let resource = EntityUid::from_str(&resource_str).map_err(|e| {
                    ApiError::InvalidValue(format!("invalid resource entity UID: {e}"))
                })?;

                let req_context = body.context.unwrap_or_else(|| match template_choice {
                    "admin_only" => json!({ "is_admin": true, "is_anonymous": false }),
                    "emergency_break_glass" => {
                        json!({ "break_glass": true, "justification": "P1 Production Outage" })
                    }
                    _ => json!({ "department": "engineering", "clearance": "confidential", "untrusted_ip": false }),
                });

                let cedar_policies = match template_choice {
                    "admin_only" => TEMPLATE_ADMIN_ONLY,
                    "emergency_break_glass" => TEMPLATE_EMERGENCY_BREAK_GLASS,
                    _ => TEMPLATE_DEPARTMENT_ABAC,
                };

                let loader = MemoryResourceLoader::new();
                let engine = PolicyEngine::builder()
                    .policies(cedar_policies)
                    .loader(loader)
                    .build()
                    .map_err(|e| ApiError::Scenario(format!("engine build error: {e}")))?;

                let req = AuthorizationRequest::builder()
                    .principal(principal)
                    .action(action_uid)
                    .resource(resource)
                    .context(req_context)
                    .build()
                    .map_err(|e| ApiError::InvalidValue(e.to_string()))?;

                let start = Instant::now();
                let decision = engine.is_authorized(&req).await.map_err(|e| {
                    ApiError::Scenario(format!("Cedar authorization evaluation error: {e}"))
                })?;
                let duration = start.elapsed();
                let latency_micros = duration.as_micros() as u64;

                let allowed = decision.is_allowed();
                let decision_str = if allowed { "allow" } else { "deny" };
                let reasons: Vec<String> = decision.reasons().to_vec();

                let step_desc = if allowed {
                    format!("Cedar Policy evaluation permitted in {}µs", latency_micros)
                } else {
                    format!("Cedar Policy evaluation denied in {}µs", latency_micros)
                };

                ctx.record(
                    if allowed {
                        Step::success("policy", "Cedar evaluation permitted")
                    } else {
                        Step::rejected("policy", "Cedar evaluation denied")
                    }
                    .detail(step_desc)
                    .fact("decision", decision_str)
                    .fact("latency", format!("{latency_micros}µs"))
                    .fact(
                        "reasons",
                        if reasons.is_empty() {
                            "default-deny (no matching permit clause)".to_string()
                        } else {
                            reasons.join(", ")
                        },
                    ),
                )
                .await;

                Ok(serde_json::to_value(PolicyEvaluationResult {
                    decision: decision_str.to_string(),
                    allowed,
                    diagnostics: reasons.clone(),
                    latency_micros,
                    detail: if allowed {
                        format!("Permit decision reached via [{}] in {}µs.", reasons.join(", "), latency_micros)
                    } else if reasons.is_empty() {
                        format!("Deny decision: no permit rules matched the request context (evaluated in {}µs).", latency_micros)
                    } else {
                        format!("Deny decision: forbid clause [{}] was satisfied (evaluated in {}µs).", reasons.join(", "), latency_micros)
                    },
                })
                .expect("serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: self.id().to_string(),
                action: other.to_string(),
            }),
        }
    }

    fn kit_fragment(&self, value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        if !value.is_active() {
            return None;
        }

        Some(KitFragment {
            imports: vec![
                "use authkestra_policy::{PolicyEngine, AuthorizationRequest, parse_entity_uid};".to_string(),
            ],
            prelude: Vec::new(),
            builder_calls: vec![
                "        // Cedar Policy Engine integration
        .with_policy_engine(policy_engine)".to_string(),
            ],
            routes: vec![
                r#"        .route("/protected/resource", get(protected_handler))"#.to_string(),
            ],
            handlers: vec![
                r#"/// Handler protected by Cedar policy engine
async fn protected_handler(
    axum::extract::State(_state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "status": "permitted",
        "message": "Authorized by AWS Cedar policy engine."
    }))
}"#.to_string(),
            ],
            state_fields: Vec::new(),
            state_init: Vec::new(),
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![
                CrateRequirement::new("authkestra-policy", &[]),
                CrateRequirement::new("authkestra-resource", &[]),
            ],
            env: Vec::new(),
            notes: vec![
                "**Cedar ABAC**: Policies live outside the binary and evaluate deterministically in \
                 sub-100µs without database I/O."
                    .to_string(),
            ],
            setup: Vec::new(),
            links: vec![KitLink::docs("Policy Engine Guide", "guides/policy-engine")],
            needs_credential_store: false,
        })
    }
}
